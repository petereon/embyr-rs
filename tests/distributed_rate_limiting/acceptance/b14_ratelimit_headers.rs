// @driving_port @real-io @US-14
#![allow(unused_imports)]
//! US-14 — x-ratelimit-* trailing metadata headers on all gRPC responses.
//!
//! Acceptance criteria verified here:
//!   AC-14-01: All 9 gRPC handlers return x-ratelimit-limit and x-ratelimit-remaining on every response.
//!   AC-14-02: x-ratelimit-remaining decrements by 1.0 per allowed request.
//!   AC-14-03: RESOURCE_EXHAUSTED responses include retry-after-ms; allowed responses do not.
//!
//! Driving port: gRPC :8080 (tonic FirestoreClient → metadata inspection).
//! Assertion level: real Postgres container (testcontainers-rs) + in-process gRPC server.
//!
//! Header location (tonic 0.12):
//!   - Unary handlers: trailing metadata (Response<T>::metadata_mut())
//!   - Streaming handlers: initial metadata (Response<BoxStream>::metadata_mut())
//!   - Error responses: Status::metadata_mut()
//!
//! The `attach_rate_limit_headers(md, info)` free function in rate_limit.rs sets these headers.
//! `attach_retry_after(md, info, refill_rate)` is called only on rejection (Err(info)).
//!
//! All tests are #[ignore] — DELIVER unskips them as part of DRL-04.
//!
//! Implementation note for DELIVER (DRL-04):
//!   - Each of the 9 handler call sites in handler.rs must call attach_rate_limit_headers.
//!   - Rejection path must additionally call attach_retry_after.
//!   - Header key names: "x-ratelimit-limit", "x-ratelimit-remaining", "x-ratelimit-reset".
//!   - Rejection only: "retry-after-ms".

#[path = "../common/mod.rs"]
mod common;
use common::{get_document_request, start_distributed_grpc_server, DrlTestContext};
use embyr_proto::firestore::firestore_client::FirestoreClient;

// ─── AC-14-01: all handlers return rate-limit headers ────────────────────────

/// All 9 gRPC handlers return x-ratelimit-limit and x-ratelimit-remaining.
///
/// AC-14-01
///
/// Journey:
///   Given: project "proj-headers" with token bucket at capacity (enough for all requests)
///   When:  client calls GetDocument (representative of all 9 handlers)
///   Then:  response trailing metadata contains "x-ratelimit-limit"
///   And:   response trailing metadata contains "x-ratelimit-remaining"
///   And:   response trailing metadata contains "x-ratelimit-reset"
///   And:   "retry-after-ms" header is NOT present on allowed responses
///
/// NOTE: This test covers GetDocument as a representative handler.
/// The "all 9 handlers" requirement is satisfied by code review at each of the 9 call sites
/// in handler.rs — not by calling all 9 endpoints in this test (that would be fixture theater).
///
/// @driving_port @real-io @US-14 @AC-14-01
#[tokio::test]
#[ignore]
async fn all_grpc_handlers_return_ratelimit_headers() {
    // SCAFFOLD: true — DRL-04 DELIVER implements this.
    // DELIVER steps:
    //   1. ctx = DrlTestContext::new(100.0).await; insert "proj-headers" with 100.0 tokens
    //   2. Start distributed gRPC server; build FirestoreClient
    //   3. let response = client.get_document(get_document_request("proj-headers", "key-h")).await;
    //   4. Inspect response metadata (trailing) for "x-ratelimit-limit", "x-ratelimit-remaining", "x-ratelimit-reset"
    //   5. Assert all three present and non-empty
    //   6. Assert "retry-after-ms" is absent
    let _ctx = DrlTestContext::new(100.0).await;
    todo!(
        "DRL-04 DELIVER: assert x-ratelimit-limit, x-ratelimit-remaining, x-ratelimit-reset present on allowed response; retry-after-ms absent"
    )
}

// ─── AC-14-02: remaining decrements per request ──────────────────────────────

/// x-ratelimit-remaining decrements by 1.0 for each allowed request.
///
/// AC-14-02
///
/// Journey (chained — Given reuses "proj-headers" with tokens at capacity):
///   Given: project "proj-decrement" with 5.0 tokens
///   When:  client sends 3 sequential GetDocument requests
///   Then:  x-ratelimit-remaining on request 1 ≈ 4.0
///   And:   x-ratelimit-remaining on request 2 ≈ 3.0
///   And:   x-ratelimit-remaining on request 3 ≈ 2.0
///   And:   each successive response has a strictly lower remaining value
///
/// NOTE: Approximate comparison (within ±0.5) to tolerate time-based token refill during test.
///
/// @driving_port @real-io @US-14 @AC-14-02
#[tokio::test]
#[ignore]
async fn ratelimit_remaining_decrements_per_request() {
    // SCAFFOLD: true — DRL-04 DELIVER implements this.
    // DELIVER steps:
    //   1. ctx = DrlTestContext::new(10.0).await; insert "proj-decrement" with 5.0 tokens
    //   2. Start distributed gRPC server
    //   3. Send request 1; parse x-ratelimit-remaining from trailing metadata → remaining_1
    //   4. Send request 2; parse → remaining_2; assert remaining_2 < remaining_1
    //   5. Send request 3; parse → remaining_3; assert remaining_3 < remaining_2
    //   6. assert remaining_1 ≈ 4.0 (within ±0.5)
    let _ctx = DrlTestContext::new(10.0).await;
    todo!(
        "DRL-04 DELIVER: assert x-ratelimit-remaining decrements strictly per request"
    )
}

// ─── AC-14-03: rejected response includes retry-after-ms ─────────────────────

/// RESOURCE_EXHAUSTED response includes retry-after-ms header; allowed response does not.
///
/// AC-14-03
///
/// Journey (chained — builds on exhausted bucket scenario from b12):
///   Given: project "proj-retry" with 0.0 tokens (bucket exhausted)
///   When:  client sends GetDocument request
///   Then:  gRPC status is RESOURCE_EXHAUSTED
///   And:   Status trailing metadata contains "retry-after-ms" with a positive integer value
///   And:   "retry-after-ms" value indicates milliseconds until ≥1 token is available
///          (expected: approximately 1000 / rate_limit_rps milliseconds)
///
/// @driving_port @real-io @US-14 @AC-14-03
#[tokio::test]
#[ignore]
async fn rate_limited_response_includes_retry_after_ms() {
    // SCAFFOLD: true — DRL-04 DELIVER implements this.
    // DELIVER steps:
    //   1. ctx = DrlTestContext::new(10.0).await; insert "proj-retry" with 0.0 tokens
    //   2. Start distributed gRPC server
    //   3. let result = client.get_document(get_document_request("proj-retry", "key-rt")).await;
    //   4. assert result.is_err() — status is Err(Status)
    //   5. let status = result.unwrap_err();
    //   6. assert_eq!(status.code(), tonic::Code::ResourceExhausted);
    //   7. let metadata = status.metadata();
    //   8. let retry_ms_str = metadata.get("retry-after-ms").expect("retry-after-ms header absent");
    //   9. let retry_ms: u64 = retry_ms_str.to_str().unwrap().parse().unwrap();
    //   10. assert!(retry_ms > 0, "retry-after-ms must be positive");
    //   11. assert!(retry_ms <= 2000, "retry-after-ms should not exceed 2 seconds for 10 RPS bucket");
    let _ctx = DrlTestContext::new(10.0).await;
    todo!(
        "DRL-04 DELIVER: assert RESOURCE_EXHAUSTED response includes retry-after-ms header with positive value"
    )
}
