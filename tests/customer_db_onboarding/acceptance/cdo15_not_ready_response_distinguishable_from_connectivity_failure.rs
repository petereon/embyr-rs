// @error @driving_port @real-io @US-02
//! The "not ready" failure response is distinguishable, in content, from a
//! plain connectivity failure to the target database.
//!
//! Journey (chained — reuses cdo13's own "not prepped" precondition and
//! action as one half of the comparison):
//!   Given: (a) a reachable-but-not-prepped database, and (b) an
//!          unreachable database.
//!   When:  Sam submits provisioning against each.
//!   Then:  the two 400 responses have distinguishable `error` field
//!          values — the database is reachable but not ready is not
//!          conflated with the database being unreachable at all.
//!
//! AC-02-04.

#[path = "../common/mod.rs"]
mod common;
use common::{
    create_customer_database, create_postgres_role, provision, role_connection_url,
    start_healthy_server, start_postgres_container, TEST_ADMIN_KEY,
};

#[tokio::test]
async fn not_ready_response_is_distinguishable_from_a_connectivity_failure() {
    let (_pg, base_url) = start_postgres_container().await;
    let customer_db_url = create_customer_database(&base_url, "cdo15_customer").await;

    // Given (a): reachable, but never prepped, DML-only role.
    let customer_sys_pool = sqlx::PgPool::connect(&customer_db_url).await.unwrap();
    create_postgres_role(&customer_sys_pool, "embyr_app", &[]).await;
    let embyr_app_dsn = role_connection_url(&customer_db_url, "embyr_app");

    let server = start_healthy_server(&base_url).await;

    let (not_ready_status, not_ready_body) = provision(
        &server,
        TEST_ADMIN_KEY,
        serde_json::json!({
            "project_id": "cdo15-not-ready",
            "dsn": embyr_app_dsn,
            "backend_mode": "direct_pg",
        }),
    )
    .await;

    // Given (b): an unreachable database (RFC 5737 TEST-NET-1).
    let (unreachable_status, unreachable_body) = provision(
        &server,
        TEST_ADMIN_KEY,
        serde_json::json!({
            "project_id": "cdo15-unreachable",
            "dsn": "postgres://nobody:nobody@192.0.2.1:5432/nowhere",
            "backend_mode": "direct_pg",
        }),
    )
    .await;

    // Then: both are 400s, but with distinguishable content.
    assert_eq!(not_ready_status, 400, "body: {not_ready_body}");
    assert_eq!(unreachable_status, 400, "body: {unreachable_body}");
    assert_ne!(
        not_ready_body["error"], unreachable_body["error"],
        "AC-02-04: 'not prepped' and 'unreachable' must produce distinct \
         error field values; not_ready={not_ready_body} unreachable={unreachable_body}"
    );
    assert_eq!(
        unreachable_body["error"], "backend_unavailable",
        "AC-02-04: unreachable-database path must remain the existing \
         backend_unavailable classification (unchanged); body: {unreachable_body}"
    );
}
