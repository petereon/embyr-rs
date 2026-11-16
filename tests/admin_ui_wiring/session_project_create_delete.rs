// @driving_port @real-io @adapter-integration
//! admin-ui-api-wiring slice 05 (AC-AUW-25/26): `POST /admin/v1/projects` and
//! `DELETE /admin/v1/projects/:id` accept a session cookie as well as the operator Bearer key,
//! against the real admin router with real Postgres (system + customer). CI only: Docker.
//!
//!   create  Owner and Admin create under THEIR account (body `account_id` ignored), key shown once
//!           and usable over gRPC; Viewer 403; no credential 401; billing hold 409; BYOC DSN validator
//!           applies on the session path; operator Bearer still creates an unowned project
//!   delete  Owner only (Admin 403); other account's project 403, unknown 404; deleted project's key
//!           stops authenticating at once (credential cache evicted); operator Bearer still deletes

#[path = "../dsn_single_store/common/mod.rs"]
mod common;
use common::*;

use embyr_server::TestServer;

struct Ctx {
    env: Env,
    _cust: testcontainers_modules::testcontainers::ContainerAsync<testcontainers_modules::postgres::Postgres>,
    cust_url: String,
    cust_pool: sqlx::PgPool,
    server: TestServer,
    account: uuid::Uuid,
    owner: String,
    admin: String,
    viewer: String,
    client: reqwest::Client,
}

async fn ctx() -> Ctx {
    let env = start_system_db().await;
    let (cust, cust_url, cust_pool) = start_ready_customer_db().await;
    let server = server_for(&env).await;
    let account = new_account(&env.sys_pool).await;
    let owner = seed_session(&env.sys_pool, account, "Owner").await;
    let admin = seed_session(&env.sys_pool, account, "Admin").await;
    let viewer = seed_session(&env.sys_pool, account, "Viewer").await;
    Ctx { env, _cust: cust, cust_url, cust_pool, server, account, owner, admin, viewer, client: reqwest::Client::new() }
}

enum Auth<'a> {
    Cookie(&'a str),
    Operator,
    None,
}

impl Ctx {
    fn url(&self, path: &str) -> String {
        format!("http://{}/admin/v1{path}", self.server.admin_addr)
    }

    fn with_auth(&self, rb: reqwest::RequestBuilder, auth: Auth<'_>) -> reqwest::RequestBuilder {
        match auth {
            Auth::Cookie(c) => rb.header("Cookie", c),
            Auth::Operator => rb.header("Authorization", format!("Bearer {ADMIN_KEY}")),
            Auth::None => rb,
        }
    }

    async fn create(&self, auth: Auth<'_>, body: serde_json::Value) -> (u16, serde_json::Value) {
        let resp = self.with_auth(self.client.post(self.url("/projects")), auth).json(&body).send().await.unwrap();
        (resp.status().as_u16(), resp.json().await.unwrap_or_default())
    }

    async fn create_direct(&self, auth: Auth<'_>, id: &str) -> (u16, serde_json::Value) {
        self.create(auth, serde_json::json!({ "project_id": id, "dsn": self.cust_url, "backend_mode": "direct_pg" }))
            .await
    }

    async fn delete(&self, auth: Auth<'_>, id: &str) -> u16 {
        self.with_auth(self.client.delete(self.url(&format!("/projects/{id}"))), auth)
            .send()
            .await
            .unwrap()
            .status()
            .as_u16()
    }

    async fn project_row(&self, id: &str) -> Option<(Option<uuid::Uuid>, Option<String>, String)> {
        sqlx::query_as("SELECT account_id, name, status FROM projects WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.env.sys_pool)
            .await
            .unwrap()
    }
}

fn api_key(body: &serde_json::Value) -> String {
    body["api_key"].as_str().expect("api_key in 201 body").to_string()
}

/// AC-AUW-25: Owner and Admin both create; the project lands in the caller's account whatever the
/// body claims; the key is returned once and authenticates; the list shows it.
#[tokio::test]
async fn owner_and_admin_create_under_their_own_account_ignoring_body_account_id() {
    let c = ctx().await;
    let other = new_account(&c.env.sys_pool).await;

    for (cookie, id) in [(&c.owner, "sp-owner-db"), (&c.admin, "sp-admin-db")] {
        let (status, body) = c
            .create(
                Auth::Cookie(cookie),
                serde_json::json!({
                    "project_id": id, "dsn": c.cust_url, "backend_mode": "direct_pg",
                    "name": "  Orders  ", "account_id": other.to_string(),
                }),
            )
            .await;
        assert_eq!(status, 201, "{id}: {body}");
        assert_eq!(body["project_id"], id);
        assert_eq!(
            c.project_row(id).await,
            Some((Some(c.account), Some("Orders".to_string()), "active".to_string())),
            "{id} must belong to the caller's account, not the body's"
        );

        seed_greeting(&c.cust_pool, id, "hello").await;
        assert_eq!(get_greeting(&c.server, id, &api_key(&body)).await.unwrap(), "hello");
    }

    let list: serde_json::Value = c.client.get(c.url("/projects")).header("Cookie", &c.owner).send().await.unwrap().json().await.unwrap();
    let ids: Vec<&str> = list.as_array().unwrap().iter().map(|p| p["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"sp-owner-db") && ids.contains(&"sp-admin-db"), "{ids:?}");
}

/// Viewer 403 and no credential 401 create nothing; invalid name 400; the BYOC DSN validator runs
/// on the session path (shape rule is always on, independent of the dev flag).
#[tokio::test]
async fn create_refused_for_viewer_anonymous_bad_name_and_invalid_dsn() {
    let c = ctx().await;

    assert_eq!(c.create_direct(Auth::Cookie(&c.viewer), "sp-viewer").await.0, 403);
    assert_eq!(c.create_direct(Auth::None, "sp-anon").await.0, 401);
    let (status, body) = c
        .create(
            Auth::Cookie(&c.owner),
            serde_json::json!({ "project_id": "sp-badname", "dsn": c.cust_url, "name": "x".repeat(101) }),
        )
        .await;
    assert_eq!((status, body["error"].as_str()), (400, Some("invalid_name")));
    let (status, body) = c
        .create(
            Auth::Cookie(&c.owner),
            serde_json::json!({ "project_id": "sp-baddsn", "dsn": "mysql://u:p@8.8.8.8/db?sslmode=require" }),
        )
        .await;
    assert_eq!((status, body["error"].as_str(), body["detail"].as_str()), (400, Some("invalid_dsn"), Some("invalid_dsn")));

    for id in ["sp-viewer", "sp-anon", "sp-badname", "sp-baddsn"] {
        assert_eq!(c.project_row(id).await, None, "{id} must not exist");
    }
}

/// An account on a billing hold cannot create (409); once the hold lifts it can.
#[tokio::test]
async fn billing_hold_blocks_session_create() {
    let c = ctx().await;
    for (i, hold) in ["past_due", "free_cap_exceeded"].iter().enumerate() {
        sqlx::query(
            "INSERT INTO subscriptions (account_id, status) VALUES ($1, $2) \
             ON CONFLICT (account_id) DO UPDATE SET status = $2",
        )
        .bind(c.account)
        .bind(hold)
        .execute(&c.env.sys_pool)
        .await
        .unwrap();
        let id = format!("sp-hold-{i}");
        let (status, body) = c.create_direct(Auth::Cookie(&c.owner), &id).await;
        assert_eq!((status, body["error"].as_str()), (409, Some("account_billing_hold")), "{hold}");
        assert_eq!(c.project_row(&id).await, None);
    }

    sqlx::query("UPDATE subscriptions SET status = 'active' WHERE account_id = $1")
        .bind(c.account)
        .execute(&c.env.sys_pool)
        .await
        .unwrap();
    assert_eq!(c.create_direct(Auth::Cookie(&c.owner), "sp-hold-lifted").await.0, 201);
}

/// Operator Bearer keeps creating (no account) and deleting (any project), unchanged.
#[tokio::test]
async fn operator_bearer_still_creates_and_deletes() {
    let c = ctx().await;

    let (status, body) = c.create_direct(Auth::Operator, "sp-op-db").await;
    assert_eq!(status, 201, "{body}");
    assert_eq!(c.project_row("sp-op-db").await, Some((None, None, "active".to_string())));

    assert_eq!(c.delete(Auth::Operator, "sp-op-db").await, 200);
    assert_eq!(c.project_row("sp-op-db").await.map(|r| r.2), Some("deleted".to_string()));
    assert_eq!(c.delete(Auth::Operator, "sp-op-db").await, 404);

    // Operator may also delete a customer-owned project.
    insert_owned_project(&c.env.sys_pool, c.account, "sp-op-del-owned", "k", "active").await;
    assert_eq!(c.delete(Auth::Operator, "sp-op-del-owned").await, 200);
}

/// AC-AUW-26: delete is Owner-only and account-scoped; after delete the key stops working at once
/// even though a request just warmed the credential cache.
#[tokio::test]
async fn delete_is_owner_only_account_scoped_and_kills_the_key() {
    let c = ctx().await;
    let (status, body) = c.create_direct(Auth::Cookie(&c.owner), "sp-del-db").await;
    assert_eq!(status, 201, "{body}");
    let key = api_key(&body);
    seed_greeting(&c.cust_pool, "sp-del-db", "alive").await;
    assert_eq!(get_greeting(&c.server, "sp-del-db", &key).await.unwrap(), "alive");

    let other_account = new_account(&c.env.sys_pool).await;
    let stranger = seed_session(&c.env.sys_pool, other_account, "Owner").await;

    assert_eq!(c.delete(Auth::None, "sp-del-db").await, 401);
    assert_eq!(c.delete(Auth::Cookie(&c.viewer), "sp-del-db").await, 403);
    assert_eq!(c.delete(Auth::Cookie(&c.admin), "sp-del-db").await, 403);
    assert_eq!(c.delete(Auth::Cookie(&stranger), "sp-del-db").await, 403);
    assert_eq!(c.delete(Auth::Cookie(&c.owner), "sp-no-such").await, 404);
    assert_eq!(c.project_row("sp-del-db").await.map(|r| r.2), Some("active".to_string()));
    assert_eq!(get_greeting(&c.server, "sp-del-db", &key).await.unwrap(), "alive");

    assert_eq!(c.delete(Auth::Cookie(&c.owner), "sp-del-db").await, 200);
    let (status, deleted_at): (String, Option<chrono::DateTime<chrono::Utc>>) =
        sqlx::query_as("SELECT status, deleted_at FROM projects WHERE id = 'sp-del-db'")
            .fetch_one(&c.env.sys_pool)
            .await
            .unwrap();
    assert!(status == "deleted" && deleted_at.is_some());
    assert!(get_greeting(&c.server, "sp-del-db", &key).await.is_err(), "deleted project's key must stop working");
    assert_eq!(c.delete(Auth::Cookie(&c.owner), "sp-del-db").await, 404);
}
