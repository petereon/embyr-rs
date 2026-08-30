//! Google OAuth provider registration handler (feature `oauth-providers`,
//! ADR-037).
//!
//! register_google_oauth_provider (POST
//! /admin/v1/projects/:project_id/oauth_providers/google):
//!   Session auth, Owner/Admin only (US-01, mirrors
//!   `client_identity.rs::register_client_identity_credential`'s and
//!   `hosted_identity.rs::enable_hosted_identity`'s identical role-gate
//!   shape). Body: { client_id } — no `api_key` field (ADR-037 Decision 2's
//!   own positive consequence). Generates a fresh project-scoped Ed25519
//!   signing key server-side (mirrors `enable_hosted_identity`'s own
//!   `SigningKey::generate(&mut OsRng)` call shape exactly), AES-256-GCM
//!   -encrypts the private seed under `EMBYR_ENCRYPTION_KEY` (mirrors
//!   `oidc_providers.rs`/`projects.rs`'s inline encryption pattern), and
//!   registers both the provider-credential UPSERT and the signing-key
//!   idempotent-INSERT in ONE `SystemDb` transaction
//!   (`register_oauth_provider`). 201 on first-time registration / 200 on
//!   idempotent redefine (ADR-037 Decision 4) — the SAME response shape
//!   either way, no signing material of any kind in the response (mirrors
//!   AC-16-01/AC-18-01's "never echo raw material" discipline). 404 on a
//!   non-existent/deleted project or a project owned by a different account
//!   (folded into `verify_project_ownership`, reused unchanged).

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
use serde::{Deserialize, Serialize};

use crate::admin::extractors::session_context::SessionContext;
use crate::admin::handlers::shared::verify_project_ownership;
use crate::admin::state::UserAdminState;
use embyr_core::admin::account::Role;

/// Body for POST .../oauth_providers/google. No `api_key` field — the
/// signing key is encrypted under `EMBYR_ENCRYPTION_KEY`, not ECIES/`api_key`
/// (ADR-037 Decision 2).
#[derive(Deserialize)]
pub struct RegisterGoogleOAuthProviderBody {
    pub client_id: String,
}

/// Response for POST .../oauth_providers/google — 201 or 200, identical
/// shape either way (ADR-037 Decision 4). Deliberately narrow: no
/// `public_key`, no fingerprint, nothing derived from signing material.
#[derive(Serialize)]
pub struct RegisterGoogleOAuthProviderResponse {
    pub project_id: String,
    pub provider: &'static str,
    pub client_id: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// POST /admin/v1/projects/:project_id/oauth_providers/google
///
/// Owner or Admin only (Viewer -> 403, mirrors
/// `client_identity.rs::register_client_identity_credential`/
/// `hosted_identity.rs::enable_hosted_identity`).
pub async fn register_google_oauth_provider(
    Path(project_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
    Json(body): Json<RegisterGoogleOAuthProviderBody>,
) -> Result<Response, StatusCode> {
    if session.role < Role::Admin {
        return Err(StatusCode::FORBIDDEN);
    }

    // AC-19-04: 404 on a non-existent/deleted project, or a project owned by
    // a different account.
    let pool = state.system_db.pool();
    verify_project_ownership(pool, &project_id, session.account_id).await?;

    // Fresh project-scoped Ed25519 signing key, generated server-side —
    // mirrors `enable_hosted_identity`'s exact call shape (ADR-037 Decision 2).
    let signing_key = SigningKey::generate(&mut OsRng);
    let public_key = signing_key.verifying_key().to_bytes();
    let private_key_seed = signing_key.to_bytes();

    // AES-256-GCM encrypt under EMBYR_ENCRYPTION_KEY — mirrors
    // `oidc_providers.rs::create_oidc_provider`'s/`projects.rs`'s exact
    // inline encryption call shape (ADR-037 Decision 2): nonce (12 bytes) ||
    // ciphertext stored in `private_key_enc`.
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

    // AC-19-01/AC-19-02: one transaction — the provider-credential UPSERT
    // and the signing-key idempotent-INSERT succeed or fail together.
    let (row, is_first_registration) = state
        .system_db
        .register_oauth_provider(&project_id, &body.client_id, &public_key, &private_key_enc)
        .await
        .map_err(|e| {
            tracing::error!("register_google_oauth_provider insert error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let status = if is_first_registration {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };

    Ok((
        status,
        Json(RegisterGoogleOAuthProviderResponse {
            project_id,
            provider: "google",
            client_id: row.client_id,
            created_at: row.created_at,
        }),
    )
        .into_response())
}
