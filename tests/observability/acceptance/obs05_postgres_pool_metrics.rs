// @driving_port @real-io @US-OBS-05
#![allow(unused_imports)]
//! US-OBS-05 — embyr_pg_pool_size and embyr_pg_pool_idle gauges for system DB pool.
//!
//! Acceptance criteria verified here:
//!   AC-OBS-05-01: embyr_pg_pool_size{pool="system"} gauge reflects pool.size() as f64.
//!   AC-OBS-05-02: embyr_pg_pool_idle{pool="system"} gauge reflects pool.num_idle() as f64.
//!   AC-OBS-05-03: Gauges updated every 15 seconds by background task.
//!   AC-OBS-05-04: Gauges also updated immediately before handle.render() in /metrics handler.
//!
//! Driving port: Admin :9090 (reqwest → GET /metrics to observe pool gauges).
//! Assertion level: real Postgres container (testcontainers-rs) + in-process admin server.
//!
//! All tests are #[ignore] — OBS-01 must be green first.
//!
//! Implementation notes for DELIVER (OBS-05):
//!   - Background task spawned in lib.rs alongside sweeper tasks.
//!   - `tokio::time::interval(Duration::from_secs(15))` loop; updates gauges on each tick.
//!   - `get_prometheus_metrics` handler calls pool.size() + pool.num_idle() before handle.render().
//!   - System DB only — customer DB pools are NOT instrumented in V1 (cardinality unbounded).
//!   - SQLite AnyPool path (dev mode) must not panic — use conditional branch or no-op.

#[path = "../common/mod.rs"]
mod common;
use common::ObsTestContext;

// ─── AC-OBS-05-01 / AC-OBS-05-02: Gauges present in initial scrape ───────────

/// Pool size and idle gauges appear in the very first /metrics scrape after server start.
///
/// AC-OBS-05-01, AC-OBS-05-02, AC-OBS-05-04
///
/// Journey:
///   Given: embyr running with system DB pool (testcontainers Postgres)
///   When:  GET /metrics scraped
///   Then:  body contains embyr_pg_pool_size{pool="system"} with numeric value ≥ 1
///   And:   body contains embyr_pg_pool_idle{pool="system"} with numeric value ≥ 0
///   And:   embyr_pg_pool_idle ≤ embyr_pg_pool_size (idle cannot exceed total)
///
/// @driving_port @real-io @US-OBS-05 @happy
#[tokio::test]
#[ignore]
async fn pool_size_gauge_present_in_metrics() {
    todo!()
}

// ─── AC-OBS-05-02 / AC-OBS-05-04: Idle gauge present ────────────────────────

/// Pool idle gauge reflects current idle connection count immediately before scrape render.
///
/// AC-OBS-05-02, AC-OBS-05-04
///
/// Journey (chained — Given reuses running server from previous test):
///   Given: embyr running with system DB pool and no active gRPC requests
///   When:  GET /metrics scraped
///   Then:  body contains embyr_pg_pool_idle{pool="system"} with value ≥ 0
///   And:   value equals pool.num_idle() observed at scrape time (within 1 for concurrent updates)
///
/// @driving_port @real-io @US-OBS-05 @happy
#[tokio::test]
#[ignore]
async fn pool_idle_gauge_present_in_metrics() {
    todo!()
}
