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

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use argon2::{Algorithm as Argon2Algorithm, Argon2, Params, PasswordHash, PasswordVerifier, Version};
use axum::{
    extract::{Json, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use totp_rs::{Algorithm as TotpAlgorithm, TOTP};
use uuid::Uuid;

use crate::admin::state::UserAdminState;

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

// ── Cookie extraction (mirrors session_auth_middleware helper) ─────────────────

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

    let user_id: Uuid = match row.try_get("id") {
        Ok(v) => v,
        Err(e) => return internal_err("read user id", e),
    };
    let account_id: Uuid = match row.try_get("account_id") {
        Ok(v) => v,
        Err(e) => return internal_err("read account_id", e),
    };
    let display_name: String = match row.try_get("display_name") {
        Ok(v) => v,
        Err(e) => return internal_err("read display_name", e),
    };
    let password_hash: String = match row.try_get("password_hash") {
        Ok(v) => v,
        Err(e) => return internal_err("read password_hash", e),
    };
    let totp_secret_enc: Option<Vec<u8>> = match row.try_get("totp_secret_enc") {
        Ok(v) => v,
        Err(e) => return internal_err("read totp_secret_enc", e),
    };
    let failed_totp_attempts: i32 = match row.try_get("failed_totp_attempts") {
        Ok(v) => v,
        Err(e) => return internal_err("read failed_totp_attempts", e),
    };
    let locked_until: Option<chrono::DateTime<chrono::Utc>> =
        match row.try_get("locked_until") {
            Ok(v) => v,
            Err(e) => return internal_err("read locked_until", e),
        };

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

        let rc_id: Uuid = match rc_row.try_get("id") {
            Ok(v) => v,
            Err(e) => return internal_err("read recovery code id", e),
        };

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

        // Decrypt AES-256-GCM: first 12 bytes = nonce, remainder = ciphertext + tag.
        let cipher = match Aes256Gcm::new_from_slice(&state.encryption_key) {
            Ok(c) => c,
            Err(e) => return internal_err("build AES-256-GCM cipher", e),
        };
        let nonce = Nonce::from_slice(&enc_bytes[..12]);
        let totp_secret_bytes = match cipher.decrypt(nonce, &enc_bytes[12..]) {
            Ok(p) => p,
            Err(_) => {
                tracing::error!("signin: AES-GCM decryption of totp_secret_enc failed");
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
    let role: String = match role_row.try_get("role") {
        Ok(v) => v,
        Err(e) => return internal_err("read role", e),
    };

    // ── 7. Generate session token ─────────────────────────────────────────────
    // 32 random bytes → base64url (no padding) → opaque token string.
    let mut buf = [0u8; 32];
    OsRng.fill_bytes(&mut buf);
    let token = URL_SAFE_NO_PAD.encode(buf);

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
    let cookie = format!(
        "embyr_session={token}; HttpOnly; Secure; SameSite=Strict; Path=/admin; Max-Age=86400"
    );
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

/// GET /admin/v1/auth/oidc/callback
///
/// # RED scaffold — implemented in a later step.
pub async fn oidc_callback() {
    panic!("Not yet implemented -- RED scaffold: oidc_callback handler")
}
