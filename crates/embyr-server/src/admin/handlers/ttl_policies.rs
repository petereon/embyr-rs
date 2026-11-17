//! TTL policy admin CRUD (go-live finding P5).
//!
//! `GET/PUT /admin/v1/projects/:project_id/ttl_policies` and
//! `DELETE .../ttl_policies/:collection_group`. One policy per collection
//! group (like Firestore); `PUT` upserts. Enforcement is the background
//! `sweepers::ttl_sweeper`; this module only stores the configuration.
//! Auth is the same as the sibling `indexes` routes: session cookie or admin
//! API key, project must belong to the caller's account (404 otherwise),
//! Admin+ to change, any role to list.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

use crate::admin::extractors::session_context::SessionContext;
use crate::admin::handlers::shared::verify_project_ownership;
use crate::admin::state::UserAdminState;
use embyr_core::admin::account::Role;
use embyr_core::domain::ttl::validate_ttl_policy;

#[derive(Debug, Deserialize)]
pub struct PutTtlPolicyBody {
    pub collection_group: String,
    pub field_path: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct TtlPolicyResponse {
    pub collection_group: String,
    pub field_path: String,
    pub enabled: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

fn db_err(op: &str, e: sqlx::Error) -> StatusCode {
    tracing::error!("ttl_policies {op}: DB error: {e}");
    StatusCode::INTERNAL_SERVER_ERROR
}

/// GET /admin/v1/projects/:project_id/ttl_policies (any role, read-only).
pub async fn list_ttl_policies(
    Path(project_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
) -> Result<Json<Vec<TtlPolicyResponse>>, StatusCode> {
    let pool = state.system_db.pool();
    verify_project_ownership(pool, &project_id, session.account_id).await?;

    let rows = sqlx::query_as::<_, TtlPolicyResponse>(
        "SELECT collection_group, field_path, enabled, created_at, updated_at \
         FROM ttl_policies WHERE project_id = $1 ORDER BY collection_group",
    )
    .bind(&project_id)
    .fetch_all(pool)
    .await
    .map_err(|e| db_err("list", e))?;
    Ok(Json(rows))
}

/// PUT /admin/v1/projects/:project_id/ttl_policies (Owner/Admin only).
///
/// Upsert on `(project, collection_group)`: a different `field_path` replaces
/// the policy's field (Firestore allows one TTL field per collection group).
/// `422` for an invalid collection group / field path (validated before any
/// DB write), `409` for an `agent`-mode project (the agent cannot run the
/// collection-group query the sweeper needs, so a policy would never fire).
pub async fn put_ttl_policy(
    Path(project_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
    Json(body): Json<PutTtlPolicyBody>,
) -> Result<Json<TtlPolicyResponse>, StatusCode> {
    if session.role < Role::Admin {
        return Err(StatusCode::FORBIDDEN);
    }
    let pool = state.system_db.pool();
    verify_project_ownership(pool, &project_id, session.account_id).await?;

    validate_ttl_policy(&body.collection_group, &body.field_path)
        .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;

    let mode: String = sqlx::query_scalar("SELECT backend_mode FROM projects WHERE id = $1")
        .bind(&project_id)
        .fetch_one(pool)
        .await
        .map_err(|e| db_err("put/mode", e))?;
    if mode == "agent" {
        return Err(StatusCode::CONFLICT);
    }

    let row = sqlx::query_as::<_, TtlPolicyResponse>(
        "INSERT INTO ttl_policies (project_id, collection_group, field_path, enabled) \
         VALUES ($1, $2, $3, $4) \
         ON CONFLICT (project_id, collection_group) DO UPDATE \
         SET field_path = EXCLUDED.field_path, enabled = EXCLUDED.enabled, updated_at = now() \
         RETURNING collection_group, field_path, enabled, created_at, updated_at",
    )
    .bind(&project_id)
    .bind(&body.collection_group)
    .bind(&body.field_path)
    .bind(body.enabled)
    .fetch_one(pool)
    .await
    .map_err(|e| db_err("put", e))?;
    Ok(Json(row))
}

/// DELETE /admin/v1/projects/:project_id/ttl_policies/:collection_group
/// (Owner/Admin only). Documents already deleted stay deleted; nothing else
/// is touched. 404 when the group has no policy.
pub async fn delete_ttl_policy(
    Path((project_id, collection_group)): Path<(String, String)>,
    State(state): State<UserAdminState>,
    session: SessionContext,
) -> Result<StatusCode, StatusCode> {
    if session.role < Role::Admin {
        return Err(StatusCode::FORBIDDEN);
    }
    let pool = state.system_db.pool();
    verify_project_ownership(pool, &project_id, session.account_id).await?;

    let result = sqlx::query("DELETE FROM ttl_policies WHERE project_id = $1 AND collection_group = $2")
        .bind(&project_id)
        .bind(&collection_group)
        .execute(pool)
        .await
        .map_err(|e| db_err("delete", e))?;
    if result.rows_affected() == 0 {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(StatusCode::NO_CONTENT)
}
