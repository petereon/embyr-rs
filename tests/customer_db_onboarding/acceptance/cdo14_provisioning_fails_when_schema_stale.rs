// @error @driving_port @real-io @US-02
//! Provisioning fails with a specific complaint when the schema is stale.
//!
//! Journey (DISCUSS domain example 3 — "Elena ran an older copy of the prep
//! tool that applied only schema version 1 of the current 2"):
//!   Given: a database was prepped with an older schema version (only
//!          migration `0001`) than currently expected (`2`), and Sam
//!          submits a DML-only connection string.
//!   When:  Sam submits provisioning for it.
//!   Then:  the response is a 400 naming both the expected schema version
//!          and the version found.
//!
//! Fixture note: the "stale" precondition is produced via sqlx's own
//! `Migrate` trait (same technique as cdo03) — applying exactly migration
//! `0001` with sqlx's real bookkeeping/checksum.
//!
//! AC-02-03.

#[path = "../common/mod.rs"]
mod common;
use common::{
    assert_state_delta, create_postgres_role, provision, role_connection_url, set_to,
    start_postgres_container, ServerProcess, TEST_ENCRYPTION_KEY,
};
use sqlx::migrate::Migrate;
use std::collections::HashMap;
use std::time::Duration;

#[tokio::test]
#[ignore]
async fn provisioning_fails_with_a_specific_complaint_when_schema_is_stale() {
    let (_pg, base_url) = start_postgres_container().await;
    let sys_pool = sqlx::PgPool::connect(&base_url).await.unwrap();
    sqlx::query("CREATE DATABASE cdo14_customer")
        .execute(&sys_pool)
        .await
        .expect("create customer database");
    let last_slash = base_url.rfind('/').unwrap();
    let customer_db_url = format!("{}/cdo14_customer", &base_url[..last_slash]);

    // Given: only migration 0001 applied (stale — expected 2, found 1).
    let migrator = sqlx::migrate!("../../migrations/customer");
    use sqlx::Connection;
    let mut conn = sqlx::PgConnection::connect(&customer_db_url)
        .await
        .expect("connect for fixture setup");
    conn.ensure_migrations_table()
        .await
        .expect("ensure_migrations_table");
    let first_migration = migrator.iter().next().expect("at least one migration");
    conn.apply(first_migration)
        .await
        .expect("apply only migration 0001 (simulated stale prep)");
    drop(conn);

    let customer_sys_pool = sqlx::PgPool::connect(&customer_db_url).await.unwrap();
    create_postgres_role(
        &customer_sys_pool,
        "embyr_app",
        &["GRANT SELECT, INSERT, UPDATE, DELETE ON documents TO embyr_app"],
    )
    .await;
    let embyr_app_dsn = role_connection_url(&customer_db_url, "embyr_app");

    let server = ServerProcess::start(
        &base_url,
        &[
            ("EMBYR_ADMIN_KEY", "testkey"),
            ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
        ],
    );
    assert!(server.wait_for_healthy(Duration::from_secs(30)).await);

    let before: HashMap<&str, String> = HashMap::new();

    let (status, body) = provision(
        &server,
        "testkey",
        serde_json::json!({
            "project_id": "cdo14-proj",
            "dsn": embyr_app_dsn,
            "backend_mode": "direct_pg",
        }),
    )
    .await;

    let mut after: HashMap<&str, String> = HashMap::new();
    after.insert("provision_response.http_status", status.to_string());
    after.insert(
        "provision_response.error_field",
        body["error"].as_str().unwrap_or("").to_string(),
    );

    let universe = &[
        "provision_response.http_status",
        "provision_response.error_field",
    ];
    let mut expected = HashMap::new();
    expected.insert("provision_response.http_status", set_to("400".to_string()));
    expected.insert(
        "provision_response.error_field",
        set_to("customer_db_schema_stale".to_string()),
    );

    assert_state_delta(&before, &after, universe, &expected);
    let detail = body["detail"].as_str().unwrap_or("");
    assert!(
        detail.contains('2') && detail.contains('1'),
        "AC-02-03: detail must name both expected (2) and found (1) \
         versions; body: {body}"
    );
}
