// SCAFFOLD: true
//! Common test infrastructure — observability acceptance tests.
//!
//! Infrastructure policy (docs/architecture/atdd-infrastructure-policy.md):
//!   Driving port:    reqwest HTTP client against embyr admin server on ephemeral port
//!                    (`GET /metrics` with operator Bearer auth).
//!   Driven internal: testcontainers-rs Postgres 15-alpine; sqlx::migrate!; system DB.
//!   Driven external: none in scope for this feature.
//!
//! Implementation state (DELIVER wave, 2026-08-08):
//!   - ObsTestContext::start():    LIVE — Postgres container + migrations + test server
//!   - get_metrics():              LIVE — GET /metrics with operator Bearer auth
//!   - get_metrics_status():       LIVE — GET /metrics with caller-supplied auth header
//!   - provision_project():        LIVE — POST /admin/v1/projects with direct_pg DSN
//!   - make_grpc_call():           LIVE — tonic GetDocument (unauthenticated; counter fires regardless)
//!   - parse_metric_value():       LIVE — Prometheus text-format line parser

#![allow(dead_code, unused_imports, unused_variables)]

// ─── State-delta re-export ────────────────────────────────────────────────────
#[path = "../../common/state_delta.rs"]
pub mod state_delta;
pub use state_delta::{appended_with, assert_state_delta, containing, set_to, unchanged};

// ─── Universe — port-exposed observable names ─────────────────────────────────
/// Universe observable names for observability acceptance assertions.
/// All keys are port-exposed (HTTP response fields, Prometheus text format tokens).
/// Never internal struct fields or crate-private types.
pub mod universe {
    /// HTTP response status code from GET /metrics (as string "200", "401", etc.).
    pub const HTTP_STATUS: &str = "http.response.status_code";
    /// HTTP Content-Type header value from GET /metrics response body.
    pub const HTTP_CONTENT_TYPE: &str = "http.response.content_type";
    /// Whether GET /metrics body contains at least one "# HELP" line.
    pub const METRICS_HAS_HELP_LINES: &str = "metrics.response.has_help_lines";
    /// Whether GET /metrics body contains at least one "# TYPE" line.
    pub const METRICS_HAS_TYPE_LINES: &str = "metrics.response.has_type_lines";
    /// embyr_grpc_requests_total counter presence in /metrics text (non-empty = present).
    pub const GRPC_REQUESTS_COUNTER_PRESENT: &str = "metrics.embyr_grpc_requests_total.present";
    /// embyr_grpc_request_duration_seconds histogram presence in /metrics text.
    pub const GRPC_DURATION_HISTOGRAM_PRESENT: &str =
        "metrics.embyr_grpc_request_duration_seconds.present";
    /// embyr_rate_limit_requests_total{outcome="allowed"} counter value as string.
    pub const RATE_LIMIT_ALLOWED_COUNTER: &str =
        "metrics.embyr_rate_limit_requests_total.outcome.allowed";
    /// embyr_rate_limit_requests_total{outcome="rejected"} counter value as string.
    pub const RATE_LIMIT_REJECTED_COUNTER: &str =
        "metrics.embyr_rate_limit_requests_total.outcome.rejected";
    /// embyr_rate_limit_pg_timeout_total counter value as string.
    pub const PG_TIMEOUT_COUNTER: &str = "metrics.embyr_rate_limit_pg_timeout_total";
    /// embyr_pg_pool_size{pool="system"} gauge value as string.
    pub const PG_POOL_SIZE: &str = "metrics.embyr_pg_pool_size.system";
    /// embyr_pg_pool_idle{pool="system"} gauge value as string.
    pub const PG_POOL_IDLE: &str = "metrics.embyr_pg_pool_idle.system";
}

// ─── Imports ──────────────────────────────────────────────────────────────────
use std::sync::Arc;

use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt},
};

use embyr_server::adapters::system_db::SystemDb;
use embyr_proto::firestore::firestore_client::FirestoreClient;
use embyr_proto::firestore::GetDocumentRequest;

// ─── ObsTestContext ───────────────────────────────────────────────────────────

/// Shared test context for observability acceptance tests.
///
/// Holds the running embyr server (admin port bound on a random ephemeral port)
/// and a pre-configured HTTP client for scraping GET /metrics.
pub struct ObsTestContext {
    /// Base URL of the admin server, e.g. "http://127.0.0.1:12345".
    pub admin_base_url: String,
    /// Operator Bearer token used to authenticate GET /metrics requests.
    pub admin_key: String,
    /// Pre-configured HTTP client (no default TLS; connects to localhost).
    pub http_client: reqwest::Client,
    /// gRPC server address — used by make_grpc_call().
    grpc_addr: std::net::SocketAddr,
    /// DSN for the test Postgres container — passed in provision_project() requests.
    db_url: String,
    /// Postgres container — kept alive for the duration of the test.
    _container: ContainerAsync<Postgres>,
    /// In-process test server — dropping it sends the shutdown signal.
    _server: embyr_server::TestServer,
}

impl ObsTestContext {
    /// Spin up Postgres + migrations + embyr-server, return context.
    ///
    /// Steps:
    ///   1. Start testcontainers Postgres 15-alpine.
    ///   2. Run sqlx migrations from workspace `migrations/` directory.
    ///   3. Call `start_test_server` (which installs PrometheusHandle via OnceLock
    ///      and binds all three TCP listeners on ephemeral ports).
    ///   4. Return ObsTestContext with the bound URLs and the fixed test admin_key.
    pub async fn start() -> Self {
        let container = Postgres::default()
            .with_tag("15-alpine")
            .start()
            .await
            .expect("Failed to start Postgres container for OBS tests");

        let port = container
            .get_host_port_ipv4(5432)
            .await
            .expect("Failed to get host port for OBS Postgres");

        let db_url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");

        let system_db = Arc::new(
            SystemDb::new(&db_url)
                .await
                .expect("SystemDb::new failed for OBS test context"),
        );
        system_db.migrate().await.expect("Migrations failed for OBS test context");

        let server = embyr_server::start_test_server(system_db).await;

        let admin_key = "test-admin-key-secret".to_string();
        let admin_base_url = format!("http://127.0.0.1:{}", server.admin_addr.port());
        let grpc_addr = server.grpc_addr;

        let http_client = reqwest::Client::builder()
            .build()
            .expect("reqwest client build failed");

        ObsTestContext {
            admin_base_url,
            admin_key,
            http_client,
            grpc_addr,
            db_url,
            _container: container,
            _server: server,
        }
    }

    /// GET /metrics with operator Bearer auth. Returns response body as String.
    ///
    /// Equivalent to:
    ///   curl -s -H "Authorization: Bearer {admin_key}" {admin_base_url}/metrics
    pub async fn get_metrics(&self) -> String {
        self.http_client
            .get(format!("{}/metrics", self.admin_base_url))
            .header("Authorization", format!("Bearer {}", self.admin_key))
            .send()
            .await
            .expect("GET /metrics request failed")
            .text()
            .await
            .expect("GET /metrics body read failed")
    }

    /// GET /metrics and return the HTTP status code.
    ///
    /// Used for auth-rejection tests (expecting 401).
    /// Pass `None` to omit the Authorization header entirely.
    pub async fn get_metrics_status(&self, auth_header: Option<&str>) -> u16 {
        let mut builder = self.http_client.get(format!("{}/metrics", self.admin_base_url));
        if let Some(auth) = auth_header {
            builder = builder.header("Authorization", auth);
        }
        builder
            .send()
            .await
            .expect("GET /metrics status request failed")
            .status()
            .as_u16()
    }

    /// POST /admin/v1/projects to provision a test project.
    ///
    /// Uses the test Postgres container DSN with `backend_mode=direct_pg`.
    /// The operator Bearer key is the fixed "test-admin-key-secret" set by
    /// `start_test_server`.
    pub async fn provision_project(&self, project_id: &str) {
        let body = serde_json::json!({
            "project_id": project_id,
            "dsn": self.db_url,
            "backend_mode": "direct_pg"
        });
        self.http_client
            .post(format!("{}/admin/v1/projects", self.admin_base_url))
            .header("Authorization", format!("Bearer {}", self.admin_key))
            .json(&body)
            .send()
            .await
            .expect("POST /admin/v1/projects request failed")
            .error_for_status()
            .expect("POST /admin/v1/projects must return 2xx");
    }

    /// Make a gRPC GetDocument call through the gRPC port to generate metric events.
    ///
    /// Uses a synthetic API key ("obs-test-key") — the call will fail with
    /// `unauthenticated` unless the project has been provisioned with that key.
    /// The counter `embyr_grpc_requests_total{method="GetDocument", ...}` is
    /// incremented regardless of success or failure.
    pub async fn make_grpc_call(&self, project_id: &str) {
        let channel =
            tonic::transport::Channel::from_shared(format!("http://{}", self.grpc_addr))
                .expect("valid gRPC channel URI")
                .connect_lazy();

        let mut client = FirestoreClient::new(channel);

        let name = format!(
            "projects/{project_id}/databases/(default)/documents/obs_col/doc1"
        );
        let mut req = tonic::Request::new(GetDocumentRequest {
            name,
            ..Default::default()
        });
        // Inject a synthetic API key — counter fires on any outcome including auth errors.
        req.metadata_mut().insert(
            "authorization",
            "bearer obs-test-key"
                .parse()
                .expect("valid metadata value"),
        );

        // Result is intentionally discarded — the test only checks that the metric
        // counter was incremented, not whether the document was found.
        let _ = client.get_document(req).await;
    }

    /// Parse a specific metric value from the /metrics Prometheus text output.
    ///
    /// Scans lines, skipping `# HELP` and `# TYPE` comment lines.
    /// Matches the first data line whose name+labels prefix is
    /// `"{metric_name}{label_suffix} "` and returns the trailing float string.
    ///
    /// # Examples
    ///
    /// ```text
    /// // body contains:
    /// //   embyr_grpc_requests_total{method="GetDocument",status="ok"} 3
    /// assert_eq!(
    ///     ObsTestContext::parse_metric_value(body, "embyr_grpc_requests_total",
    ///         r#"{method="GetDocument",status="ok"}"#),
    ///     Some("3")
    /// );
    /// ```
    pub fn parse_metric_value<'a>(
        body: &'a str,
        metric_name: &str,
        label_suffix: &str,
    ) -> Option<&'a str> {
        let target = format!("{}{} ", metric_name, label_suffix);
        for line in body.lines() {
            if line.starts_with('#') {
                continue;
            }
            if line.starts_with(target.as_str()) {
                // The value is the last whitespace-separated token on the line.
                return line.split_whitespace().next_back();
            }
        }
        None
    }
}
