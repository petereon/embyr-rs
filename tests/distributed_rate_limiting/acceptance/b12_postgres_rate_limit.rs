// @walking_skeleton @driving_port @real-io @US-12
#![allow(unused_imports)]
//! US-12 — Atomic Postgres UPDATE enforces distributed rate limit.
//!
//! Acceptance criteria verified here:
//!   AC-12-01: When token bucket is empty, gRPC returns RESOURCE_EXHAUSTED.
//!   AC-12-02: Rate limit enforced exactly at the configured capacity (covered by
//!             `b19_fail_open_on_pg_error::pg_healthy_rate_limiting_behaves_unchanged`:
//!             capacity allowed, then RESOURCE_EXHAUSTED — the former scaffold here
//!             was deleted 2026-11-13 as a duplicate).
//!   AC-12-03: Each allowed request decrements the shared Postgres token count.
//!   AC-12-04: Token bucket refills at the configured RPS rate after 1 second.
//!
//! Driving port: gRPC :8080 (tonic FirestoreClient → GetDocument).
//! Assertion level: real Postgres container (testcontainers-rs) + in-process gRPC server.
//!
//! Walking skeleton: `distributed_rate_limit_rejects_when_bucket_exhausted`
//!   — the first test implemented in DRL-02.
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
    warm_credential_cache_and_reset_bucket, DrlTestContext,
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

// ─── AC-12-03: atomic UPDATE is the shared enforcement point ──────────────────

/// Token count in Postgres decrements by ~1.0 for each allowed request.
///
/// AC-12-03
///
/// Journey:
///   Given: project "proj-atomic" with token bucket at 5.0 tokens (refill 5/s)
///   When:  3 requests are sent sequentially
///   Then:  rate_buckets.tokens is ~4, ~3, ~2 after each (tolerance for refill)
///
/// @driving_port @real-io @US-12 @AC-12-03
#[tokio::test]
async fn rate_bucket_tokens_decrease_atomically() {
    let ctx = DrlTestContext::new(5.0).await;
    ctx.insert_project_with_bucket("proj-atomic", "key-atomic", 5.0).await;
    let (addr, _server) = start_distributed_grpc_server(&ctx).await;
    let ctx = ctx.with_grpc_addr(addr);
    let mut client = FirestoreClient::new(ctx.grpc_channel());
    // Pay the one-time Argon2id cost first so refill drift between requests is negligible.
    warm_credential_cache_and_reset_bucket(&ctx, &mut client, "proj-atomic", "key-atomic", 5.0).await;

    for (i, expected) in [4.0_f64, 3.0, 2.0].into_iter().enumerate() {
        let result = client
            .get_document(get_document_request("proj-atomic", "key-atomic"))
            .await;
        assert_ne!(classify_grpc_status(&result), "resource_exhausted", "request {i} must be allowed");
        let (tokens, _) = ctx.query_rate_bucket("proj-atomic").await.expect("bucket row");
        assert!(
            (expected - 0.2..=expected + 0.5).contains(&tokens),
            "after request {i} expected ~{expected} tokens, got {tokens}"
        );
    }
}

// ─── AC-12-04: tokens refill after 1 second ──────────────────────────────────

/// Token bucket refills at the configured rate after 1 second of inactivity.
///
/// AC-12-04
///
/// Journey:
///   Given: project "proj-refill" with an exhausted bucket (0.0 tokens), 10 rps
///   When:  a request is sent immediately -> RESOURCE_EXHAUSTED
///   And:   the client waits 1.1 seconds, then sends another request
///   Then:  the second request is allowed and tokens ~= rps - 1.0
///
/// @driving_port @real-io @US-12 @AC-12-04
#[tokio::test]
async fn tokens_refill_after_one_second() {
    let ctx = DrlTestContext::new(10.0).await;
    ctx.insert_project_with_bucket("proj-refill", "key-refill", 10.0).await;
    let (addr, _server) = start_distributed_grpc_server(&ctx).await;
    let ctx = ctx.with_grpc_addr(addr);
    let mut client = FirestoreClient::new(ctx.grpc_channel());
    warm_credential_cache_and_reset_bucket(&ctx, &mut client, "proj-refill", "key-refill", 0.0).await;

    let first = client
        .get_document(get_document_request("proj-refill", "key-refill"))
        .await;
    assert_eq!(classify_grpc_status(&first), "resource_exhausted");

    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;

    let second = client
        .get_document(get_document_request("proj-refill", "key-refill"))
        .await;
    assert_ne!(classify_grpc_status(&second), "resource_exhausted", "bucket must have refilled");
    let (tokens, _) = ctx.query_rate_bucket("proj-refill").await.expect("bucket row");
    assert!(
        (8.0..=9.6).contains(&tokens),
        "expected ~9.0 tokens (10 rps refilled, capped at capacity, minus 1), got {tokens}"
    );
}
