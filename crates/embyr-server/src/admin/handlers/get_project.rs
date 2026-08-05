use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Serialize;

use crate::admin::state::OperatorState;

#[derive(Serialize)]
pub struct GetProjectResponse {
    pub project_id: String,
    pub status: String,
    pub backend_mode: String,
    pub auth_mode: String,
}

pub async fn get_project(
    Path(project_id): Path<String>,
    State(state): State<OperatorState>,
) -> Result<Json<GetProjectResponse>, StatusCode> {
    // Auth is enforced by the calling sub-router's middleware layer.

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
