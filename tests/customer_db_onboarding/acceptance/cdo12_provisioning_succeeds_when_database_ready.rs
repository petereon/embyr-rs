// @walking_skeleton @driving_port @real-io @US-02
//! US-02 walking skeleton — Sam submits provisioning with the DML-only
//! connection string Elena handed over, against a database Elena's prep
//! tool already fully prepared. Provisioning proceeds normally.
//!
//! Journey (DISCUSS domain example 1 — the "verify -> provision succeeds"
//! half of the feature's overall WS pairing; cdo01 is the "prep" half):
//!   Given: a database has already been fully prepared to the current
//!          expected schema (schema version 2), and the DML-only role has
//!          already been granted read access to `_sqlx_migrations` — the
//!          end-state a real `embyr-db-prep` run produces (fixture setup,
//!          not the outcome under test — that mechanism is US-01's concern,
//!          covered by cdo07-cdo11).
//!   When:  Sam submits provisioning for it using a DML-only connection
//!          string.
//!   Then:  provisioning completes and returns the same project/API-key
//!          response shape as any other successful direct_pg provisioning.
//!
//! Driving port: `POST /admin/v1/projects` via embyr-server subprocess
//! (CARGO_BIN_EXE_embyr-server).
//! AC-02-01.
//!
//! NOT #[ignore] — walking skeleton.

#[path = "../common/mod.rs"]
mod common;
use common::{
    create_postgres_role, grant_migrations_table_read, provision, role_connection_url,
    start_postgres_container, ServerProcess, TEST_ENCRYPTION_KEY,
};
use embyr_pg_storage::backend_adapter::PostgresBackendAdapter;
use std::time::Duration;

#[tokio::test]
async fn provisioning_succeeds_when_database_is_fully_prepped() {
    let (_pg, base_url) = start_postgres_container().await;

    let sys_pool = sqlx::PgPool::connect(&base_url).await.unwrap();
    sqlx::query("CREATE DATABASE cdo12_customer")
        .execute(&sys_pool)
        .await
        .expect("create customer database");
    let last_slash = base_url.rfind('/').expect("db_url has a path separator");
    let customer_db_url = format!("{}/cdo12_customer", &base_url[..last_slash]);

    // Given: database already fully prepared (real migrate(), production
    // code that already works — not a mock) + DML role already granted
    // read access to _sqlx_migrations (end-state fixture, see module doc).
    let seed_adapter = PostgresBackendAdapter::new(&customer_db_url)
        .await
        .expect("connect to seed the customer database");
    seed_adapter
        .migrate()
        .await
        .expect("seed migration must succeed");

    // Role creation is cluster-wide (any connection may run CREATE ROLE),
    // but table-level GRANTs are database-scoped — they must run against a
    // connection to cdo12_customer itself, not the "postgres" system db
    // sys_pool is connected to.
    create_postgres_role(&sys_pool, "embyr_app", &[]).await;
    let customer_pool = sqlx::PgPool::connect(&customer_db_url).await.unwrap();
    sqlx::query("GRANT SELECT, INSERT, UPDATE, DELETE ON documents, transactions TO embyr_app")
        .execute(&customer_pool)
        .await
        .expect("grant DML privileges to embyr_app on the customer database");
    grant_migrations_table_read(&customer_pool, "embyr_app").await;
    let embyr_app_customer_dsn = role_connection_url(&customer_db_url, "embyr_app");

    // When: Sam submits provisioning with the DML-only credential.
    let server = ServerProcess::start(
        &base_url,
        &[
            ("EMBYR_ADMIN_KEY", "testkey"),
            ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
        ],
    );
    let healthy = server.wait_for_healthy(Duration::from_secs(30)).await;
    assert!(healthy, "embyr-server must be healthy before provisioning");

    let (status, body) = provision(
        &server,
        "testkey",
        serde_json::json!({
            "project_id": "cdo12-proj",
            "dsn": embyr_app_customer_dsn,
            "backend_mode": "direct_pg",
        }),
    )
    .await;

    // Then: provisioning succeeds with the standard response shape.
    assert_eq!(
        status, 201,
        "AC-02-01: provisioning must succeed (201 Created); body: {body}"
    );
    assert_eq!(
        body["project_id"], "cdo12-proj",
        "response must echo the project_id; body: {body}"
    );
    assert!(
        body["api_key"].as_str().is_some_and(|k| !k.is_empty()),
        "response must include a non-empty api_key; body: {body}"
    );
}
