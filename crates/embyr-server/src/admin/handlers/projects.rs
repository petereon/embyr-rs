//! Project handlers for session-auth routes.
//!
//! list_projects (GET /admin/v1/projects):
//!   Session auth, any role. WHERE account_id = $session_account AND status != 'deleted'.
//!
//! patch_project (PATCH /admin/v1/projects/:id):
//!   Session auth, Owner or Admin.
//!   Partial update: name, backend_pg_dsn, logging_enabled, log_retention_days, status.
//!   backend_pg_dsn → AES-256-GCM encrypt before storage. Evict credential cache.
//!   status 'deleted' not patchable (422).

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use crate::admin::extractors::session_context::SessionContext;
use crate::admin::handlers::get_project::GetProjectDetail;
use crate::admin::state::UserAdminState;

/// Minimal project summary returned by GET /admin/v1/projects.
#[derive(Serialize)]
pub struct ProjectSummary {
    pub id: String,
    pub name: Option<String>,
    pub status: String,
    pub backend_mode: String,
    pub logging_enabled: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub account_id: String,
}

/// GET /admin/v1/projects — account-scoped project list.
///
/// Returns all non-deleted projects owned by the authenticated account.
/// Session auth required (any role).
pub async fn list_projects(
    State(state): State<UserAdminState>,
    session: SessionContext,
) -> Result<Json<Vec<ProjectSummary>>, StatusCode> {
    let pool = state.system_db.pool();

    let rows = sqlx::query(
        "SELECT id, name, status, backend_mode, logging_enabled, created_at, account_id \
         FROM projects \
         WHERE account_id = $1 AND status != 'deleted' \
         ORDER BY created_at DESC",
    )
    .bind(session.account_id)
    .fetch_all(pool)
    .await
    .map_err(|e| {
        tracing::error!("list_projects DB error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let projects: Vec<ProjectSummary> = rows
        .into_iter()
        .map(|row| ProjectSummary {
            id: row.try_get("id").unwrap_or_default(),
            name: row.try_get("name").unwrap_or(None),
            status: row.try_get("status").unwrap_or_default(),
            backend_mode: row.try_get("backend_mode").unwrap_or_default(),
            logging_enabled: row.try_get("logging_enabled").unwrap_or(false),
            created_at: row
                .try_get("created_at")
                .unwrap_or_else(|_| chrono::Utc::now()),
            account_id: row
                .try_get::<uuid::Uuid, _>("account_id")
                .map(|u| u.to_string())
                .unwrap_or_default(),
        })
        .collect();

    Ok(Json(projects))
}

/// Partial-update body for PATCH /admin/v1/projects/:id.
/// All fields are optional; absent fields are preserved via COALESCE.
#[derive(Deserialize, Default)]
pub struct PatchProjectBody {
    pub name: Option<String>,
    pub status: Option<String>,
    pub logging_enabled: Option<bool>,
    pub log_retention_days: Option<i32>,
    pub backend_pg_dsn: Option<String>,
}

/// PATCH /admin/v1/projects/:project_id — session-auth partial update.
///
/// Rules enforced:
/// - `status = "deleted"` is rejected with 422 (soft-delete is operator-only).
/// - Account isolation: project must belong to `session.account_id`; otherwise 403.
/// - `backend_pg_dsn` is AES-256-GCM encrypted before being written to
///   `backend_pg_dsn_enc`; the plaintext DSN is never persisted.
/// - Credential cache is evicted after a DSN change.
/// - Absent patch fields are preserved via COALESCE.
pub async fn patch_project(
    Path(project_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
    Json(body): Json<PatchProjectBody>,
) -> Result<Json<GetProjectDetail>, StatusCode> {
    // ── Guard: soft-delete not patchable via PATCH ────────────────────────────
    if body.status.as_deref() == Some("deleted") {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }

    let pool = state.system_db.pool();

    // ── Pre-check: account isolation ──────────────────────────────────────────
    // Fetching account_id before UPDATE allows us to return 403 vs 404 precisely.
    let pre = sqlx::query(
        "SELECT account_id, status FROM projects WHERE id = $1",
    )
    .bind(&project_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| {
        tracing::error!("patch_project pre-check DB error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?
    .ok_or(StatusCode::NOT_FOUND)?;

    let existing_status: String = pre
        .try_get("status")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if existing_status == "deleted" {
        return Err(StatusCode::NOT_FOUND);
    }

    let project_account: Option<Uuid> = pre.try_get("account_id").unwrap_or(None);
    if project_account.as_ref() != Some(&session.account_id) {
        return Err(StatusCode::FORBIDDEN);
    }

    // ── Encrypt DSN if provided ───────────────────────────────────────────────
    // When backend_pg_dsn is absent (None), dsn_enc = None → COALESCE preserves
    // the existing backend_pg_dsn_enc column value (even if it is NULL).
    let dsn_enc: Option<Vec<u8>> = if let Some(ref dsn) = body.backend_pg_dsn {
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let cipher = Aes256Gcm::new_from_slice(&state.encryption_key)
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ct = cipher
            .encrypt(nonce, dsn.as_bytes())
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        let mut enc: Vec<u8> = nonce_bytes.to_vec();
        enc.extend_from_slice(&ct);
        Some(enc)
    } else {
        None
    };

    // ── UPDATE with COALESCE — only touches columns present in the body ───────
    let row = sqlx::query(
        "UPDATE projects \
         SET \
           name                = COALESCE($2, name), \
           status              = COALESCE($3, status), \
           logging_enabled     = COALESCE($4, logging_enabled), \
           log_retention_days  = COALESCE($5, log_retention_days), \
           backend_pg_dsn_enc  = COALESCE($6, backend_pg_dsn_enc), \
           updated_at          = now() \
         WHERE id = $1 \
         RETURNING id, name, status, backend_mode, logging_enabled, \
                   log_retention_days, created_at",
    )
    .bind(&project_id)
    .bind(&body.name)
    .bind(&body.status)
    .bind(body.logging_enabled)
    .bind(body.log_retention_days)
    .bind(&dsn_enc)
    .fetch_one(pool)
    .await
    .map_err(|e| {
        tracing::error!("patch_project UPDATE error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // ── Evict credential cache so next SDK request uses the updated DSN ───────
    if body.backend_pg_dsn.is_some() {
        state.credential_cache.evict_project(&project_id).await;
    }

    // ── Return updated project (same shape as GET /admin/v1/projects/:id) ─────
    Ok(Json(GetProjectDetail {
        id: row.try_get("id").unwrap_or_default(),
        name: row.try_get("name").unwrap_or(None),
        status: row.try_get("status").unwrap_or_default(),
        backend_mode: row.try_get("backend_mode").unwrap_or_default(),
        auth_mode: "key".to_string(),
        logging_enabled: row.try_get("logging_enabled").unwrap_or(false),
        log_retention_days: row.try_get("log_retention_days").unwrap_or(None),
        created_at: row
            .try_get("created_at")
            .unwrap_or_else(|_| chrono::Utc::now()),
    }))
}
