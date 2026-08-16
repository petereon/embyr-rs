// @error @real-io @US-01
//! Negative privilege test (ADR-023 § Enforcement, added per the targeted
//! security review): a role OTHER than the granted DML role cannot read
//! `_sqlx_migrations`. This is the exact test that would have caught a
//! `GRANT ... TO PUBLIC` regression — with `PUBLIC`, this scenario would
//! incorrectly succeed.
//!
//! Journey (chained — reuses cdo07's own action: run embyr-db-prep with a
//! DML-role DSN):
//!   Given: `embyr-db-prep` ran with `EMBYR_DB_PREP_DML_ROLE_DSN` set to
//!          role `embyr_app`, and a second, distinct role
//!          (`embyr_app_other`) exists with no privileges granted by the
//!          test fixture.
//!   When:  `embyr_app_other` attempts `SELECT * FROM _sqlx_migrations`.
//!   Then:  the query fails with `permission denied for table
//!          _sqlx_migrations` — the grant is role-scoped, not `PUBLIC`.

#[path = "../common/mod.rs"]
mod common;
use common::{create_ddl_role, create_postgres_role, role_connection_url, run_db_prep, start_postgres_container};

#[tokio::test]
#[ignore]
async fn a_different_non_granted_role_cannot_read_the_migrations_table() {
    let (_pg, db_url) = start_postgres_container().await;
    let sys_pool = sqlx::PgPool::connect(&db_url).await.unwrap();
    create_ddl_role(&sys_pool, "elena_dba", "postgres").await;
    create_postgres_role(&sys_pool, "embyr_app", &[]).await;
    // Distinct role, granted NOTHING by the test fixture — this is the
    // negative-test role the security review specifically asked for.
    create_postgres_role(&sys_pool, "embyr_app_other", &[]).await;
    drop(sys_pool);

    let elena_dsn = role_connection_url(&db_url, "elena_dba");
    let embyr_app_dsn = role_connection_url(&db_url, "embyr_app");
    let embyr_app_other_dsn = role_connection_url(&db_url, "embyr_app_other");

    // Given: prep run supplied embyr_app's DSN (chains cdo07's action).
    let run = run_db_prep(&[
        ("EMBYR_DB_PREP_DSN", &elena_dsn),
        ("EMBYR_DB_PREP_DML_ROLE_DSN", &embyr_app_dsn),
    ]);
    assert_eq!(
        run.exit_code,
        Some(0),
        "precondition: prep run with DML role DSN must succeed; stderr: {}",
        run.stderr
    );

    // When: embyr_app_other (never named to embyr-db-prep) tries to read.
    let other_pool = sqlx::PgPool::connect(&embyr_app_other_dsn)
        .await
        .expect("connect as embyr_app_other");
    let result: Result<i64, sqlx::Error> =
        sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
            .fetch_one(&other_pool)
            .await;

    // Then: permission denied — the grant is scoped to exactly embyr_app,
    // not every role on the database (this is what a PUBLIC grant would
    // have missed).
    let err = result.expect_err(
        "ADR-023: a role never named to embyr-db-prep must NOT be able to \
         read _sqlx_migrations (this is the PUBLIC-grant regression test)",
    );
    assert!(
        err.to_string().to_lowercase().contains("permission denied"),
        "expected 'permission denied', got: {err}"
    );
}
