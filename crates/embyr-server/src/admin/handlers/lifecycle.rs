use axum::{
    extract::{Path, State},
    http::StatusCode,
};

use crate::admin::state::OperatorState;

pub async fn suspend_project(
    Path(project_id): Path<String>,
    State(state): State<OperatorState>,
) -> StatusCode {
    // Auth is enforced by operator_auth_middleware applied at the router layer.

    // Idempotent: UPDATE succeeds if project is 'active' OR already 'suspended'.
    let result = sqlx::query(
        "UPDATE projects SET status='suspended', updated_at=now() \
         WHERE id=$1 AND status IN ('active','suspended')",
    )
    .bind(&project_id)
    .execute(state.system_db.pool())
    .await;

    match result {
        Ok(r) if r.rows_affected() > 0 => {
            state.credential_cache.evict_project(&project_id).await;
            StatusCode::OK
        }
        Ok(_) => StatusCode::NOT_FOUND, // project does not exist or is deleted
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

pub async fn activate_project(
    Path(project_id): Path<String>,
    State(state): State<OperatorState>,
) -> StatusCode {
    // Auth is enforced by operator_auth_middleware applied at the router layer.

    // Idempotent: UPDATE succeeds if project is 'suspended' OR already 'active'.
    let result = sqlx::query(
        "UPDATE projects SET status='active', updated_at=now() \
         WHERE id=$1 AND status IN ('active','suspended')",
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

pub async fn delete_project(
    Path(project_id): Path<String>,
    State(state): State<OperatorState>,
) -> StatusCode {
    // Auth is enforced by operator_auth_middleware applied at the router layer.

    let result = sqlx::query(
        "UPDATE projects SET status='deleted', deleted_at=now(), updated_at=now() \
         WHERE id=$1 AND status != 'deleted'",
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
