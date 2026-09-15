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
//! Assertion style: plain `assert!`/descriptive panic messages, matching the
//! established precedent for this exact layer (subprocess/FS acceptance) in
//! this repo — `tests/production_readiness/acceptance/pr01_config_from_env.rs`
//! — rather than `assert_state_delta`/Universe. Chosen for consistency with
//! the already-reviewed sibling test at the identical layer, not as a
//! deviation from Mandate 8.
//!
//! Scaffold classification target: RED until `main.rs` Step 12 is extended
//! with the `version` field (DELIVER, per DESIGN's exact diff in
//! feature-delta.md § US-DRP-01 Design, item 4).

use std::time::Duration;

use crate::common::{start_postgres_container, ServerProcess, TEST_ENCRYPTION_KEY};

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

    assert!(
        output.contains(&format!("version={expected_version}"))
            || output.contains(&format!("version=\"{expected_version}\"")),
        "startup log must report the running version ({expected_version}) as a \
         structured `version` field; captured output:\n{output}"
    );
    assert!(
        output.contains(&format!("v{expected_version}")),
        "startup log's human-readable message must name the version as \
         v{expected_version} (matching the vX.Y.Z git tag format); captured output:\n{output}"
    );
    // _pg dropped here — container stopped after server exits.
}
