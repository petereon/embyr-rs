//! Metrics handler.
//!
//! get_project_metrics (GET /admin/v1/projects/:id/metrics):
//!   Session auth, any role. 404 if project not in account.
//!   Returns: { p95_read_ms, p95_write_ms, reads_today, writes_today, deletes_today,
//!              sparkline: [{ hour, reads, writes }] }.
//!   Sourced from daily_project_metrics. Sparkline: 24 equal buckets.

use axum::{
    extract::{Path, State},
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

/// Per-hour read/write bucket for the sparkline.
#[derive(Serialize)]
pub struct SparklinePoint {
    pub hour: u8,
    pub reads: i64,
    pub writes: i64,
}

/// Response body for GET /admin/v1/projects/:id/metrics.
#[derive(Serialize)]
pub struct ProjectMetrics {
    /// p95 read latency in ms. Not stored in V1 — returns 0.0.
    pub p95_read_ms: f64,
    /// p95 write latency in ms. Not stored in V1 — returns 0.0.
    pub p95_write_ms: f64,
    /// Total read operations today.
    pub reads_today: i64,
    /// Total write operations today.
    pub writes_today: i64,
    /// Total delete operations today.
    pub deletes_today: i64,
    /// 24 hourly buckets (equal-bucket V1 approximation).
    pub sparkline: Vec<SparklinePoint>,
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

/// GET /admin/v1/projects/:project_id/metrics
///
/// Session auth, any role.
///   - 404 if project does not exist, is deleted, or belongs to a different account.
///   - 200 + ProjectMetrics on success.
pub async fn get_project_metrics(
    Path(project_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
) -> Result<Json<ProjectMetrics>, StatusCode> {
    let pool = state.system_db.pool();

    // Step 1: verify project exists and belongs to this account.
    let row = sqlx::query(
        "SELECT account_id FROM projects WHERE id = $1 AND status != 'deleted'",
    )
    .bind(&project_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| {
        tracing::error!("get_project_metrics: DB error verifying ownership: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?
    .ok_or(StatusCode::NOT_FOUND)?;

    let project_account: Uuid = row
        .try_get("account_id")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if project_account != session.account_id {
        return Err(StatusCode::NOT_FOUND);
    }

    // Step 2: read today's metrics row; fall back to zeros if absent.
    let (read_ops, write_ops, delete_ops): (i64, i64, i64) = sqlx::query(
        "SELECT read_ops, write_ops, delete_ops \
         FROM daily_project_metrics \
         WHERE project_id = $1 AND date = CURRENT_DATE",
    )
    .bind(&project_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| {
        tracing::error!("get_project_metrics: DB error fetching metrics: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?
    .map(|r| {
        let read: i64 = r.try_get("read_ops").unwrap_or(0);
        let write: i64 = r.try_get("write_ops").unwrap_or(0);
        let delete: i64 = r.try_get("delete_ops").unwrap_or(0);
        (read, write, delete)
    })
    .unwrap_or((0, 0, 0));

    // Step 3: build 24-element sparkline with equal buckets (V1 simplification).
    let reads_per_hour = read_ops / 24;
    let writes_per_hour = write_ops / 24;
    let sparkline: Vec<SparklinePoint> = (0u8..24)
        .map(|hour| SparklinePoint {
            hour,
            reads: reads_per_hour,
            writes: writes_per_hour,
        })
        .collect();

    Ok(Json(ProjectMetrics {
        p95_read_ms: 0.0,
        p95_write_ms: 0.0,
        reads_today: read_ops,
        writes_today: write_ops,
        deletes_today: delete_ops,
        sparkline,
    }))
}
