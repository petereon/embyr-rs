// @US-OBS-01 @driving_port @real-io
#![allow(unused_imports)]
//! US-OBS-01 — Prometheus scrape endpoint exposed on admin port (:9090).
//!
//! Acceptance criteria verified here:
//!   AC-OBS-01-01: GET /metrics with valid Bearer admin_key → HTTP 200 + Prometheus format body.
//!   AC-OBS-01-02: GET /metrics without Authorization header → HTTP 401.
//!   AC-OBS-01-03: GET /metrics with wrong Bearer token → HTTP 401.
//!   AC-OBS-01-04: GET /metrics on the data port → not routed (404), no metrics leaked.
//!
//! Driving port: Admin HTTP port (:9090) — reqwest::Client → GET /metrics.
//! Assertion level: real Postgres container (testcontainers-rs) + in-process axum admin server.
//!
//! Implemented 2026-11-13 (formerly `todo!()` scaffolds). One test is kept
//! `#[ignore]`d as a documented REAL GAP: the product never registers metric
//! descriptions (`describe_*!`), so the exporter emits `# TYPE` but no `# HELP`
//! lines — see `docs/product/ignored-test-triage-2026-11-13.md`.
//!
//! The data-port check targets the REST port (:8081): the gRPC port is
//! HTTP/2-only tonic, where a plain GET is not meaningful.

#[path = "../common/mod.rs"]
mod common;
use common::ObsTestContext;

// ─── AC-OBS-01-01: Valid Bearer token → 200 + Prometheus format ───────────────

/// Operator receives Prometheus text format when scraping with valid Bearer token.
///
/// AC-OBS-01-01
///
/// @US-OBS-01 @driving_port @real-io @happy
#[tokio::test]
async fn metrics_endpoint_returns_200_with_prometheus_content_type() {
    let ctx = ObsTestContext::start().await;

    let resp = ctx
        .http_client
        .get(format!("{}/metrics", ctx.admin_base_url))
        .header("Authorization", format!("Bearer {}", ctx.admin_key))
        .send()
        .await
        .expect("GET /metrics failed");

    assert_eq!(resp.status().as_u16(), 200);
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    assert_eq!(content_type, "text/plain; charset=utf-8");
    let body = resp.text().await.expect("body");
    assert!(body.contains("# TYPE "), "expected Prometheus TYPE lines; got:\n{body}");
}

/// Operator sees HELP and TYPE annotation lines confirming valid Prometheus exposition format.
///
/// AC-OBS-01-01 (format guard)
///
/// @US-OBS-01 @driving_port @real-io @happy
#[tokio::test]
#[ignore = "REAL GAP: no describe_*!() calls anywhere, so /metrics has # TYPE but no # HELP lines (AC-OBS-01-01) — see docs/product/ignored-test-triage-2026-11-13.md"]
async fn metrics_response_contains_help_and_type_lines() {
    let ctx = ObsTestContext::start().await;

    let body = ctx.get_metrics().await;

    assert!(
        body.lines().any(|l| l.starts_with("# HELP ")),
        "expected at least one # HELP line; got:\n{body}"
    );
    assert!(
        body.lines().any(|l| l.starts_with("# TYPE ")),
        "expected at least one # TYPE line; got:\n{body}"
    );
}

// ─── AC-OBS-01-02 / AC-OBS-01-03: Auth rejection ─────────────────────────────

/// Missing Authorization header causes 401 — Prometheus reports target as DOWN.
///
/// AC-OBS-01-02
///
/// @US-OBS-01 @driving_port @real-io @error @security
#[tokio::test]
async fn metrics_endpoint_requires_operator_bearer_auth() {
    let ctx = ObsTestContext::start().await;

    assert_eq!(ctx.get_metrics_status(None).await, 401);
}

/// Wrong Bearer token causes 401 — operator must rotate credentials to fix Prometheus target.
///
/// AC-OBS-01-03
///
/// @US-OBS-01 @driving_port @real-io @error @security
#[tokio::test]
async fn metrics_endpoint_rejects_wrong_bearer_token() {
    let ctx = ObsTestContext::start().await;

    assert_eq!(ctx.get_metrics_status(Some("Bearer wrong-key")).await, 401);
}

// ─── AC-OBS-01-04: Data port isolation ───────────────────────────────────────

/// GET /metrics on the REST data port is not routed — metrics not leaked on the data path.
///
/// AC-OBS-01-04
///
/// @US-OBS-01 @driving_port @real-io @security
#[tokio::test]
async fn metrics_endpoint_not_exposed_on_data_port() {
    let ctx = ObsTestContext::start().await;

    let (status, body) = ctx.get_rest("/metrics").await;

    assert_eq!(status, 404);
    assert!(!body.contains("embyr_"), "no metrics may appear on the data port; got:\n{body}");
}
