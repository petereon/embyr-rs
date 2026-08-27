//! Hosted-identity enablement handler (feature `client-auth-hosted-identity`,
//! ADR-036).
//!
//! enable_hosted_identity (POST /admin/v1/projects/:project_id/hosted_identity/enable):
//!   Session auth, Owner/Admin only (US-01, mirrors `client_identity.rs`'s
//!   `register_client_identity_credential` exact role gate). Body:
//!   { api_key }. Generates a fresh project-scoped Ed25519 signing key
//!   server-side, ECIES-encrypts the private seed under a pubkey derived
//!   from the project's own `api_key` (Argon2id-verified against
//!   `api_key_hash_current` first — ADR-036 Decision 5 "Gap found and
//!   closed during pre-DELIVER review"), stores it in
//!   `hosted_identity_signing_keys` via an idempotent UPSERT (AC-18-02: a
//!   second enablement returns the SAME row, no regeneration). 201
//!   { project_id, algorithm, created_at } — no signing material of any
//!   kind in the response (mirrors AC-16-01's "never echo raw material"
//!   discipline). 404 on a non-existent/deleted project (or a project owned
//!   by a different account — folded into the same query, ADR-036 Decision
//!   5). 401 { reason: "INVALID_API_KEY" } on a wrong/stale api_key. 403
//!   { reason: "HOSTED_IDENTITY_UNAVAILABLE_FOR_BACKEND_MODE" } for
//!   `backend_mode=agent` projects.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use serde::{Deserialize, Serialize};

use crate::admin::extractors::session_context::SessionContext;
use crate::admin::state::UserAdminState;
use embyr_core::admin::account::Role;
use embyr_core::auth::{argon2, ecies};

/// Body for POST .../hosted_identity/enable. `api_key` is the project's own
/// current, raw API key (Alex already possesses it) — required to derive
/// the ECIES recipient pubkey at enablement time (ADR-036 Decision 5).
#[derive(Deserialize)]
pub struct EnableHostedIdentityBody {
    pub api_key: String,
}

/// Response for POST .../hosted_identity/enable — 201. Deliberately narrow:
/// no `public_key`, no fingerprint, nothing derived from signing material
/// (stricter than `client_identity.rs`'s own AC-16-01 discipline, per this
/// slice's own contract).
#[derive(Serialize)]
pub struct EnableHostedIdentityResponse {
    pub project_id: String,
    pub algorithm: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Serialize)]
struct ErrorReason {
    reason: &'static str,
}

/// POST /admin/v1/projects/:project_id/hosted_identity/enable
///
/// Owner or Admin only (Viewer -> 403, mirrors
/// `client_identity.rs::register_client_identity_credential`).
pub async fn enable_hosted_identity(
    Path(project_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
    Json(body): Json<EnableHostedIdentityBody>,
) -> Result<Response, StatusCode> {
    if session.role < Role::Admin {
        return Err(StatusCode::FORBIDDEN);
    }

    // AC-18-04: 404 on a non-existent/deleted project, or a project owned by
    // a different account (folded into one query, ADR-036 Decision 5).
    // AC-18-06: `backend_mode=agent` projects are refused enablement
    // outright — a hard rejection at the enablement action itself.
    let backend_row = state
        .system_db
        .get_project_backend_mode(&project_id, session.account_id)
        .await
        .map_err(|e| {
            tracing::error!("enable_hosted_identity backend_mode lookup error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    if backend_row.backend_mode == "agent" {
        return Ok((
            StatusCode::FORBIDDEN,
            Json(ErrorReason {
                reason: "HOSTED_IDENTITY_UNAVAILABLE_FOR_BACKEND_MODE",
            }),
        )
            .into_response());
    }

    // AC-18-05: the raw api_key is never on this session-authenticated
    // request by default — Alex supplies it in the body, Argon2id-verified
    // against `api_key_hash_current` (the identical primitive
    // `resolve_customer_db_adapter` uses at every data-plane call site)
    // before it is trusted to derive the ECIES recipient pubkey.
    let auth_row = state
        .system_db
        .get_project_for_auth(&project_id)
        .await
        .map_err(|e| {
            tracing::error!("enable_hosted_identity project_for_auth lookup error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    let verified = argon2::verify_api_key(body.api_key.as_bytes(), &auth_row.api_key_hash_current)
        .map_err(|e| {
            tracing::error!("enable_hosted_identity argon2 verify error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    if !verified {
        return Ok((
            StatusCode::UNAUTHORIZED,
            Json(ErrorReason {
                reason: "INVALID_API_KEY",
            }),
        )
            .into_response());
    }

    // Fresh project-scoped Ed25519 signing key, generated server-side —
    // never customer-submitted (mirrors `sdk_keys.rs::create_sdk_key`'s
    // own inline-`OsRng`-generation shape, not `ecies.rs`'s core-crate
    // detour — no other call site in this codebase needs this key shape).
    let signing_key = SigningKey::generate(&mut OsRng);
    let public_key = signing_key.verifying_key().to_bytes();
    let private_key_seed = signing_key.to_bytes();

    let recipient_pubkey = ecies::derive_public_key(body.api_key.as_bytes());
    let private_key_enc = ecies::encrypt(&recipient_pubkey, &private_key_seed).map_err(|e| {
        tracing::error!("enable_hosted_identity ecies encrypt error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // AC-18-01/AC-18-02: idempotent UPSERT — a second enablement call
    // returns the SAME row, byte-identical, no regeneration.
    let row = state
        .system_db
        .enable_hosted_identity(&project_id, &public_key, &private_key_enc)
        .await
        .map_err(|e| {
            tracing::error!("enable_hosted_identity insert error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok((
        StatusCode::CREATED,
        Json(EnableHostedIdentityResponse {
            project_id,
            algorithm: row.algorithm,
            created_at: row.created_at,
        }),
    )
        .into_response())
}
