//! Slice 3 (ADR-083): GET/PUT /admin/v1/projects/:id/connection against the real admin router
//! (shared credential cache with the gRPC service). AC-DSS-08, 09, 10 (agent -> direct_pg; the
//! direct_pg -> agent direction needs the mTLS mock agent and lives in us_12_agent_backend.rs),
//! 11, 12, 13 (writes land under the current key; previous-key decrypt is unit-tested in
//! adapters::connection_secrets).

#[path = "../common/mod.rs"]
mod common;
use common::*;
use sqlx::migrate::Migrate;
use sqlx::Connection;

struct Ctx {
    env: Env,
    server: embyr_server::TestServer,
    account: uuid::Uuid,
    owner: String,
    admin: String,
    viewer: String,
    client: reqwest::Client,
}

async fn ctx() -> Ctx {
    let env = start_system_db().await;
    let server = server_for(&env).await;
    let account = new_account(&env.sys_pool).await;
    let owner = seed_session(&env.sys_pool, account, "Owner").await;
    let admin = seed_session(&env.sys_pool, account, "Admin").await;
    let viewer = seed_session(&env.sys_pool, account, "Viewer").await;
    Ctx { env, server, account, owner, admin, viewer, client: reqwest::Client::new() }
}

impl Ctx {
    fn url(&self, project: &str) -> String {
        format!("http://{}/admin/v1/projects/{project}/connection", self.server.admin_addr)
    }

    async fn put(&self, cookie: &str, project: &str, body: serde_json::Value) -> (u16, serde_json::Value) {
        let r = self.client.put(self.url(project)).header("Cookie", cookie).json(&body).send().await.unwrap();
        (r.status().as_u16(), r.json().await.unwrap_or_default())
    }

    async fn get(&self, cookie: &str, project: &str) -> (u16, String) {
        let r = self.client.get(self.url(project)).header("Cookie", cookie).send().await.unwrap();
        (r.status().as_u16(), r.text().await.unwrap())
    }

    async fn set_server_dsn(&self, project: &str, dsn: &str) {
        sqlx::query("UPDATE projects SET backend_pg_dsn_enc = $2 WHERE id = $1")
            .bind(project)
            .bind(keys().encrypt(dsn))
            .execute(&self.env.sys_pool)
            .await
            .unwrap();
    }
}

fn dsn_put(dsn: &str) -> serde_json::Value {
    serde_json::json!({ "backend_mode": "direct_pg", "dsn": dsn })
}

/// AC-DSS-12: GET returns mode + masked summary only, for any role, never credentials.
#[tokio::test]
async fn get_returns_masked_summary_and_never_credentials() {
    let c = ctx().await;
    insert_owned_project(&c.env.sys_pool, c.account, "ds04-get", "k-get", "active").await;
    c.set_server_dsn("ds04-get", "postgres://admin:s3cr3t@db.prod.internal:6432/embyr_prod?sslmode=require&password=q9")
        .await;

    let (status, body) = c.get(&c.viewer, "ds04-get").await;
    assert_eq!(status, 200);
    for leaked in ["s3cr3t", "admin", "q9", "postgres://"] {
        assert!(!body.contains(leaked), "response leaked {leaked}: {body}");
    }
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["backend_mode"], "direct_pg");
    assert_eq!(v["summary"], serde_json::json!({"host":"db.prod.internal","port":6432,"database":"embyr_prod","tls":true}));
    assert_eq!(v["secret_stored"], true);
    assert_eq!(v["converted"], true);

    // Unconverted legacy row: the server cannot read it, and says so.
    insert_owned_project(&c.env.sys_pool, c.account, "ds04-legacy", "k-leg", "active").await;
    sqlx::query("UPDATE projects SET ecies_encrypted_dsn = $1 WHERE id = 'ds04-legacy'")
        .bind(ecies_of("k-leg", "postgres://u:p@h/db"))
        .execute(&c.env.sys_pool)
        .await
        .unwrap();
    let (_, body) = c.get(&c.owner, "ds04-legacy").await;
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!((v["summary"].is_null(), v["secret_stored"].clone(), v["converted"].clone()),
               (true, serde_json::json!(true), serde_json::json!(false)));
}

/// AC-DSS-08: unreachable / not-prepped / stale targets → 400 with the specific error and the
/// stored value untouched.
#[tokio::test]
async fn failed_probe_returns_400_and_leaves_stored_value_unchanged() {
    let c = ctx().await;
    insert_owned_project(&c.env.sys_pool, c.account, "ds04-probe", "k-probe", "active").await;
    c.set_server_dsn("ds04-probe", "postgres://keep:me@10.9.9.9/db").await;
    let before = column_bytes(&c.env.sys_pool, "ds04-probe", "backend_pg_dsn_enc").await;

    let (_not_prepped, not_prepped_url) = start_postgres().await;
    let (_stale, stale_url) = start_postgres().await;
    let mut conn = sqlx::PgConnection::connect(&stale_url).await.unwrap();
    conn.ensure_migrations_table().await.unwrap();
    let migrator = sqlx::migrate!("../../migrations/customer");
    conn.apply(migrator.iter().next().unwrap()).await.unwrap();
    drop(conn);

    for (dsn, expected) in [
        ("postgres://u:p@127.0.0.1:1/none".to_string(), "backend_unavailable"),
        (not_prepped_url, "customer_db_not_prepped"),
        (stale_url, "customer_db_schema_stale"),
    ] {
        let (status, body) = c.put(&c.owner, "ds04-probe", dsn_put(&dsn)).await;
        assert_eq!((status, body["error"].as_str().unwrap_or("")), (400, expected));
        assert_eq!(column_bytes(&c.env.sys_pool, "ds04-probe", "backend_pg_dsn_enc").await, before);
    }
}

/// AC-DSS-09 (+13 write side): a probe-passing DSN is saved under the current server key, the legacy
/// copy is removed, the cache entry is evicted and the very next Firestore request reads the new DB.
#[tokio::test]
async fn put_saves_dsn_evicts_cache_and_next_request_uses_new_database() {
    let c = ctx().await;
    let (_c1, url_one, pool_one) = start_ready_customer_db().await;
    let (_c2, url_two, pool_two) = start_ready_customer_db().await;
    insert_owned_project(&c.env.sys_pool, c.account, "ds04-sw", "k-sw", "active").await;
    sqlx::query("UPDATE projects SET ecies_encrypted_dsn = $1 WHERE id = 'ds04-sw'")
        .bind(ecies_of("k-sw", &url_one))
        .execute(&c.env.sys_pool)
        .await
        .unwrap();
    seed_greeting(&pool_one, "ds04-sw", "one").await;
    seed_greeting(&pool_two, "ds04-sw", "two").await;
    assert_eq!(get_greeting(&c.server, "ds04-sw", "k-sw").await.unwrap(), "one"); // warms the cache

    let (status, body) = c.put(&c.admin, "ds04-sw", dsn_put(&url_two)).await;
    assert_eq!((status, body["probe"].as_str()), (200, Some("ok")));

    let stored = column_bytes(&c.env.sys_pool, "ds04-sw", "backend_pg_dsn_enc").await.unwrap();
    assert!(!stored.windows(url_two.len()).any(|w| w == url_two.as_bytes()), "DSN must not be stored in clear");
    assert_eq!(keys().decrypt(&stored).as_deref(), Some(url_two.as_str()));
    assert_eq!(column_bytes(&c.env.sys_pool, "ds04-sw", "ecies_encrypted_dsn").await, None);
    assert_eq!(get_greeting(&c.server, "ds04-sw", "k-sw").await.unwrap(), "two");
}

/// AC-DSS-10 (agent -> direct_pg): backend_mode and the new secrets are written, the agent
/// endpoint/bundles (server and legacy) are NULLed, and the cache is evicted.
#[tokio::test]
async fn switching_agent_to_direct_pg_nulls_agent_secrets() {
    let c = ctx().await;
    let (_c1, url, pool) = start_ready_customer_db().await;
    insert_owned_project(&c.env.sys_pool, c.account, "ds04-mode", "k-mode", "active").await;
    sqlx::query(
        "UPDATE projects SET backend_mode = 'agent', backend_agent_endpoint = 'agent.example:9191', \
         agent_tls_bundle_srv_enc = $1, agent_tls_bundle_enc = $2 WHERE id = 'ds04-mode'",
    )
    .bind(vec![1u8; 40])
    .bind(vec![2u8; 40])
    .execute(&c.env.sys_pool)
    .await
    .unwrap();
    seed_greeting(&pool, "ds04-mode", "direct").await;

    let (status, _) = c.put(&c.owner, "ds04-mode", dsn_put(&url)).await;
    assert_eq!(status, 200);

    let (mode, endpoint): (String, Option<String>) =
        sqlx::query_as("SELECT backend_mode, backend_agent_endpoint FROM projects WHERE id = 'ds04-mode'")
            .fetch_one(&c.env.sys_pool)
            .await
            .unwrap();
    assert_eq!((mode.as_str(), endpoint), ("direct_pg", None));
    for col in ["agent_tls_bundle_srv_enc", "agent_tls_bundle_enc"] {
        assert_eq!(column_bytes(&c.env.sys_pool, "ds04-mode", col).await, None, "{col}");
    }
    assert_eq!(server_store_dsn(&c.env.sys_pool, "ds04-mode").await.as_deref(), Some(url.as_str()));
    assert_eq!(get_greeting(&c.server, "ds04-mode", "k-mode").await.unwrap(), "direct");
}

/// AC-DSS-11: Viewer 403, foreign account 403, suspended 409, deleted 404 -- and no write.
#[tokio::test]
async fn put_is_refused_for_viewer_foreign_suspended_and_deleted_without_writing() {
    let c = ctx().await;
    let other = new_account(&c.env.sys_pool).await;
    insert_owned_project(&c.env.sys_pool, c.account, "ds04-own", "k1", "active").await;
    insert_owned_project(&c.env.sys_pool, other, "ds04-foreign", "k2", "active").await;
    insert_owned_project(&c.env.sys_pool, c.account, "ds04-susp", "k3", "suspended").await;
    insert_owned_project(&c.env.sys_pool, c.account, "ds04-del", "k4", "deleted").await;

    let body = dsn_put("postgres://u:p@127.0.0.1:1/none");
    for (cookie, project, expected) in [
        (&c.viewer, "ds04-own", 403),
        (&c.owner, "ds04-foreign", 403),
        (&c.owner, "ds04-susp", 409),
        (&c.owner, "ds04-del", 404),
    ] {
        let (status, _) = c.put(cookie, project, body.clone()).await;
        assert_eq!(status, expected, "{project}");
        assert_eq!(column_bytes(&c.env.sys_pool, project, "backend_pg_dsn_enc").await, None, "{project}");
    }
}

/// Switching TO a secret-manager mode is not offered here.
#[tokio::test]
async fn put_rejects_secret_manager_target_modes() {
    let c = ctx().await;
    insert_owned_project(&c.env.sys_pool, c.account, "ds04-aws", "k5", "active").await;
    let (status, body) = c.put(&c.owner, "ds04-aws", serde_json::json!({"backend_mode": "aws_secret"})).await;
    assert_eq!((status, body["error"].as_str()), (422, Some("unsupported_backend_mode")));
}
