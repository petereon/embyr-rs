#![allow(dead_code)]
//! Common test infrastructure — tls-startup-warning acceptance test.
//!
//! Trimmed duplicate of `tests/deployment_release_process/common/mod.rs::
//! ServerProcess` (same "no cross-`[[test]]`-target sharing" precedent).
//! No Postgres testcontainer: the startup warning fires in `main()` Step 2,
//! before DB connect (Step 4) — pointing `DATABASE_URL` at a closed local
//! port makes the process fail fast at Step 4 after Step 2 has already
//! written to stderr, so this test never needs real Postgres.

use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

pub const TEST_ENCRYPTION_KEY: &str =
    "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20";

/// Bind :0 and return the assigned ephemeral port (used both as an
/// unreachable DB port and free listener ports).
pub fn find_free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind :0 failed")
        .local_addr()
        .expect("local_addr")
        .port()
}

pub fn embyr_server_binary() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_embyr-server"))
}

pub struct ServerProcess {
    child: Child,
    output_buf: std::sync::Arc<std::sync::Mutex<String>>,
}

impl ServerProcess {
    /// Spawn `embyr-server` pointed at an unreachable DB (closed local
    /// port) plus `extra_env`. The process always exits non-zero shortly
    /// after Step 4's connect attempt fails — Step 2's warning (or its
    /// absence) is already captured in stderr by then.
    pub fn start(extra_env: &[(&str, &str)]) -> Self {
        let dead_db_port = find_free_port();
        let db_url = format!("postgres://postgres:postgres@127.0.0.1:{dead_db_port}/postgres");

        let bin = embyr_server_binary();
        let mut cmd = Command::new(&bin);
        cmd.env("DATABASE_URL", db_url)
            .env("GRPC_PORT", find_free_port().to_string())
            .env("REST_PORT", find_free_port().to_string())
            .env("ADMIN_PORT", find_free_port().to_string())
            .env("EMBYR_ADMIN_KEY", "testkey")
            .env("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY)
            .env("RUST_LOG", "info")
            .env_remove("STRIPE_SECRET_KEY")
            .env_remove("STRIPE_WEBHOOK_SIGNING_SECRET")
            .env_remove("EMBYR_TLS_CERT_PATH")
            .env_remove("EMBYR_TLS_KEY_PATH")
            .stderr(Stdio::piped())
            .stdout(Stdio::piped());

        for (key, val) in extra_env {
            cmd.env(key, val);
        }

        let mut child = cmd
            .spawn()
            .unwrap_or_else(|e| panic!("failed to spawn embyr-server at {bin:?}: {e}"));

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

        ServerProcess { child, output_buf }
    }

    /// Poll `try_wait()` until the process exits or the timeout elapses,
    /// then return the captured stdout+stderr (interleaved).
    pub async fn wait_for_exit_and_drain(&mut self, timeout: Duration) -> String {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {
                    if tokio::time::Instant::now() >= deadline {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(_) => break,
            }
        }
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
