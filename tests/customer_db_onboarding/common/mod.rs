//! Common test infrastructure — customer-db-onboarding acceptance tests.
//!
//! Infrastructure policy (docs/architecture/atdd-infrastructure-policy.md):
//!   US-01 driving port: `embyr-db-prep` subprocess (one-shot; exit code +
//!                        captured stdout/stderr, no healthz polling).
//!   US-02 driving port: `embyr-server` subprocess, `POST /admin/v1/projects`
//!                        via reqwest (mirrors tests/production_readiness's
//!                        ServerProcess pattern).
//!   Driven internal:    testcontainers-rs Postgres 15-alpine; fresh per test.
//!   Driven external:    none in scope for this feature.
//!
//! Test placement: `tests/customer_db_onboarding/acceptance/`.
//! Precedent: mirrors `tests/production_readiness/common/mod.rs` (subprocess
//! harness shape) and `tests/card_payments_backend/common/mod.rs` (per-feature
//! self-contained common module — no cross-feature imports).

#![allow(dead_code, unused_imports)]

// ─── State-delta re-export ─────────────────────────────────────────────────
#[path = "../../common/state_delta.rs"]
pub mod state_delta;
pub use state_delta::{appended_with, assert_state_delta, containing, set_to, unchanged};

// ─── Universe — port-exposed observable names ──────────────────────────────

/// Port-exposed observable names for customer-db-onboarding acceptance
/// assertions. All keys are port-exposed (process exit codes, captured
/// stdout/stderr content, HTTP status codes, response body fields) — never
/// internal struct fields.
pub mod universe {
    // ── embyr-db-prep subprocess (US-01) ───────────────────────────────────
    /// Exit code of the `embyr-db-prep` process (as string: "0", "1", "101").
    pub const PREP_EXIT_CODE: &str = "prep_process.exit_code";
    /// Whether stdout contains the expected success message.
    pub const PREP_STDOUT_HAS_SUCCESS: &str = "prep_process.stdout.success_message_present";
    /// Whether stderr contains the expected named-failure message.
    pub const PREP_STDERR_HAS_NAMED_FAILURE: &str = "prep_process.stderr.named_failure_present";

    // ── provisioning HTTP response (US-02) ──────────────────────────────────
    /// HTTP status code of `POST /admin/v1/projects` (as string: "201", "400").
    pub const PROVISION_HTTP_STATUS: &str = "provision_response.http_status";
    /// The `"error"` field of the JSON response body, or `""` if absent.
    pub const PROVISION_ERROR_FIELD: &str = "provision_response.error_field";
}

// ─── Imports ────────────────────────────────────────────────────────────────
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::time::Duration;

use sqlx::PgPool;
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt},
};

// ─── Postgres container helper ──────────────────────────────────────────────

/// Start a Postgres 15-alpine testcontainer and return the connection URL.
///
/// Caller must keep the returned `ContainerAsync<Postgres>` alive for the
/// duration of any process/pool that connects to it.
pub async fn start_postgres_container() -> (ContainerAsync<Postgres>, String) {
    let container = Postgres::default()
        .with_tag("15-alpine")
        .start()
        .await
        .expect("failed to start Postgres testcontainer");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("failed to get host port for Postgres");
    let db_url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    (container, db_url)
}

/// Create a role with the given `GRANT` clause fragments, connecting as the
/// container's superuser (`postgres`). Test-fixture setup only — this
/// establishes PRECONDITIONS (role/privilege state before the action under
/// test), never the expected outcome of the feature under test.
///
/// `grants` are executed in order after `CREATE ROLE <name> LOGIN PASSWORD
/// '<name>'`, e.g. `&["GRANT SELECT, INSERT, UPDATE, DELETE ON documents, \
/// transactions TO embyr_app"]`. Pass `&[]` for a role with no additional
/// grants (used for the "genuinely restricted" negative-test role).
pub async fn create_postgres_role(pool: &PgPool, role_name: &str, grants: &[&str]) {
    sqlx::query(&format!(
        "CREATE ROLE {role_name} LOGIN PASSWORD '{role_name}'"
    ))
    .execute(pool)
    .await
    .unwrap_or_else(|e| panic!("failed to create role {role_name}: {e}"));

    for grant in grants {
        sqlx::query(grant)
            .execute(pool)
            .await
            .unwrap_or_else(|e| panic!("failed to apply grant '{grant}' to {role_name}: {e}"));
    }
}

/// Create a DDL-capable role scoped to one database — mirrors the shape of
/// `elena_dba` in US-01's domain examples (`CREATE`, `CONNECT` only, not a
/// full superuser).
///
/// `pool` MUST be a connection to `database` itself (not merely the
/// cluster) — `CREATE ROLE` is cluster-wide, but `GRANT ... ON DATABASE`
/// and `GRANT CREATE ON SCHEMA public` are database-scoped. Postgres 15+
/// no longer grants `CREATE` on the `public` schema to non-owners by
/// default, even with `GRANT ALL ON DATABASE` — the explicit schema-level
/// grant below is required for the role to genuinely be able to run DDL
/// (`sqlx::migrate!`'s `CREATE TABLE` statements), not just connect.
pub async fn create_ddl_role(pool: &PgPool, role_name: &str, database: &str) {
    sqlx::query(&format!(
        "CREATE ROLE {role_name} LOGIN PASSWORD '{role_name}' CREATEDB"
    ))
    .execute(pool)
    .await
    .unwrap_or_else(|e| panic!("failed to create DDL role {role_name}: {e}"));
    sqlx::query(&format!("GRANT ALL ON DATABASE {database} TO {role_name}"))
        .execute(pool)
        .await
        .unwrap_or_else(|e| panic!("failed to grant DDL rights to {role_name}: {e}"));
    sqlx::query(&format!("GRANT CREATE ON SCHEMA public TO {role_name}"))
        .execute(pool)
        .await
        .unwrap_or_else(|e| {
            panic!("failed to grant CREATE ON SCHEMA public to {role_name}: {e}")
        });
}

/// Grant `SELECT` on `_sqlx_migrations` to `role_name` directly, as a raw
/// SQL statement executed by the test fixture.
///
/// Used ONLY in US-02 (provisioning verify-and-complain) test setup, where
/// the fixture needs a database already in the "prepped + granted" end
/// state a real `embyr-db-prep` run would have produced — the grant
/// MECHANISM itself (`discover_current_user()` +
/// `grant_schema_readiness_read()`) is US-01's concern, covered by
/// cdo07-cdo11. This is a precondition (Given), never the expected outcome
/// of the scenario under test (Critical Rule 7 — No Fixture Theater).
pub async fn grant_migrations_table_read(pool: &PgPool, role_name: &str) {
    sqlx::query(&format!(
        "GRANT SELECT ON _sqlx_migrations TO {role_name}"
    ))
    .execute(pool)
    .await
    .unwrap_or_else(|e| panic!("failed to grant _sqlx_migrations read to {role_name}: {e}"));
}

/// Create a fresh customer-scoped Postgres database on the cluster
/// `base_url` connects to, and return the connection URL for that database.
///
/// `base_url` must be of the `postgres://user:pass@host:port/db` shape
/// produced by `start_postgres_container`. Used by the US-02 provisioning
/// scenarios (cdo12-cdo18), each of which needs its own isolated customer
/// database within the shared cluster container.
pub async fn create_customer_database(base_url: &str, db_name: &str) -> String {
    let sys_pool = sqlx::PgPool::connect(base_url)
        .await
        .expect("connect to cluster to create customer database");
    sqlx::query(&format!("CREATE DATABASE {db_name}"))
        .execute(&sys_pool)
        .await
        .expect("create customer database");
    let last_slash = base_url.rfind('/').expect("db_url has a path separator");
    format!("{}/{db_name}", &base_url[..last_slash])
}

/// Build a connection URL substituting the role's own login for the given
/// base URL's host/port/database (assumes `postgres:postgres@host:port/db`
/// shape from `start_postgres_container`).
pub fn role_connection_url(base_url: &str, role_name: &str) -> String {
    let after_scheme = base_url
        .strip_prefix("postgres://")
        .expect("base_url must start with postgres://");
    let host_and_db = after_scheme
        .split_once('@')
        .map(|(_, rest)| rest)
        .unwrap_or(after_scheme);
    format!("postgres://{role_name}:{role_name}@{host_and_db}")
}

// ─── embyr-db-prep binary path resolution ───────────────────────────────────

/// Resolve the path to the pre-built `embyr-db-prep` binary via the
/// `CARGO_BIN_EXE_embyr-db-prep` env var Cargo sets for `[[test]]` targets
/// declared inside `crates/embyr-db-prep/Cargo.toml` (the owning package).
///
/// Uses `option_env!` (checked at runtime, not `env!`'s compile-time hard
/// error) because this `common/mod.rs` is `#[path]`-included into BOTH
/// `embyr-db-prep`'s and `embyr-server`'s `[[test]]` targets — the var is
/// only defined when compiled under the owning package. `env!` here would
/// be a hard compile error for whichever package isn't currently building,
/// so `clippy::option_env_unwrap`'s suggestion doesn't apply to this
/// cross-package-shared file (finding #36).
#[allow(clippy::option_env_unwrap)]
pub fn embyr_db_prep_binary() -> PathBuf {
    PathBuf::from(
        option_env!("CARGO_BIN_EXE_embyr-db-prep")
            .expect("CARGO_BIN_EXE_embyr-db-prep only set when compiled as an embyr-db-prep [[test]] target"),
    )
}

/// Result of running `embyr-db-prep` to completion (it is a one-shot
/// process — no healthz polling; completion is signalled by process exit,
/// per the ATDD Infrastructure Policy row for this binary).
pub struct DbPrepRun {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Run `embyr-db-prep` to completion with the given env vars and a 30s
/// timeout, capturing stdout/stderr.
///
/// # Panics when binary path not found
/// If the binary does not exist at the resolved path, this panics — this
/// test fails (RED: "binary missing" = implementation absent).
pub fn run_db_prep(env_vars: &[(&str, &str)]) -> DbPrepRun {
    let bin = embyr_db_prep_binary();
    let mut cmd = Command::new(&bin);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    for (key, val) in env_vars {
        cmd.env(key, val);
    }

    let mut child = cmd
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn embyr-db-prep at {bin:?}: {e}"));

    let output = wait_with_timeout(&mut child, Duration::from_secs(30));

    DbPrepRun {
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn wait_with_timeout(child: &mut Child, timeout: Duration) -> Output {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => {
                // Process exited — drain stdio via wait_with_output on a
                // fresh handle is not possible after try_wait(), so we take
                // the pipes directly.
                break;
            }
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => break,
        }
    }
    // Take ownership of stdio and finish waiting — safe to call after the
    // exit-detection loop above, whether it exited normally or was killed.
    let mut stdout_buf = Vec::new();
    let mut stderr_buf = Vec::new();
    if let Some(mut out) = child.stdout.take() {
        use std::io::Read;
        let _ = out.read_to_end(&mut stdout_buf);
    }
    if let Some(mut err) = child.stderr.take() {
        use std::io::Read;
        let _ = err.read_to_end(&mut stderr_buf);
    }
    let status = child.wait().expect("child.wait() after exit/kill");
    Output {
        status,
        stdout: stdout_buf,
        stderr: stderr_buf,
    }
}

// ─── embyr-server ServerProcess harness (US-02) ─────────────────────────────

/// Bind :0 and return the assigned ephemeral port.
pub fn find_free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind :0 failed")
        .local_addr()
        .expect("local_addr")
        .port()
}

/// 64-hex-char test encryption key (32 bytes, arbitrary — not a real secret).
pub const TEST_ENCRYPTION_KEY: &str =
    "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20";

/// Admin bearer key the US-02 provisioning scenarios spawn `embyr-server`
/// with, and authenticate `provision()` calls against — arbitrary, not a
/// real secret.
pub const TEST_ADMIN_KEY: &str = "testkey";

/// Wraps a spawned `embyr-server` subprocess. Mirrors
/// `tests/production_readiness/common/mod.rs::ServerProcess` (trimmed to
/// what US-02's provisioning scenarios need). Killed on `Drop`.
pub struct ServerProcess {
    pub child: Child,
    pub admin_port: u16,
}

impl ServerProcess {
    #[allow(clippy::option_env_unwrap)]
    pub fn embyr_server_binary() -> PathBuf {
        // option_env! (not env!) for the same cross-package-compile reason
        // as embyr_db_prep_binary() above (finding #36).
        PathBuf::from(
            option_env!("CARGO_BIN_EXE_embyr-server")
                .expect("CARGO_BIN_EXE_embyr-server only set when compiled as an embyr-server [[test]] target"),
        )
    }

    /// Spawn `embyr-server` with the given system-DB URL and extra env vars.
    pub fn start(db_url: &str, extra_env: &[(&str, &str)]) -> Self {
        let grpc_port = find_free_port();
        let rest_port = find_free_port();
        let admin_port = find_free_port();

        let bin = Self::embyr_server_binary();
        let mut cmd = Command::new(&bin);
        cmd.env("DATABASE_URL", db_url)
            .env("GRPC_PORT", grpc_port.to_string())
            .env("REST_PORT", rest_port.to_string())
            .env("ADMIN_PORT", admin_port.to_string())
            .env("RUST_LOG", "info")
            .stderr(Stdio::piped())
            .stdout(Stdio::piped());
        for (key, val) in extra_env {
            cmd.env(key, val);
        }
        let child = cmd
            .spawn()
            .unwrap_or_else(|e| panic!("failed to spawn embyr-server at {bin:?}: {e}"));
        ServerProcess { child, admin_port }
    }

    /// Poll `GET /healthz` until HTTP 200 or timeout. Returns `true` if
    /// healthy within `timeout`.
    pub async fn wait_for_healthy(&self, timeout: Duration) -> bool {
        let client = reqwest::Client::new();
        let url = format!("http://127.0.0.1:{}/healthz", self.admin_port);
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            if let Ok(resp) = client.get(&url).send().await {
                if resp.status().as_u16() == 200 {
                    return true;
                }
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    pub fn admin_base(&self) -> String {
        format!("http://127.0.0.1:{}", self.admin_port)
    }

    /// Like `wait_for_healthy`, but returns the last-observed `/healthz`
    /// status/error on timeout instead of a bare bool, so a caller can
    /// report *why* the server never came up instead of just that it didn't.
    pub async fn wait_for_healthy_verbose(&self, timeout: Duration) -> Result<(), String> {
        let client = reqwest::Client::new();
        let url = format!("http://127.0.0.1:{}/healthz", self.admin_port);
        let deadline = tokio::time::Instant::now() + timeout;
        let mut last = "no response received yet".to_string();
        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(last);
            }
            match client.get(&url).send().await {
                Ok(resp) if resp.status().as_u16() == 200 => return Ok(()),
                Ok(resp) => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    last = format!("HTTP {status}: {body}");
                }
                Err(e) => last = format!("request error: {e}"),
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }
}

/// Spawn `embyr-server` against `db_url` with the standard test admin key +
/// encryption key env vars, and assert it becomes healthy within 90s.
///
/// Shared by the US-02 provisioning scenarios (cdo12-cdo18), which otherwise
/// each repeated this identical spawn-and-wait sequence.
///
/// Timeout was 30s until 2026-09-20, when a contended CI runner (25m run vs
/// the usual 13-17m) blew through it — binary cold-start + migrations + pool
/// warmup genuinely took longer than 30s under load, not a server regression.
/// Bumped to 90s and switched to `wait_for_healthy_verbose` so a future
/// timeout panic carries the last poll's actual status/error, not just
/// "must be healthy" with zero diagnostic.
pub async fn start_healthy_server(db_url: &str) -> ServerProcess {
    let server = ServerProcess::start(
        db_url,
        &[
            ("EMBYR_ADMIN_KEY", TEST_ADMIN_KEY),
            ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
        ],
    );
    if let Err(last) = server.wait_for_healthy_verbose(Duration::from_secs(90)).await {
        panic!("embyr-server must be healthy before provisioning; last /healthz poll: {last}");
    }
    server
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// `POST /admin/v1/projects` and return (status_code, parsed JSON body).
pub async fn provision(
    server: &ServerProcess,
    admin_key: &str,
    body: serde_json::Value,
) -> (u16, serde_json::Value) {
    let http = reqwest::Client::new();
    let resp = http
        .post(format!("{}/admin/v1/projects", server.admin_base()))
        .header("Authorization", format!("Bearer {admin_key}"))
        .json(&body)
        .send()
        .await
        .expect("POST /admin/v1/projects request failed");
    let status = resp.status().as_u16();
    let text = resp.text().await.expect("response body text");
    let json: serde_json::Value = serde_json::from_str(&text)
        .unwrap_or_else(|_| serde_json::json!({ "raw_body": text }));
    (status, json)
}
