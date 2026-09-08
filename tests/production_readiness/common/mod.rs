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

impl Drop for ServerProcess {
    fn drop(&mut self) {
        // Best-effort SIGKILL to avoid zombie processes. Errors ignored.
        let _ = self.child.kill();
        // Reap the child to release OS resources.
        let _ = self.child.wait();
    }
}
