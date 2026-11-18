//! Auth handlers: signin, signout, session probe.
//!
//! POST /admin/v1/auth/signin
//!   Validates email + Argon2id password + TOTP code (or recovery code).
//!   Creates session row (BLAKE3 token hash). Sets HttpOnly cookie.
//!   Returns 200 { account_id, display_name, role }.
//!
//! POST /admin/v1/auth/signout
//!   Deletes session row. Sets Max-Age=0 on cookie. Returns 204.
//!
//! GET /admin/v1/auth/session
//!   Session-authed. Returns who the cookie belongs to so the SPA can restore its
//!   signed-in shell after a reload (admin-ui-api-wiring, AC-AUW-24).
//!
//! OIDC SSO start/callback live in `oidc_sso.rs` (ADR-086); they mint sessions through `issue_session`.

use axum::{
    extract::{ConnectInfo, Json, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::net::SocketAddr;
use totp_rs::{Algorithm as TotpAlgorithm, TOTP};
use uuid::Uuid;

use crate::adapters::encryption::{decrypt_with_rotation, RotationDecryptError};
use crate::admin::extractors::session_context::SessionContext;
use crate::admin::state::UserAdminState;

/// Session cookie lifetime in seconds (24 hours).
const SESSION_COOKIE_MAX_AGE_SECS: u32 = 86_400;

// ── Request / response types ──────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct SigninRequest {
    pub email: String,
    pub password: String,
    pub totp_code: Option<String>,
    pub recovery_code: Option<String>,
}

#[derive(Serialize)]
struct SigninResponse {
    account_id: String, // UUID serialised as hyphenated string
    display_name: String,
    role: String,
}

#[derive(Serialize)]
struct ErrorBody {
    message: String,
}

// ── Shared response constructors ──────────────────────────────────────────────

fn invalid_credentials() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        axum::Json(ErrorBody {
            message: "Invalid credentials".to_string(),
        }),
    )
        .into_response()
}

fn invalid_code() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        axum::Json(ErrorBody {
            message: "Invalid or expired code".to_string(),
        }),
    )
        .into_response()
}

/// admin-signin-hardening (AC-ASH-03/06, ADR-076): per-source-IP rate limit
/// exceeded. `retry-after-ms` header lets a well-behaved client back off.
pub(crate) fn signin_throttled(retry_after_ms: u64) -> Response {
    let mut response = (
        StatusCode::TOO_MANY_REQUESTS,
        axum::Json(ErrorBody {
            message: "Too many signin attempts — try again later".to_string(),
        }),
    )
        .into_response();
    if let Ok(v) = HeaderValue::from_str(&retry_after_ms.to_string()) {
        response.headers_mut().insert("retry-after-ms", v);
    }
    response
}

fn internal_err(context: &str, e: impl std::fmt::Display) -> Response {
    tracing::error!("signin: {context}: {e}");
    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}

// ── Cookie helpers ────────────────────────────────────────────────────────────

/// Build the `Set-Cookie` header value for the session cookie.
fn build_session_cookie(token: &str) -> String {
    format!(
        "embyr_session={token}; HttpOnly; Secure; SameSite=Strict; Path=/admin; Max-Age={SESSION_COOKIE_MAX_AGE_SECS}"
    )
}

/// Extract a named cookie value from the `Cookie` request header.
pub(crate) fn extract_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    let cookie_header = headers.get("cookie")?.to_str().ok()?;
    for pair in cookie_header.split(';') {
        let pair = pair.trim();
        if let Some((key, value)) = pair.split_once('=') {
            if key.trim() == name {
                return Some(value.trim().to_string());
            }
        }
    }
    None
}

// ── Row-decode helper ────────────────────────────────────────────────────────

/// Read column `$field` from `$row`, returning `internal_err($context, e)`
/// from the enclosing handler on decode failure. Collapses the repeated
/// `match row.try_get(...) { Ok(v) => v, Err(e) => return internal_err(...) }`
/// boilerplate used throughout `signin`.
macro_rules! try_get_or_err {
    ($row:expr, $field:literal, $context:literal) => {
        match $row.try_get($field) {
            Ok(v) => v,
            Err(e) => return internal_err($context, e),
        }
    };
}

// ── Session issuing (signin + SSO callback) ──────────────────────────────────

/// How the session's owner proved who they are. SSO sessions remember their provider so that
/// disabling the provider can end them (ADR-086, O6).
pub(crate) enum SessionOrigin {
    Password,
    Sso { provider_id: Uuid },
}

/// A freshly minted session. The raw token exists only here and in the cookie; the DB has its hash.
pub(crate) struct IssuedSession {
    token: String,
}

impl IssuedSession {
    /// The `Set-Cookie` value, identical for every origin.
    pub(crate) fn set_cookie(&self) -> String {
        build_session_cookie(&self.token)
    }
}

/// Signin steps 7-8: 32 random bytes → base64url token, store BLAKE3(token) with a 24 h expiry.
/// AC-7: the raw token is NEVER written to the DB. Takes any executor so a caller can mint the
/// session inside its own transaction.
pub(crate) async fn issue_session<'e>(
    executor: impl sqlx::PgExecutor<'e>,
    user_id: Uuid,
    account_id: Uuid,
    origin: SessionOrigin,
) -> Result<IssuedSession, sqlx::Error> {
    let mut token_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut token_bytes);
    let token = URL_SAFE_NO_PAD.encode(token_bytes);
    let token_hash = blake3::hash(token.as_bytes()).as_bytes().to_vec();
    let (auth_method, provider_id) = match origin {
        SessionOrigin::Password => ("password", None),
        SessionOrigin::Sso { provider_id } => ("sso", Some(provider_id)),
    };
    sqlx::query(
        "INSERT INTO sessions (user_id, account_id, token_hash, expires_at, auth_method, sso_provider_id) \
         VALUES ($1, $2, $3, now() + interval '24 hours', $4, $5)",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(&token_hash)
    .bind(auth_method)
    .bind(provider_id)
    .execute(executor)
    .await?;
    Ok(IssuedSession { token })
}

// ── Shared credential verification (signin + account deletion) ───────────────

/// The `users` columns credential verification needs.
pub(crate) struct UserSecrets {
    id: Uuid,
    /// `None` = SSO-only user (ADR-086): no password to verify.
    pub(crate) password_hash: Option<String>,
    totp_secret_enc: Option<Vec<u8>>,
    failed_totp_attempts: i32,
    locked_until: Option<chrono::DateTime<chrono::Utc>>,
}

impl UserSecrets {
    fn from_row(row: &sqlx::postgres::PgRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            id: row.try_get("id")?,
            password_hash: row.try_get("password_hash")?,
            totp_secret_enc: row.try_get("totp_secret_enc")?,
            failed_totp_attempts: row.try_get("failed_totp_attempts")?,
            locked_until: row.try_get("locked_until")?,
        })
    }

    /// Load by user id (re-authentication of an already-signed-in user).
    pub(crate) async fn fetch_by_id(
        pool: &sqlx::PgPool,
        user_id: Uuid,
    ) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query(
            "SELECT id, password_hash, totp_secret_enc, failed_totp_attempts, locked_until \
             FROM users WHERE id = $1",
        )
        .bind(user_id)
        .fetch_optional(pool)
        .await?
        .map(|r| Self::from_row(&r))
        .transpose()
    }
}

/// A valid Argon2id hash (production parameters) of a random password nobody knows; verified in
/// place of a missing hash so SSO-only users cost the same as password users.
const DUMMY_PASSWORD_HASH: &str =
    "$argon2id$v=19$m=65536,t=3,p=4$BVBrAvsju03hsmQ50WVy5Q$hFNARucdse6Kkw3F3uWQtkwnXxAQ7uBZh8UmuZh/j6U";

/// Signin steps 2-5, shared verbatim with account deletion so re-entry is never
/// a weaker or lockout-bypassing path: lockout check (429), Argon2id password
/// (401), TOTP / recovery code with attempt counting (401, lockout after 3),
/// counter reset on success. `Err` carries the ready-made HTTP response.
pub(crate) async fn verify_credentials(
    state: &UserAdminState,
    user: &UserSecrets,
    password: &str,
    totp_code: Option<&str>,
    recovery_code: Option<&str>,
) -> Result<(), Response> {
    check_lockout(user)?;

    // ── 3. Argon2id password verification ────────────────────────────────────
    // AC-3: wrong password → identical 401 shape to unknown email (oracle protection).
    // AC-ASH-01: runs on the blocking pool via `argon2_blocking`.
    // AC-SSO-14: an SSO-only user (NULL hash) gets the same 401 after one dummy verify, so
    // neither body nor timing says "this account has no password".
    let phc = user.password_hash.as_deref().unwrap_or(DUMMY_PASSWORD_HASH);
    match crate::adapters::argon2_blocking::verify_password(password.as_bytes(), phc).await {
        Ok(true) if user.password_hash.is_some() => {}
        Ok(_) => return Err(invalid_credentials()),
        Err(e) => return Err(internal_err("verify password hash", e)),
    }

    verify_second_factor(state, user, totp_code, recovery_code).await
}

/// ── 2. Lockout check ─────────────────────────────────────────────────────────
/// AC-5: fourth attempt after 3 consecutive TOTP failures returns 429.
// `Response` is the handler's error type everywhere; same allowance as invitations.rs.
#[allow(clippy::result_large_err)]
pub(crate) fn check_lockout(user: &UserSecrets) -> Result<(), Response> {
    match user.locked_until {
        Some(until) if until > chrono::Utc::now() => Err((
            StatusCode::TOO_MANY_REQUESTS,
            axum::Json(ErrorBody {
                message: "Too many failed attempts — account locked for 15 minutes".to_string(),
            }),
        )
            .into_response()),
        _ => Ok(()),
    }
}

/// Signin steps 4-5 — TOTP / recovery code with attempt counting and counter
/// reset on success. Shared with password reset, where it is the only proof
/// of identity beyond the emailed token (the caller runs `check_lockout` first).
pub(crate) async fn verify_second_factor(
    state: &UserAdminState,
    user: &UserSecrets,
    totp_code: Option<&str>,
    recovery_code: Option<&str>,
) -> Result<(), Response> {
    let pool = state.system_db.pool();
    let user_id = user.id;

    // ── 4. TOTP / recovery-code check ────────────────────────────────────────
    if let Some(recovery_code) = recovery_code {
        // AC-6: valid one-time recovery code grants access; immediately invalidated.
        let code_hash = blake3::hash(recovery_code.as_bytes()).as_bytes().to_vec();
        let rc_id: Uuid = match sqlx::query_scalar(
            "SELECT id FROM mfa_recovery_codes \
             WHERE user_id = $1 AND code_hash = $2 AND used_at IS NULL \
             LIMIT 1",
        )
        .bind(user_id)
        .bind(&code_hash)
        .fetch_optional(pool)
        .await
        {
            Ok(Some(id)) => id,
            Ok(None) => return Err(invalid_code()),
            Err(e) => return Err(internal_err("DB error fetching recovery code", e)),
        };

        // Mark as used (AC-6: second use of same code → 401)
        if let Err(e) = sqlx::query("UPDATE mfa_recovery_codes SET used_at = now() WHERE id = $1")
            .bind(rc_id)
            .execute(pool)
            .await
        {
            return Err(internal_err("invalidate recovery code", e));
        }
    } else if let Some(totp_code) = totp_code {
        // AC-4: wrong TOTP → 401; AC-5: three consecutive failures → lockout.
        let Some(enc_bytes) = user.totp_secret_enc.as_deref() else {
            return Err(invalid_code()); // NULL = not enrolled
        };
        if enc_bytes.len() < 12 {
            tracing::error!(
                "signin: totp_secret_enc malformed ({} bytes < 12)",
                enc_bytes.len()
            );
            return Err(invalid_code());
        }

        // Decrypt AES-256-GCM, trying the current key then falling back to the
        // previous key during a rotation window (ADR-018 §5).
        let totp_secret_bytes = match decrypt_with_rotation(
            &state.encryption_key,
            state.encryption_key_previous.as_ref(),
            enc_bytes,
        ) {
            Ok(p) => p,
            Err(RotationDecryptError::Malformed) => {
                tracing::error!("signin: totp_secret_enc malformed (< 12-byte nonce)");
                return Err(invalid_code());
            }
            Err(RotationDecryptError::AuthenticationFailed) => {
                tracing::error!(
                    "signin: AES-GCM decryption of totp_secret_enc failed under all configured keys"
                );
                return Err(StatusCode::INTERNAL_SERVER_ERROR.into_response());
            }
        };

        let totp = match TOTP::new(
            TotpAlgorithm::SHA1,
            6,
            1,
            30,
            totp_secret_bytes,
            None,
            String::new(),
        ) {
            Ok(t) => t,
            Err(e) => return Err(internal_err("build TOTP struct", e)),
        };

        if !totp.check_current(totp_code).unwrap_or(false) {
            let new_count = user.failed_totp_attempts + 1;
            if new_count >= 3 {
                // AC-5: set lockout; 429 fires on the NEXT attempt (this one returns 401).
                let _ = sqlx::query(
                    "UPDATE users SET failed_totp_attempts = $1, \
                     locked_until = now() + interval '15 minutes' \
                     WHERE id = $2",
                )
                .bind(new_count)
                .bind(user_id)
                .execute(pool)
                .await;
            } else {
                let _ = sqlx::query("UPDATE users SET failed_totp_attempts = $1 WHERE id = $2")
                    .bind(new_count)
                    .bind(user_id)
                    .execute(pool)
                    .await;
            }
            return Err(invalid_code());
        }
    } else {
        // Neither totp_code nor recovery_code provided.
        return Err(invalid_code());
    }

    // ── 5. Reset TOTP failure counters on success ─────────────────────────────
    if let Err(e) =
        sqlx::query("UPDATE users SET failed_totp_attempts = 0, locked_until = NULL WHERE id = $1")
            .bind(user_id)
            .execute(pool)
            .await
    {
        return Err(internal_err("reset TOTP failure counters", e));
    }

    Ok(())
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// POST /admin/v1/auth/signin
///
/// Full flow (AC 01-04):
///   1. Fetch user by email (unknown email → 401 "Invalid credentials")
///   2. Lockout check (429 if locked_until > now())
///   3. Argon2id password verify (wrong password → 401 "Invalid credentials")
///   4. TOTP or recovery-code verify
///   5. Reset failed_totp_attempts + locked_until
///   6. Fetch member role
///   7. Generate 32-byte random session token, base64url-encode (no padding)
///   8. Store BLAKE3(token) in sessions.token_hash
///   9. Return 200 + Set-Cookie: embyr_session=<token>; HttpOnly; Secure; SameSite=Strict; Path=/admin; Max-Age=86400
pub async fn signin(
    State(state): State<UserAdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<SigninRequest>,
) -> Response {
    let pool = state.system_db.pool();

    // ── 0. Per-source-IP rate limit (AC-ASH-03/06, ADR-076) ──────────────────
    // First statement — before the Step 1 DB lookup — so a throttled request
    // never reaches Argon2id or the DB. Keyed by peer.ip() only (not the
    // full SocketAddr: ephemeral source port varies per TCP connection from
    // the same client). Behind a load balancer, `source_ip` resolves the real
    // client from X-Forwarded-For when EMBYR_TRUSTED_PROXY_HOPS > 0.
    let source_key = state.signin_rate_limiter.source_ip(peer, &headers).to_string();
    if let Err(retry_after_ms) = state.signin_rate_limiter.check(&source_key).await {
        return signin_throttled(retry_after_ms);
    }

    // ── 1. Fetch user by email ────────────────────────────────────────────────
    // account-deletion: users of a soft-deleted account are unknown emails.
    let row = match sqlx::query(
        "SELECT u.id, u.account_id, u.display_name, u.password_hash, u.totp_secret_enc, \
         u.failed_totp_attempts, u.locked_until \
         FROM users u JOIN accounts a ON a.id = u.account_id \
         WHERE u.email = $1 AND a.deleted_at IS NULL",
    )
    .bind(&body.email)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return invalid_credentials(),
        Err(e) => return internal_err("DB error fetching user", e),
    };

    let account_id: Uuid = try_get_or_err!(row, "account_id", "read account_id");
    let display_name: String = try_get_or_err!(row, "display_name", "read display_name");
    let user = match UserSecrets::from_row(&row) {
        Ok(u) => u,
        Err(e) => return internal_err("read user credentials", e),
    };
    let user_id = user.id;

    // ── 2-5. Lockout, password, second factor, counter reset ─────────────────
    if let Err(response) = verify_credentials(
        &state,
        &user,
        &body.password,
        body.totp_code.as_deref(),
        body.recovery_code.as_deref(),
    )
    .await
    {
        return response;
    }

    // ── 6. Fetch member role ──────────────────────────────────────────────────
    let role_row = match sqlx::query(
        "SELECT role FROM account_members WHERE user_id = $1 AND account_id = $2",
    )
    .bind(user_id)
    .bind(account_id)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => {
            tracing::error!(
                "signin: no account_member row for user={user_id} account={account_id}"
            );
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
        Err(e) => return internal_err("fetch member role", e),
    };
    let role: String = try_get_or_err!(role_row, "role", "read role");

    // ── 7-8. Mint the session (shared with the SSO callback) ──────────────────
    let session = match issue_session(pool, user_id, account_id, SessionOrigin::Password).await {
        Ok(s) => s,
        Err(e) => return internal_err("insert session row", e),
    };

    // ── 9. Return 200 + Set-Cookie ────────────────────────────────────────────
    // AC-1: cookie attributes per spec.
    let cookie = session.set_cookie();
    (
        StatusCode::OK,
        [(header::SET_COOKIE, cookie)],
        axum::Json(SigninResponse {
            account_id: account_id.to_string(),
            display_name,
            role,
        }),
    )
        .into_response()
}

/// POST /admin/v1/auth/signout
///
/// AC-8: returns 204 + Set-Cookie: embyr_session=; Max-Age=0.
/// Idempotent — no cookie or unknown session still returns 204.
pub async fn signout(
    State(state): State<UserAdminState>,
    headers: HeaderMap,
) -> Response {
    let pool = state.system_db.pool();

    if let Some(cookie_value) = extract_cookie(&headers, "embyr_session") {
        let hash_bytes = blake3::hash(cookie_value.as_bytes()).as_bytes().to_vec();
        if let Err(e) = sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
            .bind(&hash_bytes)
            .execute(pool)
            .await
        {
            tracing::error!("signout: DB error deleting session: {e}");
            // Still clear the cookie even on DB error — idempotent by design.
        }
    }

    (
        StatusCode::NO_CONTENT,
        [(
            header::SET_COOKIE,
            "embyr_session=; Max-Age=0; Path=/admin".to_string(),
        )],
    )
        .into_response()
}

// ── Session probe ─────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct SessionInfo {
    account_id: String,
    account_name: String,
    user_id: String,
    email: String,
    display_name: String,
    role: String,
}

/// GET /admin/v1/auth/session
///
/// 401 when the caller has no user identity (a service-account Bearer key has
/// `user_id = nil`): the console is for people.
pub async fn session_info(
    State(state): State<UserAdminState>,
    session: SessionContext,
) -> Result<Json<SessionInfo>, StatusCode> {
    let row = sqlx::query(
        "SELECT u.email, u.display_name, a.name AS account_name, am.role \
         FROM users u \
         JOIN accounts a ON a.id = u.account_id \
         JOIN account_members am ON am.user_id = u.id AND am.account_id = u.account_id \
         WHERE u.id = $1 AND u.account_id = $2",
    )
    .bind(session.user_id)
    .bind(session.account_id)
    .fetch_optional(state.system_db.pool())
    .await
    .map_err(|e| {
        tracing::error!("session_info DB error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?
    .ok_or(StatusCode::UNAUTHORIZED)?;

    let read = |col: &str| {
        row.try_get::<String, _>(col).map_err(|e| {
            tracing::error!("session_info: read {col}: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })
    };
    Ok(Json(SessionInfo {
        account_id: session.account_id.to_string(),
        account_name: read("account_name")?,
        user_id: session.user_id.to_string(),
        email: read("email")?,
        display_name: read("display_name")?,
        role: read("role")?,
    }))
}

#[cfg(test)]
mod tests {
    use super::DUMMY_PASSWORD_HASH;

    /// The dummy must cost what a real verify costs (same Argon2id parameters as production
    /// hashes) and must never verify.
    #[tokio::test]
    async fn dummy_hash_has_production_parameters_and_verifies_nothing() {
        let real = crate::adapters::argon2_blocking::hash_password(b"pw").await.unwrap();
        let params = |h: &str| h.split('$').take(4).collect::<Vec<_>>().join("$");
        assert_eq!(params(DUMMY_PASSWORD_HASH), params(&real));
        for guess in [&b""[..], b"pw", b"password", b"dummy"] {
            assert!(!crate::adapters::argon2_blocking::verify_password(guess, DUMMY_PASSWORD_HASH)
                .await
                .unwrap());
        }
    }
}
