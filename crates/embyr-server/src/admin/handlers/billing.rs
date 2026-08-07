//! Billing handler.
//!
//! get_billing (GET /admin/v1/billing?range=<7d|30d|month|last_month>):
//!   Session auth, any role.
//!   Returns: { range, databases: [ { id, name, reads, writes, deletes,
//!              peak_connections (null V1), log_storage_bytes } ], totals: {...} }.
//!   Sourced from daily_project_metrics GROUP BY project WHERE account_id = session.account_id.
//!   Projects with no metric rows included with all counters = 0 (LEFT JOIN, AC-B06-08).

use std::collections::HashMap;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use serde::Serialize;
use sqlx::Row;

use crate::admin::extractors::session_context::SessionContext;
use crate::admin::state::UserAdminState;

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Per-database entry in the billing response.
#[derive(Serialize)]
pub struct DatabaseBillingEntry {
    pub id: String,
    pub name: Option<String>,
    pub reads: i64,
    pub writes: i64,
    pub deletes: i64,
    /// Always None in V1 (deferred; UI shows '—').
    pub peak_connections: Option<i64>,
    pub log_storage_bytes: i64,
}

/// Aggregate totals across all databases.
#[derive(Serialize)]
pub struct BillingTotals {
    pub reads: i64,
    pub writes: i64,
    pub deletes: i64,
    pub log_storage_bytes: i64,
}

/// Response body for GET /admin/v1/billing.
#[derive(Serialize)]
pub struct BillingResponse {
    pub range: String,
    pub databases: Vec<DatabaseBillingEntry>,
    pub totals: BillingTotals,
}

// ---------------------------------------------------------------------------
// Range → SQL date condition
// ---------------------------------------------------------------------------

/// Returns the SQL fragment to restrict `m.date` to the requested range.
/// Returns `None` for unrecognised range strings (caller returns 422).
///
/// Safety: returned strings are hard-coded constants — not user-supplied data.
/// The range parameter is validated against this match before the constant
/// is injected into the query, so there is no SQL injection risk.
fn date_condition_for(range: &str) -> Option<&'static str> {
    match range {
        "7d" => Some("m.date >= CURRENT_DATE - INTERVAL '6 days'"),
        "30d" => Some("m.date >= CURRENT_DATE - INTERVAL '29 days'"),
        "month" => Some("m.date >= DATE_TRUNC('month', CURRENT_DATE)"),
        "last_month" => Some(
            "m.date >= DATE_TRUNC('month', CURRENT_DATE - INTERVAL '1 month') \
             AND m.date < DATE_TRUNC('month', CURRENT_DATE)",
        ),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Handler
// ---------------------------------------------------------------------------

/// GET /admin/v1/billing?range=<7d|30d|month|last_month>
///
/// Session auth, any role.
///   - 422 if `range` query parameter is missing or not one of the four
///     supported values.
///   - 200 + BillingResponse on success. Projects with no metric rows in the
///     requested range appear in `databases` with all counters = 0 (LEFT JOIN).
pub async fn get_billing(
    State(state): State<UserAdminState>,
    session: SessionContext,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<BillingResponse>, StatusCode> {
    let range = params.get("range").map(|s| s.as_str()).unwrap_or("");

    let date_cond =
        date_condition_for(range).ok_or(StatusCode::UNPROCESSABLE_ENTITY)?;

    let pool = state.system_db.pool();

    // LEFT JOIN: projects with no matching metric rows get COALESCE → 0.
    // The date condition is injected as a hard-coded SQL literal derived from
    // an enum-matched constant — not from user-supplied data. No injection risk.
    let sql = format!(
        "SELECT \
             p.id, \
             p.name, \
             COALESCE(SUM(m.read_ops),   0) AS reads, \
             COALESCE(SUM(m.write_ops),  0) AS writes, \
             COALESCE(SUM(m.delete_ops), 0) AS deletes, \
             0::BIGINT AS log_storage_bytes \
         FROM projects p \
         LEFT JOIN daily_project_metrics m \
             ON m.project_id = p.id \
             AND {date_cond} \
         WHERE p.account_id = $1 \
           AND p.status != 'deleted' \
         GROUP BY p.id, p.name \
         ORDER BY p.name"
    );

    let rows = sqlx::query(&sql)
        .bind(session.account_id)
        .fetch_all(pool)
        .await
        .map_err(|e| {
            tracing::error!("get_billing: DB error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let databases: Vec<DatabaseBillingEntry> = rows
        .iter()
        .map(|r| DatabaseBillingEntry {
            id: r.try_get("id").unwrap_or_default(),
            name: r.try_get("name").unwrap_or(None),
            reads: r.try_get("reads").unwrap_or(0),
            writes: r.try_get("writes").unwrap_or(0),
            deletes: r.try_get("deletes").unwrap_or(0),
            peak_connections: None,
            log_storage_bytes: r.try_get("log_storage_bytes").unwrap_or(0),
        })
        .collect();

    let totals = BillingTotals {
        reads: databases.iter().map(|d| d.reads).sum(),
        writes: databases.iter().map(|d| d.writes).sum(),
        deletes: databases.iter().map(|d| d.deletes).sum(),
        log_storage_bytes: databases.iter().map(|d| d.log_storage_bytes).sum(),
    };

    Ok(Json(BillingResponse {
        range: range.to_string(),
        databases,
        totals,
    }))
}
