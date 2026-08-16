// @driving_port @real-io @US-02
//! OQ-1 / CDO-AD-05 (feature-delta.md § Wave: DESIGN / Application-Level
//! Decisions Table): `found_version >= expected_version` (not strict
//! equality) counts as `Ready` — a deliberate forward-compatibility choice
//! so provisioning is not blocked when a DBA has run a newer prep-tool
//! build than the currently-deployed embyr-server expects (rolling-deploy
//! safe).
//!
//! Journey:
//!   Given: a database's `_sqlx_migrations` bookkeeping shows a HIGHER
//!          successfully-applied version than the currently-deployed
//!          server's compiled-in expected version (simulated via a direct
//!          bookkeeping-row insert — the realistic mechanism a newer
//!          prep-tool build would have produced, not literally re-running
//!          a future binary this codebase doesn't have yet).
//!   When:  provisioning is submitted against it with a DML-only
//!          credential.
//!   Then:  the database is treated as `Ready` (not `Stale`, not blocked) —
//!          provisioning succeeds.
//!
//! Not part of the "5 minimum error-path scenarios" list — an additional
//! scenario covering a DESIGN-flagged, deliberately-documented edge case
//! (OQ-1) that the ACs alone do not otherwise exercise.

#[path = "../common/mod.rs"]
mod common;
use common::{
    create_customer_database, create_postgres_role, grant_migrations_table_read, provision,
    role_connection_url, start_healthy_server, start_postgres_container, TEST_ADMIN_KEY,
};
use embyr_pg_storage::backend_adapter::PostgresBackendAdapter;

#[tokio::test]
async fn a_found_version_higher_than_expected_is_treated_as_ready() {
    let (_pg, base_url) = start_postgres_container().await;
    let customer_db_url = create_customer_database(&base_url, "cdo18_customer").await;

    let seed_adapter = PostgresBackendAdapter::new(&customer_db_url)
        .await
        .expect("connect to seed the customer database");
    seed_adapter.migrate().await.expect("seed migration");

    // Given: a bookkeeping row for a hypothetical migration 3 — simulates a
    // newer prep-tool build having applied a migration this server's
    // compiled-in Migrator (expected_version = 2) does not itself know
    // about yet.
    let customer_sys_pool = sqlx::PgPool::connect(&customer_db_url).await.unwrap();
    sqlx::query(
        "INSERT INTO _sqlx_migrations \
         (version, description, installed_on, success, checksum, execution_time) \
         VALUES (3, 'forward_compat_probe', now(), true, '\\x00'::bytea, 0)",
    )
    .execute(&customer_sys_pool)
    .await
    .expect("insert forward-compatibility bookkeeping row");

    create_postgres_role(
        &customer_sys_pool,
        "embyr_app",
        &["GRANT SELECT, INSERT, UPDATE, DELETE ON documents, transactions TO embyr_app"],
    )
    .await;
    grant_migrations_table_read(&customer_sys_pool, "embyr_app").await;
    let embyr_app_dsn = role_connection_url(&customer_db_url, "embyr_app");

    let server = start_healthy_server(&base_url).await;

    // When: provisioning submitted with the DML-only credential.
    let (status, body) = provision(
        &server,
        TEST_ADMIN_KEY,
        serde_json::json!({
            "project_id": "cdo18-proj",
            "dsn": embyr_app_dsn,
            "backend_mode": "direct_pg",
        }),
    )
    .await;

    // Then: treated as Ready — provisioning succeeds, not blocked as Stale.
    assert_eq!(
        status, 201,
        "OQ-1/CDO-AD-05: found_version (3) >= expected_version (2) must be \
         Ready, not Stale; body: {body}"
    );
}
