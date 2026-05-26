use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    Json,
};
use serde::Serialize;

use super::provision::{extract_bearer, AdminState};

#[derive(Serialize)]
pub struct GetProjectResponse {
    pub project_id: String,
    pub status: String,
    pub backend_mode: String,
    pub auth_mode: String,
}

pub async fn get_project(
    Path(project_id): Path<String>,
    headers: HeaderMap,
    State(state): State<AdminState>,
) -> Result<Json<GetProjectResponse>, StatusCode> {
    let token = extract_bearer(&headers).ok_or(StatusCode::UNAUTHORIZED)?;
    if token != state.admin_key {
        return Err(StatusCode::UNAUTHORIZED);
    }

    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT status, backend_mode FROM projects WHERE id=$1",
    )
    .bind(&project_id)
    .fetch_optional(state.system_db.pool())
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    match row {
        None => Err(StatusCode::NOT_FOUND),
        Some((status, _)) if status == "deleted" => Err(StatusCode::NOT_FOUND),
        Some((status, backend_mode)) => Ok(Json(GetProjectResponse {
            project_id,
            status,
            backend_mode,
            auth_mode: "key".into(),
        })),
    }
}
