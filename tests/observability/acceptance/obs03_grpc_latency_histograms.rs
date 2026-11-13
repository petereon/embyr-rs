// @driving_port @real-io @US-OBS-03
#![allow(unused_imports)]
//! US-OBS-03 — embyr_grpc_request_duration_seconds histogram per gRPC call.
//!
//! Acceptance criteria verified here:
//!   AC-OBS-03-01: a gRPC call records a sample in the histogram (`_count` >= 1).
//!   AC-OBS-03-03 / -04: the exposition has `_bucket` (incl. `+Inf`), `_sum` and `_count` series.
//!
//! Driving port: gRPC :8080 (tonic GetDocument) then Admin :9090 (GET /metrics).
//! Assertion level: real Postgres container + in-process servers. The Prometheus
//! recorder is process-global, so assertions use `>=` / presence, never exact counts.
//!
//! Implemented 2026-11-13 (formerly `todo!()` scaffolds, `#[ignore]`d).

#[path = "../common/mod.rs"]
mod common;
use common::{metric_value, ObsTestContext};

const COUNT: &str = "embyr_grpc_request_duration_seconds_count";
const BUCKET: &str = "embyr_grpc_request_duration_seconds_bucket";
const SUM: &str = "embyr_grpc_request_duration_seconds_sum";

// ─── AC-OBS-03-01 / AC-OBS-03-04: Histogram recorded after a call ─────────────

/// One gRPC call causes the latency histogram `_count` for that method to be >= 1.
///
/// @driving_port @real-io @US-OBS-03 @happy
#[tokio::test]
async fn grpc_request_records_latency_histogram() {
    let ctx = ObsTestContext::start().await;

    ctx.make_grpc_call("obs-hist-proj").await;
    let body = ctx.get_metrics().await;

    assert!(
        metric_value(&body, COUNT, &[("method", "GetDocument")]) >= 1.0,
        "expected {COUNT}{{method=\"GetDocument\"}} >= 1; body:\n{body}"
    );
}

// ─── AC-OBS-03-03 / AC-OBS-03-04: Bucket boundaries in output ────────────────

/// The histogram exposition contains the configured buckets (12 + `+Inf`), `_sum` and `_count`.
///
/// @driving_port @real-io @US-OBS-03 @happy
#[tokio::test]
async fn histogram_output_contains_bucket_and_sum_and_count() {
    let ctx = ObsTestContext::start().await;

    ctx.make_grpc_call("obs-hist-proj").await;
    let body = ctx.get_metrics().await;

    let bucket_prefix = format!("{BUCKET}{{method=\"GetDocument\"");
    let buckets: Vec<&str> = body.lines().filter(|l| l.starts_with(&bucket_prefix)).collect();
    assert!(buckets.len() >= 13, "expected 12 buckets + +Inf, got {}:\n{body}", buckets.len());
    assert!(buckets.iter().any(|l| l.contains("le=\"+Inf\"")), "missing +Inf bucket:\n{body}");
    assert!(
        body.lines().any(|l| l.starts_with(&format!("{SUM}{{method=\"GetDocument\""))),
        "missing {SUM} series:\n{body}"
    );
    assert!(
        body.lines().any(|l| l.starts_with(&format!("{COUNT}{{method=\"GetDocument\""))),
        "missing {COUNT} series:\n{body}"
    );
}
