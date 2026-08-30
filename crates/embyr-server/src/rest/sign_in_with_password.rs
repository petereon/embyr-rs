//! `accounts:signInWithPassword()` hosted-identity signin endpoint (US-03,
//! ADR-036 Decision 6).
//!
//! POST /v1/projects/{project_id}/accounts:signInWithPassword?key={api_key}
//!   Body:  { "email": "...", "password": "..." }
//!   200:   { "localId": "<end_user_id>", "email": "...", "idToken": "<token>", "expiresIn": "<seconds>" }
//!   400:   { "reason": "HOSTED_IDENTITY_NOT_ENABLED" }
//!   401:   { "reason": "INVALID_API_KEY" }              -- api_key itself invalid
//!   401:   { "message": "Invalid credentials" }         -- oracle-protected: wrong
//!                                                           password OR unknown email,
//!                                                           byte-identical response
//!                                                           shape (mirrors
//!                                                           admin/handlers/auth.rs's
//!                                                           own invalid_credentials())
//!
//! Reuses Slice 02's `resolve_customer_db_adapter` (Customer DB resolution),
//! `HostedIdentityState`, `invalid_api_key`/`hosted_identity_not_enabled`
//! (same `{"reason": ...}` shape, same route family), and
//! `mint_client_identity_token` (same token-minting call shape as
//! `sign_up.rs`) unchanged.
//!
//! The oracle-protected rejection deliberately uses a DIFFERENT response
//! shape (`{"message": "Invalid credentials"}`) than the rest of this
//! feature's own `{"reason": "..."}` convention — a direct, deliberate copy
//! of `admin/handlers/auth.rs::invalid_credentials()`'s own already-accepted
//! convention, not a drift to "fix".

use std::collections::HashMap;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use embyr_core::{
    auth::{argon2, ecies},
    client_identity,
};
use serde::{Deserialize, Serialize};

use crate::adapters::project_auth::{resolve_customer_db_adapter, ProjectAuthError};
use crate::rest::sign_up::{
    hosted_identity_not_enabled, invalid_api_key, HostedIdentityState, SignUpQuery,
    SignUpSuccessResponse, TOKEN_TTL_SECS,
};

#[derive(Deserialize)]
pub struct SignInWithPasswordBody {
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
}

#[derive(Serialize)]
struct InvalidCredentialsResponse {
    message: &'static str,
}

/// Mirrors `admin/handlers/auth.rs::invalid_credentials()` exactly: same
/// status code, same body shape, same message text. Covers unknown email,
/// wrong password, AND missing email/password fields — all indistinguishable
/// from each other on the wire (AC-18-11's own oracle-protection guarantee).
fn invalid_credentials() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(InvalidCredentialsResponse {
            message: "Invalid credentials",
        }),
    )
        .into_response()
}

/// POST /v1/projects/{project_id}/accounts:signInWithPassword
///
/// Route pattern shares the same literal-`:`-in-final-segment matchit
/// conflict `sign_up`/`sign_in_with_custom_token` already solved — dispatched
/// via `accounts_bridge_dispatch` (`lib.rs`), not registered as its own
/// route.
pub async fn sign_in_with_password(
    Path(params): Path<HashMap<String, String>>,
    State(state): State<HostedIdentityState>,
    Query(query): Query<SignUpQuery>,
    Json(body): Json<SignInWithPasswordBody>,
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
            tracing::error!("sign_in_with_password: resolve_customer_db_adapter failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // AC-18-12: hosted identity must be enabled for this project —
    // distinguishable from the oracle-protected credential rejection below.
    let hosted_key_row = match state
        .system_db
        .get_hosted_identity_signing_key(&project_id)
        .await
    {
        Ok(Some(row)) => row,
        Ok(None) => return hosted_identity_not_enabled(),
        Err(e) => {
            tracing::error!("sign_in_with_password: get_hosted_identity_signing_key failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // AC-18-11: missing credentials fold into the SAME oracle-protected
    // rejection as an unknown email / wrong password — no separate
    // MISSING_FIELD signal that would let a caller distinguish "no email
    // sent" from "email sent, account doesn't exist".
    let (Some(email), Some(password)) = (
        body.email.filter(|e| !e.is_empty()),
        body.password.filter(|p| !p.is_empty()),
    ) else {
        return invalid_credentials();
    };

    // AC-18-11: unknown email -> identical rejection to wrong password below.
    let row = match sqlx::query_as::<_, (uuid::Uuid, String)>(
        "SELECT end_user_id, password_hash FROM hosted_identity_accounts \
         WHERE project_id = $1 AND email = $2",
    )
    .bind(&project_id)
    .bind(&email)
    .fetch_optional(adapter.pool())
    .await
    {
        Ok(Some(r)) => r,
        Ok(None) => return invalid_credentials(),
        Err(e) => {
            tracing::error!("sign_in_with_password: lookup hosted_identity_accounts failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let (end_user_id, password_hash) = row;

    // AC-18-11: wrong password -> identical rejection to unknown email above.
    match argon2::verify_password(password.as_bytes(), &password_hash) {
        Ok(true) => {}
        Ok(false) => return invalid_credentials(),
        Err(e) => {
            tracing::error!("sign_in_with_password: verify_password failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }

    // Decrypt the project's embyr-owned signing key seed and mint a token —
    // identical call shape to `sign_up.rs`.
    let seed_bytes = match ecies::decrypt(api_key.as_bytes(), &hosted_key_row.private_key_enc) {
        Ok(b) => b,
        Err(e) => {
            tracing::error!("sign_in_with_password: ecies decrypt signing key failed: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let seed: [u8; 32] = match seed_bytes.as_slice().try_into() {
        Ok(s) => s,
        Err(_) => {
            tracing::error!("sign_in_with_password: decrypted signing key seed is not 32 bytes");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let end_user_id_str = end_user_id.to_string();
    let expires_at_unix = chrono::Utc::now().timestamp() + TOKEN_TTL_SECS;
    let token = client_identity::mint_client_identity_token(
        &seed,
        &end_user_id_str,
        &project_id,
        expires_at_unix,
    );

    (
        StatusCode::OK,
        Json(SignUpSuccessResponse {
            local_id: end_user_id_str,
            email,
            id_token: token,
            expires_in: TOKEN_TTL_SECS.to_string(),
        }),
    )
        .into_response()
}
