//! `accounts:sendOobCode()` / `accounts:resetPassword()` hosted-identity
//! password-reset endpoints (US-04, ADR-036 Decision 6).
//!
//! POST /v1/projects/{project_id}/accounts:sendOobCode?key={api_key}
//!   Body:  { "email": "..." }
//!   200:   { "message": "if this account exists, a reset was sent" }  -- ALWAYS this shape (AC-18-14)
//!
//! POST /v1/projects/{project_id}/accounts:resetPassword?key={api_key}
//!   Body:  { "oobCode": "<reset token>", "newPassword": "..." }
//!   200:   { "email": "..." }
//!   400:   { "reason": "WEAK_PASSWORD" }
//!   401:   { "reason": "RESET_TOKEN_EXPIRED" | "RESET_TOKEN_INVALID" }  -- covers already-used/malformed/unknown
//!
//! Reuses Slice 02/03's `resolve_customer_db_adapter`, `HostedIdentityState`,
//! `invalid_api_key`/`hosted_identity_not_enabled` unchanged.
//!
//! Reset token: 32 random bytes -> base64url (no padding) -> opaque string,
//! mirroring `admin/handlers/auth.rs::signin`'s own session-token shape.
//! Only BLAKE3(raw token) is ever persisted (`hosted_identity_reset_tokens
//! .token_hash`) -- the raw token is never stored, only ever mailed once via
//! `IEmailSender` (V1: `NoopEmailSender`, log-only -- AC-18-18).
//!
//! Single-use + expiry are enforced by ONE atomic
//!   `UPDATE ... SET used_at = now() WHERE token_hash = $1 AND used_at IS
//!   NULL AND expires_at > now() RETURNING email`
//! (migrations/customer/0004_hosted_identity_reset_tokens.sql's own documented
//! mechanism -- race-free, no separate SELECT-then-UPDATE window). On zero
//! rows, a SEPARATE read-only lookup (failure path only, doesn't touch the
//! atomicity of the success path) distinguishes expired (AC-18-16) from
//! already-used/malformed/unknown (one class, RESET_TOKEN_INVALID).

use std::collections::HashMap;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use embyr_core::{admin::email::EmailMessage, auth::argon2, hosted_identity};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};

use crate::adapters::project_auth::{resolve_customer_db_adapter, ProjectAuthError};
use crate::rest::sign_up::{
    hosted_identity_not_enabled, invalid_api_key, HostedIdentityState, SignUpQuery,
};

#[derive(Deserialize)]
pub struct SendOobCodeBody {
    #[serde(default)]
    pub email: Option<String>,
}

#[derive(Serialize)]
struct SendOobCodeResponse {
    message: &'static str,
}

#[derive(Deserialize)]
pub struct ResetPasswordBody {
    #[serde(default, rename = "oobCode")]
    pub oob_code: Option<String>,
    #[serde(default, rename = "newPassword")]
    pub new_password: Option<String>,
}

#[derive(Serialize)]
struct ResetPasswordSuccessResponse {
    email: String,
}

#[derive(Serialize)]
struct ResetFailureResponse {
    reason: &'static str,
}

fn failure(status: StatusCode, reason: &'static str) -> Response {
    (status, Json(ResetFailureResponse { reason })).into_response()
}

fn reset_token_expired() -> Response {
    failure(StatusCode::UNAUTHORIZED, "RESET_TOKEN_EXPIRED")
}

fn reset_token_invalid() -> Response {
    failure(StatusCode::UNAUTHORIZED, "RESET_TOKEN_INVALID")
}

fn weak_password() -> Response {
    failure(StatusCode::BAD_REQUEST, "WEAK_PASSWORD")
}

/// The identical generic response, always -- regardless of whether the
/// requested email is registered (AC-18-14, the core oracle-protection
/// requirement).
fn generic_sent_response() -> Response {
    (
        StatusCode::OK,
        Json(SendOobCodeResponse {
            message: "if this account exists, a reset was sent",
        }),
    )
        .into_response()
}

/// POST /v1/projects/{project_id}/accounts:sendOobCode
///
/// Route pattern shares the same literal-`:`-in-final-segment matchit
/// conflict `sign_up`/`sign_in_with_password` already solved -- dispatched
/// via `accounts_bridge_dispatch` (`lib.rs`), not registered as its own route.
pub async fn send_oob_code(
    Path(params): Path<HashMap<String, String>>,
    State(state): State<HostedIdentityState>,
    Query(query): Query<SignUpQuery>,
    Json(body): Json<SendOobCodeBody>,
) -> Response {
    let project_id = params.get("project_id").cloned().unwrap_or_default();

    let Some(api_key) = query.key.filter(|k| !k.is_empty()) else {
        return invalid_api_key();
    };

    let adapter = match resolve_customer_db_adapter(
        &state.system_db,
        &state.credential_cache,
        state.aws_secret_fetcher.as_deref(),
        state.gcp_secret_fetcher.as_deref(),
        &project_id,
        &api_key,
    )
    .await
    {
        Ok(adapter) => adapter,
        Err(ProjectAuthError::ProjectNotFound | ProjectAuthError::InvalidApiKey) => {
            return invalid_api_key();
        }
        Err(ProjectAuthError::HostedIdentityUnavailable) => {
            return hosted_identity_not_enabled();
        }
        Err(ProjectAuthError::Internal(e)) => {
            tracing::error!("send_oob_code: resolve_customer_db_adapter failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Project-config (is hosted identity enabled at all?) is NOT a secret,
    // unlike account-existence below -- this check IS distinguishable, per
    // this feature's own established convention (mirrors sign_up/sign_in).
    match state
        .system_db
        .get_hosted_identity_signing_key(&project_id)
        .await
    {
        Ok(Some(_)) => {}
        Ok(None) => return hosted_identity_not_enabled(),
        Err(e) => {
            tracing::error!("send_oob_code: get_hosted_identity_signing_key failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }

    let Some(email) = body.email.filter(|e| !e.is_empty()) else {
        return generic_sent_response();
    };

    // AC-18-14: perform the SAME lookup regardless of the eventual outcome,
    // so the registered/unregistered timing stays close -- no early return
    // before this point that would depend on registration status.
    let existing_email: Option<String> = match sqlx::query_scalar(
        "SELECT email FROM hosted_identity_accounts WHERE project_id = $1 AND email = $2",
    )
    .bind(&project_id)
    .bind(&email)
    .fetch_optional(adapter.pool())
    .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("send_oob_code: lookup hosted_identity_accounts failed: {e}");
            None
        }
    };

    if existing_email.is_some() {
        let mut token_bytes = [0u8; 32];
        OsRng.fill_bytes(&mut token_bytes);
        let raw_token = URL_SAFE_NO_PAD.encode(token_bytes);
        let token_hash = blake3::hash(raw_token.as_bytes()).as_bytes().to_vec();

        // 1-hour reset-token lifetime (ADR-036 Decision 2: DESIGN-level
        // default, no existing precedent to mirror -- a conventional,
        // unsurprising password-reset window).
        let insert_result = sqlx::query(
            "INSERT INTO hosted_identity_reset_tokens (project_id, email, token_hash, expires_at) \
             VALUES ($1, $2, $3, now() + interval '1 hour')",
        )
        .bind(&project_id)
        .bind(&email)
        .bind(&token_hash)
        .execute(adapter.pool())
        .await;

        match insert_result {
            Ok(_) => {
                // Fire-and-forget: V1 NoopEmailSender drops the message;
                // errors are ignored (mirrors invite_member, AC-18-18).
                let _ = state
                    .email_sender
                    .send(EmailMessage {
                        to: email.clone(),
                        subject: "Reset your password".to_string(),
                        body_text: format!("{raw_token}\n\nThis code expires in 1 hour."),
                        body_html: None,
                    })
                    .await;
            }
            Err(e) => {
                tracing::error!("send_oob_code: insert hosted_identity_reset_tokens failed: {e}");
            }
        }
    }

    // AC-18-14: identical response no matter what happened above.
    generic_sent_response()
}

/// POST /v1/projects/{project_id}/accounts:resetPassword
pub async fn reset_password(
    Path(params): Path<HashMap<String, String>>,
    State(state): State<HostedIdentityState>,
    Query(query): Query<SignUpQuery>,
    Json(body): Json<ResetPasswordBody>,
) -> Response {
    let project_id = params.get("project_id").cloned().unwrap_or_default();

    let Some(api_key) = query.key.filter(|k| !k.is_empty()) else {
        return invalid_api_key();
    };

    let adapter = match resolve_customer_db_adapter(
        &state.system_db,
        &state.credential_cache,
        state.aws_secret_fetcher.as_deref(),
        state.gcp_secret_fetcher.as_deref(),
        &project_id,
        &api_key,
    )
    .await
    {
        Ok(adapter) => adapter,
        Err(ProjectAuthError::ProjectNotFound | ProjectAuthError::InvalidApiKey) => {
            return invalid_api_key();
        }
        Err(ProjectAuthError::HostedIdentityUnavailable) => {
            return hosted_identity_not_enabled();
        }
        Err(ProjectAuthError::Internal(e)) => {
            tracing::error!("reset_password: resolve_customer_db_adapter failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let (Some(oob_code), Some(new_password)) = (
        body.oob_code.filter(|c| !c.is_empty()),
        body.new_password.filter(|p| !p.is_empty()),
    ) else {
        return reset_token_invalid();
    };

    let token_hash = blake3::hash(oob_code.as_bytes()).as_bytes().to_vec();

    // Single atomic statement: lookup + single-use enforcement + expiry
    // check, race-free (migration's own documented mechanism).
    let consumed_email: Option<String> = match sqlx::query_scalar(
        "UPDATE hosted_identity_reset_tokens SET used_at = now() \
         WHERE project_id = $1 AND token_hash = $2 AND used_at IS NULL AND expires_at > now() \
         RETURNING email",
    )
    .bind(&project_id)
    .bind(&token_hash)
    .fetch_optional(adapter.pool())
    .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::error!("reset_password: consume reset token failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let email = match consumed_email {
        Some(email) => email,
        None => {
            // AC-18-16: separate read-only lookup, failure path only --
            // distinguishes expired (row exists, expires_at <= now()) from
            // already-used/malformed/unknown (one class, RESET_TOKEN_INVALID).
            let is_expired: Option<bool> = match sqlx::query_scalar(
                "SELECT expires_at <= now() FROM hosted_identity_reset_tokens \
                 WHERE project_id = $1 AND token_hash = $2",
            )
            .bind(&project_id)
            .bind(&token_hash)
            .fetch_optional(adapter.pool())
            .await
            {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!("reset_password: expiry lookup failed: {e}");
                    return StatusCode::INTERNAL_SERVER_ERROR.into_response();
                }
            };
            return match is_expired {
                Some(true) => reset_token_expired(),
                Some(false) | None => reset_token_invalid(),
            };
        }
    };

    if hosted_identity::validate_password_strength(&new_password).is_err() {
        return weak_password();
    }

    let password_hash = match argon2::hash_password(new_password.as_bytes()) {
        Ok(h) => h,
        Err(e) => {
            tracing::error!("reset_password: hash_password failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if let Err(e) = sqlx::query(
        "UPDATE hosted_identity_accounts SET password_hash = $1, updated_at = now() \
         WHERE project_id = $2 AND email = $3",
    )
    .bind(&password_hash)
    .bind(&project_id)
    .bind(&email)
    .execute(adapter.pool())
    .await
    {
        tracing::error!("reset_password: update hosted_identity_accounts failed: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    (StatusCode::OK, Json(ResetPasswordSuccessResponse { email })).into_response()
}
