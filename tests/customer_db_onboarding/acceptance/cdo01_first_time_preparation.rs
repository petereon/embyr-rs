// @walking_skeleton @driving_port @real-io @US-01
//! US-01 walking skeleton — Elena preps a fresh database and sees the
//! applied schema version.
//!
//! Journey (DISCUSS domain example 1 / UAT "First-time preparation succeeds
//! and reports the applied schema version"):
//!   Given: Elena has a fresh, empty database and is logged in as a role
//!          with sufficient (CREATE) privilege on it.
//!   When:  Elena runs embyr-db-prep against it.
//!   Then:  all expected migrations are applied and Elena sees confirmation
//!          naming the schema version applied; the process exits 0.
//!
//! Walking skeleton per DISCUSS's WS pairing (prep -> verify -> provision
//! succeeds): this is the "prep" half. cdo12 is the "verify -> provision"
//! half, using a database this scenario's own tool run has already prepped.
//!
//! Driving port: embyr-db-prep subprocess (CARGO_BIN_EXE_embyr-db-prep).
//! AC-01-01.
//!
//! NOT #[ignore] — walking skeleton, RED via config::DbPrepConfig::from_env()
//! panicking. All other cdo0N tests in this crate are #[ignore] — DELIVER
//! unskips them one at a time.

#[path = "../common/mod.rs"]
mod common;
use common::{create_ddl_role, role_connection_url, run_db_prep, start_postgres_container};

#[tokio::test]
async fn elena_preps_a_fresh_database_and_sees_the_applied_schema_version() {
    let (_pg, db_url) = start_postgres_container().await;

    let sys_pool = sqlx::PgPool::connect(&db_url)
        .await
        .expect("connect as superuser to create elena_dba role");
    create_ddl_role(&sys_pool, "elena_dba", "postgres").await;
    drop(sys_pool);

    let elena_dsn = role_connection_url(&db_url, "elena_dba");

    let run = run_db_prep(&[("EMBYR_DB_PREP_DSN", &elena_dsn)]);

    assert_eq!(
        run.exit_code,
        Some(0),
        "AC-01-01: successful preparation must exit 0; stderr: {}",
        run.stderr
    );
    assert!(
        run.stdout.to_lowercase().contains("ready") && run.stdout.contains('2'),
        "AC-01-01: stdout must confirm readiness and name the schema version \
         applied (expected 2 of 2 migrations); got: {}",
        run.stdout
    );

    // Real I/O confirmation the migration set was actually applied — not
    // just that the process claimed success.
    let verify_pool = sqlx::PgPool::connect(&db_url)
        .await
        .expect("connect to verify applied migrations");
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name FROM information_schema.tables \
         WHERE table_schema='public' ORDER BY table_name",
    )
    .fetch_all(&verify_pool)
    .await
    .expect("query information_schema.tables");
    assert!(
        tables.contains(&"documents".to_string()),
        "tables: {tables:?}"
    );
    assert!(
        tables.contains(&"transactions".to_string()),
        "tables: {tables:?}"
    );
}
