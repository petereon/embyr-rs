//! Admin-console OIDC SSO: `start` and `callback` (ADR-086, `docs/feature/admin-oidc-sso`).
//!
//! POST /admin/v1/auth/oidc/start     -> 200 `{authorization_url}` or 200 `{error:"sso_unavailable"}`
//! GET  /admin/v1/auth/oidc/callback  -> always a 302 (`/admin/` on success, `/admin/?error=sso_failed` otherwise)
//!
//! The session user comes ONLY from `admin_sso_identities` keyed `(provider_id, sub)` (audit #58):
//! this module never queries `users` and never looks anything up by e-mail. Intent `signin` is
//! implemented here; intents `invite` (slice 4) and `link` (slice 5) share the start/callback
//! plumbing and delegate the account-changing part to `oidc_sso_links`, which is the only code
//! that reads the e-mail claim or writes `users`.
//!
//! No oracle: every failure after the state row is read produces the same redirect; the granular
//! reason goes to `admin_audit_events` and a metrics label, never to the browser.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::{
    extract::{ConnectInfo, Json, RawQuery, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::{PgPool, Row};
use subtle::ConstantTimeEq;
use uuid::Uuid;

use embyr_core::admin::audit::{events, AuditEvent, AuditOutcome};
use embyr_core::admin::sso::claims::{validate_id_token, ExpectedClaims, SsoReject};
use embyr_core::admin::sso::discovery::{authorization_url, AuthorizationRequest};
use embyr_core::admin::invitation::{hash_token, is_well_formed_token};
use embyr_core::admin::sso::pkce::{code_challenge, hash_value, random_token};

use crate::adapters::admin_audit::record;
use crate::adapters::connection_secrets::ServerKeys;
use crate::adapters::encryption::decrypt_with_rotation;
use crate::adapters::sso_client::{fetch_capped, Body, FetchError, Limits, TOKEN_CAP};
use crate::admin::handlers::auth::{
    extract_cookie, issue_session, signin_throttled, verify_credentials, SessionOrigin, UserSecrets,
};
use crate::admin::handlers::oidc_sso_links::{complete_invite, complete_link};
use crate::admin::state::UserAdminState;

/// Pre-auth browser-binding cookie (ADR-086 decision 2). Lax, not Strict: the IdP returns by a
/// cross-site top-level GET, where Strict cookies are not sent.
const SSO_COOKIE: &str = "embyr_sso";
const SSO_COOKIE_PATH: &str = "/admin/v1/auth/oidc";
const STATE_TTL_SECS: u32 = 600;
/// Live (unexpired) authorization requests per provider; `start` is unauthenticated and writes.
const MAX_LIVE_REQUESTS_PER_PROVIDER: i64 = 200;
const FAILED_LOCATION: &str = "/admin/?error=sso_failed";
const SUCCESS_LOCATION: &str = "/admin/";
/// A successful self-link keeps the user's own session; the SPA shows a confirmation for this.
const LINKED_LOCATION: &str = "/admin/?sso_linked=1";

fn set_sso_cookie(value: &str) -> String {
    format!("{SSO_COOKIE}={value}; HttpOnly; Secure; SameSite=Lax; Path={SSO_COOKIE_PATH}; Max-Age={STATE_TTL_SECS}")
}

fn clear_sso_cookie() -> String {
    format!("{SSO_COOKIE}=; HttpOnly; Secure; SameSite=Lax; Path={SSO_COOKIE_PATH}; Max-Age=0")
}

/// Rate-limit key shared by `start` and `callback` (ADR-086 decision 9). Fits the VARCHAR(45)
/// column of the Postgres-backed limiter (`sso:` + at most 39 characters of IPv6).
fn rate_key(ip: IpAddr) -> String {
    format!("sso:{ip}")
}

/// `{EMBYR_PUBLIC_BASE_URL}/admin/v1/auth/oidc/callback`, never derived from a request. `None`
/// (SSO unavailable) when the variable is unset, or is not https outside the dev flag.
fn redirect_uri(state: &UserAdminState) -> Option<String> {
    let base = state.stripe_gateway.public_base_url()?;
    if !base.starts_with("https://") && !crate::adapters::dsn_policy::allow_private() {
        return None;
    }
    Some(format!("{base}/admin/v1/auth/oidc/callback"))
}

struct Provider {
    account_id: Uuid,
    issuer: String,
    client_id: String,
    secret_enc: Vec<u8>,
}

/// An enabled provider of a live account, or `None` (unknown, disabled and deleted-account are
/// indistinguishable to every caller).
async fn load_provider(pool: &PgPool, id: Uuid) -> Result<Option<Provider>, sqlx::Error> {
    sqlx::query(
        "SELECT p.account_id, p.issuer, p.client_id, p.client_secret_enc \
         FROM oidc_providers p JOIN accounts a ON a.id = p.account_id \
         WHERE p.id = $1 AND p.enabled AND a.deleted_at IS NULL",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    .map(|r| {
        Ok(Provider {
            account_id: r.try_get("account_id")?,
            issuer: r.try_get("issuer")?,
            client_id: r.try_get("client_id")?,
            secret_enc: r.try_get("client_secret_enc")?,
        })
    })
    .transpose()
}

/// Decrypt a provider client secret. The flag is true when only the previous key opened it, so the
/// caller re-encrypts under the current key (lazy rotation, as `connection_secrets` does for DSNs).
fn open_secret(keys: &ServerKeys, enc: &[u8]) -> Option<(String, bool)> {
    let (plain, from_previous) = match decrypt_with_rotation(&keys.current, None, enc) {
        Ok(p) => (p, false),
        Err(_) => (decrypt_with_rotation(&keys.current, keys.previous.as_ref(), enc).ok()?, true),
    };
    String::from_utf8(plain).ok().map(|s| (s, from_previous))
}

fn db_err(context: &'static str) -> impl Fn(sqlx::Error) -> &'static str {
    move |e| {
        tracing::error!("oidc_sso: {context}: {e}");
        "db_error"
    }
}

// ── start ─────────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct StartRequest {
    provider_id: String,
    /// `signin` (default), `invite` (+ `invite_token`) or `link` (+ `password`, `totp_code`, and a
    /// session cookie). No `Debug`: the body carries a password and an invitation token.
    #[serde(default)]
    intent: Option<String>,
    #[serde(default)]
    invite_token: Option<String>,
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    totp_code: Option<String>,
}

/// What the authorization request is for; resolved (and authorised) before any state row exists.
enum Intent {
    SignIn,
    Invite { invitation_id: Uuid, account_id: Uuid },
    Link { user_id: Uuid, account_id: Uuid },
}

impl Intent {
    fn label(&self) -> &'static str {
        match self {
            Self::SignIn => "signin",
            Self::Invite { .. } => "invite",
            Self::Link { .. } => "link",
        }
    }
    /// The account the provider must belong to (`None` for sign-in: the provider decides).
    fn account_id(&self) -> Option<Uuid> {
        match self {
            Self::SignIn => None,
            Self::Invite { account_id, .. } | Self::Link { account_id, .. } => Some(*account_id),
        }
    }
    fn invitation_id(&self) -> Option<Uuid> {
        match self {
            Self::Invite { invitation_id, .. } => Some(*invitation_id),
            _ => None,
        }
    }
    fn user_id(&self) -> Option<Uuid> {
        match self {
            Self::Link { user_id, .. } => Some(*user_id),
            _ => None,
        }
    }
}

enum StartError {
    /// Same body for an unknown, disabled or deleted-account provider, an unset public URL and a
    /// failed discovery: no oracle. The label goes to the log only.
    Unavailable(&'static str),
    /// Too many live requests for this provider.
    Capped,
    /// Self-link for a provider the user is already linked to (authenticated caller, so no oracle).
    AlreadyLinked,
    Internal,
}

fn unavailable() -> Response {
    Json(json!({ "error": "sso_unavailable" })).into_response()
}

/// POST /admin/v1/auth/oidc/start
pub async fn oidc_start(
    State(state): State<UserAdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<StartRequest>,
) -> Response {
    let ip = state.signin_rate_limiter.source_ip(peer, &headers);
    if let Err(retry_after_ms) = state.signin_rate_limiter.check(&rate_key(ip)).await {
        return signin_throttled(retry_after_ms);
    }
    let intent = match body.intent.as_deref() {
        None | Some("signin") => Intent::SignIn,
        Some("invite") => match resolve_invite(&state, body.invite_token.as_deref()).await {
            Ok(i) => i,
            Err(r) => return r,
        },
        Some("link") => match resolve_link(&state, &headers, &body).await {
            Ok(i) => i,
            Err(r) => return r,
        },
        Some(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    match start_flow(&state, &body.provider_id, &intent).await {
        Ok((authorization_url, cookie)) => {
            let mut headers = HeaderMap::new();
            if let Ok(v) = HeaderValue::from_str(&set_sso_cookie(&cookie)) {
                headers.insert(header::SET_COOKIE, v);
            }
            headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            (StatusCode::OK, headers, Json(json!({ "authorization_url": authorization_url }))).into_response()
        }
        Err(StartError::Unavailable(why)) => {
            tracing::debug!(reason = why, "oidc start: unavailable");
            unavailable()
        }
        Err(StartError::Capped) => signin_throttled(60_000),
        Err(StartError::AlreadyLinked) => {
            (StatusCode::CONFLICT, Json(json!({ "error": "already_linked" }))).into_response()
        }
        Err(StartError::Internal) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

fn server_error(context: &'static str) -> impl Fn(sqlx::Error) -> Response {
    move |e| {
        tracing::error!("oidc_sso: start: {context}: {e}");
        StatusCode::INTERNAL_SERVER_ERROR.into_response()
    }
}

/// Intent `invite`: the invitation token proves the invitee holds the mailbox the invitation went
/// to. Unknown, expired, revoked, used, password-method and Owner invitations are all the same
/// `sso_unavailable` (no oracle on invitation state).
#[allow(clippy::result_large_err)]
async fn resolve_invite(state: &UserAdminState, token: Option<&str>) -> Result<Intent, Response> {
    let token = token.filter(|t| is_well_formed_token(t)).ok_or_else(unavailable)?;
    let row = sqlx::query(
        "SELECT id, account_id FROM invitations \
         WHERE token_hash = $1 AND accepted_at IS NULL AND revoked_at IS NULL AND expires_at > now() \
           AND auth_method = 'sso' AND role <> 'Owner'",
    )
    .bind(hash_token(token))
    .fetch_optional(state.system_db.pool())
    .await
    .map_err(server_error("load invitation"))?
    .ok_or_else(unavailable)?;
    Ok(Intent::Invite {
        invitation_id: row.try_get("id").map_err(server_error("read invitation"))?,
        account_id: row.try_get("account_id").map_err(server_error("read invitation"))?,
    })
}

/// Intent `link`: a valid session cookie AND the password + TOTP re-entered (`verify_credentials`,
/// the same check account deletion uses), so a stolen session cannot plant a second login. An
/// SSO-only user has no password and always fails here (decision O4).
#[allow(clippy::result_large_err)]
async fn resolve_link(state: &UserAdminState, headers: &HeaderMap, body: &StartRequest) -> Result<Intent, Response> {
    let pool = state.system_db.pool();
    let cookie = extract_cookie(headers, "embyr_session").ok_or_else(|| StatusCode::UNAUTHORIZED.into_response())?;
    let row = sqlx::query(
        "SELECT s.user_id, s.account_id FROM sessions s \
         JOIN account_members am ON am.user_id = s.user_id AND am.account_id = s.account_id \
         WHERE s.token_hash = $1 AND s.expires_at > now()",
    )
    .bind(hash_value(&cookie))
    .fetch_optional(pool)
    .await
    .map_err(server_error("load session"))?
    .ok_or_else(|| StatusCode::UNAUTHORIZED.into_response())?;
    let user_id: Uuid = row.try_get("user_id").map_err(server_error("read session"))?;
    let account_id: Uuid = row.try_get("account_id").map_err(server_error("read session"))?;
    let user = UserSecrets::fetch_by_id(pool, user_id)
        .await
        .map_err(server_error("load user"))?
        .ok_or_else(|| StatusCode::UNAUTHORIZED.into_response())?;
    verify_credentials(state, &user, body.password.as_deref().unwrap_or(""), body.totp_code.as_deref(), None).await?;
    Ok(Intent::Link { user_id, account_id })
}

/// Returns the IdP authorization URL and the raw `embyr_sso` cookie value.
async fn start_flow(state: &UserAdminState, raw_provider_id: &str, intent: &Intent) -> Result<(String, String), StartError> {
    use StartError::*;
    let pool = state.system_db.pool();
    let redirect_uri = redirect_uri(state).ok_or(Unavailable("public_base_url_unset"))?;
    let provider_id = Uuid::parse_str(raw_provider_id).map_err(|_| Unavailable("provider_id_malformed"))?;
    let provider = load_provider(pool, provider_id)
        .await
        .map_err(|e| {
            tracing::error!("oidc_sso: start: load provider: {e}");
            Internal
        })?
        .ok_or(Unavailable("provider_unavailable"))?;
    // An invitation or session of one account can never start a flow through another account's provider.
    if intent.account_id().is_some_and(|a| a != provider.account_id) {
        return Err(Unavailable("provider_account_mismatch"));
    }
    if let Intent::Link { user_id, .. } = intent {
        let linked: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM admin_sso_identities WHERE provider_id = $1 AND user_id = $2)",
        )
        .bind(provider_id)
        .bind(user_id)
        .fetch_one(pool)
        .await
        .map_err(|e| {
            tracing::error!("oidc_sso: start: link check: {e}");
            Internal
        })?;
        if linked {
            return Err(AlreadyLinked);
        }
    }
    let doc = state
        .sso_client
        .discovery(provider_id, &provider.issuer)
        .await
        .map_err(|e| Unavailable(e.reason()))?;

    let (flow_state, nonce, verifier, cookie) =
        (random_token(), random_token(), random_token(), random_token());
    let verifier_enc = state.server_keys.encrypt(&verifier);

    // Bounded opportunistic purge of this provider's expired rows (the sweeper does the rest).
    let _ = sqlx::query(
        "DELETE FROM sso_auth_requests WHERE ctid IN \
         (SELECT ctid FROM sso_auth_requests WHERE provider_id = $1 AND expires_at < now() LIMIT 100)",
    )
    .bind(provider_id)
    .execute(pool)
    .await;

    // One statement: insert only while under the per-provider cap of live rows.
    let inserted = sqlx::query(
        "INSERT INTO sso_auth_requests \
         (state_hash, provider_id, intent, invitation_id, user_id, nonce, code_verifier_enc, browser_binding, expires_at) \
         SELECT $1, $2, $8, $9, $10, $3, $4, $5, now() + make_interval(secs => $6) \
         WHERE (SELECT count(*) FROM sso_auth_requests WHERE provider_id = $2 AND expires_at > now()) < $7",
    )
    .bind(hash_value(&flow_state))
    .bind(provider_id)
    .bind(&nonce)
    .bind(&verifier_enc)
    .bind(hash_value(&cookie))
    .bind(f64::from(STATE_TTL_SECS))
    .bind(MAX_LIVE_REQUESTS_PER_PROVIDER)
    .bind(intent.label())
    .bind(intent.invitation_id())
    .bind(intent.user_id())
    .execute(pool)
    .await
    .map_err(|e| {
        tracing::error!("oidc_sso: start: insert state: {e}");
        Internal
    })?
    .rows_affected();
    if inserted == 0 {
        return Err(Capped);
    }

    let url = authorization_url(
        &doc.authorization_endpoint,
        &AuthorizationRequest {
            client_id: &provider.client_id,
            redirect_uri: &redirect_uri,
            state: &flow_state,
            nonce: &nonce,
            code_challenge: &code_challenge(&verifier),
        },
    );
    Ok((url, cookie))
}

// ── callback ──────────────────────────────────────────────────────────────────

/// What a failure knows about itself, for the audit row. Ids are `None` until they are resolved.
#[derive(Default)]
struct Ctx {
    provider_id: Option<Uuid>,
    account_id: Option<Uuid>,
    /// `None` until the state row is read; failures before that are audited as sign-in failures.
    intent: Option<&'static str>,
}

struct Signed {
    /// `None` for a self-link: the user keeps the session they already have.
    cookie: Option<String>,
    user_id: Uuid,
    account_id: Uuid,
    provider_id: Uuid,
    intent: &'static str,
}

fn redirect(location: &'static str, session_cookie: Option<String>) -> Response {
    let mut h = HeaderMap::new();
    h.insert(header::LOCATION, HeaderValue::from_static(location));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    if let Ok(v) = HeaderValue::from_str(&clear_sso_cookie()) {
        h.append(header::SET_COOKIE, v);
    }
    if let Some(c) = session_cookie.and_then(|c| HeaderValue::from_str(&c).ok()) {
        h.append(header::SET_COOKIE, c);
    }
    (StatusCode::FOUND, h).into_response()
}

fn token_reason(e: &FetchError) -> &'static str {
    match e {
        FetchError::Policy(r) => r.reason(),
        _ => "token_endpoint_error",
    }
}

/// GET /admin/v1/auth/oidc/callback
///
/// Query parameters are read leniently (a malformed query is just another failure, never a 400);
/// an `id_token` parameter is never read.
pub async fn oidc_callback(
    State(state): State<UserAdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    RawQuery(query): RawQuery,
) -> Response {
    let ip = state.signin_rate_limiter.source_ip(peer, &headers);
    let params: HashMap<String, String> =
        form_urlencoded::parse(query.unwrap_or_default().as_bytes()).into_owned().collect();
    let mut ctx = Ctx::default();
    let pool = state.system_db.pool();

    match callback_flow(&state, ip, &headers, &params, &mut ctx).await {
        Ok(signed) => {
            metrics::counter!("embyr_sso_signin_total", "result" => "ok").increment(1);
            let names: &[&str] = match signed.intent {
                "invite" => &[events::SSO_INVITE_ACCEPTED, events::SSO_LINK_CREATED],
                "link" => &[events::SSO_LINK_CREATED],
                _ => &[events::SSO_SIGNIN],
            };
            for name in names {
                let mut ev = AuditEvent::new(name, AuditOutcome::Ok);
                ev.account_id = Some(signed.account_id);
                ev.actor_user_id = Some(signed.user_id);
                ev.provider_id = Some(signed.provider_id);
                ev.source_ip = Some(ip);
                record(pool, &ev).await;
            }
            redirect(if signed.intent == "link" { LINKED_LOCATION } else { SUCCESS_LOCATION }, signed.cookie)
        }
        Err(reason) => {
            metrics::counter!("embyr_sso_signin_total", "result" => reason).increment(1);
            // Throttled and state-less requests are not audited: an unauthenticated flood must not
            // become a write amplifier (the limiter bounds the rest).
            if !matches!(reason, "rate_limited" | "state_missing") {
                let name = match ctx.intent {
                    Some("invite") => events::SSO_INVITE_ACCEPTED,
                    Some("link") => events::SSO_LINK_CREATED,
                    _ => events::SSO_SIGNIN,
                };
                let mut ev = AuditEvent::new(name, AuditOutcome::Failed);
                ev.account_id = ctx.account_id;
                ev.provider_id = ctx.provider_id;
                ev.reason = Some(reason);
                ev.source_ip = Some(ip);
                record(pool, &ev).await;
            }
            redirect(FAILED_LOCATION, None)
        }
    }
}

/// The whole sign-in, as a sequence of checks that each end in a static reason label.
async fn callback_flow(
    state: &UserAdminState,
    ip: IpAddr,
    headers: &HeaderMap,
    params: &HashMap<String, String>,
    ctx: &mut Ctx,
) -> Result<Signed, &'static str> {
    let pool = state.system_db.pool();

    if state.signin_rate_limiter.check(&rate_key(ip)).await.is_err() {
        return Err("rate_limited");
    }

    // Single use on EVERY outcome: the row is deleted before anything else is checked (AC-SSO-03).
    let flow_state = params.get("state").filter(|s| !s.is_empty() && s.len() <= 256).ok_or("state_missing")?;
    let row = sqlx::query(
        "DELETE FROM sso_auth_requests WHERE state_hash = $1 AND expires_at > now() \
         RETURNING provider_id, intent, invitation_id, user_id, nonce, code_verifier_enc, browser_binding",
    )
    .bind(hash_value(flow_state))
    .fetch_optional(pool)
    .await
    .map_err(db_err("consume state"))?
    .ok_or("state_unknown_or_expired")?;
    let provider_id: Uuid = row.try_get("provider_id").map_err(db_err("read state"))?;
    let intent: String = row.try_get("intent").map_err(db_err("read state"))?;
    let invitation_id: Option<Uuid> = row.try_get("invitation_id").map_err(db_err("read state"))?;
    let state_user_id: Option<Uuid> = row.try_get("user_id").map_err(db_err("read state"))?;
    let nonce: String = row.try_get("nonce").map_err(db_err("read state"))?;
    let verifier_enc: Vec<u8> = row.try_get("code_verifier_enc").map_err(db_err("read state"))?;
    let binding: Vec<u8> = row.try_get("browser_binding").map_err(db_err("read state"))?;
    ctx.provider_id = Some(provider_id);
    let intent: &'static str = match intent.as_str() {
        "signin" => "signin",
        "invite" => "invite",
        "link" => "link",
        _ => return Err("intent_unsupported"),
    };
    ctx.intent = Some(intent); // from here on a failure is audited under the intent's own event

    // Browser binding: the cookie `start` set must come back with the same browser.
    let cookie = extract_cookie(headers, SSO_COOKIE).ok_or("cookie_missing")?;
    if !bool::from(hash_value(&cookie).as_slice().ct_eq(binding.as_slice())) {
        return Err("cookie_mismatch");
    }
    if params.contains_key("error") {
        return Err("idp_error");
    }
    let code = params.get("code").filter(|c| !c.is_empty() && c.len() <= 2048).ok_or("code_missing")?;

    let provider = load_provider(pool, provider_id)
        .await
        .map_err(db_err("load provider"))?
        .ok_or("provider_unavailable")?;
    ctx.account_id = Some(provider.account_id);
    let (secret, from_previous) =
        open_secret(&state.server_keys, &provider.secret_enc).ok_or("secret_undecryptable")?;
    if from_previous {
        let _ = sqlx::query("UPDATE oidc_providers SET client_secret_enc = $1 WHERE id = $2 AND client_secret_enc = $3")
            .bind(state.server_keys.encrypt(&secret))
            .bind(provider_id)
            .bind(&provider.secret_enc)
            .execute(pool)
            .await;
    }
    let doc = state
        .sso_client
        .discovery(provider_id, &provider.issuer)
        .await
        .map_err(|e| e.reason())?;
    let verifier = state.server_keys.decrypt(&verifier_enc).ok_or("state_corrupt")?;
    let redirect_uri = redirect_uri(state).ok_or("public_base_url_unset")?;

    // Code exchange, server side, with the PKCE verifier and the client secret.
    let basic = doc.uses_basic_auth();
    let mut fields = vec![
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("redirect_uri", redirect_uri.as_str()),
        ("code_verifier", verifier.as_str()),
    ];
    if !basic {
        fields.push(("client_id", provider.client_id.as_str()));
        fields.push(("client_secret", secret.as_str()));
    }
    let bytes = fetch_capped(
        &doc.token_endpoint,
        Body::Form { basic: basic.then_some((provider.client_id.as_str(), secret.as_str())), fields: &fields },
        Limits::capped(TOKEN_CAP),
    )
    .await
    .map_err(|e| token_reason(&e))?;
    let token_response: Value = serde_json::from_slice(&bytes).map_err(|_| "token_response_malformed")?;
    let id_token = token_response.get("id_token").and_then(Value::as_str).ok_or("id_token_missing")?;

    // ID-token validation against the provider's own JWKS; one forced refetch on an unknown kid.
    let expected = ExpectedClaims {
        issuer: &provider.issuer,
        client_id: &provider.client_id,
        nonce: &nonce,
        now_unix: chrono::Utc::now().timestamp(),
    };
    let jwks = state.sso_client.jwks(provider_id, &doc, false).await.map_err(|e| e.reason())?;
    let verified = match validate_id_token(id_token, &jwks, &expected) {
        Err(SsoReject::UnknownKey) => {
            let fresh = state.sso_client.jwks(provider_id, &doc, true).await.map_err(|e| e.reason())?;
            if Arc::ptr_eq(&fresh, &jwks) {
                Err(SsoReject::UnknownKey)
            } else {
                validate_id_token(id_token, &fresh, &expected)
            }
        }
        other => other,
    }
    .map_err(|r| r.reason())?;

    match intent {
        "signin" => sign_in_linked(pool, &provider, provider_id, &verified.subject).await,
        "invite" => {
            // One transaction: user, member, link, accepted invitation and the session.
            let invitation_id = invitation_id.ok_or("state_corrupt")?;
            let mut tx = pool.begin().await.map_err(db_err("begin"))?;
            let user_id =
                complete_invite(&mut tx, provider_id, provider.account_id, invitation_id, &verified).await?;
            let session = issue_session(&mut *tx, user_id, provider.account_id, SessionOrigin::Sso { provider_id })
                .await
                .map_err(db_err("issue session"))?;
            tx.commit().await.map_err(db_err("commit"))?;
            Ok(Signed {
                cookie: Some(session.set_cookie()),
                user_id,
                account_id: provider.account_id,
                provider_id,
                intent,
            })
        }
        _ => {
            let user_id = state_user_id.ok_or("state_corrupt")?;
            let mut tx = pool.begin().await.map_err(db_err("begin"))?;
            complete_link(&mut tx, provider_id, provider.account_id, user_id, &verified.subject).await?;
            tx.commit().await.map_err(db_err("commit"))?;
            Ok(Signed { cookie: None, user_id, account_id: provider.account_id, provider_id, intent })
        }
    }
}

/// Intent `signin`. Identity: (provider, sub) -> user, nothing else (never by e-mail). The LEFT JOIN
/// only tells "no link" from "link whose user left the account" apart for the audit row.
async fn sign_in_linked(
    pool: &PgPool,
    provider: &Provider,
    provider_id: Uuid,
    subject: &str,
) -> Result<Signed, &'static str> {
    let link = sqlx::query(
        "SELECT i.user_id, i.account_id, (am.user_id IS NOT NULL) AS is_member \
         FROM admin_sso_identities i \
         LEFT JOIN account_members am ON am.user_id = i.user_id AND am.account_id = i.account_id \
         WHERE i.provider_id = $1 AND i.subject = $2",
    )
    .bind(provider_id)
    .bind(subject)
    .fetch_optional(pool)
    .await
    .map_err(db_err("resolve identity"))?
    .ok_or("identity_unlinked")?;
    let user_id: Uuid = link.try_get("user_id").map_err(db_err("read identity"))?;
    let account_id: Uuid = link.try_get("account_id").map_err(db_err("read identity"))?;
    let is_member: bool = link.try_get("is_member").map_err(db_err("read identity"))?;
    if !is_member {
        return Err("member_removed");
    }
    if account_id != provider.account_id {
        // Unreachable under the composite foreign keys; refuse rather than trust.
        return Err("account_mismatch");
    }

    let mut tx = pool.begin().await.map_err(db_err("begin"))?;
    sqlx::query("UPDATE admin_sso_identities SET last_login_at = now() WHERE provider_id = $1 AND subject = $2")
        .bind(provider_id)
        .bind(subject)
        .execute(&mut *tx)
        .await
        .map_err(db_err("touch identity"))?;
    let session = issue_session(&mut *tx, user_id, account_id, SessionOrigin::Sso { provider_id })
        .await
        .map_err(db_err("issue session"))?;
    tx.commit().await.map_err(db_err("commit"))?;

    Ok(Signed { cookie: Some(session.set_cookie()), user_id, account_id, provider_id, intent: "signin" })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(prev: Option<[u8; 32]>) -> ServerKeys {
        ServerKeys { current: [1u8; 32], previous: prev }
    }

    #[test]
    fn cookie_attributes_match_the_design() {
        assert_eq!(
            set_sso_cookie("abc"),
            "embyr_sso=abc; HttpOnly; Secure; SameSite=Lax; Path=/admin/v1/auth/oidc; Max-Age=600"
        );
        assert!(clear_sso_cookie().ends_with("Max-Age=0"));
    }

    #[test]
    fn rate_key_fits_the_limiter_column_for_any_address() {
        for ip in ["255.255.255.255", "2001:0db8:85a3:0000:0000:8a2e:0370:7334", "::ffff:255.255.255.255"] {
            assert!(rate_key(ip.parse().unwrap()).len() <= 45, "{ip}");
        }
    }

    #[test]
    fn secret_opens_under_current_key_without_rotation() {
        let k = keys(None);
        let enc = k.encrypt("s3cret");
        assert_eq!(open_secret(&k, &enc), Some(("s3cret".into(), false)));
    }

    #[test]
    fn secret_encrypted_under_the_previous_key_is_flagged_for_re_encryption() {
        let old = ServerKeys { current: [9u8; 32], previous: None };
        let enc = old.encrypt("s3cret");
        assert_eq!(open_secret(&keys(Some([9u8; 32])), &enc), Some(("s3cret".into(), true)));
        // no rotation window configured: unreadable
        assert_eq!(open_secret(&keys(None), &enc), None);
        // wrong previous key: unreadable
        assert_eq!(open_secret(&keys(Some([7u8; 32])), &enc), None);
    }

    #[test]
    fn garbage_secret_is_unreadable() {
        assert_eq!(open_secret(&keys(None), b"\x00"), None);
        assert_eq!(open_secret(&keys(None), &[0u8; 64]), None);
    }

    #[test]
    fn failure_reasons_are_audit_labels() {
        use embyr_core::admin::audit::is_label;
        for r in [
            "rate_limited", "state_missing", "state_unknown_or_expired", "cookie_missing", "cookie_mismatch",
            "intent_unsupported", "idp_error", "code_missing", "provider_unavailable", "secret_undecryptable",
            "state_corrupt", "public_base_url_unset", "token_endpoint_error", "token_response_malformed",
            "id_token_missing", "identity_unlinked", "member_removed", "account_mismatch", "db_error",
            "invitation_unavailable", "owner_invitation", "email_taken", "link_conflict",
        ] {
            assert!(is_label(r), "{r}");
        }
    }

    /// AC-SSO-09 / audit #58, structurally: this module never reads `users` and never looks anything
    /// up by e-mail, so the only way to a session user is the `(provider_id, sub)` link row.
    #[test]
    fn callback_module_never_queries_users_or_email() {
        let src = include_str!("oidc_sso.rs");
        let code = src.split("#[cfg(test)]").next().unwrap();
        for forbidden in ["FROM users", "JOIN users", "INTO users", "UPDATE users", "email_verified", ".email"] {
            let hits = code.matches(forbidden).count();
            assert_eq!(hits, 0, "oidc_sso.rs must not contain {forbidden:?}");
        }
        // every SELECT that resolves identity starts from the link table
        assert!(code.contains("FROM admin_sso_identities i"));
    }
}
