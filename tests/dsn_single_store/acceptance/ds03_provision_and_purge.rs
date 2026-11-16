//! Slice 2 (ADR-083): provision writes the server-key store only; sweeper DSN resolution and purge
//! cover the new column. AC-DSS-01, AC-DSS-07, AC-DSS-14. AC-DSS-02 (agent provision) lives in
//! tests/acceptance/us_12_agent_backend.rs next to the mTLS mock-agent fixtures.

#[path = "../common/mod.rs"]
mod common;
use common::*;

use embyr_server::adapters::customer_db_connect::{resolve_dsn_without_api_key, PgConnectInfo};
use embyr_server::sweepers::soft_delete_purge_sweeper;

/// AC-DSS-01 + AC-DSS-07: provisioning a direct_pg project stores the DSN only under the server
/// key, requests with the returned API key are served, and the sweeper's key-less resolution
/// reaches it (the ADR-055 gap is closed).
#[tokio::test]
async fn provision_stores_server_key_dsn_only_and_sweeper_can_resolve_it() {
    let env = start_system_db().await;
    let (_c, cust_url, cust_pool) = start_ready_customer_db().await;
    let server = server_for(&env).await;

    let resp = reqwest::Client::new()
        .post(format!("http://{}/admin/v1/projects", server.admin_addr))
        .header("Authorization", format!("Bearer {ADMIN_KEY}"))
        .json(&serde_json::json!({ "project_id": "ds03-a", "dsn": cust_url, "backend_mode": "direct_pg" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let api_key = resp.json::<serde_json::Value>().await.unwrap()["api_key"].as_str().unwrap().to_string();

    assert_eq!(column_bytes(&env.sys_pool, "ds03-a", "ecies_encrypted_dsn").await, None);
    assert_eq!(server_store_dsn(&env.sys_pool, "ds03-a").await.as_deref(), Some(cust_url.as_str()));

    seed_greeting(&cust_pool, "ds03-a", "provisioned").await;
    assert_eq!(get_greeting(&server, "ds03-a", &api_key).await.unwrap(), "provisioned");

    let resolved = resolve_dsn_without_api_key(
        "ds03-a",
        &PgConnectInfo {
            backend_mode: "direct_pg".into(),
            backend_secret_arn: None,
            backend_secret_gcp: None,
            backend_pg_dsn_enc: column_bytes(&env.sys_pool, "ds03-a", "backend_pg_dsn_enc").await,
        },
        None,
        None,
        &TEST_KEY,
        None,
    )
    .await;
    assert_eq!(resolved.as_deref(), Some(cust_url.as_str()));
}

/// AC-DSS-14: purge of a soft-deleted project past grace nulls the new bundle column too.
#[tokio::test]
async fn purge_nulls_server_key_columns_after_grace() {
    let env = start_system_db().await;
    sqlx::query(
        "INSERT INTO projects (id, status, backend_mode, api_key_hash_current, deleted_at, \
         backend_pg_dsn_enc, agent_tls_bundle_srv_enc) \
         VALUES ('ds03-p', 'deleted', 'agent', 'h', now() - interval '30 days', $1, $2)",
    )
    .bind(vec![1u8; 30])
    .bind(vec![2u8; 30])
    .execute(&env.sys_pool)
    .await
    .unwrap();

    soft_delete_purge_sweeper::run_cycle(&env.system_db, 7).await;

    assert_eq!(column_bytes(&env.sys_pool, "ds03-p", "backend_pg_dsn_enc").await, None);
    assert_eq!(column_bytes(&env.sys_pool, "ds03-p", "agent_tls_bundle_srv_enc").await, None);
}

/// ADR-083 enforcement: provision never ECIES-encrypts connection secrets again.
#[test]
fn provision_source_has_no_ecies_encrypt() {
    let src = include_str!("../../../crates/embyr-server/src/admin/handlers/provision.rs");
    assert!(!src.contains("ecies::encrypt") && !src.contains("ecies_encrypted_dsn"));
}
