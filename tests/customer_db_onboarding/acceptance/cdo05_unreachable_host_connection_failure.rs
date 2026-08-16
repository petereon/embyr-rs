// @error @driving_port @real-io @US-01 @AC-01-05
//! An unreachable database reports a connection failure, distinct from a
//! privilege failure.
//!
//! Journey:
//!   Given: the target Postgres host is unreachable from where Elena is
//!          running the tool.
//!   When:  Elena runs embyr-db-prep.
//!   Then:  Elena sees a connection-failure message distinguishable from an
//!          insufficient-privilege message, and the tool exits non-zero.
//!
//! This is the "hard failure — distinct 'connection failed' message, exit 1,
//! migrate() never attempted" fault-injection scenario named in
//! `docs/product/architecture/brief.md` § Driven Ports + Adapters (Earned
//! Trust probe design table, fault-injection scenario 1).
//!
//! AC-01-05.

#[path = "../common/mod.rs"]
mod common;
use common::run_db_prep;

#[tokio::test]
#[ignore]
async fn unreachable_host_reports_a_connection_failure_distinct_from_privilege_failure() {
    // Given: an unreachable host (unroutable TEST-NET-1 address, RFC 5737,
    // guaranteed not to accept connections; port bound to nothing).
    let unreachable_dsn = "postgres://elena_dba:elena_dba@192.0.2.1:5432/postgres";

    // When: Elena runs the prep step.
    let run = run_db_prep(&[("EMBYR_DB_PREP_DSN", unreachable_dsn)]);

    // Then: non-zero exit; message names connection failure, not privilege.
    assert_ne!(
        run.exit_code,
        Some(0),
        "AC-01-05: unreachable-host run must exit non-zero"
    );
    let combined = format!("{}{}", run.stdout, run.stderr).to_lowercase();
    assert!(
        combined.contains("connect") || combined.contains("unreachable") || combined.contains("timed out"),
        "AC-01-05: message must name a connection failure; got stdout={:?} stderr={:?}",
        run.stdout,
        run.stderr
    );
    assert!(
        !combined.contains("privilege") && !combined.contains("permission denied"),
        "AC-01-05: connection-failure message must be distinguishable from an \
         insufficient-privilege message; got stdout={:?} stderr={:?}",
        run.stdout,
        run.stderr
    );
}
