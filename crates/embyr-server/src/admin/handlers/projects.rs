// SCAFFOLD: true
//! Project handlers for session-auth routes.
//!
//! list_projects (GET /admin/v1/projects):
//!   Session auth, any role. WHERE account_id = $session_account AND status != 'deleted'.
//!
//! patch_project (PATCH /admin/v1/projects/:id):
//!   Session auth, Owner or Admin.
//!   Partial update: name, backend_mode, backend_pg_dsn, logging_enabled, log_retention_days, status.
//!   backend_pg_dsn → ECIES-encrypt before storage. Evict credential cache.
//!   status 'deleted' not patchable (422).

use axum::{extract::State, http::StatusCode, Json};
use serde::Serialize;
use sqlx::Row;

use crate::admin::extractors::session_context::SessionContext;
use crate::admin::state::UserAdminState;

/// Minimal project summary returned by GET /admin/v1/projects.
#[derive(Serialize)]
pub struct ProjectSummary {
    pub id: String,
    pub status: String,
    pub backend_mode: String,
    pub logging_enabled: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
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
        "SELECT id, status, backend_mode, logging_enabled, created_at \
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
            status: row.try_get("status").unwrap_or_default(),
            backend_mode: row.try_get("backend_mode").unwrap_or_default(),
            logging_enabled: row.try_get("logging_enabled").unwrap_or(false),
            created_at: row
                .try_get("created_at")
                .unwrap_or_else(|_| chrono::Utc::now()),
        })
        .collect();

    Ok(Json(projects))
}

/// PATCH /admin/v1/projects/:id — partial update.
///
/// # RED scaffold
pub async fn patch_project() {
    panic!("Not yet implemented -- RED scaffold: patch_project handler")
}
