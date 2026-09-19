// @driving_port @real-io @regression
//! Finding #48 (production-readiness-audit-2026-09-08.md, Follow-Up Scan):
//! `provision.rs`'s Stale/NotPrepped reprovision path never called
//! `backfill_collection_id()` -- pre-existing rows on a reprovisioned
//! project stayed permanently un-backfilled until a DBA manually ran
//! `embyr-db-prep`. `migrate()` itself was never at fault (it correctly adds
//! the `collection_id` column) -- only the backfill of *existing* rows was
//! missing from this driving port.
//!
//! Journey: a customer database already has migrations 0001-0005 applied
//! (and a real pre-existing document) from before the collection_id
//! migration (0006) existed -- an honest "Stale" precondition, same
//! technique `cdo14_provisioning_fails_when_schema_stale.rs` uses for the
//! error-path sibling scenario. An operator (re)provisions this project via
//! `POST /admin/v1/projects`. `verify_schema_readiness()` detects `Stale`,
//! `migrate()` runs (adds `collection_id`, NULL for the pre-existing row --
//! `ALTER TABLE ADD COLUMN` never backfills), and the reprovision path must
//! now also backfill that pre-existing row -- observable at the driven port
//! (the customer database itself), not just by re-running embyr-db-prep by
//! hand.
//!
//! Driving port: `embyr-server` subprocess, `POST /admin/v1/projects`
//! (reuses `customer_db_onboarding`'s own common module, same cross-feature
//! reuse convention `cgi_ws_db_prep_backfills_and_indexes.rs` already
//! established for this feature).
//!
//! Backfill is backgrounded (fire-and-forget `tokio::spawn`, not awaited by
//! the HTTP response) -- polls the driven port directly, bounded, mirroring
//! `composite_index_real_creation`'s own `wait_until_status_is_one_of`
//! polling convention for backgrounded admin-triggered Postgres work.

#[path = "../../customer_db_onboarding/common/mod.rs"]
mod common;
use common::{
    assert_state_delta, create_customer_database, create_ddl_role, provision,
    role_connection_url, set_to, start_healthy_server, start_postgres_container, TEST_ADMIN_KEY,
};
use sqlx::migrate::Migrate;
use sqlx::Connection;
use std::collections::HashMap;
use std::time::Duration;

#[tokio::test]
async fn reprovisioning_backfills_collection_id_for_pre_existing_documents() {
    let (_pg, base_url) = start_postgres_container().await;
    let customer_db_url = create_customer_database(&base_url, "cgi48_customer").await;

    // create_ddl_role's pool must be connected to the target database itself
    // (schema-level grants are database-scoped) -- mirrors cdo17's own note.
    let customer_sys_pool = sqlx::PgPool::connect(&customer_db_url)
        .await
        .expect("connect to customer database to create DDL role");
    create_ddl_role(&customer_sys_pool, "full_priv_role", "cgi48_customer").await;
    let full_priv_dsn = role_connection_url(&customer_db_url, "full_priv_role");

    // Given: an honest Stale precondition -- migrations 0001-0005 applied
    // (real sqlx bookkeeping/checksums, same technique as cdo14), 0006 (the
    // one that adds collection_id) deliberately NOT applied yet. Applied via
    // full_priv_role's own connection so it owns every object it creates --
    // no extra grants needed for migrate() to alter `documents` later.
    let migrator = sqlx::migrate!("../../migrations/customer");
    let all_migrations: Vec<_> = migrator.iter().collect();
    let (before_last, _last) = all_migrations.split_at(all_migrations.len() - 1);
    let mut conn = sqlx::PgConnection::connect(&full_priv_dsn)
        .await
        .expect("connect as full_priv_role for fixture setup");
    conn.ensure_migrations_table()
        .await
        .expect("ensure_migrations_table");
    for migration in before_last {
        conn.apply(migration)
            .await
            .expect("apply pre-collection_id migration (simulated stale prep)");
    }
    drop(conn);

    // A real pre-existing document -- plain INSERT, no trigger to disable:
    // the collection_id trigger (created by migration 0006) doesn't exist
    // yet at this point, exactly matching a genuine pre-0006 customer row.
    let pre_existing_pool = sqlx::PgPool::connect(&full_priv_dsn)
        .await
        .expect("connect as full_priv_role to seed pre-existing document");
    sqlx::query(
        "INSERT INTO documents (project_id, collection_path, document_id, fields, version, \
         create_time, update_time) VALUES ($1, 'products/p1/reviews', 'pre-1', '{}'::jsonb, 1, NOW(), NOW())",
    )
    .bind("cgi48-proj")
    .execute(&pre_existing_pool)
    .await
    .expect("insert pre-existing document");

    let server = start_healthy_server(&base_url).await;

    let before: HashMap<&str, String> = HashMap::new();

    // When: an operator (re)provisions this project.
    let (status, body) = provision(
        &server,
        TEST_ADMIN_KEY,
        serde_json::json!({
            "project_id": "cgi48-proj",
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

    // Then: provisioning succeeds via the existing Stale auto-migrate path
    // (unchanged) --
    assert_state_delta(&before, &after, universe, &expected);
    assert!(
        body["api_key"].as_str().is_some_and(|k| !k.is_empty()),
        "body: {body}"
    );

    // -- AND (the fix under test): the pre-existing document is backfilled.
    // Backgrounded, so poll the driven port (bounded, never hangs CI).
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut collection_id: Option<String> = None;
    while std::time::Instant::now() < deadline {
        collection_id = sqlx::query_scalar(
            "SELECT collection_id FROM documents WHERE project_id = $1 AND document_id = 'pre-1'",
        )
        .bind("cgi48-proj")
        .fetch_one(&pre_existing_pool)
        .await
        .expect("query pre-existing document's collection_id");
        if collection_id.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    assert_eq!(
        collection_id.as_deref(),
        Some("reviews"),
        "reprovisioning must backfill collection_id for pre-existing documents (finding #48)"
    );
}
