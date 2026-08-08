// SCAFFOLD: true
//! Common test infrastructure — distributed-rate-limiting acceptance tests.
//!
//! Infrastructure policy (docs/architecture/atdd-infrastructure-policy.md):
//!   Driving port:    tonic gRPC client against embyr-server on ephemeral port
//!                    (start_test_server_with_distributed_rate_limit — added in DRL-02).
//!   Driven internal: testcontainers-rs Postgres 15-alpine; sqlx::migrate!; system DB.
//!   Driven external: none in scope for this feature.
//!
//! Scaffold state (DISTILL wave, 2026-08-07):
//!   - Postgres container startup: LIVE (compiles + runs)
//!   - Migration (incl. 0018_rate_buckets.sql): LIVE after DRL-01 DELIVER creates the file
//!   - rate_buckets query helpers: RED (table absent until migration 0018 is applied)
//!   - gRPC server start: RED (todo! until DRL-02 DELIVER adds start_test_server_with_distributed_rate_limit)
//!   - Admin HTTP setup: RED (todo! until DRL-05 DELIVER updates provision_project)
//!
//! Items are `#[allow(dead_code)]` — they are consumed incrementally as each
//! slice's tests are unskipped by DELIVER.

#![allow(dead_code, unused_imports)]

// ─── State-delta re-export ────────────────────────────────────────────────────
#[path = "../../common/state_delta.rs"]
pub mod state_delta;
pub use state_delta::{appended_with, assert_state_delta, containing, set_to, unchanged};

// ─── Universe — port-exposed observable names ─────────────────────────────────
/// Universe observable names for rate-limiting acceptance assertions.
/// All keys are port-exposed (response metadata, DB public fields, counter values).
/// Never internal struct fields.
pub mod universe {
    /// Whether a rate_buckets row exists for the project in the system DB.
    pub const RATE_BUCKET_EXISTS: &str = "db.rate_buckets.row_exists";
    /// Current token count in rate_buckets for the project (as string "123.0").
    pub const RATE_BUCKET_TOKENS: &str = "db.rate_buckets.tokens";
    /// Whether a projects row exists in the system DB.
    pub const PROJECT_ROW_EXISTS: &str = "db.projects.row_exists";
    /// gRPC response status code string ("ok", "resource_exhausted", etc.).
    pub const GRPC_STATUS_CODE: &str = "grpc.response.status_code";
    /// Value of x-ratelimit-limit trailing metadata header.
    pub const RATELIMIT_LIMIT_HEADER: &str = "grpc.response.metadata.x-ratelimit-limit";
    /// Value of x-ratelimit-remaining trailing metadata header.
    pub const RATELIMIT_REMAINING_HEADER: &str = "grpc.response.metadata.x-ratelimit-remaining";
    /// Whether x-ratelimit-reset header is present (non-empty string = present).
    pub const RATELIMIT_RESET_HEADER: &str = "grpc.response.metadata.x-ratelimit-reset";
    /// Value of retry-after-ms trailing metadata on rejected responses.
    pub const RETRY_AFTER_MS_HEADER: &str = "grpc.response.metadata.retry-after-ms";
    /// Cumulative count of Postgres 20ms timeouts (metrics counter).
    pub const PG_TIMEOUT_COUNTER: &str = "metrics.rate_limit_pg_timeout_total";
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

// ─── DrlTestContext ───────────────────────────────────────────────────────────

/// Ephemeral test context for distributed-rate-limiting acceptance tests.
///
/// Holds the Postgres container alive for the test's duration.
/// The gRPC server is started separately via `start_distributed_grpc_server()`.
pub struct DrlTestContext {
    /// Keeps the Postgres container alive for the test's duration.
    _container: ContainerAsync<Postgres>,
    /// System DB handle — held so the pool remains open and for passing to server constructors.
    pub system_db: Arc<embyr_server::adapters::system_db::SystemDb>,
    /// Direct pool for DB assertion queries (rate_buckets, projects, etc.).
    pub pool: sqlx::PgPool,
    /// Configured RPS limit for this test context (default 10.0 — small enough to exhaust quickly).
    pub rate_limit_rps: f64,
    /// gRPC server address — set via `with_grpc_addr()` after starting the distributed server.
    pub grpc_addr: Option<std::net::SocketAddr>,
    /// reqwest client for admin HTTP calls (provisioning tests).
    pub admin_client: reqwest::Client,
    /// Base URL of the admin HTTP server, if started for provisioning tests.
    pub admin_base_url: Option<String>,
}

impl DrlTestContext {
    /// Build a DrlTestContext:
    ///   1. Start testcontainers Postgres 15-alpine.
    ///   2. Run sqlx migrations from workspace `migrations/` directory.
    ///      After DRL-01 DELIVER creates `0018_rate_buckets.sql`, this will
    ///      also apply the rate_buckets table and backfill.
    ///   3. Return context with pool for direct DB assertions.
    ///
    /// Does NOT start the gRPC server — tests that need gRPC call
    /// `start_distributed_grpc_server()` separately (which is todo! until DRL-02).
    pub async fn new(rate_limit_rps: f64) -> Self {
        let container = Postgres::default()
            .with_tag("15-alpine")
            .start()
            .await
            .expect("Failed to start Postgres container for DRL tests");
        let port = container
            .get_host_port_ipv4(5432)
            .await
            .expect("Failed to get host port for DRL Postgres");
        let db_url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");

        let system_db = Arc::new(
            SystemDb::new(&db_url)
                .await
                .expect("SystemDb::new failed for DRL test context"),
        );
        system_db.migrate().await.expect("Migrations failed for DRL test context");

        let pool = system_db.pool().clone();

        let admin_client = reqwest::Client::builder()
            .build()
            .expect("reqwest client");

        DrlTestContext {
            _container: container,
            system_db,
            pool,
            rate_limit_rps,
            grpc_addr: None,
            admin_client,
            admin_base_url: None,
        }
    }

    /// Set the gRPC server address on this context (called after starting the distributed server).
    pub fn with_grpc_addr(mut self, addr: std::net::SocketAddr) -> Self {
        self.grpc_addr = Some(addr);
        self
    }

    /// Set the admin HTTP base URL on this context.
    pub fn with_admin_url(mut self, url: String) -> Self {
        self.admin_base_url = Some(url);
        self
    }

    /// Returns the full URL for the given admin API path.
    pub fn admin_url(&self, path: &str) -> String {
        let base = self.admin_base_url.as_deref().expect("admin_base_url not set");
        format!("{}{}", base, path)
    }

    /// Return a lazy gRPC channel to the embedded test server.
    ///
    /// # SCAFFOLD
    /// Panics with todo! until DRL-02 DELIVER sets `self.grpc_addr`.
    pub fn grpc_channel(&self) -> tonic::transport::Channel {
        let addr = self.grpc_addr.unwrap_or_else(|| {
            // SCAFFOLD: true
            panic!("DrlTestContext.grpc_addr is None — call start_distributed_grpc_server() first (DRL-02 DELIVER)")
        });
        tonic::transport::Channel::from_shared(format!("http://{addr}"))
            .expect("invalid gRPC channel URI")
            .connect_lazy()
    }

    // ── DB assertion helpers ──────────────────────────────────────────────────

    /// Query the rate_buckets row for a given project.
    ///
    /// Returns `Some((tokens, last_refill))` or `None` if no row exists.
    ///
    /// NOTE: This query will fail at runtime until DRL-01 DELIVER applies
    /// `migrations/0018_rate_buckets.sql`. At that point the table exists
    /// and this helper becomes functional.
    pub async fn query_rate_bucket(
        &self,
        project_id: &str,
    ) -> Option<(f64, chrono::DateTime<chrono::Utc>)> {
        // SCAFFOLD: true — rate_buckets table absent until migration 0018 is applied.
        // This will return a sqlx error at runtime; tests using it will be RED.
        sqlx::query_as::<_, (f64, chrono::DateTime<chrono::Utc>)>(
            "SELECT tokens, last_refill FROM rate_buckets WHERE project_id = $1",
        )
        .bind(project_id)
        .fetch_optional(&self.pool)
        .await
        .unwrap_or(None)
    }

    /// Count rows in rate_buckets for a given project.
    /// Returns 0 if the table does not exist yet (DRL-01 pending) or the project has no row.
    pub async fn rate_bucket_row_count(&self, project_id: &str) -> i64 {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM rate_buckets WHERE project_id = $1",
        )
        .bind(project_id)
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0)
    }

    /// Count rows in the projects table for a given project_id.
    pub async fn project_row_count(&self, project_id: &str) -> i64 {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM projects WHERE id = $1",
        )
        .bind(project_id)
        .fetch_one(&self.pool)
        .await
        .unwrap_or(0)
    }

    // ── Project seeding helpers ───────────────────────────────────────────────

    /// Insert a project directly into the projects table WITHOUT a rate_buckets row.
    /// Used to test the provisioning path that is supposed to create the rate_buckets row.
    pub async fn insert_project_without_bucket(&self, project_id: &str, api_key: &str) {
        use embyr_core::auth::{argon2, ecies};
        let api_key_hash = argon2::hash_api_key(api_key.as_bytes())
            .expect("argon2::hash_api_key failed");
        let pub_key = ecies::derive_public_key(api_key.as_bytes());
        let encrypted_dsn =
            ecies::encrypt(&pub_key, b"postgres://placeholder:5432/test")
                .expect("ecies::encrypt failed");

        sqlx::query(
            "INSERT INTO projects \
             (id, status, backend_mode, api_key_hash_current, ecies_encrypted_dsn) \
             VALUES ($1, 'active', 'direct_pg', $2, $3)",
        )
        .bind(project_id)
        .bind(&api_key_hash)
        .bind(&encrypted_dsn)
        .execute(&self.pool)
        .await
        .expect("insert project without rate_bucket row");
    }

    /// Insert a project WITH a pre-populated rate_buckets row (expected state after DRL-01+DRL-05).
    ///
    /// NOTE: Will fail at runtime until `rate_buckets` table exists (DRL-01 pending).
    pub async fn insert_project_with_bucket(
        &self,
        project_id: &str,
        api_key: &str,
        initial_tokens: f64,
    ) {
        use embyr_core::auth::{argon2, ecies};
        let api_key_hash = argon2::hash_api_key(api_key.as_bytes())
            .expect("argon2::hash_api_key failed");
        let pub_key = ecies::derive_public_key(api_key.as_bytes());
        let encrypted_dsn =
            ecies::encrypt(&pub_key, b"postgres://placeholder:5432/test")
                .expect("ecies::encrypt failed");

        sqlx::query(
            "INSERT INTO projects \
             (id, status, backend_mode, api_key_hash_current, ecies_encrypted_dsn) \
             VALUES ($1, 'active', 'direct_pg', $2, $3)",
        )
        .bind(project_id)
        .bind(&api_key_hash)
        .bind(&encrypted_dsn)
        .execute(&self.pool)
        .await
        .expect("insert project row for insert_project_with_bucket");

        // SCAFFOLD: true — rate_buckets INSERT will fail until migration 0018 exists.
        sqlx::query(
            "INSERT INTO rate_buckets (project_id, tokens, last_refill) VALUES ($1, $2, now())",
        )
        .bind(project_id)
        .bind(initial_tokens)
        .execute(&self.pool)
        .await
        .expect("insert rate_buckets row");
    }
}

// ─── gRPC helpers ─────────────────────────────────────────────────────────────

/// Build a GetDocument request for a given project and API key.
/// Uses the canonical Firestore resource name format.
pub fn get_document_request(
    project_id: &str,
    api_key: &str,
) -> tonic::Request<GetDocumentRequest> {
    let name = format!(
        "projects/{project_id}/databases/(default)/documents/test/doc1"
    );
    let mut req = tonic::Request::new(GetDocumentRequest {
        name,
        ..Default::default()
    });
    req.metadata_mut().insert(
        "authorization",
        format!("bearer {api_key}")
            .parse()
            .expect("valid metadata value"),
    );
    req
}

/// Classify a tonic gRPC call result as a status code string.
/// Returns "ok", "not_found", "resource_exhausted", "permission_denied", or "other:{code}".
pub fn classify_grpc_status<T>(
    result: &Result<tonic::Response<T>, tonic::Status>,
) -> String {
    match result {
        Ok(_) => "ok".to_string(),
        Err(s) => match s.code() {
            tonic::Code::NotFound => "not_found".to_string(),
            tonic::Code::ResourceExhausted => "resource_exhausted".to_string(),
            tonic::Code::PermissionDenied => "permission_denied".to_string(),
            code => format!("other:{code:?}"),
        },
    }
}

// ─── Distributed gRPC server starter (scaffold) ───────────────────────────────

/// Start a gRPC test server with distributed (Postgres-backed) rate limiting enabled.
///
/// Returns `(addr, server)`.  The caller **must** bind the returned `TestServer`
/// to a local variable for the entire test duration; dropping it earlier sends
/// the shutdown signal and makes the server stop listening before the test
/// assertions run.
///
/// Wires `RateLimiter::with_pg` so the server enforces rate limits via the
/// shared `rate_buckets` table in the test Postgres container.
/// The pool used for rate limiting is taken from `system_db.pool()` — the same
/// connection pool that backs the system DB with the applied migrations.
pub async fn start_distributed_grpc_server(
    ctx: &DrlTestContext,
) -> (std::net::SocketAddr, embyr_server::TestServer) {
    let system_db = Arc::clone(&ctx.system_db);
    let pg_pool = system_db.pool().clone();
    let server = embyr_server::start_test_server_with_distributed_rate_limit(
        system_db,
        ctx.rate_limit_rps,
        pg_pool,
    )
    .await;
    let addr = server.grpc_addr;
    (addr, server)
}
