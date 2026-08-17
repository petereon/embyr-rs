//! Client-identity verification credential handlers (feature `client-auth`,
//! ADR-024/025/026).
//!
//! register_client_identity_credential (POST /admin/v1/projects/:project_id/client_identity_credential):
//!   Session auth, Owner/Admin only (US-01). Registers a project's Ed25519
//!   verification credential. 201 { project_id, algorithm, fingerprint,
//!   created_at } — the raw public key is NEVER echoed back (AC-16-01).
//!   409 (unique-violation-mapped) if a credential already exists (AC-16-04),
//!   directs the caller to the rotate action. 400 on malformed key material
//!   (AC-16-02). 404 on a non-existent/deleted project (AC-16-05). Implemented.
//!
//! rotate_client_identity_credential (POST .../client_identity_credential/rotate):
//!   Session auth, Owner/Admin only (US-03). Shifts current -> previous,
//!   activates the new current key (ADR-025 dual-generation window).
//!   200 { project_id, algorithm, created_at, rotated_at } on success. 404
//!   if no credential is registered yet. Implemented (step 03-01).
//!
//! verify_client_identity_credential (POST .../client_identity_credential/verify):
//!   Session auth, any role (US-04, debug-only, read-only by construction —
//!   AC-16-14). Calls the IDENTICAL `embyr_core::client_identity::
//!   verify_client_identity_token()` used by real sign-in (ADR-025 §
//!   Debug/verify check) — never creates a live session.
//!   SCAFFOLD — implemented in step 04-01.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{Deserialize, Serialize};

use crate::admin::extractors::session_context::SessionContext;
use crate::admin::handlers::shared::verify_project_ownership;
use crate::admin::state::UserAdminState;
use embyr_core::admin::account::Role;
use embyr_core::client_identity::credential_fingerprint;
use embyr_core::error::CoreError;

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

/// Body for POST .../client_identity_credential and .../rotate.
/// `public_key` is base64url (no padding) of the raw 32-byte Ed25519 public
/// key — matches this codebase's existing base64url-no-pad convention
/// (`sdk_keys.rs`'s key-suffix encoding).
#[derive(Deserialize)]
pub struct RegisterClientIdentityCredentialBody {
    pub public_key: String,
}

/// Response for POST .../client_identity_credential — 201. Never includes
/// the raw key (AC-16-01, ADR-025 § Registration).
#[derive(Serialize)]
pub struct ClientIdentityCredentialResponse {
    pub project_id: String,
    pub algorithm: String,
    pub fingerprint: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Response for POST .../client_identity_credential/rotate — 200.
#[derive(Serialize)]
pub struct RotateClientIdentityCredentialResponse {
    pub project_id: String,
    pub algorithm: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub rotated_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Body for POST .../client_identity_credential/verify (US-04, debug-only).
#[derive(Deserialize)]
pub struct VerifyClientIdentityTokenBody {
    pub token: String,
}

/// Success response — resolved identity + expiry, no live session created
/// (AC-16-14).
#[derive(Serialize)]
pub struct VerifyClientIdentitySuccessResponse {
    pub end_user_id: String,
    pub expires_at: i64,
}

/// Failure response — IDENTICAL rejection-reason taxonomy to real sign-in
/// (AC-16-15, ADR-026 § Sign-in action contract).
#[derive(Serialize)]
pub struct VerifyClientIdentityFailureResponse {
    pub reason: &'static str,
}

fn rejection_reason(err: embyr_core::client_identity::ClientIdentityVerifyError) -> &'static str {
    use embyr_core::client_identity::ClientIdentityVerifyError as E;
    match err {
        E::MissingToken => "MISSING_TOKEN",
        E::Malformed => "MALFORMED_TOKEN",
        E::Expired => "TOKEN_EXPIRED",
        E::ProjectMismatch => "PROJECT_MISMATCH",
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// POST /admin/v1/projects/:project_id/client_identity_credential
///
/// Owner or Admin only (Viewer -> 403, mirrors `sdk_keys.rs::create_sdk_key`).
pub async fn register_client_identity_credential(
    Path(project_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
    Json(body): Json<RegisterClientIdentityCredentialBody>,
) -> Result<Response, StatusCode> {
    // AC-16-03: Owner or Admin only.
    if session.role < Role::Admin {
        return Err(StatusCode::FORBIDDEN);
    }

    let pool = state.system_db.pool();

    // AC-16-05: 404 on a non-existent/deleted project, or a project owned by
    // a different account (reuses the identical query shape as
    // `sdk_keys.rs::verify_project_ownership`, promoted to `shared.rs`).
    verify_project_ownership(pool, &project_id, session.account_id).await?;

    // AC-16-02: malformed verification material named specifically (byte
    // count found vs. the required 32), not a raw parse error.
    let raw = URL_SAFE_NO_PAD
        .decode(body.public_key.as_bytes())
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    if raw.len() != 32 {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!(
                    "public_key must be exactly 32 bytes, found {} byte(s)",
                    raw.len()
                )
            })),
        )
            .into_response());
    }
    let public_key: [u8; 32] = raw
        .try_into()
        .expect("length already verified to be exactly 32");

    match state
        .system_db
        .insert_client_identity_credential(&project_id, &public_key)
        .await
    {
        Ok(()) => Ok((
            StatusCode::CREATED,
            Json(ClientIdentityCredentialResponse {
                project_id,
                algorithm: "EdDSA".to_string(),
                fingerprint: credential_fingerprint(&public_key),
                created_at: chrono::Utc::now(),
            }),
        )
            .into_response()),
        // AC-16-04: a credential already registered for this project is
        // rejected (409), directed to the rotation action — not silently
        // overwritten.
        Err(CoreError::AlreadyExists(_)) => Ok((
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": "a verification credential is already registered for this project",
                "action": "use the rotate action instead of registering again",
            })),
        )
            .into_response()),
        Err(e) => {
            tracing::error!("register_client_identity_credential insert error: {e}");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// POST /admin/v1/projects/:project_id/client_identity_credential/rotate
///
/// Owner or Admin only (US-03, AC-16-13).
pub async fn rotate_client_identity_credential(
    Path(project_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
    Json(body): Json<RegisterClientIdentityCredentialBody>,
) -> Result<Response, StatusCode> {
    if session.role < Role::Admin {
        return Err(StatusCode::FORBIDDEN);
    }

    let pool = state.system_db.pool();
    verify_project_ownership(pool, &project_id, session.account_id).await?;

    // AC-16-02 (same shape as registration's validation): malformed
    // verification material named specifically (byte count found vs. the
    // required 32), not a raw parse error.
    let raw = URL_SAFE_NO_PAD
        .decode(body.public_key.as_bytes())
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    if raw.len() != 32 {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "error": format!(
                    "public_key must be exactly 32 bytes, found {} byte(s)",
                    raw.len()
                )
            })),
        )
            .into_response());
    }
    let public_key: [u8; 32] = raw
        .try_into()
        .expect("length already verified to be exactly 32");

    match state
        .system_db
        .rotate_client_identity_credential(&project_id, &public_key)
        .await
    {
        Ok(Some(row)) => Ok((
            StatusCode::OK,
            Json(RotateClientIdentityCredentialResponse {
                project_id,
                algorithm: row.algorithm,
                created_at: row.created_at,
                rotated_at: row.rotated_at,
            }),
        )
            .into_response()),
        // No credential registered for this project yet — nothing to
        // rotate (verify_project_ownership already confirmed the project
        // itself exists, so this is specifically "no credential", not "no
        // project").
        Ok(None) => Ok((
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({
                "error": "no verification credential is registered for this project",
                "action": "use the register action first",
            })),
        )
            .into_response()),
        Err(e) => {
            tracing::error!("rotate_client_identity_credential update error: {e}");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

/// POST /admin/v1/projects/:project_id/client_identity_credential/verify
///
/// Any role (US-04, debug-only). Read-only by construction — loads the
/// credential, calls the identical `verify_client_identity_token()` used by
/// real sign-in, returns the result. Never writes session/credential state
/// (AC-16-14).
pub async fn verify_client_identity_credential(
    Path(project_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
    Json(body): Json<VerifyClientIdentityTokenBody>,
) -> Result<
    (
        StatusCode,
        Json<serde_json::Value>,
    ),
    StatusCode,
> {
    let pool = state.system_db.pool();
    verify_project_ownership(pool, &project_id, session.account_id).await?;

    let _ = (body, rejection_reason as fn(_) -> _);
    panic!(
        "verify_client_identity_credential: RED scaffold (DISTILL, client-auth) — \
         not yet implemented. See ADR-025 § Debug/verify check (SystemDb::get_client_identity_credential \
         + embyr_core::client_identity::verify_client_identity_token — the IDENTICAL routine real \
         sign-in uses, AC-16-15; success -> {{end_user_id, expires_at}}, no live session; failure -> \
         {{reason}} matching AC-16-07's taxonomy exactly)."
    )
}
