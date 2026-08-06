use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Serialize;
use sqlx::Row;
use uuid::Uuid;

use crate::admin::extractors::dual_auth_principal::AuthPrincipal;
use crate::admin::state::UserAdminState;

/// Expanded project detail returned by GET /admin/v1/projects/:id (B-02, step 02-02).
#[derive(Serialize)]
pub struct GetProjectDetail {
    pub id: String,
    pub name: Option<String>,
    pub status: String,
    pub backend_mode: String,
    pub auth_mode: String,
    pub logging_enabled: bool,
    pub log_retention_days: Option<i32>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// GET /admin/v1/projects/:project_id
///
/// Dual-auth route (ADR-009, B-02):
///   - Session user → account-scoped: 403 if project belongs to a different account.
///   - Operator → unscoped: any project is accessible.
pub async fn get_project(
    Path(project_id): Path<String>,
    State(state): State<UserAdminState>,
    principal: AuthPrincipal,
) -> Result<Json<GetProjectDetail>, StatusCode> {
    let pool = state.system_db.pool();

    let row = sqlx::query(
        "SELECT id, name, status, backend_mode, logging_enabled, log_retention_days, \
         created_at, account_id \
         FROM projects WHERE id = $1",
    )
    .bind(&project_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| {
        tracing::error!("get_project: DB error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?
    .ok_or(StatusCode::NOT_FOUND)?;

    let status: String = row
        .try_get("status")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if status == "deleted" {
        return Err(StatusCode::NOT_FOUND);
    }

    // Account isolation for session users — operator has no scoping restriction.
    if let AuthPrincipal::User { account_id, .. } = &principal {
        let project_account: Option<Uuid> = row.try_get("account_id").unwrap_or(None);
        if project_account.as_ref() != Some(account_id) {
            return Err(StatusCode::FORBIDDEN);
        }
    }

    Ok(Json(GetProjectDetail {
        id: row.try_get("id").unwrap_or_default(),
        name: row.try_get("name").unwrap_or(None),
        status,
        backend_mode: row.try_get("backend_mode").unwrap_or_default(),
        auth_mode: "key".to_string(),
        logging_enabled: row.try_get("logging_enabled").unwrap_or(false),
        log_retention_days: row.try_get("log_retention_days").unwrap_or(None),
        created_at: row
            .try_get("created_at")
            .unwrap_or_else(|_| chrono::Utc::now()),
    }))
}
