// @driving_port @real-io @adapter-integration @X4
#![allow(unused_imports)]
//! Security finding X4 — a project's rate-limit tokens are spent only by
//! requests that authenticated successfully.
//!
//! The bucket is keyed by `project_id` alone. While it was charged BEFORE
//! authentication, anyone who merely knew a project id could drain it with
//! bad-key requests and lock out the project's real users.
//!
//! Acceptance criteria verified here:
//!   AC-X4-01: bad-key requests leave a known project's `rate_buckets` row untouched.
//!   AC-X4-02: good-key requests still decrement it.
//!   AC-X4-03: a bad-key flood is itself bounded (per-project failed-authentication
//!             budget) and does not refuse a key the server has already verified.
//!
//! Driving port: gRPC :8080 (tonic FirestoreClient -> GetDocument).
//! Assertion level: real Postgres container (testcontainers-rs) + in-process gRPC server.
//!
//! Why the RAW row is compared: `rate_buckets.tokens` is only rewritten by
//! `RateLimiter::check()` (refill is computed lazily inside that UPDATE), so
//! "no check ran" is exactly "tokens AND last_refill are byte-identical" —
//! independent of how long the bad-key Argon2id verifications take.

#[path = "../common/mod.rs"]
mod common;
use common::{
    classify_grpc_status, get_document_request, start_distributed_grpc_server,
    warm_credential_cache_and_reset_bucket, DrlTestContext,
};
use embyr_proto::firestore::firestore_client::FirestoreClient;

/// Journey:
///   Given: Fernbank Analytics, a provisioned project whose bucket holds 5 tokens
///   When:  a caller who knows only the project id sends 3 requests with wrong keys
///   Then:  each is rejected UNAUTHENTICATED and the bucket row is untouched
///   When:  Fernbank Analytics then sends one request with its real key
///   Then:  the request is allowed and the bucket holds ~4 tokens
///
/// @driving_port @real-io @X4 @AC-X4-01 @AC-X4-02
#[tokio::test]
async fn only_authenticated_requests_spend_a_projects_tokens() {
    let ctx = DrlTestContext::new(5.0).await;
    ctx.insert_project_with_bucket("fernbank-analytics", "fernbank-key", 5.0).await;
    let (addr, _server) = start_distributed_grpc_server(&ctx).await;
    let ctx = ctx.with_grpc_addr(addr);
    let mut client = FirestoreClient::new(ctx.grpc_channel());
    // Pays the real key's one-time Argon2id cost and re-pins the row at 5.0
    // tokens / last_refill = now() right before the assertions below (the row
    // seeded above has been refilling since before the server started).
    warm_credential_cache_and_reset_bucket(&ctx, &mut client, "fernbank-analytics", "fernbank-key", 5.0)
        .await;
    let pinned = ctx.query_rate_bucket("fernbank-analytics").await.expect("bucket row");

    for attempt in 0..3 {
        let status = client
            .get_document(get_document_request("fernbank-analytics", &format!("guessed-key-{attempt}")))
            .await
            .expect_err("a wrong key must be rejected");
        assert_eq!(status.code(), tonic::Code::Unauthenticated, "attempt {attempt}: {status:?}");
    }
    assert_eq!(
        ctx.query_rate_bucket("fernbank-analytics").await.expect("bucket row"),
        pinned,
        "AC-X4-01: bad-key requests must never touch the project's rate_buckets row"
    );

    let result = client
        .get_document(get_document_request("fernbank-analytics", "fernbank-key"))
        .await;
    assert_ne!(classify_grpc_status(&result), "resource_exhausted", "real key must be allowed");
    let (tokens, _) = ctx.query_rate_bucket("fernbank-analytics").await.expect("bucket row");
    // The row sat at 5.0 (= capacity) while the bad keys were verified, so the
    // good request's own UPDATE lands on LEAST(capacity, ..) - 1.0 = 4.0.
    assert!(
        (3.8..=4.2).contains(&tokens),
        "AC-X4-02: an authenticated request must still spend one token — expected ~4.0, got {tokens}"
    );
}

/// Journey:
///   Given: Solstice Retail, a provisioned project whose real key the server has verified
///   When:  30 requests with 30 different wrong keys arrive at once
///   Then:  at least 10 are refused RESOURCE_EXHAUSTED before any key verification
///          (failed-authentication budget: burst 10, refill 1/s)
///   And:   the project's bucket row is untouched
///   And:   Solstice Retail's own next request is still served
///
/// @driving_port @real-io @X4 @AC-X4-03
#[tokio::test]
async fn bad_key_flood_is_bounded_without_locking_out_the_verified_key() {
    let ctx = DrlTestContext::new(5.0).await;
    ctx.insert_project_with_bucket("solstice-retail", "solstice-key", 5.0).await;
    let (addr, _server) = start_distributed_grpc_server(&ctx).await;
    let ctx = ctx.with_grpc_addr(addr);
    let mut client = FirestoreClient::new(ctx.grpc_channel());
    warm_credential_cache_and_reset_bucket(&ctx, &mut client, "solstice-retail", "solstice-key", 5.0)
        .await;
    let pinned = ctx.query_rate_bucket("solstice-retail").await.expect("bucket row");

    let mut flood = tokio::task::JoinSet::new();
    for attempt in 0..30 {
        let mut client = client.clone();
        flood.spawn(async move {
            client
                .get_document(get_document_request("solstice-retail", &format!("guessed-key-{attempt}")))
                .await
                .expect_err("a wrong key must be rejected")
        });
    }
    let mut refused_before_verification = 0;
    while let Some(status) = flood.join_next().await {
        let status = status.expect("flood task");
        match status.code() {
            tonic::Code::ResourceExhausted => {
                assert_eq!(status.message(), "too many failed authentication attempts");
                refused_before_verification += 1;
            }
            code => assert_eq!(code, tonic::Code::Unauthenticated, "{status:?}"),
        }
    }
    assert!(
        refused_before_verification >= 10,
        "AC-X4-03: expected at least 10 of 30 simultaneous bad-key requests to be refused \
         by the failed-authentication budget, got {refused_before_verification}"
    );
    assert_eq!(
        ctx.query_rate_bucket("solstice-retail").await.expect("bucket row"),
        pinned,
        "AC-X4-03: the flood must not touch the project's rate_buckets row"
    );

    let result = client
        .get_document(get_document_request("solstice-retail", "solstice-key"))
        .await;
    assert_ne!(
        classify_grpc_status(&result),
        "resource_exhausted",
        "AC-X4-03: an already-verified key must keep working during a bad-key flood"
    );
}
