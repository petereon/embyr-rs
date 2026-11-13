//! Auth handlers: signin, signout, oidc_callback.
//!
//! POST /admin/v1/auth/signin
//!   Validates email + Argon2id password + TOTP code (or recovery code).
//!   Creates session row (BLAKE3 token hash). Sets HttpOnly cookie.
//!   Returns 200 { account_id, display_name, role }.
//!
//! POST /admin/v1/auth/signout
//!   Deletes session row. Sets Max-Age=0 on cookie. Returns 204.
//!
//! GET /admin/v1/auth/oidc/callback
//!   Disabled (audit finding #58) — always 302 /admin/?error=oidc_failed.

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
fn signin_throttled(retry_after_ms: u64) -> Response {
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
fn extract_cookie(headers: &HeaderMap, name: &str) -> Option<String> {
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
    Json(body): Json<SigninRequest>,
) -> Response {
    let pool = state.system_db.pool();

    // ── 0. Per-source-IP rate limit (AC-ASH-03/06, ADR-076) ──────────────────
    // First statement — before the Step 1 DB lookup — so a throttled request
    // never reaches Argon2id or the DB. Keyed by peer.ip() only (not the
    // full SocketAddr: ephemeral source port varies per TCP connection from
    // the same client).
    let source_key = peer.ip().to_string();
    if let Err(retry_after_ms) = state.signin_rate_limiter.check(&source_key).await {
        return signin_throttled(retry_after_ms);
    }

    // ── 1. Fetch user by email ────────────────────────────────────────────────
    let row = match sqlx::query(
        "SELECT id, account_id, display_name, password_hash, totp_secret_enc, \
         failed_totp_attempts, locked_until \
         FROM users WHERE email = $1",
    )
    .bind(&body.email)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return invalid_credentials(),
        Err(e) => return internal_err("DB error fetching user", e),
    };

    let user_id: Uuid = try_get_or_err!(row, "id", "read user id");
    let account_id: Uuid = try_get_or_err!(row, "account_id", "read account_id");
    let display_name: String = try_get_or_err!(row, "display_name", "read display_name");
    let password_hash: String = try_get_or_err!(row, "password_hash", "read password_hash");
    let totp_secret_enc: Option<Vec<u8>> =
        try_get_or_err!(row, "totp_secret_enc", "read totp_secret_enc");
    let failed_totp_attempts: i32 =
        try_get_or_err!(row, "failed_totp_attempts", "read failed_totp_attempts");
    let locked_until: Option<chrono::DateTime<chrono::Utc>> =
        try_get_or_err!(row, "locked_until", "read locked_until");

    // ── 2. Lockout check ──────────────────────────────────────────────────────
    // AC-5: fourth attempt after 3 consecutive TOTP failures returns 429.
    if let Some(until) = locked_until {
        if until > chrono::Utc::now() {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                axum::Json(ErrorBody {
                    message: "Too many failed attempts — account locked for 15 minutes"
                        .to_string(),
                }),
            )
                .into_response();
        }
    }

    // ── 3. Argon2id password verification ────────────────────────────────────
    // AC-3: wrong password → identical 401 shape to unknown email (oracle protection).
    // AC-ASH-01: runs off the async reactor's worker threads via spawn_blocking,
    // per ADR-003's mandate — mirrors provision.rs:187 / sdk_keys.rs:162's own
    // established shape, adapted to signin's own match-and-early-return idiom
    // (signin returns Response directly, not Result<_, StatusCode>). Reuses
    // embyr_core::auth::argon2::verify_password (ADR-036 Decision 8) instead of
    // signin's own prior duplicate inline Argon2::new(...) call.
    let password_owned = body.password.clone();
    let hash_owned = password_hash.clone();
    let verify_result = tokio::task::spawn_blocking(move || {
        embyr_core::auth::argon2::verify_password(password_owned.as_bytes(), &hash_owned)
    })
    .await;

    let password_matches = match verify_result {
        Ok(Ok(matches)) => matches,
        Ok(Err(e)) => return internal_err("verify password hash", e),
        Err(e) => return internal_err("spawn_blocking join error (password verify)", e),
    };

    if !password_matches {
        return invalid_credentials();
    }

    // ── 4. TOTP / recovery-code check ────────────────────────────────────────
    if let Some(recovery_code) = &body.recovery_code {
        // AC-6: valid one-time recovery code grants access; immediately invalidated.
        let code_hash = blake3::hash(recovery_code.as_bytes()).as_bytes().to_vec();
        let rc_row = match sqlx::query(
            "SELECT id FROM mfa_recovery_codes \
             WHERE user_id = $1 AND code_hash = $2 AND used_at IS NULL \
             LIMIT 1",
        )
        .bind(user_id)
        .bind(&code_hash)
        .fetch_optional(pool)
        .await
        {
            Ok(Some(r)) => r,
            Ok(None) => return invalid_code(),
            Err(e) => return internal_err("DB error fetching recovery code", e),
        };

        let rc_id: Uuid = try_get_or_err!(rc_row, "id", "read recovery code id");

        // Mark as used (AC-6: second use of same code → 401)
        if let Err(e) = sqlx::query(
            "UPDATE mfa_recovery_codes SET used_at = now() WHERE id = $1",
        )
        .bind(rc_id)
        .execute(pool)
        .await
        {
            return internal_err("invalidate recovery code", e);
        }
    } else if let Some(totp_code) = &body.totp_code {
        // AC-4: wrong TOTP → 401; AC-5: three consecutive failures → lockout.
        let enc_bytes = match totp_secret_enc {
            Some(b) => b,
            None => return invalid_code(), // NULL = not enrolled
        };
        if enc_bytes.len() < 12 {
            tracing::error!(
                "signin: totp_secret_enc malformed ({} bytes < 12)",
                enc_bytes.len()
            );
            return invalid_code();
        }

        // Decrypt AES-256-GCM, trying the current key then falling back to the
        // previous key during a rotation window (ADR-018 §5).
        let totp_secret_bytes = match decrypt_with_rotation(
            &state.encryption_key,
            state.encryption_key_previous.as_ref(),
            &enc_bytes,
        ) {
            Ok(p) => p,
            Err(RotationDecryptError::Malformed) => {
                tracing::error!(
                    "signin: totp_secret_enc malformed (< 12-byte nonce)"
                );
                return invalid_code();
            }
            Err(RotationDecryptError::AuthenticationFailed) => {
                tracing::error!(
                    "signin: AES-GCM decryption of totp_secret_enc failed under all configured keys"
                );
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
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
            Err(e) => return internal_err("build TOTP struct", e),
        };

        let is_valid = totp.check_current(totp_code).unwrap_or(false);
        if !is_valid {
            let new_count = failed_totp_attempts + 1;
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
                let _ = sqlx::query(
                    "UPDATE users SET failed_totp_attempts = $1 WHERE id = $2",
                )
                .bind(new_count)
                .bind(user_id)
                .execute(pool)
                .await;
            }
            return invalid_code();
        }
    } else {
        // Neither totp_code nor recovery_code provided.
        return invalid_code();
    }

    // ── 5. Reset TOTP failure counters on success ─────────────────────────────
    if let Err(e) = sqlx::query(
        "UPDATE users SET failed_totp_attempts = 0, locked_until = NULL WHERE id = $1",
    )
    .bind(user_id)
    .execute(pool)
    .await
    {
        return internal_err("reset TOTP failure counters", e);
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

    // ── 7. Generate session token ─────────────────────────────────────────────
    // 32 random bytes → base64url (no padding) → opaque token string.
    let mut token_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut token_bytes);
    let token = URL_SAFE_NO_PAD.encode(token_bytes);

    // ── 8. Store BLAKE3(token) in sessions ───────────────────────────────────
    // AC-7: raw plaintext token is NEVER written to the DB.
    let token_hash = blake3::hash(token.as_bytes()).as_bytes().to_vec();
    if let Err(e) = sqlx::query(
        "INSERT INTO sessions (user_id, account_id, token_hash, expires_at) \
         VALUES ($1, $2, $3, now() + interval '24 hours')",
    )
    .bind(user_id)
    .bind(account_id)
    .bind(&token_hash)
    .execute(pool)
    .await
    {
        return internal_err("insert session row", e);
    }

    // ── 9. Return 200 + Set-Cookie ────────────────────────────────────────────
    // AC-1: cookie attributes per spec.
    let cookie = build_session_cookie(&token);
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

// ── OIDC callback ─────────────────────────────────────────────────────────────

fn oidc_failed_redirect() -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::LOCATION,
        HeaderValue::from_static("/admin/?error=oidc_failed"),
    );
    (StatusCode::FOUND, headers).into_response()
}

/// GET /admin/v1/auth/oidc/callback
///
/// DISABLED (audit finding #58). The previous implementation validated the
/// id_token but then ignored its identity and minted a session for an
/// arbitrary member of the provider's account. Until a real
/// authorization-code + PKCE flow with identity binding ships (future feature
/// `admin-oidc-sso`), every request is refused with the same
/// `/admin/?error=oidc_failed` redirect the UI already handles. No session is
/// ever created. Query parameters (which may carry an id_token) are
/// deliberately not read or logged.
pub async fn oidc_callback() -> Response {
    tracing::warn!("oidc_callback: OIDC sign-in is disabled (audit finding #58); refusing");
    oidc_failed_redirect()
}
