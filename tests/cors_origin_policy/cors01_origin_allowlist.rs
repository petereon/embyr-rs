// @real-io @US-01 @finding-21
//! cors-origin-policy — env-var-driven origin allowlist on the :8081
//! REST/gRPC-Web/BrowserChannel surface (production-readiness-audit-2026-09-08.md
//! finding #21, Medium/Security). Closes the landmine risk of an
//! unrestricted or accidentally-permissive CORS policy while preserving
//! today's fails-closed-by-default behavior.
//!
//! Full DISCUSS+DESIGN context: docs/feature/cors-origin-policy/feature-delta.md
//!
//! Acceptance criteria verified here:
//!   AC-CORS-01: default (no `EMBYR_CORS_ALLOWED_ORIGINS`) — no
//!     `Access-Control-Allow-Origin` header on any response, including a
//!     CORS preflight OPTIONS request. Regression guard: byte-identical to
//!     today's no-CorsLayer behavior.
//!   AC-CORS-02: with an allowlist configured, a request from an allowed
//!     origin gets the correct ACAO header; a request from a non-allowed
//!     origin does not. `allow_credentials(false)` is verified inline
//!     (no ACAC header ever, even for an allowed origin).
//!   AC-CORS-03: preflight OPTIONS negotiation succeeds for an allowed
//!     origin (status success, ACAO matches, Access-Control-Allow-Methods
//!     present).
//!   AC-CORS-04: :9090 (admin router) is untouched — no CORS layer applied
//!     there, even when the SAME origin is allowlisted on :8081.
//!
//! Driving port: reqwest::Client against a real in-process embyr-server
//!   (`embyr_server::start_test_server_with_cors_origins`), real Postgres
//!   testcontainer. Driven port boundary: HTTP response headers.

use std::sync::Arc;

use reqwest::header::{
    ACCESS_CONTROL_ALLOW_CREDENTIALS, ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN,
};
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt},
};

use embyr_server::adapters::system_db::SystemDb;

/// Spin up a real Postgres container + migrations + in-process embyr-server
/// with the given `EMBYR_CORS_ALLOWED_ORIGINS`-equivalent allowlist.
async fn start_ctx(cors_allowed_origins: Vec<String>) -> (ContainerAsync<Postgres>, embyr_server::TestServer) {
    let container = Postgres::default()
        .with_tag("15-alpine")
        .start()
        .await
        .expect("failed to start Postgres testcontainer");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("failed to get host port");
    let db_url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");

    let system_db = Arc::new(SystemDb::new(&db_url).await.expect("SystemDb::new"));
    system_db.migrate().await.expect("migrate");

    let server = embyr_server::start_test_server_with_cors_origins(system_db, cors_allowed_origins).await;
    (container, server)
}

// ─── AC-CORS-01: default empty allowlist — no CORS headers ever ─────────────

/// With no allowed origins configured (default), neither a simple
/// cross-origin GET nor a CORS preflight OPTIONS ever receives an
/// `Access-Control-Allow-Origin` header — byte-identical to today's
/// no-CorsLayer behavior.
///
/// @real-io @US-01 @AC-CORS-01
#[tokio::test]
async fn default_empty_allowlist_denies_all_origins() {
    let (_pg, server) = start_ctx(Vec::new()).await;
    let base = format!("http://127.0.0.1:{}", server.rest_addr.port());
    let client = reqwest::Client::new();

    let simple = client
        .get(format!("{base}/livez"))
        .header("Origin", "https://evil.example.com")
        .send()
        .await
        .expect("GET /livez");
    assert!(
        simple.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN).is_none(),
        "simple cross-origin GET must not receive ACAO header when allowlist is empty"
    );

    let preflight = client
        .request(reqwest::Method::OPTIONS, format!("{base}/livez"))
        .header("Origin", "https://evil.example.com")
        .header("Access-Control-Request-Method", "GET")
        .send()
        .await
        .expect("OPTIONS preflight /livez");
    assert!(
        preflight.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN).is_none(),
        "preflight OPTIONS must not receive ACAO header when allowlist is empty"
    );
}

// ─── AC-CORS-02: allowed origin matches, non-allowed does not ───────────────

/// A request from an allowlisted origin gets the correct ACAO header; a
/// request from a non-allowed origin does not — and `allow_credentials(false)`
/// means no `Access-Control-Allow-Credentials` header is ever emitted, even
/// for the allowed origin.
///
/// @real-io @US-01 @AC-CORS-02
#[tokio::test]
async fn allowlisted_origin_matches_non_listed_origin_does_not() {
    let allowed_origin = "https://app.customer1.com";
    let (_pg, server) = start_ctx(vec![allowed_origin.to_string()]).await;
    let base = format!("http://127.0.0.1:{}", server.rest_addr.port());
    let client = reqwest::Client::new();

    for (origin, should_match) in [
        (allowed_origin, true),
        ("https://app.not-listed.com", false),
    ] {
        let resp = client
            .get(format!("{base}/livez"))
            .header("Origin", origin)
            .send()
            .await
            .expect("GET /livez");

        let acao = resp.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN);
        if should_match {
            assert_eq!(
                acao.map(|v| v.to_str().unwrap()),
                Some(allowed_origin),
                "allowed origin {origin} must receive matching ACAO header"
            );
            assert!(
                resp.headers().get(ACCESS_CONTROL_ALLOW_CREDENTIALS).is_none(),
                "ACAC header must never appear (allow_credentials(false))"
            );
        } else {
            assert!(
                acao.is_none(),
                "non-allowed origin {origin} must not receive an ACAO header"
            );
        }
    }
}

// ─── AC-CORS-03: preflight OPTIONS negotiation for allowed origin ───────────

/// A CORS preflight OPTIONS request from an allowed origin negotiates
/// successfully: success status, matching ACAO, and
/// Access-Control-Allow-Methods present.
///
/// @real-io @US-01 @AC-CORS-03
#[tokio::test]
async fn preflight_options_succeeds_for_allowed_origin() {
    let allowed_origin = "https://app.customer1.com";
    let (_pg, server) = start_ctx(vec![allowed_origin.to_string()]).await;
    let base = format!("http://127.0.0.1:{}", server.rest_addr.port());
    let client = reqwest::Client::new();

    let resp = client
        .request(reqwest::Method::OPTIONS, format!("{base}/livez"))
        .header("Origin", allowed_origin)
        .header("Access-Control-Request-Method", "POST")
        .send()
        .await
        .expect("OPTIONS preflight /livez");

    assert!(
        resp.status().is_success(),
        "preflight for an allowed origin must succeed; got {}",
        resp.status()
    );
    assert_eq!(
        resp.headers()
            .get(ACCESS_CONTROL_ALLOW_ORIGIN)
            .map(|v| v.to_str().unwrap()),
        Some(allowed_origin)
    );
    assert!(
        resp.headers().get(ACCESS_CONTROL_ALLOW_METHODS).is_some(),
        "preflight response must advertise allowed methods"
    );
    assert!(
        resp.headers().get(ACCESS_CONTROL_ALLOW_CREDENTIALS).is_none(),
        "ACAC header must never appear (allow_credentials(false))"
    );
}

// ─── AC-CORS-04: :9090 admin router untouched ────────────────────────────────

/// The admin router (:9090) has no CORS layer — a request carrying an
/// `Origin` header that WOULD match the :8081 allowlist never receives an
/// ACAO header on the admin port.
///
/// @real-io @US-01 @AC-CORS-04
#[tokio::test]
async fn admin_port_has_no_cors_layer() {
    let allowed_origin = "https://app.customer1.com";
    let (_pg, server) = start_ctx(vec![allowed_origin.to_string()]).await;
    let base = format!("http://127.0.0.1:{}", server.admin_addr.port());
    let client = reqwest::Client::new();

    let resp = client
        .get(format!("{base}/livez"))
        .header("Origin", allowed_origin)
        .send()
        .await
        .expect("GET admin /livez");

    assert!(
        resp.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN).is_none(),
        ":9090 admin router must never emit an ACAO header — no CorsLayer there"
    );
}
