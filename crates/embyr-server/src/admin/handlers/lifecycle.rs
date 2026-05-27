use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};

use super::provision::{extract_bearer, AdminState};

pub async fn suspend_project(
    Path(project_id): Path<String>,
    headers: HeaderMap,
    State(state): State<AdminState>,
) -> StatusCode {
    let Some(token) = extract_bearer(&headers) else {
        return StatusCode::UNAUTHORIZED;
    };
    if token != state.admin_key {
        return StatusCode::UNAUTHORIZED;
    }

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
    headers: HeaderMap,
    State(state): State<AdminState>,
) -> StatusCode {
    let Some(token) = extract_bearer(&headers) else {
        return StatusCode::UNAUTHORIZED;
    };
    if token != state.admin_key {
        return StatusCode::UNAUTHORIZED;
    }

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
    headers: HeaderMap,
    State(state): State<AdminState>,
) -> StatusCode {
    let Some(token) = extract_bearer(&headers) else {
        return StatusCode::UNAUTHORIZED;
    };
    if token != state.admin_key {
        return StatusCode::UNAUTHORIZED;
    }

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
