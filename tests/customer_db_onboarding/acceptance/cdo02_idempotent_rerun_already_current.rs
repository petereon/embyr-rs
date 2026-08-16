// @driving_port @real-io @US-01 @AC-01-02
//! Re-running preparation on an already-current database is a safe no-op.
//!
//! Journey (chained — Given reuses cdo01's own action: run embyr-db-prep
//! once to reach the "already prepared" precondition):
//!   Given: the database was already fully prepared by a previous run.
//!   When:  Elena re-runs embyr-db-prep against it.
//!   Then:  no further schema changes are made and Elena sees confirmation
//!          the database is already up to date.
//!
//! AC-01-02.

#[path = "../common/mod.rs"]
mod common;
use common::{
    assert_state_delta, create_ddl_role, role_connection_url, run_db_prep, set_to,
    start_postgres_container, unchanged,
};
use std::collections::HashMap;

#[tokio::test]
async fn rerunning_preparation_on_an_already_current_database_is_a_safe_no_op() {
    let (_pg, db_url) = start_postgres_container().await;
    let sys_pool = sqlx::PgPool::connect(&db_url).await.unwrap();
    create_ddl_role(&sys_pool, "elena_dba", "postgres").await;
    drop(sys_pool);
    let elena_dsn = role_connection_url(&db_url, "elena_dba");

    // Given: database already fully prepared (chains the same action cdo01
    // exercises).
    let first_run = run_db_prep(&[("EMBYR_DB_PREP_DSN", &elena_dsn)]);
    assert_eq!(
        first_run.exit_code,
        Some(0),
        "precondition: first run must succeed"
    );

    let verify_pool = sqlx::PgPool::connect(&db_url).await.unwrap();
    let applied_before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM _sqlx_migrations WHERE success",
    )
    .fetch_one(&verify_pool)
    .await
    .expect("query _sqlx_migrations after first run");

    let mut before: HashMap<&str, String> = HashMap::new();
    before.insert("migrations.applied_count", applied_before.to_string());

    // When: Elena re-runs the tool.
    let second_run = run_db_prep(&[("EMBYR_DB_PREP_DSN", &elena_dsn)]);

    let applied_after: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM _sqlx_migrations WHERE success",
    )
    .fetch_one(&verify_pool)
    .await
    .expect("query _sqlx_migrations after second run");

    let mut after: HashMap<&str, String> = HashMap::new();
    after.insert("migrations.applied_count", applied_after.to_string());
    after.insert(
        "prep_process.exit_code",
        second_run
            .exit_code
            .map(|c| c.to_string())
            .unwrap_or_else(|| "None".into()),
    );

    let universe = &["migrations.applied_count", "prep_process.exit_code"];
    let mut expected = HashMap::new();
    expected.insert("migrations.applied_count", unchanged());
    expected.insert("prep_process.exit_code", set_to("0".to_string()));

    assert_state_delta(&before, &after, universe, &expected);

    assert!(
        second_run.stdout.to_lowercase().contains("up to date")
            || second_run.stdout.to_lowercase().contains("already"),
        "AC-01-02: stdout must confirm already-up-to-date; got: {}",
        second_run.stdout
    );
}
