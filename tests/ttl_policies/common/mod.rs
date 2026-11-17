//! Common test infrastructure — ttl-policies acceptance tests (go-live P5).
//!
//! Reuses `security_rules`'s full-stack context (system DB + customer DB
//! containers, one active `direct_pg` project, gRPC + admin listeners).

#![allow(dead_code, unused_imports)]

#[path = "../../security_rules/common/mod.rs"]
pub mod security_rules_common;
pub use security_rules_common::SecurityRulesFullContext;

use embyr_server::sweepers::ttl_sweeper::TtlSweepConfig;

/// `[0u8; 32]` is `start_test_server`'s own hardcoded server key, the one
/// `SecurityRulesFullContext` encrypts `backend_pg_dsn_enc` under.
pub fn sweep_config() -> TtlSweepConfig {
    TtlSweepConfig {
        aws_secret_fetcher: None,
        gcp_secret_fetcher: None,
        encryption_key: [0u8; 32],
        encryption_key_previous: None,
        max_deletes_per_policy: 10_000,
    }
}

/// Insert a customer-DB document directly (the `collection_id` column is
/// filled by the customer schema's own trigger).
pub async fn insert_doc(
    ctx: &SecurityRulesFullContext,
    collection_path: &str,
    doc_id: &str,
    fields: serde_json::Value,
) {
    sqlx::query(
        "INSERT INTO documents (project_id, collection_path, document_id, fields, create_time, update_time, version) \
         VALUES ($1, $2, $3, $4, now(), now(), 1)",
    )
    .bind(&ctx.project_id)
    .bind(collection_path)
    .bind(doc_id)
    .bind(fields)
    .execute(&ctx.cust_pool)
    .await
    .expect("insert document");
}

/// `{"exp": <timestamp>}` in the stored JSON encoding, `secs_from_now` apart from now.
pub fn ts_fields(field: &str, secs_from_now: i64) -> serde_json::Value {
    let s = chrono::Utc::now().timestamp() + secs_from_now;
    serde_json::json!({ field: {"t": "TS", "s": s, "n": 0} })
}

/// Document ids still live (not tombstoned) in `collection_path`.
pub async fn live_doc_ids(ctx: &SecurityRulesFullContext, collection_path: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT document_id FROM documents \
         WHERE project_id = $1 AND collection_path = $2 AND NOT deleted ORDER BY document_id",
    )
    .bind(&ctx.project_id)
    .bind(collection_path)
    .fetch_all(&ctx.cust_pool)
    .await
    .expect("select live docs")
}

pub async fn put_policy(
    ctx: &SecurityRulesFullContext,
    cookie: &str,
    body: serde_json::Value,
) -> reqwest::Response {
    reqwest::Client::new()
        .put(ctx.admin_url(&format!("/admin/v1/projects/{}/ttl_policies", ctx.project_id)))
        .header("Cookie", cookie)
        .json(&body)
        .send()
        .await
        .expect("put ttl policy request failed")
}
