// @real-io @US-01
//! ADR-023 § Enforcement: running embyr-db-prep's full flow (migrate +
//! discover + grant) twice against the same database and the same
//! `EMBYR_DB_PREP_DML_ROLE_DSN` is a no-op on the second run.
//!
//! Journey (chained — Given reuses the first full-flow run as its own
//! precondition):
//!   Given: a fresh database, elevated role, and DML role all exist.
//!   When (1): Elena runs embyr-db-prep with both DSNs set — migrate +
//!             discover + grant all execute.
//!   When (2): Elena runs the identical command again.
//!   Then:     the second run succeeds identically (exit 0, same success
//!             message) — `migrate()`'s existing idempotency plus
//!             `grant_schema_readiness_read()`'s idempotency (re-granting
//!             an already-granted privilege is a Postgres no-op, not an
//!             error).

#[path = "../common/mod.rs"]
mod common;
use common::{
    assert_state_delta, create_ddl_role, create_postgres_role, role_connection_url, run_db_prep,
    set_to, start_postgres_container,
};
use std::collections::HashMap;

#[tokio::test]
async fn full_flow_run_twice_against_the_same_database_and_role_is_idempotent() {
    let (_pg, db_url) = start_postgres_container().await;
    let sys_pool = sqlx::PgPool::connect(&db_url).await.unwrap();
    create_ddl_role(&sys_pool, "elena_dba", "postgres").await;
    create_postgres_role(&sys_pool, "embyr_app", &[]).await;
    drop(sys_pool);
    let elena_dsn = role_connection_url(&db_url, "elena_dba");
    let embyr_app_dsn = role_connection_url(&db_url, "embyr_app");

    let env = [
        ("EMBYR_DB_PREP_DSN", elena_dsn.as_str()),
        ("EMBYR_DB_PREP_DML_ROLE_DSN", embyr_app_dsn.as_str()),
    ];

    let first_run = run_db_prep(&env);
    assert_eq!(
        first_run.exit_code,
        Some(0),
        "precondition: first full-flow run must succeed; stderr: {}",
        first_run.stderr
    );

    let before: HashMap<&str, String> = HashMap::new();

    let second_run = run_db_prep(&env);

    let mut after: HashMap<&str, String> = HashMap::new();
    after.insert(
        "prep_process.exit_code",
        second_run
            .exit_code
            .map(|c| c.to_string())
            .unwrap_or_else(|| "None".into()),
    );

    let universe = &["prep_process.exit_code"];
    let mut expected = HashMap::new();
    expected.insert("prep_process.exit_code", set_to("0".to_string()));

    assert_state_delta(&before, &after, universe, &expected);

    // Confirm the grant survives an identical re-run — a Postgres re-grant
    // no-op, not a broken privilege state.
    let embyr_app_pool = sqlx::PgPool::connect(&embyr_app_dsn).await.unwrap();
    let result: Result<i64, sqlx::Error> =
        sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
            .fetch_one(&embyr_app_pool)
            .await;
    assert!(
        result.is_ok(),
        "second run must leave embyr_app's grant intact: {:?}",
        result.err()
    );
}
