// @walking_skeleton @driving_port @real-io @US-12
#![allow(unused_imports)]
//! US-12 — Atomic Postgres UPDATE enforces distributed rate limit.
//!
//! Acceptance criteria verified here:
//!   AC-12-01: When token bucket is empty, gRPC returns RESOURCE_EXHAUSTED.
//!   AC-12-02: Concurrent requests from multiple nodes each decrement the shared token count atomically.
//!   AC-12-03: Token bucket refills at the configured RPS rate after 1 second.
//!
//! Driving port: gRPC :8080 (tonic FirestoreClient → GetDocument).
//! Assertion level: real Postgres container (testcontainers-rs) + in-process gRPC server.
//!
//! Walking skeleton: `distributed_rate_limit_rejects_when_bucket_exhausted`
//!   — NOT #[ignore]; RED via todo!(); first test to implement in DRL-02.
//!
//! Implementation note for DELIVER (DRL-02):
//!   - `start_distributed_grpc_server(system_db, capacity)` must be implemented in embyr_server::lib.
//!   - `RateLimiter::new(capacity, refill_rate, pg_pool: Some(...))` must be wired.
//!   - `check()` must return `Result<RateLimitInfo, RateLimitInfo>`.
//!   - gRPC handlers must map `Err(info)` → `Status::resource_exhausted(...)`.

#[path = "../common/mod.rs"]
mod common;
use common::{
    classify_grpc_status, get_document_request, start_distributed_grpc_server, universe,
    DrlTestContext,
};
use embyr_proto::firestore::firestore_client::FirestoreClient;
use std::collections::HashMap;

// ─── Walking skeleton ─────────────────────────────────────────────────────────

/// Walking skeleton: exhausted token bucket causes gRPC to reject with RESOURCE_EXHAUSTED.
///
/// AC-12-01
///
/// Journey:
///   Given: project "proj-ws" with token bucket initialized at 0.0 tokens
///   When:  client sends GetDocument request for "proj-ws"
///   Then:  gRPC response status is RESOURCE_EXHAUSTED
///
/// @walking_skeleton @driving_port @real-io @US-12 @AC-12-01
///
/// NOT #[ignore] — this is the walking skeleton.
#[tokio::test]
async fn distributed_rate_limit_rejects_when_bucket_exhausted() {
    let ctx = DrlTestContext::new(10.0).await;
    ctx.insert_project_with_bucket("proj-ws", "key-ws", 0.0).await;
    let (addr, _server) = start_distributed_grpc_server(&ctx).await;
    let ctx = ctx.with_grpc_addr(addr);
    let mut client = FirestoreClient::new(ctx.grpc_channel());
    let result = client.get_document(get_document_request("proj-ws", "key-ws")).await;
    assert_eq!(classify_grpc_status(&result), "resource_exhausted");
}

// ─── AC-12-02: Postgres atomic UPDATE enforces limit at configured RPS ────────

/// Rate limit is enforced exactly at the configured RPS threshold.
///
/// AC-12-02
///
/// Journey (chained — Given reuses context from walking skeleton):
///   Given: project "proj-rps" with token bucket at 3.0 tokens (rate_limit_rps=3.0)
///   When:  client sends 3 sequential GetDocument requests (each consumes 1 token)
///   Then:  first 3 requests are allowed (status: not RESOURCE_EXHAUSTED)
///   And:   4th request is rejected with RESOURCE_EXHAUSTED
///   And:   rate_buckets.tokens approaches 0.0 after the 3rd allowed request
///
/// @driving_port @real-io @US-12 @AC-12-02
#[tokio::test]
#[ignore]
async fn rate_limit_enforced_at_configured_rps() {
    // SCAFFOLD: true — DRL-02 DELIVER implements this.
    // DELIVER steps:
    //   1. ctx = DrlTestContext::new(3.0).await; (capacity=3, rate tight enough to exhaust in 3 requests)
    //   2. ctx.insert_project_with_bucket("proj-rps", "key-rps", 3.0).await;
    //   3. start distributed server with capacity=3.0
    //   4. Fire 3 requests → all allowed; 4th request → RESOURCE_EXHAUSTED
    //   5. assert_state_delta: RATE_BUCKET_TOKENS approaches "0.0"
    todo!(
        "DRL-02 DELIVER: assert exactly `capacity` requests allowed before RESOURCE_EXHAUSTED"
    )
}

// ─── AC-12-03: atomic UPDATE is the shared enforcement point ──────────────────

/// Token count decrements atomically for each allowed request.
///
/// AC-12-03
///
/// Journey (chained — Given reuses "proj-rps" with bucket initialized at capacity):
///   Given: project "proj-atomic" with token bucket at 5.0 tokens
///   When:  3 requests are sent sequentially
///   Then:  rate_buckets.tokens decrements by approximately 1.0 per request
///   And:   no two requests observe the same token count post-deduction
///
/// @driving_port @real-io @US-12 @AC-12-03
#[tokio::test]
#[ignore]
async fn rate_bucket_tokens_decrease_atomically() {
    // SCAFFOLD: true — DRL-02 DELIVER implements this.
    // DELIVER steps:
    //   1. ctx = DrlTestContext::new(10.0).await; insert "proj-atomic" with 5.0 tokens
    //   2. Send 3 GetDocument requests sequentially
    //   3. After each request, query rate_buckets.tokens via ctx.query_rate_bucket("proj-atomic")
    //   4. Assert tokens decrease by ~1.0 each step (within tolerance for time-based refill)
    todo!(
        "DRL-02 DELIVER: assert rate_buckets.tokens decrements atomically per request"
    )
}

// ─── AC-12-04: tokens refill after 1 second ──────────────────────────────────

/// Token bucket refills at the configured rate after 1 second of inactivity.
///
/// AC-12-04
///
/// Journey (chained):
///   Given: project "proj-refill" with token bucket exhausted (0.0 tokens)
///   When:  client waits 1 second (sufficient for rate_limit_rps tokens to accrue)
///   And:   client sends a GetDocument request
///   Then:  the request is allowed (status: "ok" or "not_found")
///   And:   rate_buckets.tokens is approximately rate_limit_rps - 1.0
///
/// @driving_port @real-io @US-12 @AC-12-04
#[tokio::test]
#[ignore]
async fn tokens_refill_after_one_second() {
    // SCAFFOLD: true — DRL-02 DELIVER implements this.
    // DELIVER steps:
    //   1. ctx = DrlTestContext::new(10.0).await; insert "proj-refill" with 0.0 tokens
    //   2. Verify first request → RESOURCE_EXHAUSTED (no tokens)
    //   3. tokio::time::sleep(Duration::from_secs(1)).await
    //   4. Send GetDocument → should be allowed (token refilled)
    //   5. assert classify_grpc_status != "resource_exhausted"
    //   6. Assert rate_buckets.tokens approximately == rate_limit_rps - 1.0 (within ±0.5 for timing jitter)
    todo!(
        "DRL-02 DELIVER: assert token bucket refills after 1 second (rate = EMBYR_RATE_LIMIT_RPS)"
    )
}
