//! Admin password reset (admin-password-reset). Both endpoints are public.
//!
//! request  POST /admin/v1/auth/password_reset/request   {email}
//! confirm  POST /admin/v1/auth/password_reset/confirm   {token, new_password, totp_code | recovery_code}
//!
//! Email alone never resets a password: confirm also needs a valid second
//! factor, checked by the same helpers (and lockout policy) as signin. Only the
//! BLAKE3 hash of the emailed token is stored; one live token per user.

use axum::{
    extract::{ConnectInfo, Json, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use std::net::SocketAddr;
use uuid::Uuid;

use embyr_core::admin::invitation::{
    generate_token, hash_token, is_well_formed_token, password_acceptable,
};
use embyr_core::admin::password_reset::{normalize_email, reset_email};

use super::auth::{check_lockout, verify_second_factor, UserSecrets};
use super::invitations::{internal, message, throttle, too_many_attempts};
use crate::admin::state::UserAdminState;

/// Unknown / expired / used / malformed tokens are indistinguishable.
fn invalid_reset() -> Response {
    message(StatusCode::NOT_FOUND, "Invalid or expired reset link")
}

// ── POST /admin/v1/auth/password_reset/request ────────────────────────────────

#[derive(Deserialize)]
pub struct RequestBody {
    pub email: String,
}

const ACCEPTED_TEXT: &str = "If that address belongs to an account, a reset link has been sent";

fn accepted() -> Response {
    message(StatusCode::ACCEPTED, ACCEPTED_TEXT)
}

/// Always 202 with the same body, known email or not. Throttled per source IP
/// and per normalised email *before* any lookup, so a 429 is existence-blind.
/// The email is sent from a spawned task so SMTP latency cannot leak existence.
pub async fn request_password_reset(
    State(state): State<UserAdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<RequestBody>,
) -> Response {
    let ip = state.signin_rate_limiter.source_ip(peer, &headers);
    if let Err(retry_after_ms) = state.reset_ip_limiter.check(&format!("rp:{ip}")).await {
        return too_many_attempts(retry_after_ms);
    }
    let Some(email) = normalize_email(&body.email) else {
        return accepted();
    };
    if let Err(retry_after_ms) = state.reset_email_limiter.check(&format!("rpe:{email}")).await {
        return too_many_attempts(retry_after_ms);
    }

    // Same lookup and token generation/hashing for known and unknown emails. An SSO-only user
    // (NULL password_hash, ADR-086) has no password to reset: treated as an unknown address.
    let user_id: Option<Uuid> = match sqlx::query_scalar(
        "SELECT u.id FROM users u JOIN accounts a ON a.id = u.account_id \
         WHERE lower(u.email) = $1 AND a.deleted_at IS NULL \
           AND u.password_hash IS NOT NULL LIMIT 1",
    )
    .bind(&email)
    .fetch_optional(state.system_db.pool())
    .await
    {
        Ok(id) => id,
        Err(e) => return internal("password reset: lookup user", e),
    };
    let token = generate_token();
    let token_hash = hash_token(&token);
    let Some(user_id) = user_id else {
        return accepted();
    };

    // From here on failures are logged but answered with the same 202.
    if let Err(e) = sqlx::query(
        "INSERT INTO password_reset_tokens (user_id, token_hash, expires_at) \
         VALUES ($1, $2, now() + interval '1 hour') \
         ON CONFLICT (user_id) DO UPDATE SET token_hash = EXCLUDED.token_hash, \
           expires_at = EXCLUDED.expires_at, created_at = now()",
    )
    .bind(user_id)
    .bind(&token_hash)
    .execute(state.system_db.pool())
    .await
    {
        tracing::error!("password reset: store token: {e}");
        return accepted();
    }
    tracing::info!(%user_id, "admin password reset requested");

    let sender = state.email_sender.clone();
    let email_message = reset_email(&email, state.stripe_gateway.public_base_url(), &token);
    tokio::spawn(async move {
        if let Err(e) = sender.send(email_message).await {
            tracing::warn!("password reset: email delivery failed: {e}");
        }
    });
    accepted()
}

// ── POST /admin/v1/auth/password_reset/confirm ────────────────────────────────

// No `Debug` derive: the request carries the token, password and codes.
#[derive(Deserialize)]
pub struct ConfirmBody {
    pub token: String,
    pub new_password: String,
    pub totp_code: Option<String>,
    pub recovery_code: Option<String>,
}

/// Valid unexpired token AND valid second factor (wrong factor → 401, token
/// kept, attempt counted by the signin lockout policy). On success, in one
/// transaction: consume the token, set the new password, revoke every session.
pub async fn confirm_password_reset(
    State(state): State<UserAdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<ConfirmBody>,
) -> Response {
    if let Some(throttled) = throttle(&state, peer, &headers).await {
        return throttled;
    }
    if !password_acceptable(&body.new_password) {
        return StatusCode::UNPROCESSABLE_ENTITY.into_response();
    }
    if !is_well_formed_token(&body.token) {
        return invalid_reset();
    }
    match confirm_inner(&state, &body).await {
        Ok(r) | Err(r) => r,
    }
}

async fn confirm_inner(state: &UserAdminState, body: &ConfirmBody) -> Result<Response, Response> {
    let pool = state.system_db.pool();
    let token_hash = hash_token(&body.token);

    let user_id: Uuid = sqlx::query_scalar(
        "SELECT t.user_id FROM password_reset_tokens t \
         JOIN users u ON u.id = t.user_id JOIN accounts a ON a.id = u.account_id \
         WHERE t.token_hash = $1 AND t.expires_at > now() AND a.deleted_at IS NULL \
           AND u.password_hash IS NOT NULL",
    )
    .bind(&token_hash)
    .fetch_optional(pool)
    .await
    .map_err(|e| internal("password reset: lookup token", e))?
    .ok_or_else(invalid_reset)?;
    let user = UserSecrets::fetch_by_id(pool, user_id)
        .await
        .map_err(|e| internal("password reset: load user", e))?
        .ok_or_else(invalid_reset)?;

    check_lockout(&user)?;
    verify_second_factor(
        state,
        &user,
        body.totp_code.as_deref(),
        body.recovery_code.as_deref(),
    )
    .await?;

    let password_hash = crate::adapters::argon2_blocking::hash_password(body.new_password.as_bytes())
        .await
        .map_err(|e| internal("password reset: hash password", e))?;

    let mut tx = pool
        .begin()
        .await
        .map_err(|e| internal("password reset: begin", e))?;
    // Consuming the token is the serialisation point: a concurrent confirm
    // that loses the race sees no row and answers like any invalid token.
    sqlx::query_scalar::<_, Uuid>(
        "DELETE FROM password_reset_tokens \
         WHERE token_hash = $1 AND expires_at > now() RETURNING user_id",
    )
    .bind(&token_hash)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| internal("password reset: consume token", e))?
    .ok_or_else(invalid_reset)?;
    sqlx::query("UPDATE users SET password_hash = $1 WHERE id = $2")
        .bind(&password_hash)
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| internal("password reset: set password", e))?;
    sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| internal("password reset: revoke sessions", e))?;
    tx.commit()
        .await
        .map_err(|e| internal("password reset: commit", e))?;

    tracing::info!(%user_id, "admin password reset completed; all sessions revoked");
    Ok(StatusCode::NO_CONTENT.into_response())
}
