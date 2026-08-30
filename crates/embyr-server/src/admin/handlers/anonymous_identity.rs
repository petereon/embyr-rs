//! Anonymous-identity enablement handler (feature `anonymous-sessions`,
//! ADR-043).
//!
//! enable_anonymous_identity (POST /admin/v1/projects/:project_id/anonymous_identity/enable):
//!   Session auth, Owner/Admin only (US-01, mirrors
//!   `hosted_identity.rs::enable_hosted_identity`'s/
//!   `oauth_providers.rs::register_google_oauth_provider`'s identical role-
//!   gate shape). Body: {} — no fields at all, no `api_key` (Decision 2's
//!   positive consequence, mirrors ADR-037 Decision 2's identical
//!   "no api_key field" outcome), no `backend_mode` check (ADR-044). Generates
//!   a fresh project-scoped Ed25519 signing key server-side, AES-256-GCM
//!   -encrypts the private seed under `EMBYR_ENCRYPTION_KEY` (byte-for-byte
//!   the same sequence `register_google_oauth_provider` already uses),
//!   stores it in `anonymous_signing_keys` via an idempotent UPSERT
//!   (AC-20-02: a second enablement returns the SAME row, no regeneration).
//!   201 { project_id, algorithm, created_at } — no signing material of any
//!   kind in the response (mirrors AC-16-01's "never echo raw material"
//!   discipline). 404 on a non-existent/deleted project or a project owned
//!   by a different account (folded into `verify_project_ownership`, reused
//!   unchanged).

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use ed25519_dalek::SigningKey;
use rand_core::{OsRng, RngCore};
use serde::Serialize;

use crate::admin::extractors::session_context::SessionContext;
use crate::admin::handlers::shared::verify_project_ownership;
use crate::admin::state::UserAdminState;
use embyr_core::admin::account::Role;

/// Response for POST .../anonymous_identity/enable — 201, always (AC-20-01,
/// AC-20-02). Deliberately narrow: no `public_key`, nothing derived from
/// signing material.
#[derive(Serialize)]
pub struct EnableAnonymousIdentityResponse {
    pub project_id: String,
    pub algorithm: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// POST /admin/v1/projects/:project_id/anonymous_identity/enable
///
/// Owner or Admin only (Viewer -> 403, mirrors
/// `hosted_identity.rs::enable_hosted_identity`/
/// `oauth_providers.rs::register_google_oauth_provider`). No `backend_mode`
/// lookup anywhere in this handler (ADR-044: this feature never resolves a
/// Customer DB adapter, so the precondition that gate protects never
/// arises).
pub async fn enable_anonymous_identity(
    Path(project_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
) -> Result<Response, StatusCode> {
    if session.role < Role::Admin {
        return Err(StatusCode::FORBIDDEN);
    }

    // AC-20-04: 404 on a non-existent/deleted project, or a project owned by
    // a different account.
    let pool = state.system_db.pool();
    verify_project_ownership(pool, &project_id, session.account_id).await?;

    // Fresh project-scoped Ed25519 signing key, generated server-side —
    // mirrors `register_google_oauth_provider`'s exact call shape (ADR-043
    // Decision 2).
    let signing_key = SigningKey::generate(&mut OsRng);
    let public_key = signing_key.verifying_key().to_bytes();
    let private_key_seed = signing_key.to_bytes();

    // AES-256-GCM encrypt under EMBYR_ENCRYPTION_KEY — mirrors
    // `register_google_oauth_provider`'s exact inline encryption call shape:
    // nonce (12 bytes) || ciphertext stored in `private_key_enc`.
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let cipher = Aes256Gcm::new_from_slice(&state.encryption_key)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, private_key_seed.as_slice())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut private_key_enc = nonce_bytes.to_vec();
    private_key_enc.extend_from_slice(&ct);

    // AC-20-01/AC-20-02: idempotent UPSERT — a second enablement call
    // returns the SAME row, byte-identical, no regeneration.
    let row = state
        .system_db
        .enable_anonymous_identity(&project_id, &public_key, &private_key_enc)
        .await
        .map_err(|e| {
            tracing::error!("enable_anonymous_identity insert error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok((
        StatusCode::CREATED,
        Json(EnableAnonymousIdentityResponse {
            project_id,
            algorithm: row.algorithm,
            created_at: row.created_at,
        }),
    )
        .into_response())
}
