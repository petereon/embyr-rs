// @error @real-io @US-01
//! ADR-023 § Enforcement: identifier interpolation into the dynamic `GRANT`
//! is injection-safe. A role name containing a reserved word round-trips
//! correctly through `discover_current_user()` -> `format('%I', ...)` ->
//! `grant_schema_readiness_read()` without producing a malformed or
//! injectable statement.
//!
//! Journey:
//!   Given: the DML role is literally named `select` — a reserved Postgres
//!          keyword, creatable only when quoted (`CREATE ROLE "select"`),
//!          and exactly the kind of pathological identifier a hand-rolled
//!          Rust-side quoting routine could mishandle.
//!   When:  Elena runs embyr-db-prep with `EMBYR_DB_PREP_DML_ROLE_DSN` set
//!          to that role's own connection string.
//!   Then:  the grant succeeds (the role can read `_sqlx_migrations`
//!          afterward) — Postgres's own `format('%I', ...)` quoted the
//!          identifier correctly, producing `GRANT SELECT ON
//!          _sqlx_migrations TO "select"`, not a malformed or injectable
//!          statement.

#[path = "../common/mod.rs"]
mod common;
use common::{create_ddl_role, role_connection_url, run_db_prep, start_postgres_container};

#[tokio::test]
#[ignore]
async fn reserved_word_role_name_is_quoted_safely_through_the_grant() {
    let (_pg, db_url) = start_postgres_container().await;
    let sys_pool = sqlx::PgPool::connect(&db_url).await.unwrap();
    create_ddl_role(&sys_pool, "elena_dba", "postgres").await;
    // Role literally named `select` — a reserved keyword. Created directly
    // (not via common::create_postgres_role, which does not quote names) to
    // make the pathological-identifier precondition explicit.
    sqlx::query(r#"CREATE ROLE "select" LOGIN PASSWORD 'select'"#)
        .execute(&sys_pool)
        .await
        .expect("create reserved-word-named role");
    drop(sys_pool);

    let elena_dsn = role_connection_url(&db_url, "elena_dba");
    let after_scheme = db_url.strip_prefix("postgres://").unwrap();
    let host_and_db = after_scheme.split_once('@').map(|(_, r)| r).unwrap();
    let reserved_role_dsn = format!("postgres://select:select@{host_and_db}");

    let run = run_db_prep(&[
        ("EMBYR_DB_PREP_DSN", &elena_dsn),
        ("EMBYR_DB_PREP_DML_ROLE_DSN", &reserved_role_dsn),
    ]);
    assert_eq!(
        run.exit_code,
        Some(0),
        "ADR-023: a reserved-word role name must not break the grant step; \
         stderr: {}",
        run.stderr
    );

    let reserved_role_pool = sqlx::PgPool::connect(&reserved_role_dsn)
        .await
        .expect("connect as the reserved-word-named role");
    let result: Result<i64, sqlx::Error> =
        sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations")
            .fetch_one(&reserved_role_pool)
            .await;
    assert!(
        result.is_ok(),
        "ADR-023: format('%I', ...) must correctly quote a reserved-word \
         role name so the GRANT actually applies to it; got: {:?}",
        result.err()
    );
}
