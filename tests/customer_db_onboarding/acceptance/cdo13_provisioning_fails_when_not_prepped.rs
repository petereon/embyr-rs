// @error @driving_port @real-io @US-02
//! Provisioning fails with a specific complaint when the database is not
//! prepped at all — the combined "NotPrepped detection AND the enriched
//! error body in the same scenario" case: `verify_schema_readiness()`
//! detects `NotPrepped`, the existing (unchanged) auto-migrate attempt is
//! still tried (CDO-AD-04), and because the submitted credential is
//! DML-only it ALSO fails — only then does the enriched
//! `customer_db_not_prepped` body replace today's generic
//! `backend_unavailable`.
//!
//! Journey (DISCUSS domain example 2):
//!   Given: a fresh, completely empty database has never had the prep step
//!          run against it, and Sam submits a DML-only connection string
//!          (the realistic DBA-gated scenario this feature exists for).
//!   When:  Sam submits provisioning for it.
//!   Then:  the response is a 400 naming the specific missing schema
//!          element, not a generic backend_unavailable error.
//!
//! AC-02-02.

#[path = "../common/mod.rs"]
mod common;
use common::{
    assert_state_delta, create_postgres_role, provision, role_connection_url, set_to,
    start_postgres_container, ServerProcess, TEST_ENCRYPTION_KEY,
};
use std::collections::HashMap;
use std::time::Duration;

#[tokio::test]
async fn provisioning_fails_with_a_specific_complaint_when_database_not_prepped_at_all() {
    let (_pg, base_url) = start_postgres_container().await;
    let sys_pool = sqlx::PgPool::connect(&base_url).await.unwrap();
    sqlx::query("CREATE DATABASE cdo13_customer")
        .execute(&sys_pool)
        .await
        .expect("create customer database");
    let last_slash = base_url.rfind('/').unwrap();
    let customer_db_url = format!("{}/cdo13_customer", &base_url[..last_slash]);

    // Given: fresh, completely empty database — no prep step ever run — and
    // a DML-only role (cannot itself apply the pending migrations).
    let customer_sys_pool = sqlx::PgPool::connect(&customer_db_url).await.unwrap();
    create_postgres_role(&customer_sys_pool, "embyr_app", &[]).await;
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

    // When: Sam submits provisioning.
    let (status, body) = provision(
        &server,
        "testkey",
        serde_json::json!({
            "project_id": "cdo13-proj",
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
        set_to("customer_db_not_prepped".to_string()),
    );

    // Then: 400, naming the specific missing schema element — not a generic
    // backend_unavailable.
    assert_state_delta(&before, &after, universe, &expected);
    let detail = body["detail"].as_str().unwrap_or("");
    assert!(
        detail.contains("documents") || detail.contains("transactions"),
        "AC-02-02: detail must name the specific missing table; body: {body}"
    );
}
