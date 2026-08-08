// @walking_skeleton @driving_port @real-io @US-OBS-02
#![allow(unused_imports)]
//! US-OBS-02 — embyr_grpc_requests_total counter incremented per gRPC call.
//!
//! Acceptance criteria verified here:
//!   AC-OBS-02-01: embyr_grpc_requests_total{method, status} incremented on every gRPC return.
//!   AC-OBS-02-03: status label maps tonic::Code to lowercase string.
//!   AC-OBS-02-04: Counter incremented AFTER handler result is determined (both OK and error).
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
/// Journey (chained — Given reuses running server from walking skeleton):
///   Given: embyr running with project "obs-ws-proj"
///   When:  Firebase SDK calls GetDocument with an invalid API key
///   Then:  GET /metrics shows embyr_grpc_requests_total{method="GetDocument",status="unauthenticated"}
///          with value ≥ 1
///
/// @driving_port @real-io @US-OBS-02 @error
#[tokio::test]
#[ignore]
async fn grpc_error_increments_requests_total_with_error_status() {
    todo!()
}

// ─── AC-OBS-02-01 / AC-OBS-02-03: Rate-limit rejection counter ───────────────

/// Rate-limit rejection increments embyr_rate_limit_rejected_total and the resource_exhausted counter.
///
/// AC-OBS-02-01, AC-OBS-02-03
///
/// Journey (chained — Given reuses running server, project with exhausted token bucket):
///   Given: embyr running with project "obs-rate-proj" and token bucket at 0.0
///   When:  Firebase SDK sends GetDocument for "obs-rate-proj"
///   Then:  GET /metrics shows embyr_grpc_requests_total{method="GetDocument",status="resource_exhausted"}
///          with value ≥ 1
///
/// @driving_port @real-io @US-OBS-02 @US-OBS-04 @error
#[tokio::test]
#[ignore]
async fn rate_limit_rejection_increments_embyr_rate_limit_rejected_total() {
    todo!()
}

// ─── AC-OBS-04-03: pg_timeout counter ────────────────────────────────────────

/// Postgres timeout during rate-limit check increments embyr_rate_limit_pg_timeout_total.
///
/// AC-OBS-04-03
///
/// Journey:
///   Given: embyr running with Postgres rate limiting enabled
///   And:   system DB is unreachable (simulated via connection drop / timeout injection)
///   When:  Firebase SDK sends any GetDocument request
///   Then:  GET /metrics shows embyr_rate_limit_pg_timeout_total incremented by 1
///   And:   the request falls back to the in-process token bucket (allowed or rejected per state)
///
/// @driving_port @real-io @US-OBS-04 @infrastructure-failure @error
#[tokio::test]
#[ignore]
async fn pg_timeout_increments_embyr_rate_limit_pg_timeout_total() {
    todo!()
}
