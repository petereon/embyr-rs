//! OIDC provider handlers.
//!
//! list_oidc_providers (GET /admin/v1/oidc_providers):
//!   Session auth, Owner only. client_secret_enc never returned.
//!
//! create_oidc_provider (POST /admin/v1/oidc_providers):
//!   Session auth, Owner only.
//!   client_secret → AES-256-GCM under EMBYR_ENCRYPTION_KEY before storage.
//!   201 { id, issuer, client_id, enabled: true }. client_secret absent.
//!
//! patch_oidc_provider (PATCH /admin/v1/oidc_providers/:provider_id):
//!   Session auth, Owner only. Partial update. Disabling does not invalidate sessions.
//!
//! delete_oidc_provider (DELETE /admin/v1/oidc_providers/:provider_id):
//!   Session auth, Owner only. 204. Does not invalidate sessions.

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use axum::{
    extract::{Json, Path, State},
    http::StatusCode,
};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use embyr_core::admin::account::Role;

use crate::admin::extractors::session_context::SessionContext;
use crate::admin::state::UserAdminState;

// ── Response type ─────────────────────────────────────────────────────────────

/// Shape returned by GET and POST /admin/v1/oidc_providers.
/// client_secret and client_secret_enc are intentionally absent.
#[derive(Serialize)]
pub struct OidcProviderResponse {
    pub id: String,
    pub issuer: String,
    pub client_id: String,
    pub enabled: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

// ── Request types ─────────────────────────────────────────────────────────────

/// Body for POST /admin/v1/oidc_providers.
#[derive(Deserialize)]
pub struct CreateOidcProviderBody {
    pub issuer: String,
    pub client_id: String,
    /// Plaintext OIDC client secret — AES-256-GCM encrypted before storage; never persisted.
    pub client_secret: String,
}

/// Body for PATCH /admin/v1/oidc_providers/:provider_id.
#[derive(Deserialize)]
pub struct PatchOidcProviderBody {
    pub enabled: Option<bool>,
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// GET /admin/v1/oidc_providers
///
/// Owner-only. Returns all OIDC providers for the authenticated account.
/// client_secret_enc is never included in the response.
pub async fn list_oidc_providers(
    State(state): State<UserAdminState>,
    session: SessionContext,
) -> Result<Json<Vec<OidcProviderResponse>>, StatusCode> {
    if session.role != Role::Owner {
        return Err(StatusCode::FORBIDDEN);
    }

    let pool = state.system_db.pool();

    let rows = sqlx::query(
        "SELECT id::text AS id, issuer, client_id, enabled, created_at \
         FROM oidc_providers \
         WHERE account_id = $1 \
         ORDER BY created_at ASC",
    )
    .bind(session.account_id)
    .fetch_all(pool)
    .await
    .map_err(|e| {
        tracing::error!("list_oidc_providers: DB error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let providers: Vec<OidcProviderResponse> = rows
        .into_iter()
        .map(|row| OidcProviderResponse {
            id: row.try_get("id").unwrap_or_default(),
            issuer: row.try_get("issuer").unwrap_or_default(),
            client_id: row.try_get("client_id").unwrap_or_default(),
            enabled: row.try_get("enabled").unwrap_or(true),
            created_at: row
                .try_get("created_at")
                .unwrap_or_else(|_| chrono::Utc::now()),
        })
        .collect();

    Ok(Json(providers))
}

/// POST /admin/v1/oidc_providers
///
/// Owner-only. Encrypts client_secret with AES-256-GCM before storage.
/// Returns 201 + OidcProviderResponse (client_secret absent).
/// Duplicate (account_id, issuer) → 409 Conflict.
pub async fn create_oidc_provider(
    State(state): State<UserAdminState>,
    session: SessionContext,
    Json(body): Json<CreateOidcProviderBody>,
) -> Result<(StatusCode, Json<OidcProviderResponse>), StatusCode> {
    if session.role != Role::Owner {
        return Err(StatusCode::FORBIDDEN);
    }

    // ── Encrypt client_secret with AES-256-GCM ────────────────────────────────
    // nonce (12 bytes) || ciphertext stored in client_secret_enc.
    // Plaintext client_secret is never written to the database.
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let cipher = Aes256Gcm::new_from_slice(&state.encryption_key)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, body.client_secret.as_bytes())
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let mut secret_enc = nonce_bytes.to_vec();
    secret_enc.extend_from_slice(&ct);

    let pool = state.system_db.pool();

    let row = sqlx::query(
        "INSERT INTO oidc_providers (account_id, issuer, client_id, client_secret_enc) \
         VALUES ($1, $2, $3, $4) \
         RETURNING id::text AS id, issuer, client_id, enabled, created_at",
    )
    .bind(session.account_id)
    .bind(&body.issuer)
    .bind(&body.client_id)
    .bind(&secret_enc as &[u8])
    .fetch_one(pool)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(ref db_err) = e {
            // PostgreSQL unique_violation = "23505"
            if db_err.code().as_deref() == Some("23505") {
                return StatusCode::CONFLICT;
            }
        }
        tracing::error!("create_oidc_provider: INSERT error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let response = OidcProviderResponse {
        id: row
            .try_get("id")
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        issuer: row
            .try_get("issuer")
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        client_id: row
            .try_get("client_id")
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        enabled: row
            .try_get("enabled")
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        created_at: row
            .try_get("created_at")
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    };

    Ok((StatusCode::CREATED, Json(response)))
}

/// PATCH /admin/v1/oidc_providers/:provider_id
///
/// Owner-only. Partial update: `enabled` field via COALESCE.
/// Does not invalidate existing user sessions on disable.
/// Returns 200 + updated OidcProviderResponse. 404 if provider not found.
pub async fn patch_oidc_provider(
    Path(provider_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
    Json(body): Json<PatchOidcProviderBody>,
) -> Result<Json<OidcProviderResponse>, StatusCode> {
    if session.role != Role::Owner {
        return Err(StatusCode::FORBIDDEN);
    }

    let provider_uuid = Uuid::parse_str(&provider_id).map_err(|_| StatusCode::NOT_FOUND)?;

    let pool = state.system_db.pool();

    let row = sqlx::query(
        "UPDATE oidc_providers \
         SET enabled = COALESCE($3, enabled) \
         WHERE id = $1 AND account_id = $2 \
         RETURNING id::text AS id, issuer, client_id, enabled, created_at",
    )
    .bind(provider_uuid)
    .bind(session.account_id)
    .bind(body.enabled)
    .fetch_optional(pool)
    .await
    .map_err(|e| {
        tracing::error!("patch_oidc_provider: UPDATE error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?
    .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(OidcProviderResponse {
        id: row.try_get("id").unwrap_or_default(),
        issuer: row.try_get("issuer").unwrap_or_default(),
        client_id: row.try_get("client_id").unwrap_or_default(),
        enabled: row.try_get("enabled").unwrap_or(true),
        created_at: row
            .try_get("created_at")
            .unwrap_or_else(|_| chrono::Utc::now()),
    }))
}

/// DELETE /admin/v1/oidc_providers/:provider_id
///
/// Owner-only. Returns 204 on success. 404 if provider not found or belongs to a
/// different account. Does not invalidate existing user sessions.
pub async fn delete_oidc_provider(
    Path(provider_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
) -> Result<StatusCode, StatusCode> {
    if session.role != Role::Owner {
        return Err(StatusCode::FORBIDDEN);
    }

    let provider_uuid = Uuid::parse_str(&provider_id).map_err(|_| StatusCode::NOT_FOUND)?;

    let pool = state.system_db.pool();

    let result = sqlx::query(
        "DELETE FROM oidc_providers WHERE id = $1 AND account_id = $2",
    )
    .bind(provider_uuid)
    .bind(session.account_id)
    .execute(pool)
    .await
    .map_err(|e| {
        tracing::error!("delete_oidc_provider: DELETE error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    if result.rows_affected() == 0 {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(StatusCode::NO_CONTENT)
}
