//! Query log handler.
//!
//! list_query_logs (GET /admin/v1/projects/:id/query_logs):
//!   Session auth, any role. 404 if project not in account.
//!   Query params: ?op=<operation>, ?after=<entry_id> (cursor pagination).
//!   Returns: { total, entries: [...] }. Max 500 rows per page.
//!   When logging_enabled = false: 200 { total: 0, entries: [] }.

use std::collections::HashMap;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Serialize;
use sqlx::Row;
use uuid::Uuid;

use crate::admin::extractors::session_context::SessionContext;
use crate::admin::state::UserAdminState;

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Single entry in the query log response.
#[derive(Serialize)]
pub struct QueryLogEntryResponse {
    pub id: String,
    pub op: String,
    pub collection_path: Option<String>,
    pub latency_ms: Option<i32>,
    pub status: String,
    pub error_code: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Response body for GET /admin/v1/projects/:id/query_logs.
#[derive(Serialize)]
pub struct QueryLogsResponse {
    pub total: i64,
    pub entries: Vec<QueryLogEntryResponse>,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

/// GET /admin/v1/projects/:project_id/query_logs
///
/// Session auth, any role.
///   - 404 if project not found, deleted, or owned by a different account.
///   - 200 + {total: 0, entries: []} when logging_enabled = false.
///   - 200 + {total, entries} (max 500 per page) otherwise.
pub async fn list_query_logs(
    Path(project_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<QueryLogsResponse>, StatusCode> {
    let pool = state.system_db.pool();

    // Step 1: verify project exists and belongs to this account.
    let project_row = sqlx::query(
        "SELECT account_id, logging_enabled \
         FROM projects WHERE id = $1 AND status != 'deleted'",
    )
    .bind(&project_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| {
        tracing::error!("list_query_logs: DB error verifying project: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?
    .ok_or(StatusCode::NOT_FOUND)?;

    let project_account: Uuid = project_row
        .try_get("account_id")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if project_account != session.account_id {
        return Err(StatusCode::NOT_FOUND);
    }

    let logging_enabled: bool = project_row.try_get("logging_enabled").unwrap_or(false);
    if !logging_enabled {
        return Ok(Json(QueryLogsResponse {
            total: 0,
            entries: vec![],
        }));
    }

    // Step 2: extract optional query parameters.
    let op_filter = params.get("op").cloned();
    let after_id = params.get("after").cloned();

    // Step 3: resolve cursor (created_at, uuid) if ?after= is present.
    // On invalid UUID or row-not-found, fall back to no cursor (first page).
    let cursor: Option<(chrono::DateTime<chrono::Utc>, Uuid)> =
        if let Some(ref after) = after_id {
            if let Ok(after_uuid) = Uuid::parse_str(after) {
                sqlx::query(
                    "SELECT created_at FROM query_logs \
                     WHERE id = $1 AND project_id = $2",
                )
                .bind(after_uuid)
                .bind(&project_id)
                .fetch_optional(pool)
                .await
                .ok()
                .flatten()
                .and_then(|r| {
                    r.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at")
                        .ok()
                        .map(|ts| (ts, after_uuid))
                })
            } else {
                None
            }
        } else {
            None
        };

    // Step 4: fetch up to 501 entries (one extra to detect has_next_page);
    // return at most 500 to the caller.
    //
    // Keyset pagination uses (created_at DESC, id DESC) ordering.
    // The cursor condition: rows that come after the cursor in that order.
    let select_cols =
        "SELECT id::text as id, op, collection_path, latency_ms, \
         status, error_code, created_at FROM query_logs WHERE project_id = $1";

    let rows = match (&op_filter, &cursor) {
        (None, None) => {
            sqlx::query(&format!(
                "{select_cols} ORDER BY created_at DESC, id DESC LIMIT 501"
            ))
            .bind(&project_id)
            .fetch_all(pool)
            .await
        }
        (Some(op), None) => {
            sqlx::query(&format!(
                "{select_cols} AND op = $2 \
                 ORDER BY created_at DESC, id DESC LIMIT 501"
            ))
            .bind(&project_id)
            .bind(op.as_str())
            .fetch_all(pool)
            .await
        }
        (None, Some((cursor_ts, cursor_uuid))) => {
            sqlx::query(&format!(
                "{select_cols} \
                 AND (created_at < $2 OR (created_at = $2 AND id < $3)) \
                 ORDER BY created_at DESC, id DESC LIMIT 501"
            ))
            .bind(&project_id)
            .bind(cursor_ts)
            .bind(cursor_uuid)
            .fetch_all(pool)
            .await
        }
        (Some(op), Some((cursor_ts, cursor_uuid))) => {
            sqlx::query(&format!(
                "{select_cols} AND op = $2 \
                 AND (created_at < $3 OR (created_at = $3 AND id < $4)) \
                 ORDER BY created_at DESC, id DESC LIMIT 501"
            ))
            .bind(&project_id)
            .bind(op.as_str())
            .bind(cursor_ts)
            .bind(cursor_uuid)
            .fetch_all(pool)
            .await
        }
    };

    let rows = rows.map_err(|e| {
        tracing::error!("list_query_logs: fetch entries error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Step 5: total count respects op filter but ignores cursor
    // (total reflects the full filtered set, not just the current page).
    let total: i64 = match &op_filter {
        None => sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM query_logs WHERE project_id = $1",
        )
        .bind(&project_id)
        .fetch_one(pool)
        .await
        .unwrap_or(0),

        Some(op) => sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM query_logs WHERE project_id = $1 AND op = $2",
        )
        .bind(&project_id)
        .bind(op.as_str())
        .fetch_one(pool)
        .await
        .unwrap_or(0),
    };

    // Step 6: map rows → response, capped at 500 per page.
    let entries: Vec<QueryLogEntryResponse> = rows
        .into_iter()
        .take(500)
        .map(|r| QueryLogEntryResponse {
            id: r.try_get("id").unwrap_or_default(),
            op: r.try_get("op").unwrap_or_default(),
            collection_path: r.try_get("collection_path").unwrap_or(None),
            latency_ms: r.try_get("latency_ms").unwrap_or(None),
            status: r.try_get("status").unwrap_or_default(),
            error_code: r.try_get("error_code").unwrap_or(None),
            created_at: r
                .try_get("created_at")
                .unwrap_or_else(|_| chrono::Utc::now()),
        })
        .collect();

    Ok(Json(QueryLogsResponse { total, entries }))
}
