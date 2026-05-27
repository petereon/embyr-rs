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

use std::process::{Command, Stdio};
use std::time::Duration;

use super::agent_common::start_test_postgres;

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
    // Launch agent binary, capture stdout/stderr
    // Assert log line order: "connected to Postgres" < "listening on :9191"
    panic!("Not yet implemented — RED scaffold");
}

/// @driving_port @us_a06 @real_io
///
/// Feature: Agent completes in-flight work before exiting on shutdown signal
///   Given the agent is running with an in-flight document retrieval in progress
///   When  a shutdown signal is sent
///   Then  new connections are rejected; in-flight RPC completes; exit code is 0
///   And   the log contains "shutdown complete"
#[tokio::test]
#[ignore = "requires Docker + embyr-agent binary — unskip in S06A delivery"]
async fn agent_completes_in_flight_work_before_exiting_on_shutdown_signal() {
    let (_pg, db_url) = start_test_postgres().await;
    panic!("Not yet implemented — RED scaffold");
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
    // DSN sentinel: postgres://agent:DO-NOT-LOG@127.0.0.1:<port>/db
    let sentinel = "DO-NOT-LOG";

    let (_pg, base_url) = start_test_postgres().await;
    // Inject sentinel into the password portion of the DSN
    // Run agent, perform 10 RPCs, collect all log output
    // Assert: no captured line contains the sentinel string
    panic!("Not yet implemented — RED scaffold");
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
    panic!("Not yet implemented — RED scaffold");
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
