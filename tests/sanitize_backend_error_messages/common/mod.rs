// SCAFFOLD: true
//! Common test infrastructure — sanitize-backend-error-messages acceptance
//! tests.
//!
//! Infrastructure policy (`docs/architecture/atdd-infrastructure-policy.md`):
//!   Driving ports: gRPC data port (:8080, in-process tonic client) already
//!     has a reusable row — no new policy row needed. This feature
//!     introduces zero new RPC, only a message-content change at an
//!     existing conversion boundary (ADR-075).
//!   Driven internal: testcontainers-rs Postgres (System DB row, Customer DB
//!     row) — reused unchanged.
//!   Driven external: none.
//!
//! `ServerTestContext` mirrors `tests/client_auth/common/mod.rs`'s own
//! `ClientAuthFullContext` shape (Pillar 3: the FULL production composition
//! root, `embyr_server::start_test_server` — no hand-assembled router for
//! this seam) but starts the System Postgres container only; individual
//! scenarios attach a `direct_pg` project with either a deliberately
//! unreachable backend (WS/regression-of-failure scenarios) or a real,
//! reachable customer Postgres container (cache-warming / access-rule-family
//! scenarios) via the two `insert_project_with_*` helpers below.

#![allow(dead_code, unused_imports)]

use std::sync::Arc;

use embyr_core::auth::{argon2, ecies};
use embyr_server::adapters::system_db::SystemDb;
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt},
};

// ─── Server test context ───────────────────────────────────────────────────

pub struct ServerTestContext {
    _sys_container: ContainerAsync<Postgres>,
    pub server: embyr_server::TestServer,
    pub sys_pool: sqlx::PgPool,
    pub account_id: uuid::Uuid,
}

impl ServerTestContext {
    /// Starts the production composition root (`embyr_server::start_test_server`)
    /// against a real System Postgres container. No customer Postgres is
    /// started here — each scenario attaches its own `direct_pg` project
    /// (working or deliberately broken) via the helpers below, matching
    /// what that scenario actually needs.
    pub async fn new() -> Self {
        let sys_container = Postgres::default()
            .with_tag("15-alpine")
            .start()
            .await
            .expect("start system Postgres container");
        let sys_port = sys_container
            .get_host_port_ipv4(5432)
            .await
            .expect("system Postgres host port");
        let sys_url = format!("postgres://postgres:postgres@127.0.0.1:{sys_port}/postgres");

        let system_db = Arc::new(SystemDb::new(&sys_url).await.expect("SystemDb::new failed"));
        system_db
            .migrate()
            .await
            .expect("system DB migrations failed");
        let sys_pool = system_db.pool().clone();

        let account_id: uuid::Uuid = sqlx::query_scalar(
            "INSERT INTO accounts (name) VALUES ('Sanitize Backend Error Messages Test Account') \
             RETURNING id",
        )
        .fetch_one(&sys_pool)
        .await
        .expect("insert account");

        let server = embyr_server::start_test_server(system_db).await;

        ServerTestContext {
            _sys_container: sys_container,
            server,
            sys_pool,
            account_id,
        }
    }

    /// Insert a `direct_pg` project whose stored DSN points at an address
    /// nothing is listening on (`127.0.0.1:1` — an unprivileged process can
    /// never bind port 1, so the connect attempt fails fast and
    /// deterministically with connection-refused, no container/network
    /// flakiness, and zero dependency on DNS/network egress in CI).
    /// Returns the API key.
    pub async fn insert_project_with_unreachable_backend(&self, project_id: &str) -> String {
        let api_key = format!("test-sk-sbm-{project_id}");
        self.insert_direct_pg_project(
            project_id,
            &api_key,
            "postgres://baduser:badpass@127.0.0.1:1/nonexistent_db",
        )
        .await;
        api_key
    }

    /// Insert a `direct_pg` project whose stored DSN points at a REAL,
    /// reachable customer Postgres container (migrated). Returns the API
    /// key, the container handle (keep alive for the scenario's duration —
    /// dropping it, or calling `.stop()` on it, is how a scenario later
    /// simulates the backend becoming unreachable mid-session), and the
    /// customer pool (for direct seeding).
    pub async fn insert_project_with_working_backend(
        &self,
        project_id: &str,
    ) -> (String, ContainerAsync<Postgres>, sqlx::PgPool) {
        let cust_container = Postgres::default()
            .with_tag("15-alpine")
            .start()
            .await
            .expect("start customer Postgres container");
        let cust_port = cust_container
            .get_host_port_ipv4(5432)
            .await
            .expect("customer Postgres host port");
        let cust_url = format!("postgres://postgres:postgres@127.0.0.1:{cust_port}/postgres");

        let cust_pool = sqlx::PgPool::connect(&cust_url)
            .await
            .expect("connect customer DB");
        sqlx::migrate!("../../migrations/customer")
            .run(&cust_pool)
            .await
            .expect("customer DB migrations failed");

        let api_key = format!("test-sk-sbm-{project_id}");
        self.insert_direct_pg_project(project_id, &api_key, &cust_url)
            .await;

        (api_key, cust_container, cust_pool)
    }

    async fn insert_direct_pg_project(&self, project_id: &str, api_key: &str, dsn: &str) {
        let api_key_hash = argon2::hash_api_key(api_key.as_bytes()).expect("hash api key");
        let pub_key = ecies::derive_public_key(api_key.as_bytes());
        let encrypted_dsn = ecies::encrypt(&pub_key, dsn.as_bytes()).expect("ecies encrypt dsn");

        sqlx::query(
            "INSERT INTO projects \
             (id, account_id, status, backend_mode, api_key_hash_current, ecies_encrypted_dsn) \
             VALUES ($1, $2, 'active', 'direct_pg', $3, $4)",
        )
        .bind(project_id)
        .bind(self.account_id)
        .bind(&api_key_hash)
        .bind(&encrypted_dsn)
        .execute(&self.sys_pool)
        .await
        .expect("insert project");
    }

    /// Directly seed an `access_rules` row (mirrors
    /// `tests/security_rules/common/mod.rs::seed_access_rule` exactly — same
    /// table, same bypass-the-admin-endpoint allowance, since this feature
    /// tests the ROUTING LOOKUP's own failure mode, not rule definition).
    pub async fn seed_access_rule(
        &self,
        project_id: &str,
        collection_path: &str,
        condition_source: &str,
    ) {
        sqlx::query(
            "INSERT INTO access_rules (project_id, collection_path, condition_source) \
             VALUES ($1, $2, $3)",
        )
        .bind(project_id)
        .bind(collection_path)
        .bind(condition_source)
        .execute(&self.sys_pool)
        .await
        .expect("insert access_rules row");
    }
}

// ─── gRPC helpers ───────────────────────────────────────────────────────────

pub fn make_channel(addr: std::net::SocketAddr) -> tonic::transport::Channel {
    tonic::transport::Channel::from_shared(format!("http://{addr}"))
        .expect("valid gRPC channel URI")
        .connect_lazy()
}

pub fn authed_request<T>(payload: T, api_key: &str) -> tonic::Request<T> {
    let mut req = tonic::Request::new(payload);
    req.metadata_mut().insert(
        "authorization",
        format!("bearer {api_key}")
            .parse()
            .expect("valid metadata value"),
    );
    req
}

// ─── Server-side tracing capture (AC-SBM-03 proof) ─────────────────────────

/// Minimal server-side tracing capture, used to prove the real error stays
/// observable via `tracing::error!` even after being discarded from the
/// client-facing `Status` (AC-SBM-03). No new dependency — built entirely on
/// `tracing-subscriber`'s own `fmt::MakeWriter`, which is already a
/// workspace dependency (`crates/embyr-server/src/main.rs` already installs
/// a `tracing_subscriber::fmt` subscriber in production).
///
/// One global subscriber is installed once per test BINARY (`std::sync::Once`
/// — each `[[test]]` entry compiles to its own process, so this never
/// collides across feature directories) writing into a shared, process-wide
/// buffer. A scenario snapshots the buffer's length immediately before its
/// own RPC call and again immediately after, then checks the NEW bytes for
/// an ERROR-level line — robust regardless of the exact `context` wording
/// DELIVER chooses per call site (ADR-075 leaves that wording to DELIVER),
/// and correctly RED today: none of the affected call sites currently call
/// `tracing::error!` at all, so the delta is always empty pre-fix.
pub mod tracing_capture {
    use std::io;
    use std::sync::{Arc, Mutex, Once, OnceLock};

    use tracing_subscriber::fmt::MakeWriter;

    #[derive(Clone, Default)]
    pub struct CapturedLogs(Arc<Mutex<Vec<u8>>>);

    impl io::Write for CapturedLogs {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .expect("captured logs mutex poisoned")
                .extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for CapturedLogs {
        type Writer = Self;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    static LOGS: OnceLock<CapturedLogs> = OnceLock::new();
    static INIT: Once = Once::new();

    /// Install the capture subscriber (idempotent within one test binary)
    /// and return the shared buffer.
    pub fn init() -> CapturedLogs {
        let logs = LOGS.get_or_init(CapturedLogs::default).clone();
        INIT.call_once(|| {
            let subscriber = tracing_subscriber::fmt()
                .with_writer(logs.clone())
                .with_ansi(false)
                .with_max_level(tracing::Level::TRACE)
                .finish();
            tracing::subscriber::set_global_default(subscriber)
                .expect("install global test tracing subscriber");
        });
        logs
    }

    /// Byte length of everything captured so far — take before and after an
    /// RPC call, then use `new_output` to read just the delta.
    pub fn len(logs: &CapturedLogs) -> usize {
        logs.0.lock().expect("captured logs mutex poisoned").len()
    }

    /// UTF-8 (lossy) text captured after byte offset `since`.
    pub fn new_output(logs: &CapturedLogs, since: usize) -> String {
        let buf = logs.0.lock().expect("captured logs mutex poisoned");
        let since = since.min(buf.len());
        String::from_utf8_lossy(&buf[since..]).to_string()
    }
}
