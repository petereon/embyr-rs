//! Composite-index admin CRUD — BC-4-adjacent admin surface (feature
//! `firestore-composite-indexes-admin-api`, ADR-068).
//!
//! `crates/embyr-server/src/grpc/handler.rs`'s own `RunQuery` has correctly
//! enforced a composite-index `FAILED_PRECONDITION` gate
//! (`IndexManager::is_index_ready` / `requires_composite_index`) since the
//! original walking skeleton — but no production code path has ever
//! written a row into `composite_indexes` (migration 0003). This module is
//! the FIRST: 3 handlers (`create_composite_index`, `list_composite_
//! indexes`, `delete_composite_index`), purely upstream of the query
//! path's own existing, unmodified gate — zero change to `handler.rs`.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};

use uuid::Uuid;

use crate::admin::extractors::session_context::SessionContext;
use crate::admin::handlers::shared::verify_project_ownership;
use crate::admin::state::UserAdminState;
use embyr_core::admin::account::Role;

/// One field/order pair within a composite index — matches the `fields`
/// JSONB shape already assumed by `tests/acceptance/us_04_query_
/// collection.rs`'s own fixture (`[{"field": "category", "order":
/// "ASC"}, ...]`), never a newly-invented convention.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexFieldSpec {
    pub field: String,
    pub order: IndexFieldOrder,
}

/// Serializes to/from the exact `"ASC"`/`"DESC"` strings the existing
/// fixture already uses — a typed enum, not a raw string, so a malformed
/// `order` value is a clean 422 at the request-deserialization boundary,
/// never a value silently stored that the query path would later fail to
/// compare against.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IndexFieldOrder {
    Asc,
    Desc,
}

/// Body for POST /admin/v1/projects/:project_id/indexes.
#[derive(Debug, Deserialize)]
pub struct CreateCompositeIndexBody {
    pub collection_path: String,
    pub fields: Vec<IndexFieldSpec>,
}

/// Response shape for create/list — real-Firestore-adjacent field naming
/// without transliterating real Firestore's own `google.firestore.admin.
/// v1.Index` protobuf (matches this codebase's own established "own JSON
/// conventions" precedent — `access_rules`/`write_access_rules` never
/// mirror a real Firestore proto 1:1 either).
#[derive(Debug, Serialize)]
pub struct CompositeIndexResponse {
    pub id: String,
    pub project_id: String,
    pub collection_path: String,
    pub fields: Vec<IndexFieldSpec>,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(sqlx::FromRow)]
struct CompositeIndexRow {
    id: String,
    project_id: String,
    collection_path: String,
    fields: serde_json::Value,
    status: String,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl CompositeIndexRow {
    fn into_response(self) -> Result<CompositeIndexResponse, StatusCode> {
        let fields: Vec<IndexFieldSpec> =
            serde_json::from_value(self.fields).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        Ok(CompositeIndexResponse {
            id: self.id,
            project_id: self.project_id,
            collection_path: self.collection_path,
            fields,
            status: self.status,
            created_at: self.created_at,
        })
    }
}

/// POST /admin/v1/projects/:project_id/indexes (Owner/Admin only).
///
/// Idempotent on an exact-duplicate `(collection_path, fields)` spec
/// (ADR-068 § Resolution 5, § Decision — SQL): `ON CONFLICT ... DO UPDATE`
/// with a deliberate no-op self-update, so `RETURNING` fires on the
/// conflicting row too — never a 409, never a raw DB constraint error.
pub async fn create_composite_index(
    Path(project_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
    Json(body): Json<CreateCompositeIndexBody>,
) -> Result<(StatusCode, Json<CompositeIndexResponse>), StatusCode> {
    if session.role < Role::Admin {
        return Err(StatusCode::FORBIDDEN);
    }

    let pool = state.system_db.pool();
    verify_project_ownership(pool, &project_id, session.account_id).await?;

    let fields_json =
        serde_json::to_value(&body.fields).map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;

    let row: CompositeIndexRow = sqlx::query_as(
        "INSERT INTO composite_indexes (project_id, collection_path, fields, status) \
         VALUES ($1, $2, $3::jsonb, 'ready') \
         ON CONFLICT (project_id, collection_path, fields) \
         DO UPDATE SET collection_path = EXCLUDED.collection_path \
         RETURNING id::text AS id, project_id, collection_path, fields, status, created_at",
    )
    .bind(&project_id)
    .bind(&body.collection_path)
    .bind(&fields_json)
    .fetch_one(pool)
    .await
    .map_err(|e| {
        tracing::error!("create_composite_index: DB error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok((StatusCode::OK, Json(row.into_response()?)))
}

/// GET /admin/v1/projects/:project_id/indexes (any role, read-only).
pub async fn list_composite_indexes(
    Path(project_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
) -> Result<Json<Vec<CompositeIndexResponse>>, StatusCode> {
    let pool = state.system_db.pool();
    verify_project_ownership(pool, &project_id, session.account_id).await?;

    let rows: Vec<CompositeIndexRow> = sqlx::query_as(
        "SELECT id::text AS id, project_id, collection_path, fields, status, created_at \
         FROM composite_indexes WHERE project_id = $1 ORDER BY created_at ASC",
    )
    .bind(&project_id)
    .fetch_all(pool)
    .await
    .map_err(|e| {
        tracing::error!("list_composite_indexes: DB error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let indexes: Result<Vec<CompositeIndexResponse>, StatusCode> =
        rows.into_iter().map(CompositeIndexRow::into_response).collect();
    Ok(Json(indexes?))
}

/// DELETE /admin/v1/projects/:project_id/indexes/:index_id (Owner/Admin
/// only). No dependency-safety check (ADR-068 § Resolution 4) — the next
/// query that newly requires the deleted index simply fails
/// `FAILED_PRECONDITION` again, same as if it had never been created.
pub async fn delete_composite_index(
    Path((project_id, index_id)): Path<(String, String)>,
    State(state): State<UserAdminState>,
    session: SessionContext,
) -> Result<StatusCode, StatusCode> {
    if session.role < Role::Admin {
        return Err(StatusCode::FORBIDDEN);
    }

    let index_uuid = Uuid::parse_str(&index_id).map_err(|_| StatusCode::NOT_FOUND)?;

    let pool = state.system_db.pool();
    verify_project_ownership(pool, &project_id, session.account_id).await?;

    let result = sqlx::query("DELETE FROM composite_indexes WHERE id = $1 AND project_id = $2")
        .bind(index_uuid)
        .bind(&project_id)
        .execute(pool)
        .await
        .map_err(|e| {
            tracing::error!("delete_composite_index: DB error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    if result.rows_affected() == 0 {
        return Err(StatusCode::NOT_FOUND);
    }
    Ok(StatusCode::NO_CONTENT)
}
