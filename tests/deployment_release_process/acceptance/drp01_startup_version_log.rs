// @walking_skeleton @driving_port @real-io @US-DRP-01
//! US-DRP-01 — embyr-server's startup log names its own version.
//!
//! UAT scenario (feature-delta.md, US-DRP-01):
//!   "Sam identifies exactly which release is running"
//!     Given embyr-server was built and started from the commit tagged v0.1.1
//!     When Sam reads the server's startup log
//!     Then the log line names the version "v0.1.1"
//!
//! Acceptance criteria verified here (US-DRP-01 AC):
//!   `embyr-server`'s startup log line includes its own version, sourced
//!   from `CARGO_PKG_VERSION`.
//!
//! Driving port: `embyr-server` binary subprocess (same mechanism already
//! documented in docs/architecture/atdd-infrastructure-policy.md for
//! production-readiness — reused verbatim, no new port).
//!
//! Walking skeleton for US-DRP-01: NOT #[ignore]. Proves the version-in-log
//! mechanism end-to-end against a real running server + real Postgres.
//!
//! Assertion style: real `serde_json` parsing of each captured log line, not
//! substring matching — structured-json-logging (finding #25) switched the
//! formatter to `.json()`, so `output.contains("version=...")`-style checks
//! no longer match the `"version":"0.1.1"` colon-shape JSON renders. See
//! `docs/feature/structured-json-logging/feature-delta.md` "Self-review"
//! section, which flagged this exact test as at-risk.
//!
//! Scaffold classification target: RED until `main.rs` Step 12 is extended
//! with the `version` field (DELIVER, per DESIGN's exact diff in
//! feature-delta.md § US-DRP-01 Design, item 4).

use std::time::Duration;

use crate::common::{start_postgres_container, ServerProcess, TEST_ENCRYPTION_KEY};

/// Parse every non-empty captured line as JSON. Fails with the offending
/// line if any line is not valid JSON — proves the log stream is genuinely
/// machine-parseable (structured-json-logging, finding #25), not merely
/// "looks structured" under a substring check.
fn parse_json_lines(output: &str) -> Vec<serde_json::Value> {
    output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line)
                .unwrap_or_else(|e| panic!("log line is not valid JSON ({e}): {line}"))
        })
        .collect()
}

#[tokio::test]
async fn startup_log_names_the_running_version() {
    let (_pg, db_url) = start_postgres_container().await;

    let mut server = ServerProcess::start(
        &db_url,
        &[
            ("EMBYR_ADMIN_KEY", "testkey"),
            ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
        ],
    );

    let healthy = server.wait_for_healthy(Duration::from_secs(30)).await;
    assert!(
        healthy,
        "server did not become healthy within 30s — cannot inspect its startup log"
    );

    server.sigterm();
    server.wait_for_exit(Duration::from_secs(15)).await;
    let output = server.drain_output();

    // embyr-server and this test binary share the same workspace version
    // (`version.workspace = true`), so CARGO_PKG_VERSION here is exactly
    // the value the running server should have reported.
    let expected_version = env!("CARGO_PKG_VERSION");

    // Every line the process emitted must be genuine, parseable JSON —
    // proves the whole startup stream (not just the "ready" line) is
    // structured, including the TLS-disabled warning emitted earlier in the
    // same run (no EMBYR_TLS_CERT_PATH/EMBYR_TLS_KEY_PATH set above).
    // `tracing_subscriber::fmt().json()` nests every `tracing::info!`/`warn!`
    // key-value pair (including `message`) under a top-level `"fields"`
    // object — confirmed against real captured output, not assumed.
    let lines = parse_json_lines(&output);
    let field = |v: &serde_json::Value, name: &str| v["fields"].get(name).cloned();

    let ready_line = lines
        .iter()
        .find(|v| field(v, "version").is_some())
        .unwrap_or_else(|| panic!("no JSON log line carried a `version` field; captured output:\n{output}"));
    assert_eq!(
        field(ready_line, "version").as_ref().and_then(|v| v.as_str()),
        Some(expected_version),
        "startup log's `version` JSON field must equal {expected_version}; line: {ready_line}"
    );
    assert_eq!(
        field(ready_line, "message").as_ref().and_then(|v| v.as_str()),
        Some(format!("embyr-server v{expected_version} ready").as_str()),
        "startup log's human-readable message must name the version as \
         v{expected_version} (matching the vX.Y.Z git tag format); line: {ready_line}"
    );

    let tls_warning_line = lines
        .iter()
        .find(|v| field(v, "tls_enabled").and_then(|b| b.as_bool()) == Some(false))
        .unwrap_or_else(|| panic!("no JSON log line carried tls_enabled=false; captured output:\n{output}"));
    assert_eq!(
        field(tls_warning_line, "vars").as_ref().and_then(|v| v.as_str()),
        Some("EMBYR_TLS_CERT_PATH, EMBYR_TLS_KEY_PATH"),
        "TLS-disabled warning's `vars` JSON field must name both env vars; line: {tls_warning_line}"
    );
    // _pg dropped here — container stopped after server exits.
}
