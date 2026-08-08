// @driving_port @real-io @US-OBS-04
#![allow(unused_imports)]
//! US-OBS-04 — embyr_rate_limit_requests_total{project_id, outcome} counters per rate-limit decision.
//!
//! Acceptance criteria verified here:
//!   AC-OBS-04-01: embyr_rate_limit_requests_total{project_id, outcome} incremented on every
//!                 RateLimiter::check() return (Ok → "allowed", Err → "rejected").
//!   AC-OBS-04-02: project_id label matches the project_id argument passed to check().
//!   AC-OBS-04-04: Counters appear in /metrics only after the first request involving a rate-limit
//!                 check (zero-valued labels are not pre-emitted).
//!
//! Driving port: gRPC :8080 (tonic FirestoreClient → GetDocument) then Admin :9090
//!               (reqwest → GET /metrics to observe per-project counters).
//! Assertion level: real Postgres container (testcontainers-rs) + in-process servers.
//!
//! All tests are #[ignore] — OBS-01 and OBS-02 walking skeleton must be green first.
//!
//! Implementation notes for DELIVER (OBS-04):
//!   - Counter at Ok(_) path in RateLimiter::check(): outcome = "allowed".
//!   - Counter at Err(_) path in RateLimiter::check(): outcome = "rejected".
//!   - project_id.to_owned() required for metrics macro label (cannot use &str directly in 0.23).
//!   - `embyr_rate_limit_pg_timeout_total` counter (no labels) alongside existing tracing::warn!.
//!   - HIGH CARDINALITY (D-OBS-7): one time series per project per outcome; acceptable ≤10k projects.

#[path = "../common/mod.rs"]
mod common;
use common::ObsTestContext;

// ─── AC-OBS-04-01 / AC-OBS-04-02: Allowed request counter ───────────────────

/// Allowed request within rate limit increments the "allowed" outcome counter for the correct project.
///
/// AC-OBS-04-01, AC-OBS-04-02
///
/// Journey:
///   Given: embyr running with projects "obs-proj-fintech" (bucket full) and "obs-proj-beta"
///   When:  Firebase SDK sends GetDocument for "obs-proj-fintech" within rate limit
///   Then:  GET /metrics shows embyr_rate_limit_requests_total{project_id="obs-proj-fintech",outcome="allowed"}
///          incremented by 1
///   And:   embyr_rate_limit_requests_total{project_id="obs-proj-beta",outcome="allowed"} unchanged
///
/// @driving_port @real-io @US-OBS-04 @happy
#[tokio::test]
#[ignore]
async fn allowed_request_increments_rate_limit_allowed_total() {
    todo!()
}

// ─── AC-OBS-04-01: Rejected request counter ──────────────────────────────────

/// Rate-limited request increments the "rejected" outcome counter and RESOURCE_EXHAUSTED is returned.
///
/// AC-OBS-04-01
///
/// Journey (chained — Given reuses running server, project "obs-proj-fintech" with exhausted bucket):
///   Given: embyr running with project "obs-proj-fintech" and token bucket at 0.0
///   When:  Firebase SDK sends GetDocument for "obs-proj-fintech"
///   Then:  GET /metrics shows embyr_rate_limit_requests_total{project_id="obs-proj-fintech",outcome="rejected"}
///          incremented by 1
///   And:   gRPC response status is RESOURCE_EXHAUSTED
///
/// @driving_port @real-io @US-OBS-04 @error
#[tokio::test]
#[ignore]
async fn rejected_request_increments_rate_limit_rejected_total() {
    todo!()
}

// ─── AC-OBS-04-02: Label isolation per project ───────────────────────────────

/// Rate-limit counters are labeled by project_id — one project's counter does not affect another's.
///
/// AC-OBS-04-02
///
/// Journey (chained — Given reuses running server with both projects active):
///   Given: embyr running with "obs-proj-fintech" (exhausted bucket) and "obs-proj-beta" (full bucket)
///   When:  one rejected request for "obs-proj-fintech" and one allowed request for "obs-proj-beta"
///   Then:  embyr_rate_limit_requests_total{project_id="obs-proj-fintech",outcome="rejected"} ≥ 1
///   And:   embyr_rate_limit_requests_total{project_id="obs-proj-fintech",outcome="allowed"} = 0
///   And:   embyr_rate_limit_requests_total{project_id="obs-proj-beta",outcome="allowed"} ≥ 1
///   And:   embyr_rate_limit_requests_total{project_id="obs-proj-beta",outcome="rejected"} = 0
///
/// @driving_port @real-io @US-OBS-04 @happy
#[tokio::test]
#[ignore]
async fn rate_limit_metrics_labeled_by_project_id() {
    todo!()
}
