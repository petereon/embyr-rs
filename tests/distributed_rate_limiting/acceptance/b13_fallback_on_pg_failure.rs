// @driving_port @real-io @adapter-integration @infrastructure-failure @US-13
#![allow(unused_imports)]
//! US-13 — 20ms timeout + per-instance fallback on Postgres unavailability.
//!
//! Acceptance criteria verified here:
//!   AC-13-01: When Postgres is unreachable, requests are allowed (fail-open) capped at configured RPS.
//!   AC-13-02: Fallback rate is capped at 1× configured capacity (not ∞).
//!   AC-13-03: `check()` resolves within 20ms when Postgres is slow (timeout fires; fallback used).
//!
//! Driving port: gRPC :8080 (tonic FirestoreClient → GetDocument).
//! Assertion level: real Postgres container + in-process gRPC server.
//!
//! Postgres disruption approach:
//!   These tests need to simulate Postgres unavailability AFTER the server starts.
//!   The chosen approach is to close all connections in the pool + block new ones via
//!   a Postgres-level `pg_sleep()` or by changing the `max_connections` parameter.
//!   See TODO comments below for the exact mechanism (DRL-03 DELIVER decides).
//!
//! All tests are #[ignore] — DELIVER unskips them as part of DRL-03.
//!
//! Implementation note for DELIVER (DRL-03):
//!   - `RateLimiter::check()` must call `tokio::time::timeout(20ms, check_pg(...))`.
//!   - On timeout: increment `rate_limit_pg_timeout_total`, fall back to per-instance TokenBucket.
//!   - In-process fallback uses TokenBucket with same capacity — not unlimited.
//!   - AC-13-03: measure wall-clock elapsed before and after `check()` under simulated 500ms PG delay.

#[path = "../common/mod.rs"]
mod common;
use common::{classify_grpc_status, get_document_request, start_distributed_grpc_server, DrlTestContext};
use embyr_proto::firestore::firestore_client::FirestoreClient;

// ─── AC-13-01: fallback allows requests when PG unreachable ──────────────────

/// When Postgres is unavailable, gRPC requests are allowed up to the configured capacity.
///
/// AC-13-01
///
/// Journey:
///   Given: project "proj-fallback" with token bucket at capacity
///   And:   Postgres is paused / unreachable (simulated via connection pool exhaustion or PG sleep)
///   When:  client sends GetDocument request for "proj-fallback"
///   Then:  request is allowed (fail-open: status is "ok" or "not_found", NOT "resource_exhausted")
///   And:   `rate_limit_pg_timeout_total` counter has incremented by 1
///
/// @driving_port @real-io @infrastructure-failure @US-13 @AC-13-01
#[tokio::test]
#[ignore]
async fn fallback_allows_requests_when_pg_unavailable() {
    // SCAFFOLD: true — DRL-03 DELIVER implements this.
    // TODO: implement pg pause
    // DELIVER steps:
    //   1. ctx = DrlTestContext::new(10.0).await; seed "proj-fallback" at capacity
    //   2. Start distributed gRPC server; capture metrics handle
    //   3. Record before_counter = read rate_limit_pg_timeout_total from metrics
    //   4. [pg pause] — pause Postgres via testcontainers exec or configure pool to reject new connections
    //   5. let result = client.get_document(get_document_request("proj-fallback", "key-fb")).await;
    //   6. assert classify_grpc_status(&result) != "resource_exhausted"
    //   7. Read after_counter; assert after_counter == before_counter + 1
    let _ctx = DrlTestContext::new(10.0).await;
    todo!(
        "DRL-03 DELIVER: pause PG; assert request allowed (fail-open) + pg_timeout_total incremented"
    )
}

// ─── AC-13-02: fallback is capped at configured capacity ─────────────────────

/// When Postgres is unavailable, the per-instance fallback bucket enforces the same capacity.
///
/// AC-13-02
///
/// Journey (chained — Given reuses PG-unavailable state from AC-13-01):
///   Given: project "proj-fallback-cap" with per-instance fallback active
///   And:   Postgres is still paused
///   When:  client sends (capacity + 1) requests sequentially via per-instance fallback
///   Then:  first `capacity` requests are allowed
///   And:   the (capacity + 1)th request returns RESOURCE_EXHAUSTED (fallback not unlimited)
///
/// @driving_port @real-io @infrastructure-failure @US-13 @AC-13-02
#[tokio::test]
#[ignore]
async fn fallback_capped_at_configured_limit() {
    // SCAFFOLD: true — DRL-03 DELIVER implements this.
    // TODO: implement pg pause
    // DELIVER steps:
    //   1. ctx = DrlTestContext::new(3.0).await; seed "proj-fallback-cap" (capacity=3.0)
    //   2. [pg pause]
    //   3. Send 3 requests → all allowed via fallback
    //   4. Send 4th request → RESOURCE_EXHAUSTED (fallback bucket exhausted)
    let _ctx = DrlTestContext::new(3.0).await;
    todo!(
        "DRL-03 DELIVER: assert fallback bucket enforces same capacity as distributed mode"
    )
}

// ─── AC-13-03: fallback resolves within 20ms ─────────────────────────────────

/// The gRPC request completes within 20ms when Postgres is slow (500ms simulated delay).
///
/// AC-13-03
///
/// Journey (chained — Given reuses fallback-active state):
///   Given: project "proj-latency" registered
///   And:   Postgres responds with a 500ms delay (via pg_sleep or network shaping)
///   When:  client sends GetDocument request
///   Then:  `RateLimiter::check()` returns within 20ms (timeout fires, fallback used)
///   And:   the gRPC response is received by the client within 25ms of the check() call
///
/// NOTE: This tests the hard bound — NOT a p99 assertion.
/// The 20ms timeout is hard-coded (D3); this test verifies it actually fires.
///
/// @driving_port @real-io @infrastructure-failure @US-13 @AC-13-03
#[tokio::test]
#[ignore]
async fn fallback_resolves_within_20ms() {
    // SCAFFOLD: true — DRL-03 DELIVER implements this.
    // TODO: implement pg pause
    // DELIVER steps:
    //   1. ctx = DrlTestContext::new(10.0).await; seed "proj-latency"
    //   2. Inject 500ms Postgres delay (pg_sleep via a long-running TX or connection monopoly)
    //   3. let start = std::time::Instant::now();
    //   4. let _result = client.get_document(get_document_request("proj-latency", "key-lat")).await;
    //   5. let elapsed = start.elapsed();
    //   6. assert!(elapsed < Duration::from_millis(50), "expected < 50ms; got {:?}", elapsed);
    //      (50ms is the test deadline: 20ms timeout + ~20ms server overhead + 10ms gRPC round-trip)
    let _ctx = DrlTestContext::new(10.0).await;
    todo!(
        "DRL-03 DELIVER: inject 500ms PG delay; assert check() completes within 20ms timeout"
    )
}
