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
//!   (RED scaffold — implemented in a later step)

use argon2::{Algorithm as Argon2Algorithm, Argon2, Params, PasswordHash, PasswordVerifier, Version};
use axum::{
    extract::{Json, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sqlx::Row;
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
/// boilerplate used throughout `signin` and `oidc_callback`.
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
    Json(body): Json<SigninRequest>,
) -> Response {
    let pool = state.system_db.pool();

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
    let parsed_hash = match PasswordHash::new(&password_hash) {
        Ok(h) => h,
        Err(e) => return internal_err("parse password hash", e),
    };
    let argon2 = Argon2::new(
        Argon2Algorithm::Argon2id,
        Version::V0x13,
        Params::new(65536, 3, 4, None).expect("valid Argon2id params"),
    );
    if argon2
        .verify_password(body.password.as_bytes(), &parsed_hash)
        .is_err()
    {
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

// ── OIDC callback types ───────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct OidcCallbackQuery {
    pub id_token: Option<String>,
    pub state: Option<String>,
    pub code: Option<String>,
    pub error: Option<String>,
}

// ── OIDC callback helpers ─────────────────────────────────────────────────────

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
/// Validates OIDC id_token (issuer, audience, expiry, JWKS RS256 signature).
/// On success: creates session row, sets embyr_session cookie, redirects to /admin/.
///
/// AC-B06-05:
///   - valid id_token → 302 /admin/ + embyr_session cookie
///   - invalid JWKS signature → 302 /admin/?error=oidc_failed (or 401)
///   - unknown issuer → 401
///   - missing state → 401
pub async fn oidc_callback(
    State(state): State<UserAdminState>,
    Query(query): Query<OidcCallbackQuery>,
) -> Response {
    use jsonwebtoken::{decode, decode_header, jwk::JwkSet, Algorithm, DecodingKey, Validation};

    let pool = state.system_db.pool();

    // Step 1: IdP-level error (e.g. user denied consent).
    if query.error.is_some() {
        return oidc_failed_redirect();
    }

    // Step 2: id_token must be present.
    let id_token = match query.id_token {
        Some(t) => t,
        None => return StatusCode::UNAUTHORIZED.into_response(),
    };

    // Step 3: CSRF state must be present (V1: presence-only check).
    match query.state.as_deref() {
        None => return StatusCode::UNAUTHORIZED.into_response(),
        Some("") => {
            tracing::warn!("oidc_callback: empty state parameter — CSRF nonce check skipped (V1)");
        }
        Some(_) => {}
    }

    // Step 4: Decode JWT payload without signature to extract claims.
    let parts: Vec<&str> = id_token.split('.').collect();
    if parts.len() != 3 {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let payload_bytes = match URL_SAFE_NO_PAD.decode(parts[1]) {
        Ok(b) => b,
        Err(_) => return StatusCode::UNAUTHORIZED.into_response(),
    };
    let claims: serde_json::Value = match serde_json::from_slice(&payload_bytes) {
        Ok(v) => v,
        Err(_) => return StatusCode::UNAUTHORIZED.into_response(),
    };

    // Step 5: Require `iss` claim.
    let issuer = match claims.get("iss").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => return StatusCode::UNAUTHORIZED.into_response(),
    };

    // Step 6: Look up matching enabled OIDC provider by issuer.
    let provider_row = match sqlx::query(
        "SELECT account_id, client_id FROM oidc_providers \
         WHERE issuer = $1 AND enabled = true LIMIT 1",
    )
    .bind(&issuer)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return StatusCode::UNAUTHORIZED.into_response(),
        Err(e) => return internal_err("oidc_callback: DB lookup provider", e),
    };

    let account_id: Uuid =
        try_get_or_err!(provider_row, "account_id", "oidc_callback: read account_id");
    let client_id: String =
        try_get_or_err!(provider_row, "client_id", "oidc_callback: read client_id");

    // Step 8: Verify token not expired.
    let exp = claims.get("exp").and_then(|v| v.as_i64()).unwrap_or(0);
    if exp > 0 && exp < chrono::Utc::now().timestamp() {
        return oidc_failed_redirect();
    }

    // Step 9: Verify audience matches provider client_id.
    let aud_matches = match claims.get("aud") {
        Some(serde_json::Value::String(s)) => s.as_str() == client_id.as_str(),
        Some(serde_json::Value::Array(arr)) => {
            arr.iter().any(|v| v.as_str() == Some(client_id.as_str()))
        }
        _ => false,
    };
    if !aud_matches {
        return oidc_failed_redirect();
    }

    // Step 10: Parse JWT header for key ID.
    let jwt_header = match decode_header(&id_token) {
        Ok(h) => h,
        Err(_) => return StatusCode::UNAUTHORIZED.into_response(),
    };
    let kid = jwt_header.kid.unwrap_or_default();

    // Step 11: Fetch JWKS from issuer discovery endpoint.
    let jwks_url = format!("{}/.well-known/jwks.json", issuer);
    let jwks: JwkSet = match reqwest::get(&jwks_url).await {
        Ok(resp) => match resp.json::<JwkSet>().await {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!("oidc_callback: JWKS JSON parse error: {e}");
                return oidc_failed_redirect();
            }
        },
        Err(e) => {
            tracing::warn!("oidc_callback: JWKS fetch error: {e}");
            return oidc_failed_redirect();
        }
    };

    // Step 12: Find matching JWK by kid (fall back to first key).
    let jwk = if !kid.is_empty() {
        jwks.keys
            .iter()
            .find(|k| k.common.key_id.as_deref() == Some(kid.as_str()))
            .or_else(|| jwks.keys.first())
    } else {
        jwks.keys.first()
    };
    let jwk = match jwk {
        Some(k) => k,
        None => {
            tracing::warn!("oidc_callback: no matching JWK found for kid={kid:?}");
            return oidc_failed_redirect();
        }
    };

    // Step 13: Verify RS256 signature via jsonwebtoken.
    let decoding_key = match DecodingKey::from_jwk(jwk) {
        Ok(k) => k,
        Err(e) => {
            tracing::warn!("oidc_callback: DecodingKey::from_jwk failed: {e}");
            return oidc_failed_redirect();
        }
    };
    let mut validation = Validation::new(Algorithm::RS256);
    validation.validate_exp = false; // validated manually above
    validation.required_spec_claims = std::collections::HashSet::new();
    validation.set_audience(&[client_id.as_str()]);
    if decode::<serde_json::Value>(&id_token, &decoding_key, &validation).is_err() {
        return oidc_failed_redirect();
    }

    // Step 14: CSRF state accepted (V1: presence-only; full nonce verification deferred).

    // Step 15: Resolve a user in the account to create a session for.
    // Look up the account owner; fall back to any member.
    let user_row = match sqlx::query(
        "SELECT user_id FROM account_members \
         WHERE account_id = $1 AND role = 'owner' LIMIT 1",
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => {
            match sqlx::query(
                "SELECT user_id FROM account_members WHERE account_id = $1 LIMIT 1",
            )
            .bind(account_id)
            .fetch_optional(pool)
            .await
            {
                Ok(Some(r)) => r,
                Ok(None) => return StatusCode::UNAUTHORIZED.into_response(),
                Err(e) => return internal_err("oidc_callback: no account members", e),
            }
        }
        Err(e) => return internal_err("oidc_callback: DB lookup user", e),
    };

    let user_id: Uuid = try_get_or_err!(user_row, "user_id", "oidc_callback: read user_id");

    // Generate 32-byte random session token (same pattern as signin).
    let mut token_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut token_bytes);
    let token = URL_SAFE_NO_PAD.encode(token_bytes);
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
        return internal_err("oidc_callback: insert session", e);
    }

    // Build 302 redirect with embyr_session cookie (same attributes as signin).
    let cookie = build_session_cookie(&token);
    let cookie_value = match HeaderValue::from_str(&cookie) {
        Ok(v) => v,
        Err(e) => return internal_err("oidc_callback: build cookie header", e),
    };

    let mut resp_headers = HeaderMap::new();
    resp_headers.insert(header::LOCATION, HeaderValue::from_static("/admin/"));
    resp_headers.insert(header::SET_COOKIE, cookie_value);
    (StatusCode::FOUND, resp_headers).into_response()
}
