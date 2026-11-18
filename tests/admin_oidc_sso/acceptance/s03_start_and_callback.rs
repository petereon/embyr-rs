// @driving_port @real-io
//! admin-oidc-sso slice 3 (ADR-086): `POST /auth/oidc/start`, `GET /auth/oidc/callback` (intent
//! `signin`), provider create/enable validation and the provider secret route, against the real
//! router and the in-process mock IdP (`tests/common/mock_oidc_idp.rs`).
//!
//! CI only (testcontainers Postgres). Compile locally with
//! `cargo test -p embyr-server --test admin_oidc_sso_s03_start_callback --no-run`.
//!
//! ACs: AC-SSO-01..09, 10 (sign-in side), 17, 18 (audit), 19, plus audit #58 regression coverage.
//! The SSRF refusal with the dev flag OFF lives in `s03_ssrf_policy.rs` (the flag is process-wide).

#[path = "../../admin_api_v2/common/mod.rs"]
mod common;
#[path = "../../common/mock_oidc_idp.rs"]
mod mock_oidc_idp;

use std::sync::Arc;

use common::{totp_code_for_secret, AdminTestContext};
use embyr_server::middleware::signin_rate_limit::SigninRateLimiter;
use mock_oidc_idp::{MockIdp, NonceMode, Sig};
use serde_json::{json, Value};
use uuid::Uuid;

// ── helpers ──────────────────────────────────────────────────────────────────

struct Sso {
    ctx: AdminTestContext,
    idp: MockIdp,
    provider: Uuid,
}

fn uuid(s: &str) -> Uuid {
    Uuid::parse_str(s).unwrap()
}

fn limiter() -> Arc<SigninRateLimiter> {
    SigninRateLimiter::new(150.0, 10.0 / 60.0)
}

/// Encrypted exactly as the server encrypts a provider secret (the test key is `[0u8; 32]`).
fn secret_enc(secret: &str) -> Vec<u8> {
    embyr_server::adapters::encryption::encrypt(&[0u8; 32], secret.as_bytes()).unwrap()
}

/// Insert a provider row directly (bypasses create-time validation).
async fn insert_provider(ctx: &AdminTestContext, account: Uuid, issuer: &str, client_id: &str, enabled: bool) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO oidc_providers (account_id, issuer, client_id, client_secret_enc, enabled) \
         VALUES ($1, $2, $3, $4, $5) RETURNING id",
    )
    .bind(account)
    .bind(issuer)
    .bind(client_id)
    .bind(secret_enc("mock-secret"))
    .bind(enabled)
    .fetch_one(&ctx.pool)
    .await
    .expect("insert provider")
}

async fn setup_with(limiter: Arc<SigninRateLimiter>) -> Sso {
    let ctx = AdminTestContext::with_sso(limiter).await;
    let idp = MockIdp::start().await;
    let provider = insert_provider(&ctx, uuid(&ctx.account_id), &idp.issuer(), &idp.client_id, true).await;
    Sso { ctx, idp, provider }
}

async fn setup() -> Sso {
    setup_with(limiter()).await
}

async fn link(s: &Sso, user: &str, subject: &str) {
    sqlx::query(
        "INSERT INTO admin_sso_identities (provider_id, subject, account_id, user_id) VALUES ($1, $2, $3, $4)",
    )
    .bind(s.provider)
    .bind(subject)
    .bind(uuid(&s.ctx.account_id))
    .bind(uuid(user))
    .execute(&s.ctx.pool)
    .await
    .expect("insert identity");
}

fn no_redirect() -> reqwest::Client {
    reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap()
}

fn set_cookies(resp: &reqwest::Response) -> Vec<String> {
    resp.headers().get_all("set-cookie").iter().map(|v| v.to_str().unwrap().to_string()).collect()
}

struct Started {
    authorization_url: String,
    /// `embyr_sso=<value>` as a browser would send it back.
    cookie: String,
    set_cookie: String,
}

async fn start_raw(ctx: &AdminTestContext, provider: &str) -> reqwest::Response {
    ctx.client
        .post(ctx.url("/admin/v1/auth/oidc/start"))
        .json(&json!({ "provider_id": provider }))
        .send()
        .await
        .expect("start request")
}

async fn start_ok(s: &Sso) -> Started {
    let resp = start_raw(&s.ctx, &s.provider.to_string()).await;
    assert_eq!(resp.status(), 200);
    let set_cookie = set_cookies(&resp).into_iter().find(|c| c.starts_with("embyr_sso=")).expect("embyr_sso cookie");
    let body: Value = resp.json().await.unwrap();
    Started {
        authorization_url: body["authorization_url"].as_str().expect("authorization_url").to_string(),
        cookie: set_cookie.split(';').next().unwrap().to_string(),
        set_cookie,
    }
}

async fn callback_at(location: &str, cookie: Option<&str>) -> reqwest::Response {
    let mut req = no_redirect().get(location);
    if let Some(c) = cookie {
        req = req.header("Cookie", c);
    }
    req.send().await.expect("callback request")
}

/// start + the browser's trip to the IdP; returns the callback URL the IdP redirected to.
async fn authorize(s: &Sso) -> (Started, String) {
    let st = start_ok(s).await;
    let location = s.idp.follow_authorize(&st.authorization_url).await;
    (st, location)
}

/// A whole sign-in attempt; returns the callback response.
async fn login(s: &Sso) -> reqwest::Response {
    let (st, location) = authorize(s).await;
    callback_at(&location, Some(&st.cookie)).await
}

fn location_of(r: &reqwest::Response) -> String {
    r.headers().get("location").and_then(|v| v.to_str().ok()).unwrap_or("").to_string()
}

fn session_cookie_of(r: &reqwest::Response) -> Option<String> {
    set_cookies(r).into_iter().find(|c| c.starts_with("embyr_session=")).map(|c| c.split(';').next().unwrap().to_string())
}

/// What a browser can observe of a failed sign-in: must be identical for every cause (AC-SSO-19).
fn observable(r: &reqwest::Response) -> (u16, String, Vec<String>) {
    (
        r.status().as_u16(),
        location_of(r),
        set_cookies(r).into_iter().filter(|c| !c.starts_with("embyr_sso=")).collect(),
    )
}

async fn assert_failed(r: &reqwest::Response) {
    assert_eq!(observable(r), (302, "/admin/?error=sso_failed".into(), vec![]));
    assert!(!set_cookies(r).iter().any(|c| c.starts_with("embyr_session=")));
}

async fn count(s: &Sso, sql: &str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(&s.ctx.pool).await.expect("count")
}

async fn last_audit(s: &Sso) -> (String, String, Option<String>) {
    let row: (String, String, Option<String>) =
        sqlx::query_as("SELECT event, outcome, reason FROM admin_audit_events ORDER BY id DESC LIMIT 1")
            .fetch_one(&s.ctx.pool)
            .await
            .expect("an audit row");
    row
}

async fn clear_sso_sessions(s: &Sso) {
    sqlx::query("DELETE FROM sessions WHERE auth_method = 'sso'").execute(&s.ctx.pool).await.unwrap();
}

async fn assert_failed_with(s: &Sso, r: &reqwest::Response, reason: &str) {
    assert_failed(r).await;
    assert_eq!(last_audit(s).await, ("sso_signin".into(), "failed".into(), Some(reason.into())));
    assert_eq!(count(s, "SELECT count(*) FROM sessions WHERE auth_method = 'sso'").await, 0);
    assert_eq!(count(s, "SELECT count(*) FROM sso_auth_requests").await, 0, "state is consumed on every outcome");
}

async fn owner_cookie(ctx: &AdminTestContext) -> String {
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/auth/signin"))
        .json(&json!({"email": ctx.user_email, "password": ctx.user_password,
                      "totp_code": totp_code_for_secret(&ctx.totp_secret_b32)}))
        .send()
        .await
        .unwrap();
    set_cookies(&resp)[0].split(';').next().unwrap().to_string()
}

async fn session_user(ctx: &AdminTestContext, cookie: &str) -> String {
    let resp = ctx.client.get(ctx.url("/admin/v1/auth/session")).header("Cookie", cookie).send().await.unwrap();
    assert_eq!(resp.status(), 200);
    resp.json::<Value>().await.unwrap()["user_id"].as_str().unwrap().to_string()
}

// ── AC-SSO-01 / 08: start, then a linked user signs in ───────────────────────

#[tokio::test]
async fn start_returns_the_authorization_url_and_binds_the_browser() {
    let s = setup().await;
    let st = start_ok(&s).await;

    let url = url_params(&st.authorization_url);
    assert!(st.authorization_url.starts_with(&format!("{}/authorize?", s.idp.base_url)));
    assert_eq!(url["response_type"], "code");
    assert_eq!(url["client_id"], s.idp.client_id);
    assert_eq!(url["scope"], "openid email");
    assert_eq!(url["code_challenge_method"], "S256");
    assert_eq!(url["redirect_uri"], format!("{}/admin/v1/auth/oidc/callback", s.ctx.base_url));
    for k in ["state", "nonce", "code_challenge"] {
        assert!(url[k].len() >= 43, "{k}");
    }

    // cookie attributes (AC-SSO-01)
    assert!(st.set_cookie.contains("HttpOnly") && st.set_cookie.contains("Secure"));
    assert!(st.set_cookie.contains("SameSite=Lax") && !st.set_cookie.contains("Strict"));
    assert!(st.set_cookie.contains("Path=/admin/v1/auth/oidc") && st.set_cookie.contains("Max-Age=600"));

    // one single-use row; only hashes and ciphertext are stored
    let (n, verifier_enc, binding, state_hash, nonce): (i64, Vec<u8>, Vec<u8>, Vec<u8>, String) = sqlx::query_as(
        "SELECT count(*) OVER (), code_verifier_enc, browser_binding, state_hash, nonce FROM sso_auth_requests",
    )
    .fetch_one(&s.ctx.pool)
    .await
    .unwrap();
    assert_eq!(n, 1);
    assert_eq!(state_hash, blake3::hash(url["state"].as_bytes()).as_bytes().to_vec());
    assert_eq!(binding, blake3::hash(st.cookie.trim_start_matches("embyr_sso=").as_bytes()).as_bytes().to_vec());
    assert_eq!(nonce, url["nonce"]);
    // AES-256-GCM of the 43-char verifier: 12-byte nonce + ciphertext + 16-byte tag
    assert_eq!(verifier_enc.len(), 12 + 43 + 16);
}

fn url_params(url: &str) -> std::collections::HashMap<String, String> {
    form_urlencoded::parse(url.split_once('?').unwrap().1.as_bytes()).into_owned().collect()
}

#[tokio::test]
async fn linked_member_signs_in_and_gets_a_normal_session() {
    let s = setup().await;
    link(&s, &s.ctx.admin_id, "idp-user-1").await;

    let resp = login(&s).await;

    assert_eq!(resp.status(), 302);
    assert_eq!(location_of(&resp), "/admin/", "no query string, no token in the URL");
    assert_eq!(resp.headers()["cache-control"], "no-store");
    assert_eq!(resp.headers()["referrer-policy"], "no-referrer");
    let cookies = set_cookies(&resp);
    let session = cookies.iter().find(|c| c.starts_with("embyr_session=")).expect("session cookie");
    // identical attributes to password sign-in
    assert!(session.contains("HttpOnly; Secure; SameSite=Strict; Path=/admin; Max-Age=86400"), "{session}");
    assert!(cookies.iter().any(|c| c.starts_with("embyr_sso=;") && c.contains("Max-Age=0")), "binding cookie cleared");

    let cookie = session_cookie_of(&resp).unwrap();
    assert_eq!(session_user(&s.ctx, &cookie).await, s.ctx.admin_id);
    let (auth_method, provider, user): (String, Option<Uuid>, Uuid) =
        sqlx::query_as("SELECT auth_method, sso_provider_id, user_id FROM sessions WHERE auth_method = 'sso'")
            .fetch_one(&s.ctx.pool)
            .await
            .unwrap();
    assert_eq!((auth_method.as_str(), provider, user), ("sso", Some(s.provider), uuid(&s.ctx.admin_id)));
    // the raw token is never stored
    let token = cookie.trim_start_matches("embyr_session=");
    let hash = blake3::hash(token.as_bytes()).as_bytes().to_vec();
    assert_eq!(count_hash(&s, &hash).await, 1);

    assert_eq!(count(&s, "SELECT count(*) FROM sso_auth_requests").await, 0);
    assert_eq!(last_audit(&s).await, ("sso_signin".into(), "ok".into(), None));
    assert!(sqlx::query_scalar::<_, Option<chrono::DateTime<chrono::Utc>>>(
        "SELECT last_login_at FROM admin_sso_identities"
    )
    .fetch_one(&s.ctx.pool)
    .await
    .unwrap()
    .is_some());
}

async fn count_hash(s: &Sso, hash: &[u8]) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM sessions WHERE token_hash = $1")
        .bind(hash)
        .fetch_one(&s.ctx.pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn es256_signed_tokens_are_accepted_too() {
    let s = setup().await;
    link(&s, &s.ctx.viewer_id, "idp-user-1").await;
    s.idp.set(|b| b.sig = Sig::Es256);
    let resp = login(&s).await;
    assert_eq!(location_of(&resp), "/admin/");
}

// ── AC-SSO-02: unavailable, one body for every cause ─────────────────────────

#[tokio::test]
async fn unavailable_providers_all_answer_the_same_body_and_store_nothing() {
    let s = setup().await;
    let account = uuid(&s.ctx.account_id);
    let disabled = insert_provider(&s.ctx, account, &format!("{}/disabled", s.idp.base_url), "c", false).await;
    // a provider of a soft-deleted account
    let gone_account: Uuid = sqlx::query_scalar("INSERT INTO accounts (name, deleted_at) VALUES ('gone', now()) RETURNING id")
        .fetch_one(&s.ctx.pool)
        .await
        .unwrap();
    let deleted = insert_provider(&s.ctx, gone_account, &s.idp.issuer(), "c", true).await;

    let mut bodies = vec![];
    for id in [Uuid::new_v4().to_string(), disabled.to_string(), deleted.to_string(), "not-a-uuid".into()] {
        let r = start_raw(&s.ctx, &id).await;
        assert_eq!(r.status(), 200, "{id}");
        assert!(set_cookies(&r).is_empty());
        bodies.push(r.text().await.unwrap());
    }
    assert!(bodies.iter().all(|b| b == &json!({"error": "sso_unavailable"}).to_string()), "{bodies:?}");
    assert_eq!(count(&s, "SELECT count(*) FROM sso_auth_requests").await, 0);
}

#[tokio::test]
async fn unset_public_base_url_means_unavailable() {
    // `new()` has no EMBYR_PUBLIC_BASE_URL
    let ctx = AdminTestContext::new().await;
    let idp = MockIdp::start().await;
    std::env::set_var("EMBYR_ALLOW_PRIVATE_DSN_HOSTS", "true");
    let provider = insert_provider(&ctx, uuid(&ctx.account_id), &idp.issuer(), &idp.client_id, true).await;
    let r = start_raw(&ctx, &provider.to_string()).await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.json::<Value>().await.unwrap(), json!({"error": "sso_unavailable"}));
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM sso_auth_requests").fetch_one(&ctx.pool).await.unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn unknown_intents_are_refused_and_invite_and_link_need_their_credentials() {
    // Slices 4/5 implement `invite` and `link` (see s04/s05); here only the guard rails matter.
    let s = setup().await;
    let post = |intent: &'static str| {
        let (ctx, id) = (&s.ctx, s.provider);
        async move {
            ctx.client
                .post(ctx.url("/admin/v1/auth/oidc/start"))
                .json(&json!({"provider_id": id, "intent": intent}))
                .send()
                .await
                .unwrap()
        }
    };
    assert_eq!(post("bogus").await.status(), 400);
    assert_eq!(post("link").await.status(), 401, "link needs a session");
    let r = post("invite").await; // no invitation token
    assert_eq!((r.status().as_u16(), r.json::<Value>().await.unwrap()), (200, json!({"error": "sso_unavailable"})));
    assert_eq!(count(&s, "SELECT count(*) FROM sso_auth_requests").await, 0);
}

// ── AC-SSO-03: state and cookie handling ─────────────────────────────────────

#[tokio::test]
async fn unknown_state_fails_without_a_session() {
    let s = setup().await;
    link(&s, &s.ctx.admin_id, "idp-user-1").await;
    let r = callback_at(
        &format!("{}/admin/v1/auth/oidc/callback?code=x&state=never-issued", s.ctx.base_url),
        Some("embyr_sso=whatever"),
    )
    .await;
    assert_failed_with(&s, &r, "state_unknown_or_expired").await;
}

#[tokio::test]
async fn a_client_supplied_id_token_is_ignored_audit_58() {
    let s = setup().await;
    link(&s, &s.ctx.admin_id, "idp-user-1").await;
    let r = callback_at(
        &format!("{}/admin/v1/auth/oidc/callback?id_token=eyJhbGciOiJSUzI1NiJ9.e30.x&code=c&state=csrf-nonce", s.ctx.base_url),
        None,
    )
    .await;
    assert_failed(&r).await;
    assert_eq!(count(&s, "SELECT count(*) FROM sessions").await, 0);
}

#[tokio::test]
async fn a_state_cannot_be_used_twice() {
    let s = setup().await;
    link(&s, &s.ctx.admin_id, "idp-user-1").await;
    let (st, location) = authorize(&s).await;
    let first = callback_at(&location, Some(&st.cookie)).await;
    assert_eq!(location_of(&first), "/admin/");
    let replay = callback_at(&location, Some(&st.cookie)).await;
    assert_failed(&replay).await;
    assert_eq!(count(&s, "SELECT count(*) FROM sessions WHERE auth_method = 'sso'").await, 1);
    assert_eq!(last_audit(&s).await.2.as_deref(), Some("state_unknown_or_expired"));
}

#[tokio::test]
async fn an_expired_state_fails() {
    let s = setup().await;
    link(&s, &s.ctx.admin_id, "idp-user-1").await;
    let (st, location) = authorize(&s).await;
    sqlx::query("UPDATE sso_auth_requests SET expires_at = now() - interval '1 second'")
        .execute(&s.ctx.pool)
        .await
        .unwrap();
    let r = callback_at(&location, Some(&st.cookie)).await;
    assert_failed(&r).await;
    assert_eq!(count(&s, "SELECT count(*) FROM sessions WHERE auth_method = 'sso'").await, 0);
    assert_eq!(last_audit(&s).await.2.as_deref(), Some("state_unknown_or_expired"));
}

#[tokio::test]
async fn a_different_browser_cannot_complete_the_flow() {
    let s = setup().await;
    link(&s, &s.ctx.admin_id, "idp-user-1").await;
    // victim starts; attacker replays the callback URL from another browser
    let (_victim, location) = authorize(&s).await;
    let r = callback_at(&location, Some("embyr_sso=attacker-guess")).await;
    assert_failed_with(&s, &r, "cookie_mismatch").await;
}

#[tokio::test]
async fn a_missing_binding_cookie_fails() {
    let s = setup().await;
    link(&s, &s.ctx.admin_id, "idp-user-1").await;
    let (_st, location) = authorize(&s).await;
    let r = callback_at(&location, None).await;
    assert_failed_with(&s, &r, "cookie_missing").await;
}

#[tokio::test]
async fn an_idp_error_parameter_fails_and_consumes_the_state() {
    let s = setup().await;
    link(&s, &s.ctx.admin_id, "idp-user-1").await;
    s.idp.set(|b| b.authorize_error = Some("access_denied".into()));
    let (st, location) = authorize(&s).await;
    assert!(location.contains("error=access_denied"));
    let r = callback_at(&location, Some(&st.cookie)).await;
    assert_failed_with(&s, &r, "idp_error").await;
}

// ── AC-SSO-05 / 06: token validation end to end ──────────────────────────────

#[tokio::test]
async fn a_wrong_nonce_is_refused() {
    let s = setup().await;
    link(&s, &s.ctx.admin_id, "idp-user-1").await;
    s.idp.set(|b| b.nonce = NonceMode::Wrong("not-the-nonce".into()));
    assert_failed_with(&s, &login(&s).await, "token_nonce_mismatch").await;
    s.idp.set(|b| b.nonce = NonceMode::Omit);
    assert_failed_with(&s, &login(&s).await, "token_nonce_mismatch").await;
}

#[tokio::test]
async fn wrong_issuer_audience_azp_and_expiry_are_refused() {
    let s = setup().await;
    link(&s, &s.ctx.admin_id, "idp-user-1").await;
    for (label, over) in [
        ("token_issuer_mismatch", json!({"iss": "https://evil.example"})),
        ("token_audience_mismatch", json!({"aud": "someone-else"})),
        ("token_azp_mismatch", json!({"aud": [s.idp.client_id, "other"], "azp": "other"})),
        ("token_missing_claim", json!({"exp": null})),
        ("token_bad_subject", json!({"sub": ""})),
    ] {
        s.idp.reset();
        s.idp.set(|b| {
            for (k, v) in over.as_object().unwrap() {
                b.claim_overrides.insert(k.clone(), v.clone());
            }
        });
        assert_failed_with(&s, &login(&s).await, label).await;
    }
    s.idp.reset();
    s.idp.set(|b| b.exp_in_secs = -3600);
    assert_failed_with(&s, &login(&s).await, "token_expired").await;
    s.idp.reset();
    s.idp.set(|b| b.iat_offset_secs = 3600);
    assert_failed_with(&s, &login(&s).await, "token_iat_in_future").await;
}

#[tokio::test]
async fn forged_algorithms_and_key_confusion_are_refused() {
    let s = setup().await;
    link(&s, &s.ctx.admin_id, "idp-user-1").await;
    s.idp.set(|b| b.sig = Sig::Hs256Confusion);
    assert_failed_with(&s, &login(&s).await, "token_alg_not_allowed").await;
    // header says RS256, kid points at the EC key
    s.idp.reset();
    s.idp.set(|b| b.kid = Some("ec-1".into()));
    assert_failed_with(&s, &login(&s).await, "token_key_type_mismatch").await;
    s.idp.reset();
    s.idp.set(|b| b.kid = Some("no-such-key".into()));
    assert_failed_with(&s, &login(&s).await, "token_unknown_key").await;
}

#[tokio::test]
async fn a_bad_signature_is_refused() {
    let s = setup().await;
    link(&s, &s.ctx.admin_id, "idp-user-1").await;
    assert_eq!(location_of(&login(&s).await), "/admin/", "primes the JWKS cache with the first key");
    // the IdP replaces the key under the SAME kid: the cached key no longer verifies
    s.idp.rotate_rsa_key("rsa-1");
    clear_sso_sessions(&s).await; // the first sign-in left one; the failure helper expects none
    assert_failed_with(&s, &login(&s).await, "token_bad_signature").await;
}

#[tokio::test]
async fn an_unknown_kid_refetches_at_most_once_a_minute() {
    let s = setup().await;
    link(&s, &s.ctx.admin_id, "idp-user-1").await;
    assert_eq!(location_of(&login(&s).await), "/admin/");
    assert_eq!(s.idp.hits("jwks"), 1);
    s.idp.rotate_rsa_key("rsa-2");
    clear_sso_sessions(&s).await;
    // fresh cache (< 60 s): no refetch, the unknown kid fails
    assert_failed_with(&s, &login(&s).await, "token_unknown_key").await;
    assert_eq!(s.idp.hits("jwks"), 1, "no forced refetch inside the minute");
}

#[tokio::test]
async fn jwks_and_token_endpoint_faults_fail_closed() {
    let s = setup().await;
    link(&s, &s.ctx.admin_id, "idp-user-1").await;

    s.idp.set(|b| b.token_status = 500);
    assert_failed_with(&s, &login(&s).await, "token_endpoint_error").await;
    s.idp.reset();
    s.idp.set(|b| b.omit_id_token = true);
    assert_failed_with(&s, &login(&s).await, "id_token_missing").await;
    s.idp.reset();
    s.idp.set(|b| b.jwks_status = 500);
    assert_failed_with(&s, &login(&s).await, "fetch_status").await;
}

#[tokio::test]
async fn an_oversized_jwks_is_refused() {
    let s = setup().await;
    link(&s, &s.ctx.admin_id, "idp-user-1").await;
    s.idp.set(|b| b.jwks_oversize = true);
    assert_failed_with(&s, &login(&s).await, "fetch_too_large").await;
}

#[tokio::test]
async fn a_slow_jwks_times_out() {
    let s = setup().await;
    link(&s, &s.ctx.admin_id, "idp-user-1").await;
    s.idp.set(|b| b.jwks_delay = std::time::Duration::from_secs(8));
    let started = std::time::Instant::now();
    assert_failed_with(&s, &login(&s).await, "fetch_failed").await;
    assert!(started.elapsed() < std::time::Duration::from_secs(7), "5 s timeout applies");
}

// ── AC-SSO-06 / 07: discovery ────────────────────────────────────────────────

#[tokio::test]
async fn a_discovery_issuer_mismatch_makes_the_provider_unavailable() {
    let s = setup().await;
    s.idp.set(|b| b.discovery_issuer = Some("https://evil.example".into()));
    // the seeded provider: its discovery has never been fetched, so nothing is cached
    // ((account, issuer) is unique, so no second provider can share the mock's issuer)
    let r = start_raw(&s.ctx, &s.provider.to_string()).await;
    assert_eq!(r.json::<Value>().await.unwrap(), json!({"error": "sso_unavailable"}));
    assert_eq!(count(&s, "SELECT count(*) FROM sso_auth_requests").await, 0);
}

#[tokio::test]
async fn a_discovery_redirect_is_not_followed() {
    let s = setup().await;
    s.idp.set(|b| b.discovery_redirect_to = Some(s.idp.redirect_target()));
    let r = start_raw(&s.ctx, &s.provider.to_string()).await;
    assert_eq!(r.json::<Value>().await.unwrap(), json!({"error": "sso_unavailable"}));
    assert_eq!(s.idp.hits("discovery"), 1, "fetched once, the redirect was not followed");
}

#[tokio::test]
async fn creating_a_provider_validates_the_issuer() {
    let s = setup().await;
    let cookie = owner_cookie(&s.ctx).await;
    let create = |issuer: String| {
        let (ctx, cookie) = (&s.ctx, cookie.clone());
        async move {
            ctx.client
                .post(ctx.url("/admin/v1/oidc_providers"))
                .header("Cookie", cookie)
                .json(&json!({"issuer": issuer, "client_id": "c", "client_secret": "plaintext-secret"}))
                .send()
                .await
                .unwrap()
        }
    };

    // discovery names another issuer -> 422
    s.idp.set(|b| b.discovery_issuer = Some("https://evil.example".into()));
    let r = create(s.idp.issuer()).await;
    assert_eq!(r.status(), 422);
    assert_eq!(r.json::<Value>().await.unwrap()["reason"], "discovery_issuer_mismatch");
    // unreachable issuer -> 422
    s.idp.reset();
    let r = create("http://127.0.0.1:1".into()).await;
    assert_eq!(r.status(), 422);
    assert_eq!(r.json::<Value>().await.unwrap()["reason"], "fetch_failed");
    assert_eq!(count(&s, "SELECT count(*) FROM oidc_providers").await, 1, "only the seeded provider");

    // an issuer the mock does not serve discovery for (path issuer) is a mismatch -> 422
    assert_eq!(create(format!("{}/tenant", s.idp.base_url)).await.status(), 422);

    // a second, healthy IdP -> 201; the secret is stored encrypted and never echoed
    let idp2 = MockIdp::start().await;
    let ok = create(idp2.issuer()).await;
    assert_eq!(ok.status(), 201);
    let body = ok.text().await.unwrap();
    assert!(!body.contains("plaintext-secret"), "{body}");
    let enc: Vec<u8> = sqlx::query_scalar("SELECT client_secret_enc FROM oidc_providers WHERE issuer = $1")
        .bind(idp2.issuer())
        .fetch_one(&s.ctx.pool)
        .await
        .unwrap();
    assert!(!enc.windows(16).any(|w| w == b"plaintext-secret"));
    assert_eq!(last_audit(&s).await.0, "sso_provider_created");
}

#[tokio::test]
async fn enabling_a_provider_revalidates_and_disabling_ends_sso_sessions() {
    let s = setup().await;
    link(&s, &s.ctx.admin_id, "idp-user-1").await;
    let cookie = owner_cookie(&s.ctx).await;
    let patch = |enabled: bool| {
        let (ctx, cookie, id) = (&s.ctx, cookie.clone(), s.provider);
        async move {
            ctx.client
                .patch(ctx.url(&format!("/admin/v1/oidc_providers/{id}")))
                .header("Cookie", cookie)
                .json(&json!({ "enabled": enabled }))
                .send()
                .await
                .unwrap()
        }
    };
    let sso = session_cookie_of(&login(&s).await).unwrap();
    assert_eq!(session_user(&s.ctx, &sso).await, s.ctx.admin_id);

    assert_eq!(patch(false).await.status(), 200);
    // O6: the SSO session ended with the provider
    let r = s.ctx.client.get(s.ctx.url("/admin/v1/auth/session")).header("Cookie", &sso).send().await.unwrap();
    assert_eq!(r.status(), 401);
    let r = start_raw(&s.ctx, &s.provider.to_string()).await;
    assert_eq!(r.json::<Value>().await.unwrap(), json!({"error": "sso_unavailable"}));

    // IdP now broken: enabling is a 422, the provider stays disabled
    s.idp.set(|b| b.discovery_issuer = Some("https://evil.example".into()));
    assert_eq!(patch(true).await.status(), 422);
    assert_eq!(count(&s, "SELECT count(*) FROM oidc_providers WHERE NOT enabled").await, 1);
    s.idp.reset();
    assert_eq!(patch(true).await.status(), 200);
}

// ── AC-SSO-08..10, 19: identity resolution, never by e-mail ──────────────────

#[tokio::test]
async fn the_session_user_is_always_the_linked_one_never_the_email_match_audit_58() {
    // owner + admin + viewer are members. The IdP identity is linked to the VIEWER, while the
    // token's e-mail is the OWNER's (verified). The session must belong to the viewer.
    let s = setup().await;
    link(&s, &s.ctx.viewer_id, "idp-user-1").await;
    s.idp.set(|b| {
        b.email = Some("test@example.com".into());
        b.email_verified = json!(true);
    });
    let resp = login(&s).await;
    assert_eq!(location_of(&resp), "/admin/");
    let cookie = session_cookie_of(&resp).unwrap();
    assert_eq!(session_user(&s.ctx, &cookie).await, s.ctx.viewer_id);
    assert_ne!(session_user(&s.ctx, &cookie).await, s.ctx.user_id);
}

#[tokio::test]
async fn an_unlinked_subject_fails_even_when_its_email_matches_a_member() {
    let s = setup().await;
    // nobody is linked; the token's e-mail equals the owner's and the admin's
    for email in ["test@example.com", "admin@example.com", "viewer@example.com"] {
        s.idp.set(|b| b.email = Some(email.into()));
        assert_failed_with(&s, &login(&s).await, "identity_unlinked").await;
    }
    // a link for a DIFFERENT subject does not help either
    link(&s, &s.ctx.user_id, "someone-else").await;
    assert_failed_with(&s, &login(&s).await, "identity_unlinked").await;
    assert_eq!(count(&s, "SELECT count(*) FROM sessions").await, 0);
}

#[tokio::test]
async fn a_link_whose_user_left_the_account_cannot_sign_in() {
    let s = setup().await;
    link(&s, &s.ctx.viewer_id, "idp-user-1").await;
    sqlx::query("DELETE FROM account_members WHERE user_id = $1")
        .bind(uuid(&s.ctx.viewer_id))
        .execute(&s.ctx.pool)
        .await
        .unwrap();
    assert_eq!(count(&s, "SELECT count(*) FROM users").await, 3, "the users row survives");
    assert_failed_with(&s, &login(&s).await, "member_removed").await;
}

#[tokio::test]
async fn no_failure_cause_is_distinguishable_by_the_browser() {
    let s = setup().await;
    link(&s, &s.ctx.viewer_id, "linked-sub").await;
    let mut seen = vec![];

    // unlinked subject
    seen.push(observable(&login(&s).await));
    // bad nonce
    s.idp.set(|b| {
        b.sub = "linked-sub".into();
        b.nonce = NonceMode::Wrong("x".into());
    });
    seen.push(observable(&login(&s).await));
    // removed member
    s.idp.reset();
    s.idp.set(|b| b.sub = "linked-sub".into());
    sqlx::query("DELETE FROM account_members WHERE user_id = $1")
        .bind(uuid(&s.ctx.viewer_id))
        .execute(&s.ctx.pool)
        .await
        .unwrap();
    seen.push(observable(&login(&s).await));
    // unknown state
    seen.push(observable(
        &callback_at(&format!("{}/admin/v1/auth/oidc/callback?code=x&state=nope", s.ctx.base_url), None).await,
    ));
    assert!(seen.windows(2).all(|w| w[0] == w[1]), "{seen:?}");
}

// ── AC-SSO-04: limits ────────────────────────────────────────────────────────

#[tokio::test]
async fn start_is_rate_limited_per_source_ip() {
    let s = setup_with(SigninRateLimiter::new(2.0, 0.0001)).await;
    assert_eq!(start_raw(&s.ctx, &s.provider.to_string()).await.status(), 200);
    assert_eq!(start_raw(&s.ctx, &s.provider.to_string()).await.status(), 200);
    let r = start_raw(&s.ctx, &s.provider.to_string()).await;
    assert_eq!(r.status(), 429);
    assert!(r.headers().get("retry-after-ms").is_some());
    assert_eq!(count(&s, "SELECT count(*) FROM sso_auth_requests").await, 2);
}

#[tokio::test]
async fn a_throttled_callback_fails_without_a_session() {
    // one token: `start` spends it, the callback is throttled
    let s = setup_with(SigninRateLimiter::new(1.0, 0.0001)).await;
    link(&s, &s.ctx.admin_id, "idp-user-1").await;
    let (st, location) = authorize(&s).await;
    let r = callback_at(&location, Some(&st.cookie)).await;
    assert_failed(&r).await;
    assert_eq!(count(&s, "SELECT count(*) FROM sessions WHERE auth_method = 'sso'").await, 0);
}

#[tokio::test]
async fn live_requests_per_provider_are_capped_and_expired_ones_are_purged() {
    let s = setup().await;
    sqlx::query(
        "INSERT INTO sso_auth_requests (state_hash, provider_id, intent, nonce, code_verifier_enc, browser_binding, expires_at) \
         SELECT sha256(convert_to('cap-' || g::text, 'UTF8')), $1, 'signin', 'n', '\\x00', '\\x00', now() + interval '5 minutes' \
         FROM generate_series(1, 200) g",
    )
    .bind(s.provider)
    .execute(&s.ctx.pool)
    .await
    .unwrap();
    let r = start_raw(&s.ctx, &s.provider.to_string()).await;
    assert_eq!(r.status(), 429, "cap of 200 live rows");
    // expired rows do not count; start deletes at most 100 of them per call (the sweeper does the rest)
    sqlx::query("UPDATE sso_auth_requests SET expires_at = now() - interval '1 minute'")
        .execute(&s.ctx.pool)
        .await
        .unwrap();
    assert_eq!(start_raw(&s.ctx, &s.provider.to_string()).await.status(), 200);
    assert_eq!(count(&s, "SELECT count(*) FROM sso_auth_requests WHERE expires_at > now()").await, 1);
    assert_eq!(count(&s, "SELECT count(*) FROM sso_auth_requests").await, 101, "200 expired - 100 purged + 1 new");
}

// ── AC-SSO-17: provider secret ───────────────────────────────────────────────

#[tokio::test]
async fn the_provider_secret_is_replaceable_and_never_returned() {
    let s = setup().await;
    link(&s, &s.ctx.admin_id, "idp-user-1").await;
    let owner = owner_cookie(&s.ctx).await;
    let put = |secret: &'static str, cookie: String| {
        let (ctx, id) = (&s.ctx, s.provider);
        async move {
            ctx.client
                .put(ctx.url(&format!("/admin/v1/oidc_providers/{id}/secret")))
                .header("Cookie", cookie)
                .json(&json!({ "client_secret": secret }))
                .send()
                .await
                .unwrap()
        }
    };

    // a wrong secret: the IdP refuses the code exchange
    let r = put("wrong-secret-value", owner.clone()).await;
    assert_eq!(r.status(), 204);
    assert!(r.text().await.unwrap().is_empty());
    let enc: Vec<u8> = sqlx::query_scalar("SELECT client_secret_enc FROM oidc_providers WHERE id = $1")
        .bind(s.provider)
        .fetch_one(&s.ctx.pool)
        .await
        .unwrap();
    assert!(!enc.windows(10).any(|w| w == b"wrong-secr"), "stored encrypted");
    assert_failed_with(&s, &login(&s).await, "token_endpoint_error").await;

    // the right one again
    assert_eq!(put("mock-secret", owner.clone()).await.status(), 204);
    assert_eq!(location_of(&login(&s).await), "/admin/");
    assert_eq!(last_audit(&s).await.0, "sso_signin");

    // the listing never carries it
    let list = s.ctx.client.get(s.ctx.url("/admin/v1/oidc_providers")).header("Cookie", &owner).send().await.unwrap();
    let body = list.text().await.unwrap();
    assert!(!body.contains("secret"), "{body}");

    // not for viewers, not for unknown providers, not empty
    let viewer = {
        let r = s
            .ctx
            .client
            .post(s.ctx.url("/admin/v1/auth/signin"))
            .json(&json!({"email": s.ctx.viewer_email, "password": s.ctx.viewer_password,
                          "totp_code": totp_code_for_secret(&s.ctx.viewer_totp_secret_b32)}))
            .send()
            .await
            .unwrap();
        set_cookies(&r)[0].split(';').next().unwrap().to_string()
    };
    assert_eq!(put("x-secret", viewer).await.status(), 403);
    let r = s
        .ctx
        .client
        .put(s.ctx.url(&format!("/admin/v1/oidc_providers/{}/secret", Uuid::new_v4())))
        .header("Cookie", &owner)
        .json(&json!({"client_secret": "x"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 404);
    assert_eq!(put("   ", owner).await.status(), 422);
}

#[tokio::test]
async fn provider_lifecycle_is_audited_with_labels_only() {
    let s = setup().await;
    let owner = owner_cookie(&s.ctx).await;
    let r = s
        .ctx
        .client
        .put(s.ctx.url(&format!("/admin/v1/oidc_providers/{}/secret", s.provider)))
        .header("Cookie", &owner)
        .json(&json!({"client_secret": "never-in-audit-rows"}))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    let rows: Vec<(String, String, Option<String>)> =
        sqlx::query_as("SELECT event, outcome, reason FROM admin_audit_events ORDER BY id")
            .fetch_all(&s.ctx.pool)
            .await
            .unwrap();
    assert_eq!(rows, vec![("sso_provider_secret_rotated".into(), "ok".into(), None)]);
    let dump: String = sqlx::query_scalar("SELECT string_agg(admin_audit_events::text, ' ') FROM admin_audit_events")
        .fetch_one(&s.ctx.pool)
        .await
        .unwrap();
    assert!(!dump.contains("never-in-audit-rows"));
}
