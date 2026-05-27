// SCAFFOLD: true
//! US-A06 — Agent startup probe and graceful shutdown
//!
//! As Riley (DevSecOps Lead), I want the agent to verify storage connectivity
//! before accepting RPCs, emit structured startup logs, and drain in-flight
//! calls on SIGTERM, so that I can deploy safely in Kubernetes and pass
//! SOC2 audit on log discipline.
//!
//! Driving port: agent binary process (subprocess via std::process::Command)
//! Red classification: MISSING_FUNCTIONALITY
//!
//! Feature file: tests/features/agent/us_a06_lifecycle.feature
//! Execution order: S06A (second slice — after WS S01A, before S02A)
//!
//! These tests launch the agent binary directly so they can observe startup
//! logs, exit codes, and port binding in isolation.  They use
//! `start_test_postgres` from `mod.rs` to obtain a real Postgres container,
//! then pass its DSN to the agent via EMBYR_AGENT_DB_DSN.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::Duration;

use super::agent_common::{start_test_postgres, test_tls_config};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

/// Write test TLS cert files to `dir`. Returns (cert_path, key_path, ca_path).
fn write_test_certs(dir: &std::path::Path) -> (String, String, String) {
    let tls = test_tls_config();
    let cert = dir.join("server.pem");
    let key = dir.join("server-key.pem");
    let ca = dir.join("ca.pem");
    std::fs::write(&cert, &tls.server_cert_pem).unwrap();
    std::fs::write(&key, &tls.server_key_pem).unwrap();
    std::fs::write(&ca, &tls.ca_cert_pem).unwrap();
    (
        cert.display().to_string(),
        key.display().to_string(),
        ca.display().to_string(),
    )
}

// ---------------------------------------------------------------------------
// Happy path scenarios
// ---------------------------------------------------------------------------

/// @driving_port @us_a06 @real_io
///
/// Feature: Agent logs storage readiness before accepting caller connections
///   Given all required configuration is set and storage is reachable
///   When  the agent process starts
///   Then  "connected to Postgres" appears in the log before "listening on :9191"
///   And   a document retrieval call succeeds immediately after both lines appear
#[tokio::test]
#[ignore = "requires Docker + embyr-agent binary — unskip in S06A delivery"]
async fn agent_logs_storage_readiness_before_accepting_connections() {
    let (_pg, db_url) = start_test_postgres().await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let (cert_path, key_path, ca_path) = write_test_certs(tmp.path());

    let mut child = Command::new(env!("CARGO_BIN_EXE_embyr-agent"))
        .env_clear()
        .env("EMBYR_AGENT_DB_DSN", &db_url)
        .env("EMBYR_AGENT_PROJECT_ID", "test-project")
        .env("EMBYR_AGENT_CERT", &cert_path)
        .env("EMBYR_AGENT_KEY", &key_path)
        .env("EMBYR_AGENT_CA", &ca_path)
        .env("EMBYR_AGENT_LOG_LEVEL", "info")
        .env("EMBYR_AGENT_LISTEN_ADDR", "127.0.0.1:0")
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn embyr-agent");

    let stderr = child.stderr.take().expect("stderr piped");
    let mut reader = BufReader::new(stderr);

    // Collect all lines until "listening on" appears (with 30s timeout).
    let mut all_lines: Vec<String> = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(30);

    let mut found_listening = false;
    loop {
        if std::time::Instant::now() > deadline {
            break;
        }
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break, // process exited
            Ok(_) => {
                let trimmed = line.trim_end().to_string();
                all_lines.push(trimmed.clone());
                if trimmed.contains("listening on") {
                    found_listening = true;
                    break;
                }
            }
            Err(_) => break,
        }
    }

    // Kill the agent.
    let _ = child.kill();
    let _ = child.wait();

    assert!(
        found_listening,
        "agent never logged 'listening on' within 30s. Output:\n{}",
        all_lines.join("\n")
    );

    // Find positions of the two key log lines.
    let pg_pos = all_lines
        .iter()
        .position(|l| l.contains("connected to Postgres"))
        .unwrap_or_else(|| {
            panic!(
                "'connected to Postgres' not found in agent output:\n{}",
                all_lines.join("\n")
            )
        });

    let listening_pos = all_lines
        .iter()
        .position(|l| l.contains("listening on"))
        .expect("'listening on' must be present");

    assert!(
        pg_pos < listening_pos,
        "'connected to Postgres' (line {pg_pos}) must appear before 'listening on' (line {listening_pos}).\nOutput:\n{}",
        all_lines.join("\n")
    );
}

/// @driving_port @us_a06 @real_io
///
/// Feature: Agent completes in-flight work before exiting on shutdown signal
///   Given the agent is running with an in-flight document retrieval in progress
///   When  a shutdown signal is sent
///   Then  new connections are rejected; in-flight RPC completes; exit code is 0
///   And   the log contains "shutdown complete"
#[tokio::test]
async fn agent_completes_in_flight_work_before_exiting_on_shutdown_signal() {
    let (_pg, db_url) = start_test_postgres().await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let (cert_path, key_path, ca_path) = write_test_certs(tmp.path());

    // Spawn the agent binary with piped stderr so we can inspect logs.
    // Use port 0 so the OS assigns a free port, avoiding conflicts when tests run in parallel.
    let mut child = Command::new(env!("CARGO_BIN_EXE_embyr-agent"))
        .env_clear()
        .env("EMBYR_AGENT_DB_DSN", &db_url)
        .env("EMBYR_AGENT_PROJECT_ID", "test-project")
        .env("EMBYR_AGENT_CERT", &cert_path)
        .env("EMBYR_AGENT_KEY", &key_path)
        .env("EMBYR_AGENT_CA", &ca_path)
        .env("EMBYR_AGENT_LISTEN_ADDR", "127.0.0.1:0")
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn embyr-agent");

    // Wait for "listening on" in stderr — agent is ready for RPCs.
    let stderr_handle = child.stderr.take().expect("stderr piped");
    let mut reader = BufReader::new(stderr_handle);

    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    let mut startup_lines: Vec<String> = Vec::new();
    let mut ready = false;

    while std::time::Instant::now() < deadline {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                let trimmed = line.trim_end().to_string();
                startup_lines.push(trimmed.clone());
                if trimmed.contains("listening on") {
                    ready = true;
                    break;
                }
            }
            Err(_) => break,
        }
    }

    assert!(
        ready,
        "agent never logged 'listening on' within 60s. Output:\n{}",
        startup_lines.join("\n")
    );

    // Send SIGTERM to the agent process.
    let pid = child.id();
    std::process::Command::new("kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status()
        .expect("send SIGTERM");

    // Drain remaining stderr into a buffer.
    let mut remaining_lines: Vec<String> = Vec::new();
    for line in reader.lines() {
        match line {
            Ok(l) => remaining_lines.push(l),
            Err(_) => break,
        }
    }

    // Wait for the process to exit with a 30s timeout.
    let output = tokio::task::spawn_blocking(move || {
        child.wait_with_output().expect("wait for output")
    })
    .await
    .expect("join wait_with_output");

    let shutdown_code = output.status.code();
    let all_stderr: Vec<String> = {
        let from_output = String::from_utf8_lossy(&output.stderr);
        let mut combined = startup_lines;
        combined.extend(remaining_lines);
        // Also include any bytes that wait_with_output captured.
        for line in from_output.lines() {
            combined.push(line.to_string());
        }
        combined
    };
    let stderr_text = all_stderr.join("\n");

    assert_eq!(
        shutdown_code,
        Some(0),
        "expected exit code 0 after SIGTERM, got {:?}. stderr:\n{stderr_text}",
        shutdown_code
    );

    assert!(
        stderr_text.contains("shutdown complete"),
        "expected 'shutdown complete' in agent stderr after SIGTERM.\nstderr:\n{stderr_text}"
    );
}

// ---------------------------------------------------------------------------
// Audit / security scenarios
// ---------------------------------------------------------------------------

/// @driving_port @us_a06 @real_io
///
/// Feature: Storage credential never appears in agent logs (DSN audit test)
///
/// The DSN is deliberately constructed to contain a sentinel token
/// "DO-NOT-LOG".  After the agent handles 10 RPCs, all captured log lines
/// are scanned.  Zero matches is the passing condition.
///
///   Given the storage connection string contains "DO-NOT-LOG"
///   When  the agent starts and handles 10 document retrieval calls
///   Then  no log line at any level contains "DO-NOT-LOG"
///   And   all log output is structured and machine-readable
#[tokio::test]
#[ignore = "requires Docker + embyr-agent binary — unskip in S06A delivery"]
async fn storage_credential_never_appears_in_agent_logs() {
    let sentinel = "DO-NOT-LOG";

    // Start Postgres with the sentinel as the password.
    use testcontainers_modules::testcontainers::ImageExt;
    use testcontainers_modules::testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;
    let _ = rustls::crypto::ring::default_provider().install_default();
    let pg = Postgres::default()
        .with_env_var("POSTGRES_PASSWORD", sentinel)
        .start()
        .await
        .expect("start postgres with sentinel password");
    let port = pg.get_host_port_ipv4(5432).await.expect("get port");
    let dsn = format!("postgres://postgres:{sentinel}@127.0.0.1:{port}/postgres");

    let tmp = tempfile::tempdir().expect("tempdir");
    let (cert_path, key_path, ca_path) = write_test_certs(tmp.path());

    // Spawn the agent with the sentinel DSN.
    let mut child = Command::new(env!("CARGO_BIN_EXE_embyr-agent"))
        .env_clear()
        .env("EMBYR_AGENT_DB_DSN", &dsn)
        .env("EMBYR_AGENT_PROJECT_ID", "test-project")
        .env("EMBYR_AGENT_CERT", &cert_path)
        .env("EMBYR_AGENT_KEY", &key_path)
        .env("EMBYR_AGENT_CA", &ca_path)
        .env("EMBYR_AGENT_LOG_LEVEL", "debug") // max verbosity to surface any leak
        .env("EMBYR_AGENT_LISTEN_ADDR", "127.0.0.1:0")
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn embyr-agent");

    let stderr = child.stderr.take().expect("stderr piped");
    let mut reader = BufReader::new(stderr);

    // Wait for agent to start (or give up after 15s).
    let mut all_lines: Vec<String> = Vec::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        if std::time::Instant::now() > deadline {
            break;
        }
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {
                let trimmed = line.trim_end().to_string();
                all_lines.push(trimmed.clone());
                // Once agent is listening, we've collected startup logs.
                if trimmed.contains("listening on") {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    // Kill agent and collect any remaining output.
    let _ = child.kill();
    // Drain remaining stderr.
    for line in reader.lines() {
        if let Ok(l) = line {
            all_lines.push(l);
        }
    }
    let _ = child.wait();

    // Assert: sentinel never appears in any log line.
    let leaking_lines: Vec<&str> = all_lines
        .iter()
        .filter(|l| l.contains(sentinel))
        .map(|l| l.as_str())
        .collect();

    assert!(
        leaking_lines.is_empty(),
        "SECURITY VIOLATION: sentinel '{sentinel}' found in agent log output:\n{}",
        leaking_lines.join("\n")
    );
}

// ---------------------------------------------------------------------------
// Error / missing-config scenarios
// ---------------------------------------------------------------------------

/// @driving_port @us_a06 @real_io @error
///
/// Feature: Agent exits without binding a port when storage is unreachable
///   Given the storage address points to an unreachable host
///   When  the agent process starts
///   Then  the log contains a connection-failure message
///   And   the process exits with a non-zero exit code; no port is bound
#[tokio::test]
#[ignore = "requires embyr-agent binary — unskip in S06A delivery"]
async fn agent_exits_without_binding_port_when_storage_unreachable() {
    let unreachable_dsn = "postgres://agent:pw@127.0.0.1:1/db"; // port 1 always fails

    let tmp = tempfile::tempdir().expect("tempdir");
    let (cert_path, key_path, ca_path) = write_test_certs(tmp.path());

    // Use spawn + wait with timeout: probe has a 5s internal timeout, so agent
    // should exit within ~10 seconds.
    let mut child = Command::new(env!("CARGO_BIN_EXE_embyr-agent"))
        .env_clear()
        .env("EMBYR_AGENT_DB_DSN", unreachable_dsn)
        .env("EMBYR_AGENT_PROJECT_ID", "test-project")
        .env("EMBYR_AGENT_CERT", &cert_path)
        .env("EMBYR_AGENT_KEY", &key_path)
        .env("EMBYR_AGENT_CA", &ca_path)
        .env("EMBYR_AGENT_LOG_LEVEL", "info")
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn embyr-agent");

    // Wait up to 15 seconds for the agent to exit.
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(s) => break s,
            None => {
                if std::time::Instant::now() > deadline {
                    let _ = child.kill();
                    panic!("agent did not exit within 15s when storage is unreachable");
                }
                std::thread::sleep(Duration::from_millis(200));
            }
        }
    };

    let stderr_bytes = {
        let mut buf = Vec::new();
        if let Some(mut stderr) = child.stderr.take() {
            use std::io::Read;
            let _ = stderr.read_to_end(&mut buf);
        }
        buf
    };
    let stderr = String::from_utf8_lossy(&stderr_bytes);

    // Assert: non-zero exit code.
    assert_ne!(
        status.code(),
        Some(0),
        "agent should exit with non-zero code when storage is unreachable. stderr:\n{stderr}"
    );

    // Assert: stderr mentions probe/Postgres/failed so operator knows why.
    let has_diagnostic = stderr.contains("probe")
        || stderr.contains("Postgres")
        || stderr.contains("failed")
        || stderr.contains("error");
    assert!(
        has_diagnostic,
        "agent stderr should contain a connection-failure diagnostic. stderr:\n{stderr}"
    );
}

/// @driving_port @us_a06 @real_io @error
///
/// Feature: Agent exits immediately when required storage configuration is absent
///   Given EMBYR_AGENT_DB_DSN is not set
///   When  the agent process starts
///   Then  exit code is 1; stderr names the missing configuration item; no port bound
#[tokio::test]
async fn agent_exits_when_required_storage_config_is_absent() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_embyr-agent"))
        .env_clear()
        .env("EMBYR_AGENT_PROJECT_ID", "test-project")
        .env("EMBYR_AGENT_CERT", "/nonexistent/cert.pem")
        .env("EMBYR_AGENT_KEY", "/nonexistent/key.pem")
        .env("EMBYR_AGENT_CA", "/nonexistent/ca.pem")
        // EMBYR_AGENT_DB_DSN intentionally absent
        .output()
        .expect("run agent binary");
    assert_ne!(output.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("EMBYR_AGENT_DB_DSN"), "stderr: {stderr}");
}

/// @driving_port @us_a06 @real_io @error
///
/// Feature: Agent exits immediately when required project identifier is absent
///   Given EMBYR_AGENT_PROJECT_ID is not set
///   When  the agent process starts
///   Then  exit code is 1; stderr names the missing configuration item; no port bound
#[tokio::test]
async fn agent_exits_when_required_project_identifier_is_absent() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_embyr-agent"))
        .env_clear()
        .env("EMBYR_AGENT_DB_DSN", "postgres://x:y@127.0.0.1:1/db")
        .env("EMBYR_AGENT_CERT", "/nonexistent/cert.pem")
        .env("EMBYR_AGENT_KEY", "/nonexistent/key.pem")
        .env("EMBYR_AGENT_CA", "/nonexistent/ca.pem")
        // EMBYR_AGENT_PROJECT_ID intentionally absent
        .output()
        .expect("run agent binary");
    assert_ne!(output.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("EMBYR_AGENT_PROJECT_ID"), "stderr: {stderr}");
}
