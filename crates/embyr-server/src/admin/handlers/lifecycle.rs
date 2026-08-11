use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
};

use crate::adapters::credential_cache::CredentialCache;
use crate::adapters::system_db::SystemDb;
use crate::admin::state::OperatorState;

/// Dependencies for lifecycle-status transitions invoked outside the HTTP
/// router — used by `CapUsageRefresher` (ADR-020) to call
/// `set_project_status`-shaped suspension logic from the background cap-check
/// task, which has no `OperatorState` (no HTTP request in flight).
pub struct LifecycleDeps {
    pub system_db: Arc<SystemDb>,
    pub credential_cache: Arc<CredentialCache>,
}

/// Apply a lifecycle status transition to a project and evict the credential cache.
///
/// Updates `status` and `updated_at` where the current status is in
/// `('active', 'suspended')`. Returns `NOT_FOUND` when the project does not
/// exist or has already been deleted; `INTERNAL_SERVER_ERROR` on DB failure.
async fn set_project_status(
    project_id: &str,
    new_status: &str,
    state: &OperatorState,
) -> StatusCode {
    let result = sqlx::query(
        "UPDATE projects SET status = $2, updated_at = now() \
         WHERE id = $1 AND status IN ('active', 'suspended')",
    )
    .bind(project_id)
    .bind(new_status)
    .execute(state.system_db.pool())
    .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => {
            state.credential_cache.evict_project(project_id).await;
            StatusCode::OK
        }
        Ok(_) => StatusCode::NOT_FOUND,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

pub async fn suspend_project(
    Path(project_id): Path<String>,
    State(state): State<OperatorState>,
) -> StatusCode {
    // Auth is enforced by operator_auth_middleware applied at the router layer.
    set_project_status(&project_id, "suspended", &state).await
}

pub async fn activate_project(
    Path(project_id): Path<String>,
    State(state): State<OperatorState>,
) -> StatusCode {
    // Auth is enforced by operator_auth_middleware applied at the router layer.
    set_project_status(&project_id, "active", &state).await
}

pub async fn delete_project(
    Path(project_id): Path<String>,
    State(state): State<OperatorState>,
) -> StatusCode {
    // Auth is enforced by operator_auth_middleware applied at the router layer.

    // AC-B03-06: cascade-revoke all active SDK keys before soft-deleting the project.
    let _ = sqlx::query(
        "UPDATE sdk_api_keys SET revoked_at = now() \
         WHERE project_id = $1 AND revoked_at IS NULL",
    )
    .bind(&project_id)
    .execute(state.system_db.pool())
    .await;

    let result = sqlx::query(
        "UPDATE projects SET status = 'deleted', deleted_at = now(), updated_at = now() \
         WHERE id = $1 AND status != 'deleted'",
    )
    .bind(&project_id)
    .execute(state.system_db.pool())
    .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => {
            state.credential_cache.evict_project(&project_id).await;
            StatusCode::OK
        }
        Ok(_) => StatusCode::NOT_FOUND,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
