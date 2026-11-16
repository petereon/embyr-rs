//! Audit #65 (SSRF) + X10 (TLS): any DSN an operator/admin submits (provision, PUT /connection) is
//! rejected when its host resolves to loopback/link-local/private/unspecified space, or when it does
//! not require TLS, unless EMBYR_ALLOW_PRIVATE_DSN_HOSTS=true.
//!
//! Own test binary and NOT using `start_test_server` (which sets the dev flag process-wide): the
//! admin router is served directly, and the flag is explicitly cleared.

#[path = "../common/mod.rs"]
mod common;
use common::*;
use std::sync::Arc;

const REJECTED: &[(&str, &str)] = &[
    ("postgres://u:p@169.254.169.254/db?sslmode=require", "dsn_host_not_allowed"),
    ("postgres://u:p@127.0.0.1/db?sslmode=require", "dsn_host_not_allowed"),
    ("postgres://u:p@localhost/db?sslmode=require", "dsn_host_not_allowed"),
    ("postgres://u:p@10.1.2.3/db?sslmode=verify-full", "dsn_host_not_allowed"),
    ("postgres://u:p@192.168.0.7/db?sslmode=require", "dsn_host_not_allowed"),
    ("postgres://u:p@172.16.4.4/db?sslmode=require", "dsn_host_not_allowed"),
    ("postgres://u:p@0.0.0.0/db?sslmode=require", "dsn_host_not_allowed"),
    ("postgres://u:p@[::1]/db?sslmode=require", "dsn_host_not_allowed"),
    ("postgres://u:p@[fd00::1]/db?sslmode=require", "dsn_host_not_allowed"),
    ("postgres://u:p@8.8.8.8/db?sslmode=require&hostaddr=127.0.0.1", "dsn_host_not_allowed"),
    ("postgres://u:p@8.8.8.8/db", "dsn_tls_required"),
    ("postgres://u:p@8.8.8.8/db?sslmode=prefer", "dsn_tls_required"),
    ("postgres://u:p@8.8.8.8/db?sslmode=disable", "dsn_tls_required"),
    ("mysql://u:p@8.8.8.8/db?sslmode=require", "invalid_dsn"),
];

async fn serve_admin(env: &Env) -> std::net::SocketAddr {
    std::env::remove_var("EMBYR_ALLOW_PRIVATE_DSN_HOSTS");
    let router = embyr_server::admin::router::build(
        Arc::clone(&env.system_db),
        ADMIN_KEY.to_string(),
        Arc::new(embyr_server::adapters::credential_cache::CredentialCache::new(16)),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    addr
}

#[tokio::test]
async fn provision_rejects_ssrf_targets_and_plaintext_dsns() {
    let env = start_system_db().await;
    let addr = serve_admin(&env).await;
    for (i, (dsn, detail)) in REJECTED.iter().enumerate() {
        let resp = reqwest::Client::new()
            .post(format!("http://{addr}/admin/v1/projects"))
            .header("Authorization", format!("Bearer {ADMIN_KEY}"))
            .json(&serde_json::json!({ "project_id": format!("ds05-p{i}"), "dsn": dsn }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 400, "{dsn}");
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!((body["error"].as_str(), body["detail"].as_str()), (Some("invalid_dsn"), Some(*detail)), "{dsn}");
        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM projects WHERE id = $1")
            .bind(format!("ds05-p{i}"))
            .fetch_one(&env.sys_pool)
            .await
            .unwrap();
        assert_eq!(rows, 0, "rejected provision must not create a project: {dsn}");
    }
}

#[tokio::test]
async fn put_connection_rejects_ssrf_targets_and_plaintext_dsns() {
    let env = start_system_db().await;
    let addr = serve_admin(&env).await;
    let account = new_account(&env.sys_pool).await;
    let cookie = seed_session(&env.sys_pool, account, "Owner").await;
    insert_owned_project(&env.sys_pool, account, "ds05-put", "k", "active").await;
    for (dsn, detail) in REJECTED {
        let resp = reqwest::Client::new()
            .put(format!("http://{addr}/admin/v1/projects/ds05-put/connection"))
            .header("Cookie", &cookie)
            .json(&serde_json::json!({ "backend_mode": "direct_pg", "dsn": dsn }))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 400, "{dsn}");
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!((body["error"].as_str(), body["detail"].as_str()), (Some("invalid_dsn"), Some(*detail)), "{dsn}");
        assert_eq!(column_bytes(&env.sys_pool, "ds05-put", "backend_pg_dsn_enc").await, None, "{dsn}");
    }
}
