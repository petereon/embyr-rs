// @driving_port @real-io @US-OBS-05
#![allow(unused_imports)]
//! US-OBS-05 — embyr_pg_pool_size / embyr_pg_pool_idle gauges for the system pool.
//!
//! Acceptance criteria verified here:
//!   AC-OBS-05-01 / -02: both gauges are present with `pool="system"` in the first scrape
//!   (the `/metrics` handler refreshes them immediately before rendering, D-OBS-8).
//!   AC-OBS-05-04: idle never exceeds size.
//!
//! Driving port: Admin :9090 (reqwest → GET /metrics).
//!
//! Implemented 2026-11-13 (formerly `todo!()` scaffolds, `#[ignore]`d).

#[path = "../common/mod.rs"]
mod common;
use common::{metric_value, ObsTestContext};

// ─── AC-OBS-05-01 / AC-OBS-05-02: Gauges present in initial scrape ───────────

/// The pool-size gauge is present and positive (migrations already used a connection).
///
/// @driving_port @real-io @US-OBS-05 @happy
#[tokio::test]
async fn pool_size_gauge_present_in_metrics() {
    let ctx = ObsTestContext::start().await;

    let body = ctx.get_metrics().await;

    assert!(
        body.lines().any(|l| l.starts_with("embyr_pg_pool_size{") && l.contains("pool=\"system\"")),
        "missing embyr_pg_pool_size{{pool=\"system\"}}; body:\n{body}"
    );
    assert!(metric_value(&body, "embyr_pg_pool_size", &[("pool", "system")]) >= 1.0);
}

// ─── AC-OBS-05-02 / AC-OBS-05-04: Idle gauge present ────────────────────────

/// The pool-idle gauge is present and never exceeds the pool size.
///
/// @driving_port @real-io @US-OBS-05 @happy
#[tokio::test]
async fn pool_idle_gauge_present_in_metrics() {
    let ctx = ObsTestContext::start().await;

    let body = ctx.get_metrics().await;

    assert!(
        body.lines().any(|l| l.starts_with("embyr_pg_pool_idle{") && l.contains("pool=\"system\"")),
        "missing embyr_pg_pool_idle{{pool=\"system\"}}; body:\n{body}"
    );
    let size = metric_value(&body, "embyr_pg_pool_size", &[("pool", "system")]);
    let idle = metric_value(&body, "embyr_pg_pool_idle", &[("pool", "system")]);
    assert!(idle <= size, "idle {idle} must not exceed size {size}");
}
