// @real-io @US-01
//! The revised role-scoped grant (ADR-023, security-review resolution):
//! after a prep run that supplied `EMBYR_DB_PREP_DML_ROLE_DSN`, the named
//! DML role really can read `_sqlx_migrations` — with no manual grant step
//! from the test fixture.
//!
//! Journey:
//!   Given: a fresh database, an elevated role (`elena_dba`) and a
//!          DML-only role (`embyr_app`) both exist.
//!   When:  Elena runs embyr-db-prep with both `EMBYR_DB_PREP_DSN` (elevated)
//!          and `EMBYR_DB_PREP_DML_ROLE_DSN` (embyr_app's own DSN) set.
//!   Then:  `embyr_app` can `SELECT * FROM _sqlx_migrations` — the grant was
//!          performed automatically, not by the test fixture.
//!
//! This is the positive half of the negative-privilege test pair
//! (`docs/product/architecture/adr-023-...md` § Enforcement); cdo08 is the
//! negative half.

#[path = "../common/mod.rs"]
mod common;
use common::{create_ddl_role, create_postgres_role, role_connection_url, run_db_prep, start_postgres_container};

#[tokio::test]
#[ignore]
async fn dml_role_can_read_migrations_table_after_prep_run_supplies_its_dsn() {
    let (_pg, db_url) = start_postgres_container().await;
    let sys_pool = sqlx::PgPool::connect(&db_url).await.unwrap();
    create_ddl_role(&sys_pool, "elena_dba", "postgres").await;
    create_postgres_role(&sys_pool, "embyr_app", &[]).await;
    drop(sys_pool);

    let elena_dsn = role_connection_url(&db_url, "elena_dba");
    let embyr_app_dsn = role_connection_url(&db_url, "embyr_app");

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

    let embyr_app_pool = sqlx::PgPool::connect(&embyr_app_dsn)
        .await
        .expect("connect as embyr_app to verify the grant");
    let result: Result<i64, sqlx::Error> =
        sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
            .fetch_one(&embyr_app_pool)
            .await;

    assert!(
        result.is_ok(),
        "ADR-023: embyr_app must be able to SELECT _sqlx_migrations after a \
         prep run supplied its DSN; got error: {:?}",
        result.err()
    );
}
