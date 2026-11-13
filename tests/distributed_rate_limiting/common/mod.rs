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

/// Charset-invalid `project_id` values — none can ever match
/// `^[a-z][a-z0-9-]{0,62}$` (docs/SPEC.md:73), so none can ever correspond to a
/// real, provisioned project (preauth-db-amplification, finding #14, DISCUSS §
/// Domain Examples / Example 2).
pub fn garbage_project_ids() -> Vec<String> {
    vec![
        "DROP_TABLE_123".to_string(),
        "' OR 1=1--".to_string(),
        "Not_Lowercase".to_string(),
        "-leading-dash".to_string(),
        "a".repeat(500),
    ]
}

// ─── Imports ──────────────────────────────────────────────────────────────────
use std::sync::Arc;

use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt},
};

use embyr_proto::firestore::firestore_client::FirestoreClient;
use embyr_proto::firestore::GetDocumentRequest;
use embyr_server::adapters::system_db::SystemDb;

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
    /// Superuser connection string for this test's Postgres container
    /// (`postgres://postgres:postgres@127.0.0.1:{port}/postgres`) — used by
    /// `rate-limiter-fail-open`'s fault-injection helpers to derive a
    /// scoped, non-superuser role DSN pointing at the SAME container.
    pub db_url: String,
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
        system_db
            .migrate()
            .await
            .expect("Migrations failed for DRL test context");

        let pool = system_db.pool().clone();

        let admin_client = reqwest::Client::builder().build().expect("reqwest client");

        DrlTestContext {
            _container: container,
            system_db,
            pool,
            db_url,
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
        let base = self
            .admin_base_url
            .as_deref()
            .expect("admin_base_url not set");
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
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM rate_buckets WHERE project_id = $1")
            .bind(project_id)
            .fetch_one(&self.pool)
            .await
            .unwrap_or(0)
    }

    /// Count rows in the projects table for a given project_id.
    pub async fn project_row_count(&self, project_id: &str) -> i64 {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM projects WHERE id = $1")
            .bind(project_id)
            .fetch_one(&self.pool)
            .await
            .unwrap_or(0)
    }

    // NOTE (preauth-db-amplification, finding #14): a `pg_stat_user_tables`
    // scan/insert-counter snapshot was tried first as the DB-round-trip
    // observability mechanism and REJECTED — empirically measured to lag
    // commits by Postgres's own internal `PGSTAT_MIN_INTERVAL` (~1s) stats
    // flush, producing false-zero deltas for single, fast sequential
    // requests. `rate_bucket_row_count`/`query_rate_bucket` (below) read the
    // real table directly (MVCC-consistent, no flush lag) and are used
    // instead — see the acceptance tests in `acceptance/b17_*`/`b18_*` for
    // the row-existence-based proof this feature relies on.

    /// A customer-DB DSN pointing at THIS SAME, real, reachable Postgres
    /// container with valid (superuser) credentials.
    ///
    /// rate-limiter-fail-open (finding #20): `authenticate()`'s downstream
    /// customer-DB connect/document-fetch is out of scope for b19's own
    /// rate-limiter assertions, but a bogus/unreachable/bad-credential DSN
    /// makes `authenticate()`'s adapter connect FAIL every single call —
    /// and `FirestoreServiceImpl::credential_cache` only caches a project's
    /// `SharedBackendAdapter` on a SUCCESSFUL connect. A permanently-failing
    /// DSN means every request (even repeats for the same project/api_key)
    /// pays the full cache-miss cost — a ~1.4s Argon2id verification PLUS,
    /// for an unroutable/unresolvable host, sqlx's pool retrying the
    /// connection for the full 5s `acquire_timeout`. That multi-second
    /// per-request latency lets the token bucket refill mid-test and breaks
    /// b19's bucket-exhaustion timing assertions ("N requests in quick
    /// succession"). A DSN that actually CONNECTS lets the adapter get
    /// cached after the first call, so repeat requests for the same project
    /// skip both Argon2id and reconnect (cache hit, sub-ms). The customer
    /// `documents` table is never actually queried successfully here (no
    /// customer-schema migrations applied against this DSN) — GetDocument
    /// still errors downstream, just fast and cache-eligible.
    fn cacheable_customer_dsn(&self) -> String {
        self.db_url.clone()
    }

    // ── Project seeding helpers ───────────────────────────────────────────────

    /// Insert a project directly into the projects table WITHOUT a rate_buckets row.
    /// Used to test the provisioning path that is supposed to create the rate_buckets row.
    pub async fn insert_project_without_bucket(&self, project_id: &str, api_key: &str) {
        use embyr_core::auth::{argon2, ecies};
        let api_key_hash =
            argon2::hash_api_key(api_key.as_bytes()).expect("argon2::hash_api_key failed");
        let pub_key = ecies::derive_public_key(api_key.as_bytes());
        let encrypted_dsn = ecies::encrypt(&pub_key, self.cacheable_customer_dsn().as_bytes())
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
        let api_key_hash =
            argon2::hash_api_key(api_key.as_bytes()).expect("argon2::hash_api_key failed");
        let pub_key = ecies::derive_public_key(api_key.as_bytes());
        let encrypted_dsn = ecies::encrypt(&pub_key, self.cacheable_customer_dsn().as_bytes())
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
pub fn get_document_request(project_id: &str, api_key: &str) -> tonic::Request<GetDocumentRequest> {
    let name = format!("projects/{project_id}/databases/(default)/documents/test/doc1");
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
pub fn classify_grpc_status<T>(result: &Result<tonic::Response<T>, tonic::Status>) -> String {
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

/// Pay the one-time Argon2id authentication cost for `(project_id, api_key)`
/// and populate `FirestoreService`'s in-process credential cache for it,
/// BEFORE any timing-sensitive rate-limit assertions run (rate-limiter-fail-
/// open, finding #20).
///
/// Without this, the FIRST `GetDocument` call for a never-before-seen
/// project/key pays a real ~1.4s Argon2id verification cost (CLAUDE.md's
/// non-negotiable `memory=65536KiB, iter=3, par=4` parameters) INSIDE the
/// same request whose rate-limiter decrement already ran (`authenticate()`
/// runs strictly after `rate_limiter.check()` in `handle_get_document`) —
/// delaying the CLIENT's next request by that same ~1.4s. Since
/// `rate_buckets.tokens` refills based on real elapsed wall-clock time
/// (`EXTRACT(EPOCH FROM (now() - last_refill))`), that ~1.4s gap is enough
/// for the bucket to refill mid-test, silently invalidating "N requests in
/// quick succession, N+1th rejected" assertions.
///
/// MUST be called while Postgres is still healthy — BEFORE any
/// `force_*_errors_on_rate_buckets` fault injection is installed — so the
/// warm-up's own token consumption lands on the real DB row (reset back to
/// `starting_tokens` by a direct superuser `UPDATE` afterward), never on
/// `check_in_process`'s in-memory fallback bucket (which starts fresh, per
/// project, the first time ANY test call actually hits it).
pub async fn warm_credential_cache_and_reset_bucket(
    ctx: &DrlTestContext,
    client: &mut FirestoreClient<tonic::transport::Channel>,
    project_id: &str,
    api_key: &str,
    starting_tokens: f64,
) {
    // Guarantee the warm-up call itself is ALLOWED (reaches
    // `authenticate()`/Argon2id) regardless of `starting_tokens` — some
    // callers intentionally seed at 0 tokens as their own test precondition
    // (e.g. AC-RLFO-03's meridian-labs). A rate-limiter REJECT short-circuits
    // before `authenticate()` ever runs (`handle_get_document`), so it would
    // never pay the Argon2id cost this helper exists to pre-pay.
    sqlx::query("UPDATE rate_buckets SET tokens = 1000.0 WHERE project_id = $1")
        .bind(project_id)
        .execute(&ctx.pool)
        .await
        .expect("bump tokens for credential-cache warm-up");
    let _ = client
        .get_document(get_document_request(project_id, api_key))
        .await;
    sqlx::query("UPDATE rate_buckets SET tokens = $1, last_refill = now() WHERE project_id = $2")
        .bind(starting_tokens)
        .bind(project_id)
        .execute(&ctx.pool)
        .await
        .expect("reset rate_buckets after credential-cache warm-up");
}

// ─── Metrics scrape helpers (rate-limiter-project-id-validation, AC-RLV-01) ───

/// Fixed operator Bearer key `start_test_server_with_distributed_rate_limit`
/// always configures the admin router with (see `embyr_server::lib.rs`).
pub const ADMIN_KEY: &str = "test-admin-key-secret";

/// GET /metrics on the given admin server with the fixed test operator key.
/// Returns the raw Prometheus text-format body.
pub async fn get_metrics(
    admin_client: &reqwest::Client,
    admin_addr: std::net::SocketAddr,
) -> String {
    admin_client
        .get(format!("http://{admin_addr}/metrics"))
        .header("Authorization", format!("Bearer {ADMIN_KEY}"))
        .send()
        .await
        .expect("GET /metrics request failed")
        .text()
        .await
        .expect("GET /metrics body read failed")
}

/// Parse the numeric value of a Prometheus counter/gauge line matching a
/// metric name AND all given label pairs, from a scraped text body. Returns
/// `0.0` if no matching line is found (metric not yet emitted — a fresh
/// counter defaults to 0 either way).
///
/// Used (preauth-db-amplification, finding #14) as the DB-round-trip
/// observability mechanism: `embyr_rate_limit_requests_total{project_id=
/// "unconfirmed",outcome="allowed"}` (OBS-04/ADR-069, already-existing
/// instrumentation) increments exactly once per `RateLimiter::check()`
/// INVOCATION for any never-before-seen project_id — regardless of whether
/// `check_pg`'s own `INSERT ... ON CONFLICT DO NOTHING` actually persists a
/// row (it silently fails via the `rate_buckets.project_id` FK to
/// `projects(id)` — confirmed by migration 0018 — for a project_id with no
/// `projects` row at all, e.g. pure garbage or a truly never-provisioned
/// id). Row-existence (`rate_bucket_row_count`) is therefore NOT a reliable
/// zero-vs-nonzero signal for those cases; this counter is call-site-scoped
/// (increments in `check()` itself, after `check_inner()` returns) and is
/// unaffected by what happens to the INSERT inside `check_pg`.
pub fn metric_value(body: &str, metric: &str, labels: &[(&str, &str)]) -> f64 {
    let prefix = format!("{metric}{{");
    for line in body.lines() {
        if line.starts_with('#') || !line.starts_with(&prefix) {
            continue;
        }
        if labels
            .iter()
            .all(|(k, v)| line.contains(&format!("{k}=\"{v}\"")))
        {
            if let Some(value_str) = line.rsplit(' ').next() {
                if let Ok(value) = value_str.parse::<f64>() {
                    return value;
                }
            }
        }
    }
    0.0
}

/// Extract the distinct `project_id` label VALUES present on the
/// `embyr_rate_limit_requests_total` metric family in a scraped Prometheus body.
///
/// Used to prove/disprove unbounded cardinality (ADR-069): before the fix, one
/// distinct label appears per distinct `project_id` string ever sent, regardless
/// of whether it corresponds to a real, provisioned project.
pub fn rate_limit_metric_project_id_labels(body: &str) -> std::collections::HashSet<String> {
    let mut labels = std::collections::HashSet::new();
    for line in body.lines() {
        if line.starts_with('#') {
            continue;
        }
        if !line.starts_with("embyr_rate_limit_requests_total{") {
            continue;
        }
        if let Some(start) = line.find("project_id=\"") {
            let rest = &line[start + "project_id=\"".len()..];
            if let Some(end) = rest.find('"') {
                labels.insert(rest[..end].to_string());
            }
        }
    }
    labels
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

/// Same as `start_distributed_grpc_server`, but wires the `RateLimiter`'s own
/// Postgres pool to `pg_pool` instead of deriving it from `ctx.system_db`.
///
/// Used by `rate-limiter-fail-open` (finding #20) to hand the rate limiter a
/// SCOPED, non-superuser pool (see `force_select_errors`/`force_update_errors`
/// below) while every OTHER server subsystem (auth, project lookup) keeps
/// using `ctx.system_db`'s own unaffected superuser pool.
pub async fn start_distributed_grpc_server_with_pool(
    ctx: &DrlTestContext,
    pg_pool: sqlx::PgPool,
) -> (std::net::SocketAddr, embyr_server::TestServer) {
    let system_db = Arc::clone(&ctx.system_db);
    let server = embyr_server::start_test_server_with_distributed_rate_limit(
        system_db,
        ctx.rate_limit_rps,
        pg_pool,
    )
    .await;
    let addr = server.grpc_addr;
    (addr, server)
}

// ─── Postgres fault injection (rate-limiter-fail-open, finding #20) ──────────
//
// Goal: force a REAL, fast-returning `sqlx::Error` at exactly one of
// `check_pg`'s two call sites (the atomic UPDATE, or the disambiguating
// `SELECT EXISTS`) — never a timeout, and clearly distinct from "the query
// genuinely succeeded and found no row" (which returns `Ok(None)`/`Ok(false)`,
// never `Err`).
//
// Why not `pg_terminate_backend` / container stop-start (this workspace's own
// precedent, `pr08_realtime_listener_reconnect.rs`): those break the WHOLE
// connection, so both of `check_pg`'s sequential queries fail identically —
// there is no way to make the UPDATE succeed with a legitimate "0 rows" and
// then have ONLY the follow-up EXISTS query error, which is exactly the shape
// AC-RLFO-03 needs. Postgres row-level security (RLS) can target a specific
// STATEMENT TYPE (`FOR SELECT` vs `FOR UPDATE`) independent of which columns
// each statement references — but RLS (and all privilege checks) are ALWAYS
// bypassed for a superuser role, which is what testcontainers' default
// `postgres` role is. `create_scoped_rate_limiter_role` therefore creates a
// dedicated, ordinary (non-superuser, non-owner) role scoped to exactly the
// privileges `check_pg` needs, so the RLS policies below actually apply to it.
// This changes only which DSN the TEST wires into `RateLimiter::with_pg` —
// zero production code is touched.

const RATE_LIMITER_ROLE: &str = "test_rate_limiter_app";
const RATE_LIMITER_ROLE_PASSWORD: &str = "test-rate-limiter-app-pw";

/// Create the scoped, non-superuser role (idempotent) and grant it exactly
/// the privileges `check_pg`'s three queries need on `rate_buckets`.
pub async fn create_scoped_rate_limiter_role(admin_pool: &sqlx::PgPool) {
    sqlx::query(&format!(
        "DO $$ BEGIN \
           IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{RATE_LIMITER_ROLE}') THEN \
             CREATE ROLE {RATE_LIMITER_ROLE} LOGIN PASSWORD '{RATE_LIMITER_ROLE_PASSWORD}'; \
           END IF; \
         END $$;"
    ))
    .execute(admin_pool)
    .await
    .expect("create scoped rate-limiter role");
    sqlx::query(&format!(
        "GRANT SELECT, UPDATE, INSERT ON rate_buckets TO {RATE_LIMITER_ROLE}"
    ))
    .execute(admin_pool)
    .await
    .expect("grant rate_buckets privileges to scoped role");
}

/// Build the scoped role's DSN pointing at the SAME Postgres container
/// `ctx.db_url` (superuser DSN) already targets.
pub fn scoped_rate_limiter_db_url(ctx: &DrlTestContext) -> String {
    ctx.db_url.replacen(
        "postgres:postgres@",
        &format!("{RATE_LIMITER_ROLE}:{RATE_LIMITER_ROLE_PASSWORD}@"),
        1,
    )
}

/// Connect a small dedicated pool as the scoped role — this is the pool
/// handed to `RateLimiter::with_pg` in the fault-injection tests.
pub async fn scoped_rate_limiter_pool(ctx: &DrlTestContext) -> sqlx::PgPool {
    create_scoped_rate_limiter_role(&ctx.pool).await;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .min_connections(1)
        .max_connections(1)
        .connect(&scoped_rate_limiter_db_url(ctx))
        .await
        .expect("connect scoped rate-limiter pool");
    // Warm up: establish + return the ONE connection to the pool before any
    // timed request runs. Without this, the FIRST `check_pg` call pays real
    // connection-establishment/auth latency (empirically ~9ms in this
    // sandbox) on top of the fault-injection query itself, which can push
    // total elapsed time past the hard-coded 20ms budget and trigger the
    // EXISTING timeout fallback instead of the NEW error-triggered one this
    // feature adds — confounding AC-RLFO-02/03/05 with AC-RLFO-06. A single
    // connection (`max_connections(1)`) is used throughout so `check_pg`'s
    // two sequential queries always reuse the SAME warm connection.
    sqlx::query("SELECT 1").execute(&pool).await.expect("warm up scoped pool");
    pool
}

/// Install the PL/pgSQL function used as every fault-injection policy's
/// `USING` predicate — unconditionally raises, producing a genuine
/// `sqlx::Error::Database`, not a mocked error.
async fn install_raise_fn(admin_pool: &sqlx::PgPool) {
    sqlx::query(
        "CREATE OR REPLACE FUNCTION test_force_pg_error() RETURNS boolean \
         LANGUAGE plpgsql AS $$ \
         BEGIN RAISE EXCEPTION 'DISTILL fault injection: forced Postgres error (rate-limiter-fail-open)'; \
         END; $$",
    )
    .execute(admin_pool)
    .await
    .expect("install test_force_pg_error() function");
}

async fn drop_policies_if_exist(admin_pool: &sqlx::PgPool, names: &[&str]) {
    for name in names {
        sqlx::query(&format!("DROP POLICY IF EXISTS {name} ON rate_buckets"))
            .execute(admin_pool)
            .await
            .ok();
    }
}

/// Force every SELECT the scoped role issues against `rate_buckets`
/// (including `check_pg`'s disambiguating `SELECT EXISTS(...)`, AC-RLFO-03)
/// to fail with a genuine Postgres error, while UPDATE/INSERT stay
/// functional. Call with `ctx.pool` (superuser — required to alter RLS).
pub async fn force_select_errors_on_rate_buckets(admin_pool: &sqlx::PgPool) {
    install_raise_fn(admin_pool).await;
    sqlx::query("ALTER TABLE rate_buckets ENABLE ROW LEVEL SECURITY")
        .execute(admin_pool)
        .await
        .expect("enable RLS on rate_buckets");
    drop_policies_if_exist(
        admin_pool,
        &["test_block_all", "test_block_select", "test_block_update", "test_allow_select", "test_allow_update", "test_allow_insert"],
    )
    .await;
    sqlx::query("CREATE POLICY test_block_select ON rate_buckets FOR SELECT USING (test_force_pg_error())")
        .execute(admin_pool)
        .await
        .expect("create block-select policy");
    sqlx::query("CREATE POLICY test_allow_update ON rate_buckets FOR UPDATE USING (true) WITH CHECK (true)")
        .execute(admin_pool)
        .await
        .expect("create allow-update policy");
    sqlx::query("CREATE POLICY test_allow_insert ON rate_buckets FOR INSERT WITH CHECK (true)")
        .execute(admin_pool)
        .await
        .expect("create allow-insert policy");
}

/// Force every UPDATE the scoped role issues against `rate_buckets` (the
/// atomic token-bucket UPDATE, AC-RLFO-02) to fail with a genuine Postgres
/// error, while SELECT/INSERT stay functional.
pub async fn force_update_errors_on_rate_buckets(admin_pool: &sqlx::PgPool) {
    install_raise_fn(admin_pool).await;
    sqlx::query("ALTER TABLE rate_buckets ENABLE ROW LEVEL SECURITY")
        .execute(admin_pool)
        .await
        .expect("enable RLS on rate_buckets");
    drop_policies_if_exist(
        admin_pool,
        &["test_block_all", "test_block_select", "test_block_update", "test_allow_select", "test_allow_update", "test_allow_insert"],
    )
    .await;
    sqlx::query("CREATE POLICY test_block_update ON rate_buckets FOR UPDATE USING (test_force_pg_error())")
        .execute(admin_pool)
        .await
        .expect("create block-update policy");
    sqlx::query("CREATE POLICY test_allow_select ON rate_buckets FOR SELECT USING (true)")
        .execute(admin_pool)
        .await
        .expect("create allow-select policy");
    sqlx::query("CREATE POLICY test_allow_insert ON rate_buckets FOR INSERT WITH CHECK (true)")
        .execute(admin_pool)
        .await
        .expect("create allow-insert policy");
}

/// Force EVERY statement (SELECT, UPDATE, INSERT) the scoped role issues
/// against `rate_buckets` to fail — simulates a sustained, full outage of the
/// rate limiter's own data path (AC-RLFO-05) while the rest of the server
/// (auth, project lookups — `ctx.system_db`'s own separate superuser pool)
/// stays unaffected.
pub async fn force_all_errors_on_rate_buckets(admin_pool: &sqlx::PgPool) {
    install_raise_fn(admin_pool).await;
    sqlx::query("ALTER TABLE rate_buckets ENABLE ROW LEVEL SECURITY")
        .execute(admin_pool)
        .await
        .expect("enable RLS on rate_buckets");
    drop_policies_if_exist(
        admin_pool,
        &["test_block_all", "test_block_select", "test_block_update", "test_allow_select", "test_allow_update", "test_allow_insert"],
    )
    .await;
    sqlx::query(
        "CREATE POLICY test_block_all ON rate_buckets FOR ALL USING (test_force_pg_error()) WITH CHECK (test_force_pg_error())",
    )
    .execute(admin_pool)
    .await
    .expect("create block-all policy");
}
