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

use crate::adapters::composite_index_builder::spawn_build;
use crate::adapters::composite_index_ddl::{build_drop_index_sql, validate_fields};
use crate::adapters::customer_db_connect::{resolve_dsn_without_api_key, PgConnectInfo};
use crate::adapters::postgres_backend::PostgresBackendAdapter;
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

    // AC-CXR-04: reject a field path outside the safe charset BEFORE any DB
    // write or DDL text is ever constructed (ADR-072 Decision A).
    validate_fields(&body.fields).map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;

    let fields_json =
        serde_json::to_value(&body.fields).map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;

    // ADR-072 Decision C: status starts 'building', never synchronously
    // 'ready'. Re-POSTing a 'failed' index resets it to 'building' (retry);
    // re-POSTing a 'building'/'ready' index is a true no-op.
    let row: CompositeIndexRow = sqlx::query_as(
        "INSERT INTO composite_indexes (project_id, collection_path, fields, status) \
         VALUES ($1, $2, $3::jsonb, 'building') \
         ON CONFLICT (project_id, collection_path, fields) \
         DO UPDATE SET status = CASE WHEN composite_indexes.status = 'failed' THEN 'building' \
                                      ELSE composite_indexes.status END \
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

    // Only a fresh INSERT or a failed->building retry ever needs a build —
    // an already-'ready' conflict is a true no-op (never rebuilds a working
    // index, ADR-072 Decision C).
    if row.status == "building" {
        let index_id = Uuid::parse_str(&row.id).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        match state.system_db.get_project_pg_connect_info(&project_id, session.account_id).await {
            Ok(Some(connect_info)) => {
                spawn_build(
                    state.system_db.clone(),
                    connect_info,
                    state.aws_secret_fetcher.clone(),
                    state.gcp_secret_fetcher.clone(),
                    state.encryption_key,
                    state.encryption_key_previous,
                    index_id,
                    project_id.clone(),
                    serde_json::from_value(row.fields.clone())
                        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
                );
            }
            Ok(None) => {
                // Project vanished between ownership check and now (race) —
                // never leave the row ambiguously 'building' forever.
                let _ = state.system_db.update_composite_index_status(index_id, "failed").await;
            }
            Err(e) => {
                tracing::error!("create_composite_index: connect-info lookup failed: {e}");
                let _ = state.system_db.update_composite_index_status(index_id, "failed").await;
            }
        }
    }

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
///
/// composite-index-real-creation (US-03, ADR-072): also drops the real
/// underlying Postgres index (when one exists), BEFORE the metadata-row
/// DELETE below (AC-CXR-10/11). For a `building`/`failed` row the drop is
/// best-effort — its outcome never blocks the delete (AC-CXR-11: "does not
/// error, regardless of whether a real (possibly partial) index object
/// exists yet"). For a `ready` row (a real index is KNOWN to exist), a drop
/// failure aborts the delete instead — the metadata row is left in place,
/// visible and retryable, rather than silently orphaning a real index with
/// no record of it at all (ADR-072 § Component Boundaries).
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

    let status: Option<String> =
        sqlx::query_scalar("SELECT status FROM composite_indexes WHERE id = $1 AND project_id = $2")
            .bind(index_uuid)
            .bind(&project_id)
            .fetch_optional(pool)
            .await
            .map_err(|e| {
                tracing::error!("delete_composite_index: DB error: {e}");
                StatusCode::INTERNAL_SERVER_ERROR
            })?;
    let Some(status) = status else {
        return Err(StatusCode::NOT_FOUND);
    };

    if let Some(connect_info) = state
        .system_db
        .get_project_pg_connect_info(&project_id, session.account_id)
        .await
        .map_err(|e| {
            tracing::error!("delete_composite_index: connect-info lookup failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
    {
        let dropped = drop_real_index(&state, &connect_info, &project_id, index_uuid).await;
        if status == "ready" && !dropped {
            return Err(StatusCode::INTERNAL_SERVER_ERROR);
        }
    }

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

/// Best-effort `DROP INDEX CONCURRENTLY IF EXISTS` against the customer
/// database. `false` on any failure to connect or drop (logged); the caller
/// decides how strictly to treat that based on the row's own `status`.
async fn drop_real_index(
    state: &UserAdminState,
    connect_info: &PgConnectInfo,
    project_id: &str,
    index_id: Uuid,
) -> bool {
    let Some(dsn) = resolve_dsn_without_api_key(
        project_id,
        connect_info,
        state.aws_secret_fetcher.as_deref(),
        state.gcp_secret_fetcher.as_deref(),
        &state.encryption_key,
        state.encryption_key_previous.as_ref(),
    )
    .await
    else {
        return false;
    };

    let adapter = match PostgresBackendAdapter::new(&dsn).await {
        Ok(adapter) => adapter,
        Err(e) => {
            tracing::warn!(project_id = %project_id, index_id = %index_id, error = %e, "delete_composite_index: failed to connect to customer database");
            return false;
        }
    };

    match sqlx::query(&build_drop_index_sql(index_id)).execute(adapter.pool()).await {
        Ok(_) => true,
        Err(e) => {
            tracing::warn!(project_id = %project_id, index_id = %index_id, error = %e, "delete_composite_index: DROP INDEX CONCURRENTLY failed");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn index_field_order_round_trips_through_the_exact_fixture_strings() {
        assert_eq!(
            serde_json::to_value(IndexFieldOrder::Asc).unwrap(),
            serde_json::json!("ASC")
        );
        assert_eq!(
            serde_json::to_value(IndexFieldOrder::Desc).unwrap(),
            serde_json::json!("DESC")
        );
        assert_eq!(
            serde_json::from_value::<IndexFieldOrder>(serde_json::json!("ASC")).unwrap(),
            IndexFieldOrder::Asc
        );
        assert_eq!(
            serde_json::from_value::<IndexFieldOrder>(serde_json::json!("DESC")).unwrap(),
            IndexFieldOrder::Desc
        );
    }

    #[test]
    fn a_lowercase_or_unrecognized_order_value_fails_to_deserialize() {
        assert!(serde_json::from_value::<IndexFieldOrder>(serde_json::json!("asc")).is_err());
        assert!(serde_json::from_value::<IndexFieldOrder>(serde_json::json!("SIDEWAYS")).is_err());
    }

    #[test]
    fn index_field_spec_round_trips_the_exact_shape_used_by_the_existing_fixture() {
        let spec = IndexFieldSpec { field: "category".to_string(), order: IndexFieldOrder::Asc };
        let json = serde_json::to_value(&spec).unwrap();
        assert_eq!(json, serde_json::json!({"field": "category", "order": "ASC"}));
        let round_tripped: IndexFieldSpec = serde_json::from_value(json).unwrap();
        assert_eq!(round_tripped, spec);
    }

    #[test]
    fn into_response_translates_valid_fields_json_correctly() {
        let row = CompositeIndexRow {
            id: "abc".to_string(),
            project_id: "trailmark-prod".to_string(),
            collection_path: "products".to_string(),
            fields: serde_json::json!([{"field": "category", "order": "ASC"}]),
            status: "ready".to_string(),
            created_at: chrono::Utc::now(),
        };
        let response = row.into_response().expect("valid fields JSON must translate");
        assert_eq!(response.fields.len(), 1);
        assert_eq!(response.fields[0].field, "category");
        assert_eq!(response.fields[0].order, IndexFieldOrder::Asc);
    }

    #[test]
    fn into_response_fails_closed_on_malformed_fields_json() {
        let row = CompositeIndexRow {
            id: "abc".to_string(),
            project_id: "trailmark-prod".to_string(),
            collection_path: "products".to_string(),
            fields: serde_json::json!("not an array of field specs"),
            status: "ready".to_string(),
            created_at: chrono::Utc::now(),
        };
        let result = row.into_response();
        assert_eq!(result.unwrap_err(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
