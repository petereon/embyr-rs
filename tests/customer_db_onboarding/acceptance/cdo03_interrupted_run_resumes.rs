// @driving_port @real-io @US-01 @AC-01-03
//! Re-running preparation after an interrupted partial run resumes safely.
//!
//! Journey (chained — Given simulates the same "interrupted" precondition
//! DISCUSS's domain example 2 describes: only the first of two migrations
//! committed before interruption):
//!   Given: a previous run applied only the first of two expected
//!          migrations before being interrupted (VPN drop, network blip).
//!   When:  Elena re-runs embyr-db-prep.
//!   Then:  only the remaining migration is applied, no duplicate-object
//!          error occurs, and Elena sees the same "ready" confirmation as a
//!          clean first-time run.
//!
//! Fixture note: the "interrupted" precondition is produced via sqlx's own
//! `Migrate` trait (`ensure_migrations_table()` + `apply()`), applying
//! exactly migration `0001` with sqlx's real bookkeeping/checksum — not a
//! hand-rolled simulation — so the resume this test exercises is a resume
//! against genuine sqlx migration state.
//!
//! AC-01-03.

#[path = "../common/mod.rs"]
mod common;
use common::{
    assert_state_delta, create_ddl_role, role_connection_url, run_db_prep, set_to,
    start_postgres_container,
};
use sqlx::migrate::Migrate;
use std::collections::HashMap;

// Fixture note (fixed 2026-08-16, orchestrator pass after step 01-01):
// the "interrupted" precondition must be created under `elena_dba`'s own
// connection, not the testcontainer superuser, so `_sqlx_migrations` is
// owned by the same role that resumes the run -- matching ADR-023's
// assumption that Elena's elevated role owns the tracking table by virtue
// of having created it.
#[tokio::test]
async fn rerunning_preparation_after_an_interrupted_partial_run_resumes_safely() {
    let (_pg, db_url) = start_postgres_container().await;
    let sys_pool = sqlx::PgPool::connect(&db_url).await.unwrap();
    create_ddl_role(&sys_pool, "elena_dba", "postgres").await;
    drop(sys_pool);
    let elena_dsn = role_connection_url(&db_url, "elena_dba");

    // Given: only migration 0001 applied (interrupted before 0002 started).
    let migrator = sqlx::migrate!("../../migrations/customer");
    let mut conn = sqlx::PgConnection::connect(&elena_dsn)
        .await
        .expect("connect as elena_dba for fixture setup");
    use sqlx::Connection;
    conn.ensure_migrations_table()
        .await
        .expect("ensure_migrations_table");
    let first_migration = migrator
        .iter()
        .next()
        .expect("migrations/customer/ has at least one migration");
    conn.apply(first_migration)
        .await
        .expect("apply first migration only (simulated interruption)");
    drop(conn);

    let applied_before = 1i64;
    let mut before: HashMap<&str, String> = HashMap::new();
    before.insert("migrations.applied_count", applied_before.to_string());

    // When: Elena re-runs the tool.
    let run = run_db_prep(&[("EMBYR_DB_PREP_DSN", &elena_dsn)]);

    let verify_pool = sqlx::PgPool::connect(&db_url).await.unwrap();
    let applied_after: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM _sqlx_migrations WHERE success",
    )
    .fetch_one(&verify_pool)
    .await
    .expect("query _sqlx_migrations after resume");

    let mut after: HashMap<&str, String> = HashMap::new();
    after.insert("migrations.applied_count", applied_after.to_string());
    after.insert(
        "prep_process.exit_code",
        run.exit_code
            .map(|c| c.to_string())
            .unwrap_or_else(|| "None".into()),
    );

    let universe = &["migrations.applied_count", "prep_process.exit_code"];
    let mut expected = HashMap::new();
    expected.insert("migrations.applied_count", set_to("2".to_string()));
    expected.insert("prep_process.exit_code", set_to("0".to_string()));

    assert_state_delta(&before, &after, universe, &expected);

    assert!(
        !run.stderr.to_lowercase().contains("duplicate"),
        "AC-01-03: resume must not surface a duplicate-object error; stderr: {}",
        run.stderr
    );
    assert!(
        run.stdout.to_lowercase().contains("ready"),
        "AC-01-03: resume must report the same 'ready' confirmation as a \
         clean first-time run; got: {}",
        run.stdout
    );
}
