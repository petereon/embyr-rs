#![allow(dead_code)]
//! Common test infrastructure — deployment-release-process acceptance tests.
//!
//! Infrastructure policy (docs/architecture/atdd-infrastructure-policy.md):
//!   Driving port:    `embyr-server` binary subprocess — SAME mechanism already
//!                    documented for production-readiness (row: "`embyr-server`
//!                    binary (subprocess)"). No new port class introduced by
//!                    this feature; reused verbatim.
//!   Driven internal: testcontainers-rs Postgres 15-alpine, fresh per test.
//!   Driven external: none in scope.
//!
//! `ServerProcess` is a trimmed duplicate of
//! `tests/production_readiness/common/mod.rs::ServerProcess` — Rust
//! integration test binaries don't share code across separate `[[test]]`
//! targets without a dedicated support crate (established precedent in this
//! repo; see that file's own `sign_stripe_payload` doc comment for the same
//! reasoning). Only the subset needed for a single startup-log capture test
//! is duplicated here — no pool-sizing/gRPC/Stripe helpers, this feature
//! needs none of them.

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt},
};

/// 64-hex-char test encryption key — same known-safe fixture value used in
/// `.github/workflows/ci.yml` and reused (per DESIGN) in `docker-compose.yml`
/// itself. Value is arbitrary — not a real secret.
pub const TEST_ENCRYPTION_KEY: &str =
    "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20";

/// Bind :0 and return the assigned ephemeral port.
pub fn find_free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind :0 failed")
        .local_addr()
        .expect("local_addr")
        .port()
}

/// Resolve the path to the pre-built `embyr-server` binary (set by Cargo at
/// compile time for this `[[test]]` target's own crate).
pub fn embyr_server_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_embyr-server"))
}

/// Absolute path to the repository root (two levels up from this crate's
/// `CARGO_MANIFEST_DIR`, same derivation as `pr02_dockerfile.rs::workspace_root`).
pub fn workspace_root() -> PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    std::path::Path::new(manifest_dir)
        .parent()
        .expect("crates dir")
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

/// Start a Postgres 15-alpine testcontainer and return the connection URL.
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

/// Wraps a spawned `embyr-server` subprocess. Stdout+stderr are drained
/// continuously into `output_buf` from spawn time (same deadlock-avoidance
/// rationale as the production-readiness sibling: an unread `Stdio::piped()`
/// pipe can fill its OS buffer and block the child's own `write()`).
pub struct ServerProcess {
    pub child: Child,
    pub admin_port: u16,
    output_buf: std::sync::Arc<std::sync::Mutex<String>>,
}

fn spawn_output_drain(child: &mut Child) -> std::sync::Arc<std::sync::Mutex<String>> {
    let output_buf = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    for pipe in [
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
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

impl ServerProcess {
    /// Spawn `embyr-server` with the given database URL and additional env vars.
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
            admin_port,
            output_buf,
        }
    }

    /// Poll `GET /healthz` on the admin port until HTTP 200 or timeout.
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

    /// Send SIGTERM to the child process (Unix) so it drains gracefully.
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
            let _ = self.child.kill();
        }
    }

    /// Poll `child.try_wait()` until the process exits or timeout elapses.
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
    /// after the process has exited for a complete capture.
    pub fn drain_output(&mut self) -> String {
        self.output_buf
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
