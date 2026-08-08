// @US-OBS-01 @driving_port @real-io
#![allow(unused_imports)]
//! US-OBS-01 — Prometheus scrape endpoint exposed on admin port (:9090).
//!
//! Acceptance criteria verified here:
//!   AC-OBS-01-01: GET /metrics with valid Bearer admin_key → HTTP 200 + Prometheus format body.
//!   AC-OBS-01-02: GET /metrics without Authorization header → HTTP 401.
//!   AC-OBS-01-03: GET /metrics with wrong Bearer token → HTTP 401.
//!   AC-OBS-01-04: GET /metrics on data port (:8080) → HTTP 404 (not routed).
//!
//! Driving port: Admin HTTP port (:9090) — reqwest::Client → GET /metrics.
//! Assertion level: real Postgres container (testcontainers-rs) + in-process axum admin server.
//!
//! All tests are #[ignore] — OBS-01 DELIVER unskips them in order.
//!
//! Implementation notes for DELIVER (OBS-01):
//!   - observability::get_or_install_prometheus_handle() must be called before listeners open.
//!   - GET /metrics route added to operator_router in admin/router.rs.
//!   - operator_auth_middleware must be applied to /metrics (Bearer admin_key check).
//!   - Content-Type response header: "text/plain; charset=utf-8".

#[path = "../common/mod.rs"]
mod common;
use common::ObsTestContext;

// ─── AC-OBS-01-01: Valid Bearer token → 200 + Prometheus format ───────────────

/// Operator receives Prometheus text format when scraping with valid Bearer token.
///
/// AC-OBS-01-01
///
/// Journey:
///   Given: embyr running with PrometheusHandle installed and admin_key "test-key-obs01"
///   When:  GET /metrics with header "Authorization: Bearer test-key-obs01"
///   Then:  HTTP 200 + Content-Type "text/plain; charset=utf-8" + body has "# HELP" lines
///
/// @US-OBS-01 @driving_port @real-io @happy
#[tokio::test]
#[ignore]
async fn metrics_endpoint_returns_200_with_prometheus_content_type() {
    todo!()
}

/// Operator sees HELP and TYPE annotation lines confirming valid Prometheus exposition format.
///
/// AC-OBS-01-01 (format guard)
///
/// Journey (chained — Given reuses running server from previous test):
///   Given: embyr running with valid admin_key
///   When:  GET /metrics with valid Bearer token
///   Then:  body contains at least one line starting with "# HELP"
///   And:   body contains at least one line starting with "# TYPE"
///
/// @US-OBS-01 @driving_port @real-io @happy
#[tokio::test]
#[ignore]
async fn metrics_response_contains_help_and_type_lines() {
    todo!()
}

// ─── AC-OBS-01-02 / AC-OBS-01-03: Auth rejection ─────────────────────────────

/// Missing Authorization header causes 401 — Prometheus reports target as DOWN.
///
/// AC-OBS-01-02
///
/// Journey:
///   Given: embyr running with admin_key "test-key-obs01"
///   When:  GET /metrics with no Authorization header
///   Then:  HTTP 401 (operator_auth_middleware rejects unauthenticated request)
///
/// @US-OBS-01 @driving_port @real-io @error @security
#[tokio::test]
#[ignore]
async fn metrics_endpoint_requires_operator_bearer_auth() {
    todo!()
}

/// Wrong Bearer token causes 401 — operator must rotate credentials to fix Prometheus target.
///
/// AC-OBS-01-03
///
/// Journey (chained — Given reuses running server):
///   Given: embyr running with admin_key "test-key-obs01"
///   When:  GET /metrics with header "Authorization: Bearer wrong-key"
///   Then:  HTTP 401
///
/// @US-OBS-01 @driving_port @real-io @error @security
#[tokio::test]
#[ignore]
async fn metrics_endpoint_rejects_wrong_bearer_token() {
    todo!()
}

// ─── AC-OBS-01-04: Data port isolation ───────────────────────────────────────

/// GET /metrics on the gRPC data port returns 404 — metrics not leaked on data path.
///
/// AC-OBS-01-04
///
/// Journey:
///   Given: embyr running with gRPC port on ephemeral addr and admin port on separate addr
///   When:  GET /metrics sent to the gRPC port (not admin port)
///   Then:  HTTP 404 (route not registered on data ports)
///   And:   no Prometheus metrics data appears in the response body
///
/// @US-OBS-01 @driving_port @real-io @security
#[tokio::test]
#[ignore]
async fn metrics_endpoint_not_exposed_on_grpc_port() {
    todo!()
}
