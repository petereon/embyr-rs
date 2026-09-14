// SCAFFOLD: true
//! Common test infrastructure — production-readiness acceptance tests.
//!
//! Infrastructure policy (docs/architecture/atdd-infrastructure-policy.md):
//!   Driving port:    reqwest::Client against the spawned `embyr-server` process
//!                    via `GET /healthz` on the admin port (subprocess driving adapter).
//!   Driven internal: testcontainers-rs Postgres 15-alpine; fresh per test.
//!   Driven external: none in scope for this feature.
//!
//! Scaffold state (DISTILL wave, 2026-08-08):
//!   - `find_free_port()`:             LIVE
//!   - `embyr_server_binary()`:        LIVE (resolves path; binary absent until DELIVER ships main.rs)
//!   - `ServerProcess::start()`:       LIVE (spawn fails gracefully when binary missing → RED)
//!   - `ServerProcess::wait_for_healthy()`: LIVE (polls /healthz; returns false until server up → RED)
//!   - `start_postgres_container()`:   LIVE
//!
//! Items are `#[allow(dead_code)]` — consumed incrementally as each slice is unskipped.

#![allow(dead_code, unused_imports)]

// ─── State-delta re-export ────────────────────────────────────────────────────
#[path = "../../common/state_delta.rs"]
pub mod state_delta;
pub use state_delta::{appended_with, assert_state_delta, containing, set_to, unchanged};

// ─── Universe — port-exposed observable names ─────────────────────────────────

/// Port-exposed observable names for production-readiness acceptance assertions.
/// All keys are port-exposed (HTTP status codes, process exit codes, stderr content).
/// Never internal struct fields.
pub mod universe {
    /// Whether `GET /healthz` on the admin port returned HTTP 200.
    pub const PROCESS_HEALTHY: &str = "process.healthz.http_200";
    /// Integer exit code of the server process (as string: "0", "1", "None").
    pub const PROCESS_EXIT_CODE: &str = "process.exit_code";
    /// Whether the admin port accepts a TCP connection (bound = "true"/"false").
    pub const ADMIN_PORT_BOUND: &str = "process.port.admin_bound";
    /// Whether stderr output contains the expected error message.
    pub const STDERR_HAS_ERROR: &str = "process.stderr.required_message_present";
}

// ─── Imports ─────────────────────────────────────────────────────────────────
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt},
};

// ─── Port allocation ─────────────────────────────────────────────────────────

/// Bind :0 and return the assigned ephemeral port.
///
/// The port is released immediately after binding — there is a small TOCTOU
/// window before the server claims it. Acceptable for test use on loopback.
pub fn find_free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind :0 failed")
        .local_addr()
        .expect("local_addr")
        .port()
}

// ─── Binary path resolution ───────────────────────────────────────────────────

/// Resolve the path to the pre-built `embyr-server` binary.
///
/// `CARGO_BIN_EXE_embyr-server` is set by Cargo at compile time to the exact
/// path of the `[[bin]]` it just built for this test run — correct
/// regardless of `CARGO_TARGET_DIR`/profile, unlike a hand-assembled
/// `target/{debug,release}/embyr-server` guess relative to the workspace
/// root (which breaks the moment builds are redirected to a shared target
/// directory, e.g. via `~/.cargo/config.toml`'s `[build] target-dir`).
pub fn embyr_server_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_embyr-server"))
}

// ─── Postgres container helper ────────────────────────────────────────────────

/// Start a Postgres 15-alpine testcontainer and return the connection URL.
///
/// The caller must keep the returned `ContainerAsync<Postgres>` alive for
/// the duration of the server process that connects to it.
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

// ─── ServerProcess harness ────────────────────────────────────────────────────

/// Wraps a spawned `embyr-server` subprocess with its port allocation.
///
/// The child process is killed (SIGKILL) on `Drop`. For graceful-shutdown
/// tests, call `sigterm()` explicitly and then `wait_for_exit()` before the
/// struct is dropped.
///
/// `stdout`/`stderr` are drained continuously by background threads into
/// `output_buf` from spawn time — a real bug found 2026-08-31 in this
/// struct's own sibling copy (`tests/secrets_management/common/mod.rs`):
/// `Stdio::piped()` pipes have a bounded OS buffer (~64KB on Linux);
/// `RUST_LOG=debug` output from this codebase's own (large, still-growing)
/// migration set can fill it before the process ever reaches `/healthz`,
/// deadlocking the child on its own `write()` if nobody drains the pipe
/// concurrently. Not yet observed failing here, but the same latent
/// deadlock — fixed proactively rather than waiting for it to flake too.
pub struct ServerProcess {
    pub child: Child,
    pub grpc_port: u16,
    pub rest_port: u16,
    pub admin_port: u16,
    output_buf: std::sync::Arc<std::sync::Mutex<String>>,
}

/// Spawn stdout+stderr drain threads for a freshly-spawned child, taking
/// ownership of both pipes so they're never left unread. Shared by `start`
/// and `start_env_only`.
fn spawn_output_drain(child: &mut Child) -> std::sync::Arc<std::sync::Mutex<String>> {
    let output_buf = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    for pipe in [
        child.stdout.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
        child.stderr.take().map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
    ]
    .into_iter()
    .flatten()
    {
        let buf = std::sync::Arc::clone(&output_buf);
        std::thread::spawn(move || {
            use std::io::{BufRead, BufReader};
            let reader = BufReader::new(pipe);
            for line in reader.lines().map_while(Result::ok) {
                if let Ok(mut guard) = buf.lock() {
                    guard.push_str(&line);
                    guard.push('\n');
                }
            }
        });
    }
    output_buf
}

/// 64-hex-char test encryption key (32 bytes, used across tests requiring
/// `EMBYR_ENCRYPTION_KEY`). Value is arbitrary — not a real secret.
pub const TEST_ENCRYPTION_KEY: &str =
    "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20";

impl ServerProcess {
    /// Spawn `embyr-server` with the given database URL and additional env vars.
    ///
    /// Allocates three free ports for gRPC, REST, and admin; injects them as
    /// `GRPC_PORT`, `REST_PORT`, `ADMIN_PORT`. Stderr is piped for capture.
    ///
    /// # Panics when binary path not found
    /// If the binary does not exist at the resolved path, `spawn()` returns an
    /// `Err` — this test fails (RED: "binary missing" = implementation absent).
    pub fn start(db_url: &str, extra_env: &[(&str, &str)]) -> Self {
        let grpc_port = find_free_port();
        let rest_port = find_free_port();
        let admin_port = find_free_port();

        let bin = embyr_server_binary();
        let mut cmd = Command::new(&bin);
        cmd.env("DATABASE_URL", db_url)
            .env("GRPC_PORT", grpc_port.to_string())
            .env("REST_PORT", rest_port.to_string())
            .env("ADMIN_PORT", admin_port.to_string())
            .env("RUST_LOG", "info")
            // stripe-webhook-secret-required (Decision 3): this harness does
            // not clear the environment (unlike its sibling `start_env_only`),
            // so it would otherwise silently inherit CI's job-level
            // `STRIPE_SECRET_KEY` (set at job level in ci.yml for
            // card-payments-backend's own real-Stripe tests) — making every
            // one of this helper's call sites fail-fast in CI for no real
            // misconfiguration reason. Remove both; `extra_env` below can
            // still re-add either explicitly for tests that want them.
            .env_remove("STRIPE_SECRET_KEY")
            .env_remove("STRIPE_WEBHOOK_SIGNING_SECRET")
            .stderr(Stdio::piped())
            .stdout(Stdio::piped());

        for (key, val) in extra_env {
            cmd.env(key, val);
        }

        let mut child = cmd
            .spawn()
            .unwrap_or_else(|e| panic!("failed to spawn embyr-server at {bin:?}: {e}"));
        let output_buf = spawn_output_drain(&mut child);

        ServerProcess {
            child,
            grpc_port,
            rest_port,
            admin_port,
            output_buf,
        }
    }

    /// Spawn `embyr-server` with ONLY the provided env vars (no DATABASE_URL injection).
    ///
    /// Use for testing missing-required-var scenarios where `DATABASE_URL` must
    /// be absent from the environment.
    pub fn start_env_only(env_vars: &[(&str, &str)]) -> Self {
        let grpc_port = find_free_port();
        let rest_port = find_free_port();
        let admin_port = find_free_port();

        let bin = embyr_server_binary();
        let mut cmd = Command::new(&bin);
        // Clear inherited env to avoid surprising defaults from the test runner.
        cmd.env_clear()
            .env("GRPC_PORT", grpc_port.to_string())
            .env("REST_PORT", rest_port.to_string())
            .env("ADMIN_PORT", admin_port.to_string())
            .env("RUST_LOG", "info")
            .stderr(Stdio::piped())
            .stdout(Stdio::piped());

        for (key, val) in env_vars {
            cmd.env(key, val);
        }

        let mut child = cmd
            .spawn()
            .unwrap_or_else(|e| panic!("failed to spawn embyr-server at {bin:?}: {e}"));
        let output_buf = spawn_output_drain(&mut child);

        ServerProcess {
            child,
            grpc_port,
            rest_port,
            admin_port,
            output_buf,
        }
    }

    /// Poll `GET /healthz` on the admin port until HTTP 200 or timeout.
    ///
    /// Returns `true` if 200 received within `timeout`, `false` otherwise.
    pub async fn wait_for_healthy(&self, timeout: Duration) -> bool {
        let client = reqwest::Client::new();
        let url = format!("http://127.0.0.1:{}/healthz", self.admin_port);
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            match client.get(&url).send().await {
                Ok(resp) if resp.status().as_u16() == 200 => return true,
                _ => {}
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// Send SIGTERM to the child process.
    ///
    /// On Unix, uses `kill -TERM <pid>`. On non-Unix, falls back to SIGKILL
    /// (acceptable: Docker/Linux CI is the production target per D-PR-3).
    pub fn sigterm(&mut self) {
        let pid = self.child.id();
        #[cfg(unix)]
        {
            let _ = Command::new("kill")
                .args(["-TERM", &pid.to_string()])
                .status();
        }
        #[cfg(not(unix))]
        {
            // Windows: no SIGTERM — use TerminateProcess as fallback.
            let _ = self.child.kill();
        }
    }

    /// Poll `child.try_wait()` until the process exits or timeout elapses.
    ///
    /// Returns the exit code, or `None` if the process did not exit in time.
    pub async fn wait_for_exit(&mut self, timeout: Duration) -> Option<i32> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => return status.code(),
                Ok(None) => {
                    if tokio::time::Instant::now() >= deadline {
                        return None;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                Err(_) => return None,
            }
        }
    }

    /// Return the child's captured stdout+stderr (interleaved) so far. Call
    /// after the process has exited for a complete capture — the background
    /// drain threads (see `spawn_output_drain`) finish writing to the shared
    /// buffer once the pipes hit EOF at process exit.
    pub fn drain_stderr(&mut self) -> String {
        self.output_buf.lock().map(|g| g.clone()).unwrap_or_default()
    }

    /// Check whether the given TCP port currently accepts connections.
    pub fn port_is_bound(port: u16) -> bool {
        std::net::TcpStream::connect_timeout(
            &format!("127.0.0.1:{port}").parse().expect("valid addr"),
            Duration::from_millis(100),
        )
        .is_ok()
    }
}

// ─── Resident memory (RSS) measurement — stripe-webhook-body-limit AC-WBL-01 ──

/// Read `VmRSS` (resident memory, KB) for `pid` from `/proc/<pid>/status`.
///
/// Linux-only: this repo's CI and local dev flow both require a Docker
/// daemon (testcontainers Postgres), which in practice means Linux or Docker
/// Desktop's Linux VM — `/proc` is always present on the box actually running
/// `embyr-server`. No cross-platform fallback added (ponytail: native OS
/// accounting file, zero new dependency, matches this project's existing
/// `#[cfg(unix)]` precedent in `ServerProcess::sigterm`).
///
/// Returns `None` if the file can't be read (process exited) or the
/// `VmRSS:` line is missing/malformed — callers should treat `None` as "skip
/// this measurement," not as zero.
pub fn read_rss_kb(pid: u32) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    status.lines().find_map(|line| {
        line.strip_prefix("VmRSS:")
            .and_then(|rest| rest.trim().trim_end_matches("kB").trim().parse::<u64>().ok())
    })
}

// ─── Stripe webhook signature (real HMAC, no mock) ────────────────────────────

/// Compute a valid `Stripe-Signature` header value for `payload` under
/// `webhook_secret`, matching Stripe's documented scheme exactly:
/// `t=<unix_timestamp>,v1=<hex(HMAC-SHA256(secret, "{timestamp}.{payload}"))>`.
///
/// Deterministic crypto, no network call — same shape as
/// `tests/card_payments_backend/common/mod.rs::sign_stripe_payload` (not
/// imported cross-test-target since Rust integration test binaries don't
/// share code across `[[test]]` targets without a dedicated support crate;
/// duplicating this ~15-line pure function is the smaller diff).
pub fn sign_stripe_payload(payload: &str, webhook_secret: &str) -> String {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before UNIX epoch")
        .as_secs();
    let signed_payload = format!("{timestamp}.{payload}");

    let mut mac = Hmac::<Sha256>::new_from_slice(webhook_secret.as_bytes())
        .expect("HMAC accepts any key length");
    mac.update(signed_payload.as_bytes());
    let signature_hex = hex::encode(mac.finalize().into_bytes());

    format!("t={timestamp},v1={signature_hex}")
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        // Best-effort SIGKILL to avoid zombie processes. Errors ignored.
        let _ = self.child.kill();
        // Reap the child to release OS resources.
        let _ = self.child.wait();
    }
}

// ─── pool-sizing-and-limits (finding #16/#30, ADR-079) ─────────────────────────
//
// Shared helpers for pr11/pr12/pr13 — provisioning a real tenant project
// against a real `ServerProcess` (not the in-process `start_test_server_*`
// harness, which DESIGN confirms is hardcoded and does NOT read the new
// pool-sizing env vars) and driving real Firestore gRPC writes, including a
// precondition-carrying `Commit` (needed to trigger the `FOR UPDATE` row
// lock `commit_transaction`'s own OCC path already takes on
// `MustExist`/`MustNotExist`/`UpdateTime` preconditions — the deterministic
// mechanism this feature's saturation tests use to hold a real SUT pool
// connection busy without racing on query speed).

/// Fixed operator Bearer key `ServerProcess::start`/`start_env_only` are
/// always given via `EMBYR_ADMIN_KEY` in this file's own call sites.
pub const ADMIN_KEY: &str = "testkey";

/// Provision a project via the real operator-Bearer-key admin API
/// (`POST /admin/v1/projects`, mirrors `pr10_healthz_dependency_checks.rs`'s
/// own provisioning call) against a real, already-running `ServerProcess`.
/// Returns the server-generated `api_key` for that project.
pub async fn provision_project(admin_port: u16, project_id: &str, dsn: &str) -> String {
    let http = reqwest::Client::new();
    let resp = http
        .post(format!("http://127.0.0.1:{admin_port}/admin/v1/projects"))
        .header("Authorization", format!("Bearer {ADMIN_KEY}"))
        .json(&serde_json::json!({
            "project_id": project_id,
            "dsn": dsn,
            "backend_mode": "direct_pg",
        }))
        .send()
        .await
        .expect("POST /admin/v1/projects request failed");
    assert!(
        resp.status().is_success(),
        "provisioning project {project_id} must succeed; status={}",
        resp.status()
    );
    let body: serde_json::Value = resp.json().await.expect("provision response body must be JSON");
    body["api_key"]
        .as_str()
        .expect("provision response must include api_key")
        .to_string()
}

/// Lazily-connecting gRPC channel to a real subprocess's `grpc_port`.
pub fn grpc_channel(grpc_port: u16) -> tonic::transport::Channel {
    tonic::transport::Endpoint::new(format!("http://127.0.0.1:{grpc_port}"))
        .expect("valid endpoint")
        .connect_lazy()
}

/// A single string-valued Firestore document field.
pub fn string_field(value: &str) -> embyr_proto::firestore::Value {
    embyr_proto::firestore::Value {
        value_type: Some(embyr_proto::firestore::value::ValueType::StringValue(
            value.to_string(),
        )),
    }
}

/// Real gRPC `CreateDocument` call against a real subprocess's `grpc_port`.
pub async fn create_doc(
    client: &mut embyr_proto::firestore::firestore_client::FirestoreClient<tonic::transport::Channel>,
    project_id: &str,
    api_key: &str,
    collection_id: &str,
    document_id: &str,
    fields: std::collections::HashMap<String, embyr_proto::firestore::Value>,
) {
    use embyr_proto::firestore::{CreateDocumentRequest, Document};

    let mut request = tonic::Request::new(CreateDocumentRequest {
        parent: format!("projects/{project_id}/databases/(default)/documents"),
        collection_id: collection_id.to_string(),
        document_id: document_id.to_string(),
        document: Some(Document { name: String::new(), fields, ..Default::default() }),
        ..Default::default()
    });
    request
        .metadata_mut()
        .insert("authorization", format!("Bearer {api_key}").parse().expect("valid header"));

    client
        .create_document(request)
        .await
        .expect("seed document should succeed");
}

/// Real gRPC `BeginTransaction` call — `begin_transaction`'s own INSERT
/// into `transactions` (`backend_adapter.rs`) is a single, fast, acquire-
/// and-release query. Under a pool with genuine SPARE capacity it does not
/// contend with this file's own row-lock-based saturation mechanism below
/// — but when `max_connections` is small enough that every connection is
/// already checked out (AC-PSL-04's own saturated-pool scenario), this
/// INSERT's own `pool.acquire()` legitimately queues and can itself hit
/// `acquire_timeout` before `Commit` is ever attempted. Returns `Result`
/// (not a bare value) so that a saturated-pool failure here is surfaced as
/// the SAME clean, typed error `commit_update_requiring_exists`'s own
/// callers already assert on via `result.is_err()`, rather than panicking.
async fn begin_transaction(
    client: &mut embyr_proto::firestore::firestore_client::FirestoreClient<tonic::transport::Channel>,
    project_id: &str,
    api_key: &str,
) -> Result<Vec<u8>, tonic::Status> {
    use embyr_proto::firestore::BeginTransactionRequest;

    let mut request = tonic::Request::new(BeginTransactionRequest {
        database: format!("projects/{project_id}/databases/(default)"),
        options: None,
    });
    request
        .metadata_mut()
        .insert("authorization", format!("Bearer {api_key}").parse().expect("valid header"));

    Ok(client.begin_transaction(request).await?.into_inner().transaction)
}

/// Real gRPC `BeginTransaction` + `Commit` carrying ONE
/// `Write { current_document: Exists(true) }` — the exact precondition
/// shape that forces `commit_transaction`'s own `FOR UPDATE` existence
/// check (`backend_adapter.rs`, MustExist branch) on the target document
/// row. `Commit` requires a real transaction id (an empty `transaction`
/// field is rejected as `InvalidArgument` — confirmed empirically during
/// DISTILL), so this helper begins one first, mirroring
/// `security_rules_write_path/common/mod.rs::begin_transaction`+
/// `commit_writes`'s own two-call shape exactly.
///
/// A real, uncommitted `SELECT ... FOR UPDATE` held open by another session
/// against the SAME document row will block the `Commit` call (not the
/// `BeginTransaction` call) inside Postgres for as long as that lock is
/// held — legitimately checking out and holding one of the SUT's own pool
/// connections for that duration. This is the deterministic saturation
/// mechanism AC-PSL-04/06 rely on (no reliance on racing fast queries
/// against each other).
pub async fn commit_update_requiring_exists(
    client: &mut embyr_proto::firestore::firestore_client::FirestoreClient<tonic::transport::Channel>,
    project_id: &str,
    api_key: &str,
    collection_id: &str,
    document_id: &str,
    field_value: &str,
) -> Result<tonic::Response<embyr_proto::firestore::CommitResponse>, tonic::Status> {
    use embyr_proto::firestore::{
        precondition::ConditionType, write::Operation, CommitRequest, Document, Precondition,
        Write,
    };

    let transaction = begin_transaction(client, project_id, api_key).await?;

    let resource_name = format!(
        "projects/{project_id}/databases/(default)/documents/{collection_id}/{document_id}"
    );
    let mut fields = std::collections::HashMap::new();
    fields.insert("touched".to_string(), string_field(field_value));

    let write = Write {
        update_mask: None,
        update_transforms: vec![],
        current_document: Some(Precondition {
            condition_type: Some(ConditionType::Exists(true)),
        }),
        operation: Some(Operation::Update(Document {
            name: resource_name,
            fields,
            ..Default::default()
        })),
    };

    let mut request = tonic::Request::new(CommitRequest {
        database: format!("projects/{project_id}/databases/(default)"),
        writes: vec![write],
        transaction,
    });
    request
        .metadata_mut()
        .insert("authorization", format!("Bearer {api_key}").parse().expect("valid header"));

    client.commit(request).await
}

/// Open a raw, direct (bypassing the SUT entirely) transaction against
/// `dsn` and take `FOR UPDATE` on one document row, WITHOUT committing —
/// held open until the returned transaction is dropped/rolled back by the
/// caller. Mirrors the exact WHERE clause `commit_transaction`'s own OCC
/// checks use (`backend_adapter.rs`) so the lock genuinely conflicts.
pub async fn lock_document_row_for_update<'a>(
    pool: &'a sqlx::PgPool,
    project_id: &str,
    collection_path: &str,
    document_id: &str,
) -> sqlx::Transaction<'a, sqlx::Postgres> {
    let mut txn = pool.begin().await.expect("begin raw lock transaction");
    sqlx::query(
        "SELECT 1 FROM documents \
         WHERE project_id = $1 AND collection_path = $2 AND document_id = $3 AND NOT deleted \
         FOR UPDATE",
    )
    .bind(project_id)
    .bind(collection_path)
    .bind(document_id)
    .fetch_one(&mut *txn)
    .await
    .expect("row must exist to be lockable — seed it before locking");
    txn
}
