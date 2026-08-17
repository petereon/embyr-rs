//! SDK key handlers.
//!
//! list_sdk_keys (GET /admin/v1/projects/:project_id/sdk_keys[?active=true]):
//!   Session auth, any role. Returns all SDK keys for the project (optionally active-only).
//!   Never returns the `key` field — only id, name, prefix, created_at, revoked_at.
//!
//! create_sdk_key (POST /admin/v1/projects/:project_id/sdk_keys):
//!   Session auth, Owner or Admin only (Viewer → 403).
//!   Calls new_sdk_key_material on spawn_blocking thread (Argon2id is CPU-intensive).
//!   Stores BLAKE3 hash as key_hash BYTEA. Evicts credential cache after insert.
//!   201 { id, name, key, prefix, created_at }. key shown ONCE only.
//!
//! revoke_sdk_key (DELETE /admin/v1/projects/:project_id/sdk_keys/:key_id):
//!   Session auth, Owner or Admin only (Viewer → 403).
//!   Sets revoked_at = now(). Evicts credential cache. 204.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::collections::HashMap;
use uuid::Uuid;

use crate::admin::extractors::session_context::SessionContext;
use crate::admin::handlers::shared::verify_project_ownership;
use crate::admin::state::UserAdminState;
use embyr_core::admin::account::Role;

// ---------------------------------------------------------------------------
// Response / request types
// ---------------------------------------------------------------------------

/// SDK key summary returned by GET .../sdk_keys — `key` field intentionally absent.
#[derive(Serialize)]
pub struct SdkKeySummary {
    pub id: String,
    pub name: String,
    pub prefix: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Body for POST .../sdk_keys.
#[derive(Deserialize)]
pub struct CreateSdkKeyBody {
    pub name: String,
}

/// Response body for POST .../sdk_keys — `key` shown ONCE only, never returned by GET.
#[derive(Serialize)]
pub struct CreateSdkKeyResponse {
    pub id: String,
    pub name: String,
    /// Full plaintext key: "embyr_sdk_" + 32-char base64url-no-pad suffix.
    /// Shown once at creation; only BLAKE3 hash is persisted.
    pub key: String,
    pub prefix: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET /admin/v1/projects/:project_id/sdk_keys[?active=true]
///
/// Session auth, any role. Account-scoped: only returns keys for projects
/// owned by the authenticated account. `?active=true` excludes revoked keys.
pub async fn list_sdk_keys(
    Path(project_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Vec<SdkKeySummary>>, StatusCode> {
    let pool = state.system_db.pool();

    verify_project_ownership(pool, &project_id, session.account_id).await?;

    let active_only = params.get("active").map(|v| v == "true").unwrap_or(false);

    let sql = if active_only {
        "SELECT id::text AS id, name, prefix, created_at, revoked_at \
         FROM sdk_api_keys \
         WHERE project_id = $1 AND revoked_at IS NULL \
         ORDER BY created_at DESC"
    } else {
        "SELECT id::text AS id, name, prefix, created_at, revoked_at \
         FROM sdk_api_keys \
         WHERE project_id = $1 \
         ORDER BY created_at DESC"
    };

    let rows = sqlx::query(sql)
        .bind(&project_id)
        .fetch_all(pool)
        .await
        .map_err(|e| {
            tracing::error!("list_sdk_keys DB error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let keys: Vec<SdkKeySummary> = rows
        .into_iter()
        .map(|row| SdkKeySummary {
            id: row.try_get("id").unwrap_or_default(),
            name: row.try_get("name").unwrap_or_default(),
            prefix: row.try_get("prefix").unwrap_or_default(),
            created_at: row
                .try_get("created_at")
                .unwrap_or_else(|_| chrono::Utc::now()),
            revoked_at: row.try_get("revoked_at").unwrap_or(None),
        })
        .collect();

    Ok(Json(keys))
}

/// POST /admin/v1/projects/:project_id/sdk_keys
///
/// Owner or Admin only (Viewer → 403). Generates a new SDK key, hashes it with
/// BLAKE3, runs new_sdk_key_material on a blocking thread, inserts the row, then
/// evicts the credential cache. Returns 201 with the plaintext key (shown once only).
pub async fn create_sdk_key(
    Path(project_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
    Json(body): Json<CreateSdkKeyBody>,
) -> Result<(StatusCode, Json<CreateSdkKeyResponse>), StatusCode> {
    // AC-B03-05: Owner or Admin only.
    if session.role < Role::Admin {
        return Err(StatusCode::FORBIDDEN);
    }

    // AC-B03-07: name must be 1–64 chars.
    if body.name.is_empty() || body.name.len() > 64 {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }

    let pool = state.system_db.pool();

    verify_project_ownership(pool, &project_id, session.account_id).await?;

    // Generate 24 random bytes → URL_SAFE_NO_PAD → exactly 32 chars.
    let mut raw_bytes = [0u8; 24];
    OsRng.fill_bytes(&mut raw_bytes);
    let suffix = URL_SAFE_NO_PAD.encode(raw_bytes);
    debug_assert_eq!(suffix.len(), 32, "base64url-no-pad of 24 bytes must be 32 chars");

    let key = format!("embyr_sdk_{suffix}");
    let prefix = suffix[..8].to_string();

    // AC-B03-02: Argon2id (inside new_sdk_key_material) on a blocking thread.
    // BLAKE3 hash from the returned material is stored as key_hash BYTEA.
    let key_bytes = key.as_bytes().to_vec();
    let material =
        tokio::task::spawn_blocking(move || embyr_core::admin::sdk_key::new_sdk_key_material(&key_bytes))
            .await
            .map_err(|e| {
                tracing::error!("create_sdk_key spawn_blocking join error: {e}");
                StatusCode::INTERNAL_SERVER_ERROR
            })?
            .map_err(|e| {
                tracing::error!("create_sdk_key new_sdk_key_material error: {e}");
                StatusCode::INTERNAL_SERVER_ERROR
            })?;

    let key_hash: Vec<u8> = material.blake3_hash.to_vec();

    // AC-B03-03: atomic insert (single statement; no partial state).
    let row = sqlx::query(
        "INSERT INTO sdk_api_keys (project_id, name, key_hash, prefix) \
         VALUES ($1, $2, $3, $4) \
         RETURNING id::text AS id, created_at",
    )
    .bind(&project_id)
    .bind(&body.name)
    .bind(&key_hash as &[u8])
    .bind(&prefix)
    .fetch_one(pool)
    .await
    .map_err(|e| {
        tracing::error!("create_sdk_key INSERT error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let id: String = row
        .try_get("id")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let created_at: chrono::DateTime<chrono::Utc> = row
        .try_get("created_at")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Evict credential cache so the new key is immediately usable.
    state.credential_cache.evict_project(&project_id).await;

    Ok((
        StatusCode::CREATED,
        Json(CreateSdkKeyResponse {
            id,
            name: body.name,
            key,
            prefix,
            created_at,
        }),
    ))
}

/// DELETE /admin/v1/projects/:project_id/sdk_keys/:key_id
///
/// Owner or Admin only (Viewer → 403). Sets revoked_at = now(). Returns 204.
/// Returns 404 if the key does not exist, belongs to a different project, or is
/// already revoked.
pub async fn revoke_sdk_key(
    Path((project_id, key_id)): Path<(String, String)>,
    State(state): State<UserAdminState>,
    session: SessionContext,
) -> Result<StatusCode, StatusCode> {
    // AC-B03-05: Owner or Admin only.
    if session.role < Role::Admin {
        return Err(StatusCode::FORBIDDEN);
    }

    let pool = state.system_db.pool();

    verify_project_ownership(pool, &project_id, session.account_id).await?;

    // Parse key_id; invalid UUID → 404 (same surface as "not found").
    let key_uuid = Uuid::parse_str(&key_id).map_err(|_| StatusCode::NOT_FOUND)?;

    let result = sqlx::query(
        "UPDATE sdk_api_keys \
         SET revoked_at = now() \
         WHERE id = $1 AND project_id = $2 AND revoked_at IS NULL",
    )
    .bind(key_uuid)
    .bind(&project_id)
    .execute(pool)
    .await
    .map_err(|e| {
        tracing::error!("revoke_sdk_key UPDATE error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    if result.rows_affected() == 0 {
        return Err(StatusCode::NOT_FOUND);
    }

    // Evict credential cache so revoked key stops authenticating immediately.
    state.credential_cache.evict_project(&project_id).await;

    Ok(StatusCode::NO_CONTENT)
}
