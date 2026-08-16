// @driving_port @real-io @US-02 @AC-02-05
//! A DML-only credential succeeds against a fully-prepped database without
//! requiring elevated privilege — explicit proof, not just an absence of
//! error.
//!
//! Journey:
//!   Given: the submitted connection string's role is structurally
//!          DDL-incapable (a direct `CREATE TABLE` attempt as that role
//!          fails), and the target database is fully prepped.
//!   When:  Sam submits provisioning.
//!   Then:  provisioning succeeds, AND the migration bookkeeping state is
//!          unchanged by the request (embyr's provisioning flow never
//!          attempted DDL on the submitted connection string — it only
//!          ever read `_sqlx_migrations`).
//!
//! AC-02-05.

#[path = "../common/mod.rs"]
mod common;
use common::{
    assert_state_delta, create_postgres_role, grant_migrations_table_read, provision,
    role_connection_url, set_to, start_postgres_container, unchanged, ServerProcess,
    TEST_ENCRYPTION_KEY,
};
use embyr_pg_storage::backend_adapter::PostgresBackendAdapter;
use std::collections::HashMap;
use std::time::Duration;

#[tokio::test]
#[ignore]
async fn dml_only_credential_succeeds_and_never_attempts_ddl() {
    let (_pg, base_url) = start_postgres_container().await;
    let sys_pool = sqlx::PgPool::connect(&base_url).await.unwrap();
    sqlx::query("CREATE DATABASE cdo16_customer")
        .execute(&sys_pool)
        .await
        .expect("create customer database");
    let last_slash = base_url.rfind('/').unwrap();
    let customer_db_url = format!("{}/cdo16_customer", &base_url[..last_slash]);

    let seed_adapter = PostgresBackendAdapter::new(&customer_db_url)
        .await
        .expect("connect to seed the customer database");
    seed_adapter.migrate().await.expect("seed migration");

    let customer_sys_pool = sqlx::PgPool::connect(&customer_db_url).await.unwrap();
    create_postgres_role(
        &customer_sys_pool,
        "embyr_app",
        &["GRANT SELECT, INSERT, UPDATE, DELETE ON documents, transactions TO embyr_app"],
    )
    .await;
    grant_migrations_table_read(&customer_sys_pool, "embyr_app").await;
    let embyr_app_dsn = role_connection_url(&customer_db_url, "embyr_app");

    // Given: structurally DDL-incapable — a direct CREATE attempt fails.
    let embyr_app_pool = sqlx::PgPool::connect(&embyr_app_dsn).await.unwrap();
    let ddl_attempt: Result<sqlx::postgres::PgQueryResult, sqlx::Error> =
        sqlx::query("CREATE TABLE embyr_app_ddl_probe (id INT)")
            .execute(&embyr_app_pool)
            .await;
    assert!(
        ddl_attempt.is_err(),
        "precondition: embyr_app must be structurally unable to run DDL"
    );

    let applied_before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM _sqlx_migrations WHERE success",
    )
    .fetch_one(&customer_sys_pool)
    .await
    .unwrap();

    let mut before: HashMap<&str, String> = HashMap::new();
    before.insert("migrations.applied_count", applied_before.to_string());

    let server = ServerProcess::start(
        &base_url,
        &[
            ("EMBYR_ADMIN_KEY", "testkey"),
            ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
        ],
    );
    assert!(server.wait_for_healthy(Duration::from_secs(30)).await);

    // When: Sam submits provisioning with the DML-only credential.
    let (status, body) = provision(
        &server,
        "testkey",
        serde_json::json!({
            "project_id": "cdo16-proj",
            "dsn": embyr_app_dsn,
            "backend_mode": "direct_pg",
        }),
    )
    .await;

    let applied_after: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM _sqlx_migrations WHERE success",
    )
    .fetch_one(&customer_sys_pool)
    .await
    .unwrap();

    let mut after: HashMap<&str, String> = HashMap::new();
    after.insert("provision_response.http_status", status.to_string());
    after.insert("migrations.applied_count", applied_after.to_string());

    let universe = &[
        "provision_response.http_status",
        "migrations.applied_count",
    ];
    let mut expected = HashMap::new();
    expected.insert("provision_response.http_status", set_to("201".to_string()));
    // Then: no DDL attempted — the migration bookkeeping is unchanged by
    // the provisioning request (migrate() was never called; only
    // verify_schema_readiness()'s read-only SELECT ran).
    expected.insert("migrations.applied_count", unchanged());

    assert_state_delta(&before, &after, universe, &expected);
    assert!(
        body["api_key"].as_str().is_some_and(|k| !k.is_empty()),
        "body: {body}"
    );
}
