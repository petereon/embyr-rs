// @walking_skeleton @driving_port @real-io @US-OBS-02
#![allow(unused_imports)]
//! US-OBS-02 — embyr_grpc_requests_total counter incremented per gRPC call.
//!
//! Acceptance criteria verified here:
//!   AC-OBS-02-01: embyr_grpc_requests_total{method, status} incremented on every gRPC return.
//!   AC-OBS-02-03: status label maps tonic::Code to lowercase string.
//!   AC-OBS-02-04: Counter incremented AFTER handler result is determined (both OK and error).
//!
//! Two former scaffolds were deleted 2026-11-13 as duplicates:
//!   - `rate_limit_rejection_increments_embyr_rate_limit_rejected_total` — covered by
//!     `obs04_rate_limit_metrics::rejected_request_increments_rate_limit_rejected_total`.
//!   - `pg_timeout_increments_embyr_rate_limit_pg_timeout_total` — covered by
//!     `b19_fail_open_on_pg_error::existing_timeout_fallback_is_unchanged_by_this_fix`
//!     (AC-RLFO-06 asserts the pg-timeout counter increments exactly once).
//!
//! Driving port: gRPC :8080 (tonic FirestoreClient → GetDocument) then Admin :9090
//!               (reqwest → GET /metrics to observe the counter).
//! Assertion level: real Postgres container (testcontainers-rs) + in-process servers.
//!
//! Walking skeleton: `grpc_request_increments_embyr_grpc_requests_total`
//!   — NOT #[ignore]; implemented by OBS-02 DELIVER.

#[path = "../common/mod.rs"]
mod common;
use common::ObsTestContext;

// ─── Walking skeleton ─────────────────────────────────────────────────────────

/// Walking skeleton: a single gRPC call causes embyr_grpc_requests_total to appear in /metrics.
///
/// AC-OBS-02-01
///
/// Journey:
///   Given: embyr running with PrometheusHandle installed
///   When:  Firebase SDK calls GetDocument for "obs-ws-proj"
///   Then:  GET /metrics shows a data line for embyr_grpc_requests_total with method="GetDocument"
///
/// The call is sent without a valid API key — it will fail with status="unauthenticated"
/// but the counter still fires (AC-OBS-02-04: counter fires on every exit path).
///
/// @walking_skeleton @driving_port @real-io @US-OBS-02 @AC-OBS-02-01
#[tokio::test]
async fn grpc_request_increments_embyr_grpc_requests_total() {
    let ctx = ObsTestContext::start().await;

    // Trigger the GetDocument handler — unauthenticated call is sufficient.
    ctx.make_grpc_call("obs-ws-proj").await;

    let body = ctx.get_metrics().await;

    // Assert that at least one data line for the counter exists with method="GetDocument".
    // The full line looks like:
    //   embyr_grpc_requests_total{method="GetDocument",status="unauthenticated"} 1
    let counter_present = body
        .lines()
        .filter(|l| !l.starts_with('#'))
        .any(|l| {
            l.starts_with("embyr_grpc_requests_total{")
                && l.contains(r#"method="GetDocument""#)
        });

    assert!(
        counter_present,
        "Expected embyr_grpc_requests_total{{method=\"GetDocument\",...}} in /metrics.\n\
         Actual /metrics body:\n{body}"
    );
}

// ─── AC-OBS-02-01 / AC-OBS-02-03: Error path counter ────────────────────────

/// gRPC call with invalid API key increments unauthenticated status counter.
///
/// AC-OBS-02-01, AC-OBS-02-03
///
/// @driving_port @real-io @US-OBS-02 @error
#[tokio::test]
async fn grpc_error_increments_requests_total_with_error_status() {
    let ctx = ObsTestContext::start().await;

    ctx.make_grpc_call("obs-err-proj").await;
    let body = ctx.get_metrics().await;

    let value = ObsTestContext::parse_metric_value(
        &body,
        "embyr_grpc_requests_total",
        r#"{method="GetDocument",status="unauthenticated"}"#,
    )
    .and_then(|v| v.parse::<f64>().ok())
    .unwrap_or(0.0);
    assert!(
        value >= 1.0,
        "expected embyr_grpc_requests_total{{method=\"GetDocument\",status=\"unauthenticated\"}} >= 1; body:\n{body}"
    );
}
