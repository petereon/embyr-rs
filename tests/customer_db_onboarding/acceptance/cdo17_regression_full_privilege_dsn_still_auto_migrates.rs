// @driving_port @real-io @US-02 @AC-02-06
//! Regression guardrail: existing full-privilege-DSN provisioning (today's
//! default, non-DBA-gated path) continues to succeed unchanged. This is
//! KPI #3 (guardrail) — DESIGN's single highest-consequence defect class
//! this feature can produce (CDO-AD-04, ADR-023 § Decision).
//!
//! Journey:
//!   Given: a customer's connection string carries full (DDL-capable)
//!          privilege and the database is not yet prepped, exactly as
//!          today's default flow expects.
//!   When:  an operator submits provisioning for it.
//!   Then:  provisioning succeeds via the existing automatic-migration
//!          path, unchanged from current behavior — `verify_schema_readiness()`
//!          detects `NotPrepped`, but the existing auto-migrate attempt
//!          fires and succeeds (the DSN did have DDL rights after all), so
//!          provisioning proceeds exactly as it does today.
//!
//! AC-02-06.

#[path = "../common/mod.rs"]
mod common;
use common::{
    assert_state_delta, create_ddl_role, provision, role_connection_url, set_to,
    start_postgres_container, ServerProcess, TEST_ENCRYPTION_KEY,
};
use std::collections::HashMap;
use std::time::Duration;

#[tokio::test]
#[ignore]
async fn full_privilege_dsn_against_an_unprepped_database_still_auto_migrates() {
    let (_pg, base_url) = start_postgres_container().await;
    let sys_pool = sqlx::PgPool::connect(&base_url).await.unwrap();
    sqlx::query("CREATE DATABASE cdo17_customer")
        .execute(&sys_pool)
        .await
        .expect("create customer database");
    let last_slash = base_url.rfind('/').unwrap();
    let customer_db_url = format!("{}/cdo17_customer", &base_url[..last_slash]);

    // Given: full-privilege (DDL-capable) role, database not yet prepped —
    // exactly today's default, non-DBA-gated flow. create_ddl_role's pool
    // must be connected to the target database itself (schema-level grants
    // are database-scoped) — sys_pool is connected to "postgres", not
    // cdo17_customer.
    let customer_pool = sqlx::PgPool::connect(&customer_db_url).await.unwrap();
    create_ddl_role(&customer_pool, "full_priv_role", "cdo17_customer").await;
    let full_priv_dsn = role_connection_url(&customer_db_url, "full_priv_role");

    let server = ServerProcess::start(
        &base_url,
        &[
            ("EMBYR_ADMIN_KEY", "testkey"),
            ("EMBYR_ENCRYPTION_KEY", TEST_ENCRYPTION_KEY),
        ],
    );
    assert!(server.wait_for_healthy(Duration::from_secs(30)).await);

    let before: HashMap<&str, String> = HashMap::new();

    // When: an operator submits provisioning.
    let (status, body) = provision(
        &server,
        "testkey",
        serde_json::json!({
            "project_id": "cdo17-proj",
            "dsn": full_priv_dsn,
            "backend_mode": "direct_pg",
        }),
    )
    .await;

    let mut after: HashMap<&str, String> = HashMap::new();
    after.insert("provision_response.http_status", status.to_string());

    let universe = &["provision_response.http_status"];
    let mut expected = HashMap::new();
    expected.insert("provision_response.http_status", set_to("201".to_string()));

    // Then: provisioning succeeds — no regression to the unchanged path.
    assert_state_delta(&before, &after, universe, &expected);
    assert!(
        body["api_key"].as_str().is_some_and(|k| !k.is_empty()),
        "body: {body}"
    );

    // Real I/O confirmation the auto-migrate attempt genuinely ran (not
    // skipped) — the tables must now exist, produced by this request.
    let customer_pool = sqlx::PgPool::connect(&customer_db_url).await.unwrap();
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name FROM information_schema.tables \
         WHERE table_schema='public' ORDER BY table_name",
    )
    .fetch_all(&customer_pool)
    .await
    .unwrap();
    assert!(tables.contains(&"documents".to_string()), "tables: {tables:?}");
    assert!(tables.contains(&"transactions".to_string()), "tables: {tables:?}");
}
