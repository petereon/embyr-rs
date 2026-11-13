// @driving_port @real-io @US-OBS-04
#![allow(unused_imports)]
//! US-OBS-04 — embyr_rate_limit_requests_total{project_id, outcome} counter.
//!
//! Acceptance criteria verified here:
//!   AC-OBS-04-01: every rate-limit decision increments the counter with outcome allowed|rejected.
//!   AC-OBS-04-02: the `project_id` label isolates projects from each other.
//!
//! Label cardinality note (ADR-069): the `project_id` label is the real id only
//! once the limiter already knows the project (second call onward); the first
//! call for a never-seen id is labelled `unconfirmed`. Each test therefore
//! sends several calls per project.
//!
//! Driving port: gRPC :8080 (tonic GetDocument) then Admin :9090 (GET /metrics).
//! The in-process limiter is sized to 2 tokens via `EMBYR_RATE_LIMIT_RPS` so a
//! burst of calls is rejected. The recorder is process-global and tests share
//! it, hence unique project ids per test and `>=` / `== 0` assertions only.
//!
//! Implemented 2026-11-13 (formerly `todo!()` scaffolds, `#[ignore]`d).

#[path = "../common/mod.rs"]
mod common;
use common::{metric_value, ObsTestContext};

const RPS: u32 = 2;
const COUNTER: &str = "embyr_rate_limit_requests_total";

/// Unauthenticated calls are enough: the limiter runs before authentication.
async fn burst(ctx: &ObsTestContext, project: &str, n: usize) {
    for _ in 0..n {
        ctx.make_grpc_call(project).await;
    }
}

// ─── AC-OBS-04-01: Allowed request counter ───────────────────────────────────

/// Calls within capacity increment the `allowed` outcome for the project.
///
/// @driving_port @real-io @US-OBS-04 @happy
#[tokio::test]
async fn allowed_request_increments_rate_limit_allowed_total() {
    let ctx = ObsTestContext::start_with_rate_limit(RPS).await;

    burst(&ctx, "obs-allowed-proj", 2).await;
    let body = ctx.get_metrics().await;

    assert!(
        metric_value(&body, COUNTER, &[("project_id", "obs-allowed-proj"), ("outcome", "allowed")]) >= 1.0,
        "expected an allowed sample for obs-allowed-proj; body:\n{body}"
    );
}

// ─── AC-OBS-04-01: Rejected request counter ──────────────────────────────────

/// Calls beyond capacity increment the `rejected` outcome for the project.
///
/// @driving_port @real-io @US-OBS-04 @error
#[tokio::test]
async fn rejected_request_increments_rate_limit_rejected_total() {
    let ctx = ObsTestContext::start_with_rate_limit(RPS).await;

    burst(&ctx, "obs-rejected-proj", 6).await;
    let body = ctx.get_metrics().await;

    assert!(
        metric_value(&body, COUNTER, &[("project_id", "obs-rejected-proj"), ("outcome", "rejected")]) >= 1.0,
        "expected a rejected sample for obs-rejected-proj; body:\n{body}"
    );
}

// ─── AC-OBS-04-02: Label isolation per project ───────────────────────────────

/// One project exhausting its bucket does not produce `rejected` samples for another.
///
/// @driving_port @real-io @US-OBS-04 @happy
#[tokio::test]
async fn rate_limit_metrics_labeled_by_project_id() {
    let ctx = ObsTestContext::start_with_rate_limit(RPS).await;

    burst(&ctx, "obs-label-noisy", 6).await;
    burst(&ctx, "obs-label-quiet", 2).await;
    let body = ctx.get_metrics().await;

    assert!(
        metric_value(&body, COUNTER, &[("project_id", "obs-label-noisy"), ("outcome", "rejected")]) >= 1.0,
        "noisy project must have rejected samples; body:\n{body}"
    );
    assert_eq!(
        metric_value(&body, COUNTER, &[("project_id", "obs-label-quiet"), ("outcome", "rejected")]),
        0.0,
        "quiet project must have no rejected samples; body:\n{body}"
    );
}
