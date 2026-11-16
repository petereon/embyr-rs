//! Metrics handlers (hourly-metrics-rollup, ADR-084).
//!
//! get_project_metrics (GET /admin/v1/projects/:id/metrics?window=24h|7d):
//!   Session auth, any role. 404 if project not in account, 422 on a bad window.
//!   `*_today` counters come from daily_project_metrics (unchanged, billing source);
//!   latency percentiles and the hourly `series` come from hourly_project_metrics.
//! get_metrics_summary (GET /admin/v1/metrics/summary):
//!   One call, every non-deleted project of the caller's account.

use std::collections::{BTreeMap, HashMap};

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use embyr_core::metrics_rollup::{is_saturated, percentile, truncate_to_hour, BUCKET_COUNT};
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::admin::extractors::session_context::SessionContext;
use crate::admin::handlers::shared::verify_project_ownership;
use crate::admin::state::UserAdminState;

/// Points in the deprecated `sparkline` (last 24 hours of `series`).
const SPARKLINE_HOURS: usize = 24;

const BUCKET_COLS: &str = "b00, b01, b02, b03, b04, b05, b06, b07, b08, b09, b10, b11, b12";
const BUCKET_SUMS: &str = "SUM(b00)::bigint AS b00, SUM(b01)::bigint AS b01, \
    SUM(b02)::bigint AS b02, SUM(b03)::bigint AS b03, SUM(b04)::bigint AS b04, \
    SUM(b05)::bigint AS b05, SUM(b06)::bigint AS b06, SUM(b07)::bigint AS b07, \
    SUM(b08)::bigint AS b08, SUM(b09)::bigint AS b09, SUM(b10)::bigint AS b10, \
    SUM(b11)::bigint AS b11, SUM(b12)::bigint AS b12";

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

/// Deprecated per-hour point, kept for the current SPA until its migration.
#[derive(Serialize)]
pub struct SparklinePoint {
    pub hour: u8,
    pub reads: i64,
    pub writes: i64,
}

/// Percentiles of one op class; `null` (never 0.0) when there are no samples.
#[derive(Serialize, Debug, PartialEq)]
pub struct LatencyStats {
    pub p50_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub p99_ms: Option<f64>,
    pub samples: u64,
    /// True when a percentile fell in the 5000+ ms overflow bucket, so the
    /// reported value is a lower bound (">5000").
    pub saturated: bool,
}

#[derive(Serialize)]
pub struct Latency {
    pub read: LatencyStats,
    pub write: LatencyStats,
}

/// One hour of the activity series; counts are `units`.
#[derive(Serialize)]
pub struct SeriesPoint {
    pub hour: String,
    pub reads: i64,
    pub writes: i64,
    pub deletes: i64,
    pub errors: i64,
}

/// Response body for GET /admin/v1/projects/:id/metrics.
#[derive(Serialize)]
pub struct ProjectMetrics {
    pub reads_today: i64,
    pub writes_today: i64,
    pub deletes_today: i64,
    pub latency: Latency,
    pub series: Vec<SeriesPoint>,
    /// Kept one release; 0.0 when there are no samples.
    pub p95_read_ms: f64,
    pub p95_write_ms: f64,
    /// Deprecated: derived from `series`.
    pub sparkline: Vec<SparklinePoint>,
}

/// One entry of GET /admin/v1/metrics/summary.
#[derive(Serialize)]
pub struct ProjectSummary {
    pub project_id: String,
    pub name: String,
    pub reads_today: i64,
    pub writes_today: i64,
    pub deletes_today: i64,
    pub p95_read_ms: Option<f64>,
    pub p95_write_ms: Option<f64>,
}

#[derive(Deserialize)]
pub struct MetricsQuery {
    window: Option<String>,
}

// ---------------------------------------------------------------------------
// Pure folding
// ---------------------------------------------------------------------------

/// One `hourly_project_metrics` row.
struct RollupRow {
    hour: DateTime<Utc>,
    op: i16,
    units: i64,
    errors: i64,
    buckets: [u64; BUCKET_COUNT],
}

struct Folded {
    series: Vec<SeriesPoint>,
    read: [u64; BUCKET_COUNT],
    write: [u64; BUCKET_COUNT],
}

/// Number of hourly points for a `window` value.
fn window_hours(window: Option<&str>) -> Option<i64> {
    match window.unwrap_or("24h") {
        "24h" => Some(24),
        "7d" => Some(168),
        _ => None,
    }
}

fn add(into: &mut [u64; BUCKET_COUNT], from: &[u64; BUCKET_COUNT]) {
    for (a, b) in into.iter_mut().zip(from) {
        *a += b;
    }
}

/// Zero-filled ascending series of `hours` points ending at `current_hour`;
/// reads = ops read+run_query, writes = op write, deletes = op delete.
/// Read latency covers read+run_query, write latency covers write+delete.
fn fold(rows: &[RollupRow], current_hour: DateTime<Utc>, hours: i64) -> Folded {
    let mut by_hour: BTreeMap<DateTime<Utc>, [i64; 4]> =
        (0..hours).map(|i| (current_hour - Duration::hours(hours - 1 - i), [0; 4])).collect();
    let (mut read, mut write) = ([0u64; BUCKET_COUNT], [0u64; BUCKET_COUNT]);
    for row in rows {
        let Some(point) = by_hour.get_mut(&row.hour) else { continue };
        match row.op {
            0 | 3 => {
                point[0] += row.units;
                add(&mut read, &row.buckets);
            }
            1 => {
                point[1] += row.units;
                add(&mut write, &row.buckets);
            }
            _ => {
                point[2] += row.units;
                add(&mut write, &row.buckets);
            }
        }
        point[3] += row.errors;
    }
    let series = by_hour
        .into_iter()
        .map(|(hour, [reads, writes, deletes, errors])| SeriesPoint {
            hour: hour.to_rfc3339_opts(SecondsFormat::Secs, true),
            reads,
            writes,
            deletes,
            errors,
        })
        .collect();
    Folded { series, read, write }
}

fn latency_stats(buckets: &[u64; BUCKET_COUNT]) -> LatencyStats {
    LatencyStats {
        p50_ms: percentile(buckets, 0.50),
        p95_ms: percentile(buckets, 0.95),
        p99_ms: percentile(buckets, 0.99),
        samples: buckets.iter().sum(),
        saturated: is_saturated(buckets, 0.99),
    }
}

fn read_buckets(row: &sqlx::postgres::PgRow) -> [u64; BUCKET_COUNT] {
    let mut buckets = [0u64; BUCKET_COUNT];
    for (i, slot) in buckets.iter_mut().enumerate() {
        let v: i64 = row.try_get(format!("b{i:02}").as_str()).unwrap_or(0);
        *slot = v.max(0) as u64;
    }
    buckets
}

fn internal<E: std::fmt::Display>(what: &'static str) -> impl FnOnce(E) -> StatusCode {
    move |e| {
        tracing::error!("{what}: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// GET /admin/v1/projects/:project_id/metrics
///
/// Session auth, any role.
///   - 404 if project does not exist, is deleted, or belongs to a different account.
///   - 422 if `window` is not `24h` or `7d`.
///   - 200 + ProjectMetrics on success.
pub async fn get_project_metrics(
    Path(project_id): Path<String>,
    Query(query): Query<MetricsQuery>,
    State(state): State<UserAdminState>,
    session: SessionContext,
) -> Result<Json<ProjectMetrics>, StatusCode> {
    let pool = state.system_db.pool();

    // Foreign projects are 404, never 403 (no existence leak).
    verify_project_ownership(pool, &project_id, session.account_id)
        .await
        .map_err(|s| if s == StatusCode::FORBIDDEN { StatusCode::NOT_FOUND } else { s })?;
    let hours = window_hours(query.window.as_deref()).ok_or(StatusCode::UNPROCESSABLE_ENTITY)?;

    // Today's counters: daily_project_metrics stays the source (billing); zeros if absent.
    let (reads_today, writes_today, deletes_today): (i64, i64, i64) = sqlx::query(
        "SELECT read_ops, write_ops, delete_ops \
         FROM daily_project_metrics \
         WHERE project_id = $1 AND date = CURRENT_DATE",
    )
    .bind(&project_id)
    .fetch_optional(pool)
    .await
    .map_err(internal("get_project_metrics: fetching daily counters"))?
    .map(|r| {
        (
            r.try_get("read_ops").unwrap_or(0),
            r.try_get("write_ops").unwrap_or(0),
            r.try_get("delete_ops").unwrap_or(0),
        )
    })
    .unwrap_or((0, 0, 0));

    let current_hour = truncate_to_hour(Utc::now());
    let since = current_hour - Duration::hours(hours - 1);
    let rows: Vec<RollupRow> = sqlx::query(&format!(
        "SELECT hour, op, units, errors, {BUCKET_COLS} FROM hourly_project_metrics \
         WHERE project_id = $1 AND hour >= $2"
    ))
    .bind(&project_id)
    .bind(since)
    .fetch_all(pool)
    .await
    .map_err(internal("get_project_metrics: fetching hourly rollup"))?
    .iter()
    .map(|r| RollupRow {
        hour: r.try_get("hour").unwrap_or(current_hour),
        op: r.try_get("op").unwrap_or(0),
        units: r.try_get("units").unwrap_or(0),
        errors: r.try_get("errors").unwrap_or(0),
        buckets: read_buckets(r),
    })
    .collect();

    let Folded { series, read, write } = fold(&rows, current_hour, hours);
    let latency = Latency { read: latency_stats(&read), write: latency_stats(&write) };
    let sparkline = series[series.len() - SPARKLINE_HOURS..]
        .iter()
        .enumerate()
        .map(|(i, p)| SparklinePoint { hour: i as u8, reads: p.reads, writes: p.writes })
        .collect();

    Ok(Json(ProjectMetrics {
        reads_today,
        writes_today,
        deletes_today,
        p95_read_ms: latency.read.p95_ms.unwrap_or(0.0),
        p95_write_ms: latency.write.p95_ms.unwrap_or(0.0),
        latency,
        series,
        sparkline,
    }))
}

/// GET /admin/v1/metrics/summary
///
/// Session auth, any role. Every non-deleted project of the caller's account:
/// today's counters plus the last-24h p95 (`null` without samples).
pub async fn get_metrics_summary(
    State(state): State<UserAdminState>,
    session: SessionContext,
) -> Result<Json<Vec<ProjectSummary>>, StatusCode> {
    let pool = state.system_db.pool();

    let mut summaries: Vec<ProjectSummary> = sqlx::query(
        "SELECT p.id, COALESCE(p.name, p.id) AS name, \
                COALESCE(d.read_ops, 0) AS read_ops, \
                COALESCE(d.write_ops, 0) AS write_ops, \
                COALESCE(d.delete_ops, 0) AS delete_ops \
         FROM projects p \
         LEFT JOIN daily_project_metrics d ON d.project_id = p.id AND d.date = CURRENT_DATE \
         WHERE p.account_id = $1 AND p.status != 'deleted' \
         ORDER BY p.created_at DESC, p.id",
    )
    .bind(session.account_id)
    .fetch_all(pool)
    .await
    .map_err(internal("get_metrics_summary: fetching projects"))?
    .iter()
    .map(|r| ProjectSummary {
        project_id: r.try_get("id").unwrap_or_default(),
        name: r.try_get("name").unwrap_or_default(),
        reads_today: r.try_get("read_ops").unwrap_or(0),
        writes_today: r.try_get("write_ops").unwrap_or(0),
        deletes_today: r.try_get("delete_ops").unwrap_or(0),
        p95_read_ms: None,
        p95_write_ms: None,
    })
    .collect();

    let since = truncate_to_hour(Utc::now()) - Duration::hours(23);
    let ids: Vec<String> = summaries.iter().map(|s| s.project_id.clone()).collect();
    let sums = sqlx::query(&format!(
        "SELECT project_id, op IN (0, 3) AS is_read, {BUCKET_SUMS} \
         FROM hourly_project_metrics \
         WHERE project_id = ANY($1) AND hour >= $2 \
         GROUP BY project_id, op IN (0, 3)"
    ))
    .bind(&ids)
    .bind(since)
    .fetch_all(pool)
    .await
    .map_err(internal("get_metrics_summary: fetching hourly rollup"))?;

    let mut by_project: HashMap<String, ([u64; BUCKET_COUNT], [u64; BUCKET_COUNT])> = HashMap::new();
    for r in &sums {
        let id: String = r.try_get("project_id").unwrap_or_default();
        let is_read: bool = r.try_get("is_read").unwrap_or(false);
        let entry = by_project.entry(id).or_default();
        add(if is_read { &mut entry.0 } else { &mut entry.1 }, &read_buckets(r));
    }
    for s in &mut summaries {
        if let Some((read, write)) = by_project.get(&s.project_id) {
            s.p95_read_ms = percentile(read, 0.95);
            s.p95_write_ms = percentile(write, 0.95);
        }
    }
    Ok(Json(summaries))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(day: u32, h: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, day, h, 0, 0).unwrap()
    }

    fn row(hour: DateTime<Utc>, op: i16, units: i64, errors: i64, bucket: usize) -> RollupRow {
        let mut buckets = [0u64; BUCKET_COUNT];
        buckets[bucket] = units as u64;
        RollupRow { hour, op, units, errors, buckets }
    }

    #[test]
    fn window_accepts_only_24h_and_7d() {
        assert_eq!(window_hours(None), Some(24));
        assert_eq!(window_hours(Some("24h")), Some(24));
        assert_eq!(window_hours(Some("7d")), Some(168));
        assert_eq!(window_hours(Some("30d")), None);
        assert_eq!(window_hours(Some("")), None);
    }

    /// AC-HMR-04: one entry per hour, zero-filled, ascending, ending at the current hour.
    #[test]
    fn series_is_zero_filled_ascending_and_ends_at_current_hour() {
        let now = at(13, 9);
        let rows = [
            row(at(13, 9), 0, 5, 1, 2),
            row(at(13, 7), 1, 3, 0, 3),
            row(at(12, 20), 2, 2, 2, 3),
            row(at(13, 7), 3, 4, 0, 4), // run_query counts as reads
            row(at(1, 0), 0, 99, 0, 0), // outside the window: ignored
        ];
        let f = fold(&rows, now, 24);
        assert_eq!(f.series.len(), 24);
        assert_eq!(f.series[23].hour, "2026-11-13T09:00:00Z");
        assert_eq!(f.series[0].hour, "2026-11-12T10:00:00Z");
        assert!(f.series.windows(2).all(|w| w[0].hour < w[1].hour));
        assert_eq!(fold(&rows, now, 168).series.len(), 168);

        let at_9 = &f.series[23];
        assert_eq!((at_9.reads, at_9.writes, at_9.deletes, at_9.errors), (5, 0, 0, 1));
        let at_7 = &f.series[21];
        assert_eq!((at_7.reads, at_7.writes), (4, 3));
        let total: i64 = f.series.iter().map(|p| p.reads + p.writes + p.deletes).sum();
        assert_eq!(total, 5 + 3 + 2 + 4);
        assert_eq!(f.read.iter().sum::<u64>(), 9);
        assert_eq!(f.write.iter().sum::<u64>(), 5);
    }

    /// AC-HMR-03: no samples means null percentiles and 0 samples.
    #[test]
    fn empty_window_has_null_percentiles() {
        let f = fold(&[], at(13, 9), 24);
        let stats = latency_stats(&f.read);
        assert_eq!(
            stats,
            LatencyStats { p50_ms: None, p95_ms: None, p99_ms: None, samples: 0, saturated: false }
        );
    }
}
