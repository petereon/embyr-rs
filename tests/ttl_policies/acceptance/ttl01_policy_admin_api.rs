//! TTL01 — TTL policy admin API (go-live P5): role gating, project
//! ownership (404 for a foreign project), field-path validation, upsert and
//! delete semantics.
//!
//! Driving port: admin HTTP `GET/PUT/DELETE /admin/v1/projects/:id/ttl_policies`.
//! Docker-backed; compile-only locally, runs in CI.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{put_policy, SecurityRulesFullContext};

fn url(ctx: &SecurityRulesFullContext, project_id: &str, suffix: &str) -> String {
    ctx.admin_url(&format!("/admin/v1/projects/{project_id}/ttl_policies{suffix}"))
}

async fn list(ctx: &SecurityRulesFullContext, cookie: &str, project_id: &str) -> reqwest::Response {
    reqwest::Client::new()
        .get(url(ctx, project_id, ""))
        .header("Cookie", cookie)
        .send()
        .await
        .expect("list request failed")
}

async fn policy_rows(ctx: &SecurityRulesFullContext) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM ttl_policies WHERE project_id = $1")
        .bind(&ctx.project_id)
        .fetch_one(&ctx.sys_pool)
        .await
        .expect("count policies")
}

/// An Admin creates a policy; it is listed; PUT again with another field
/// replaces it (one TTL field per collection group); DELETE removes it.
///
/// @driving_port @real-io
#[tokio::test]
async fn admin_can_create_replace_list_and_delete_a_policy() {
    let ctx = SecurityRulesFullContext::new("ttl01-crud").await;
    let cookie = ctx.seed_session("admin@ttl.example", "Admin").await;

    let created = put_policy(
        &ctx,
        &cookie,
        serde_json::json!({"collection_group": "sessions", "field_path": "expireAt"}),
    )
    .await;
    assert_eq!(created.status().as_u16(), 200);
    let body: serde_json::Value = created.json().await.unwrap();
    assert_eq!(body["collection_group"], "sessions");
    assert_eq!(body["field_path"], "expireAt");
    assert_eq!(body["enabled"], true, "enabled defaults to true");

    let replaced = put_policy(
        &ctx,
        &cookie,
        serde_json::json!({"collection_group": "sessions", "field_path": "expiresAt", "enabled": false}),
    )
    .await;
    assert_eq!(replaced.status().as_u16(), 200);

    let listed: Vec<serde_json::Value> = list(&ctx, &cookie, &ctx.project_id).await.json().await.unwrap();
    assert_eq!(listed.len(), 1, "one policy per collection group");
    assert_eq!(listed[0]["field_path"], "expiresAt");
    assert_eq!(listed[0]["enabled"], false);

    let delete = |group: &str| {
        reqwest::Client::new()
            .delete(url(&ctx, &ctx.project_id, &format!("/{group}")))
            .header("Cookie", &cookie)
            .send()
    };
    assert_eq!(delete("sessions").await.unwrap().status().as_u16(), 204);
    assert_eq!(delete("sessions").await.unwrap().status().as_u16(), 404, "already gone");
    assert_eq!(policy_rows(&ctx).await, 0);
}

/// Viewer may list but not change; unauthenticated is rejected.
///
/// @driving_port @real-io @error
#[tokio::test]
async fn viewer_can_list_but_not_change_policies() {
    let ctx = SecurityRulesFullContext::new("ttl01-roles").await;
    let owner = ctx.seed_session("owner@ttl.example", "Owner").await;
    let viewer = ctx.seed_session("viewer@ttl.example", "Viewer").await;
    let body = serde_json::json!({"collection_group": "sessions", "field_path": "expireAt"});
    assert_eq!(put_policy(&ctx, &owner, body.clone()).await.status().as_u16(), 200);

    assert_eq!(list(&ctx, &viewer, &ctx.project_id).await.status().as_u16(), 200);
    assert_eq!(put_policy(&ctx, &viewer, body).await.status().as_u16(), 403);
    let del = reqwest::Client::new()
        .delete(url(&ctx, &ctx.project_id, "/sessions"))
        .header("Cookie", &viewer)
        .send()
        .await
        .unwrap();
    assert_eq!(del.status().as_u16(), 403);
    assert_eq!(policy_rows(&ctx).await, 1, "viewer changed nothing");

    let anon = reqwest::Client::new().get(url(&ctx, &ctx.project_id, "")).send().await.unwrap();
    assert_eq!(anon.status().as_u16(), 401);
}

/// A project of another account answers 404 on every verb (no existence
/// oracle) and is never modified.
///
/// @driving_port @real-io @error
#[tokio::test]
async fn foreign_project_is_not_found_on_every_verb() {
    let ctx = SecurityRulesFullContext::new("ttl01-ownership").await;
    let cookie = ctx.seed_session("owner@ttl.example", "Owner").await;

    let other_account: uuid::Uuid =
        sqlx::query_scalar("INSERT INTO accounts (name) VALUES ('Other Account') RETURNING id")
            .fetch_one(&ctx.sys_pool)
            .await
            .unwrap();
    sqlx::query(
        "INSERT INTO projects (id, account_id, backend_mode, api_key_hash_current, status, name) \
         VALUES ('ttl01-foreign', $1, 'direct_pg', 'unused-hash', 'active', 'ttl01-foreign')",
    )
    .bind(other_account)
    .execute(&ctx.sys_pool)
    .await
    .unwrap();

    assert_eq!(list(&ctx, &cookie, "ttl01-foreign").await.status().as_u16(), 404);
    let put = reqwest::Client::new()
        .put(url(&ctx, "ttl01-foreign", ""))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({"collection_group": "sessions", "field_path": "expireAt"}))
        .send()
        .await
        .unwrap();
    assert_eq!(put.status().as_u16(), 404);
    let del = reqwest::Client::new()
        .delete(url(&ctx, "ttl01-foreign", "/sessions"))
        .header("Cookie", &cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(del.status().as_u16(), 404);

    let stored: i64 = sqlx::query_scalar("SELECT count(*) FROM ttl_policies WHERE project_id = 'ttl01-foreign'")
        .fetch_one(&ctx.sys_pool)
        .await
        .unwrap();
    assert_eq!(stored, 0);
}

/// Unsafe or unsupported field paths / collection groups are rejected with
/// 422 before anything is stored (the field path is later interpolated into
/// SQL by the query builder: audit #11/#26).
///
/// @driving_port @real-io @error
#[tokio::test]
async fn invalid_field_path_or_collection_group_is_rejected() {
    let ctx = SecurityRulesFullContext::new("ttl01-validation").await;
    let cookie = ctx.seed_session("owner@ttl.example", "Owner").await;

    for (group, field) in [
        ("sessions", "x'); DROP TABLE documents; --"),
        ("sessions", "a b"),
        ("sessions", ""),
        ("sessions", "nested.field"),
        ("", "expireAt"),
        ("a/b", "expireAt"),
        ("__name__", "expireAt"),
    ] {
        let resp = put_policy(
            &ctx,
            &cookie,
            serde_json::json!({"collection_group": group, "field_path": field}),
        )
        .await;
        assert_eq!(resp.status().as_u16(), 422, "({group:?}, {field:?}) must be rejected");
    }
    assert_eq!(policy_rows(&ctx).await, 0, "nothing stored for rejected policies");
}

/// An agent-mode project cannot enforce TTL (agent RunQuery cannot express a
/// collection-group query with a limit): PUT answers 409 instead of storing a
/// policy that would never fire.
///
/// @driving_port @real-io @error
#[tokio::test]
async fn agent_mode_project_is_refused() {
    let ctx = SecurityRulesFullContext::new("ttl01-agent").await;
    let cookie = ctx.seed_session("owner@ttl.example", "Owner").await;
    sqlx::query("UPDATE projects SET backend_mode = 'agent' WHERE id = $1")
        .bind(&ctx.project_id)
        .execute(&ctx.sys_pool)
        .await
        .unwrap();

    let resp = put_policy(
        &ctx,
        &cookie,
        serde_json::json!({"collection_group": "sessions", "field_path": "expireAt"}),
    )
    .await;
    assert_eq!(resp.status().as_u16(), 409);
    assert_eq!(policy_rows(&ctx).await, 0);
}
