// @driving_port @real-io @US-RLV-01
#![allow(unused_imports)]
//! US-RLV-01 — a malformed (non-empty, non-conforming) project_id must not panic
//! the server; it still receives a normal rate-limit decision.
//!
//! Acceptance criteria verified here:
//!   AC-RLV-05: a single malformed, non-empty project_id does not cause a panic —
//!              the request still receives a normal rate-limit decision.
//!
//! Driving port: gRPC :8080 (tonic FirestoreClient -> GetDocument, unauthenticated —
//!               reaches RateLimiter::check() before authenticate() runs).
//! Assertion level: real Postgres container (testcontainers-rs) + in-process gRPC server.
//!
//! Not #[ignore] — expected GREEN today (ADR-069's chosen mechanism adds no new
//! parsing/unwrap to the hot path — ProjectId::new()'s regex is deliberately NOT
//! reused as the labeling gate), and must stay GREEN after DELIVER's fix lands.

#[path = "../common/mod.rs"]
mod common;
use common::{classify_grpc_status, get_metrics, start_distributed_grpc_server, DrlTestContext};
use embyr_proto::firestore::firestore_client::FirestoreClient;
use embyr_proto::firestore::GetDocumentRequest;
use std::time::Duration;

/// A garbled, non-empty project_id segment (leftover %20-encoded fragment, embedded
/// slash-like bytes) still yields a normal gRPC status — no panic, no connection drop.
///
/// AC-RLV-05
///
/// Journey:
///   Given a buggy SDK client produces a malformed, non-empty project_id segment
///   When that request reaches the rate limiter before authenticate() runs
///   Then the server returns a normal gRPC status (not a transport-level crash)
///
/// @driving_port @real-io @US-RLV-01 @AC-RLV-05
#[tokio::test]
async fn malformed_project_id_does_not_panic_the_server() {
    let ctx = DrlTestContext::new(10.0).await;
    let (_addr, server) = start_distributed_grpc_server(&ctx).await;

    let channel = tonic::transport::Channel::from_shared(format!("http://{}", server.grpc_addr))
        .expect("valid gRPC channel URI")
        .connect_lazy();
    let mut client = FirestoreClient::new(channel);

    let long_id = "a".repeat(500);
    let malformed_ids: [&str; 5] = [
        "%20leftover-fragment",
        "proj\u{0}embedded-nul",
        "проект-unicode-not-ascii",
        "-leading-dash",
        long_id.as_str(),
    ];
    for malformed in malformed_ids {
        let name = format!("projects/{malformed}/databases/(default)/documents/x/doc1");
        let mut req = tonic::Request::new(GetDocumentRequest {
            name,
            ..Default::default()
        });
        // Bearer key needed to pass extract_api_key() (handler.rs:160-173, runs before
        // rate_limiter.check()) so the call actually reaches the code path under test.
        req.metadata_mut().insert(
            "authorization",
            "bearer buggy-client-key"
                .parse()
                .expect("valid metadata value"),
        );
        // A hang (never returning) is as much a symptom of a broken rate-limit
        // decision as a panic would be — bound every call so the test fails loudly
        // instead of timing out the whole suite.
        let result = tokio::time::timeout(Duration::from_secs(5), client.get_document(req))
            .await
            .unwrap_or_else(|_| panic!("GetDocument hung for malformed project_id {malformed:?}"));
        // Any gRPC status (ok, not_found, invalid_argument, resource_exhausted, ...) is
        // acceptable — what matters is the call returns at all, proving the server did
        // not panic while computing the rate-limit decision or its metric label for
        // this input.
        let _ = classify_grpc_status(&result);
    }

    // The server process itself must still be alive after all malformed inputs — a
    // panic inside the gRPC request-handling task would not necessarily surface as a
    // gRPC error on that same call, but a crashed process makes the admin HTTP port
    // (a completely independent listener) unreachable. A successful scrape here
    // proves the process survived, not just that one connection didn't drop.
    let admin_client = reqwest::Client::new();
    let metrics_body = get_metrics(&admin_client, server.admin_addr).await;
    assert!(
        metrics_body.contains("embyr_rate_limit_requests_total"),
        "admin :9090 /metrics must still be reachable and serving real metrics after \
         malformed project_id inputs (server process must not have panicked/crashed)"
    );
}
