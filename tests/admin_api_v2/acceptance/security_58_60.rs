// @driving_port @real-io
//! Audit findings #59 / #60 (docs/product/production-readiness-audit-2026-09-08.md).
//! #58 (OIDC callback disabled) is covered by the updated
//! `oidc_callback_with_valid_id_token_is_refused_while_disabled` in b06_oidc_billing.rs.
//!
//! #59: PATCH /admin/v1/projects/:id — role check, status allow-list, billing hold.
//!      Credential-cache eviction on status change is not observable through
//!      AdminTestContext (cache is not exposed), so it is not asserted here.
//! #60: POST /admin/v1/members/invite — role/email validation, no escalation;
//!      PATCH /admin/v1/members/:id/role — Admin cannot self-promote to Owner.

#[path = "../common/mod.rs"]
mod common;
use common::{totp_code_for_secret, AdminTestContext};
use uuid::Uuid;

const PROJECT: &str = "test-project-seeded-for-account";

async fn sign_in(ctx: &AdminTestContext, email: &str, password: &str, totp_secret: &str) -> String {
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/auth/signin"))
        .json(&serde_json::json!({
            "email": email,
            "password": password,
            "totp_code": totp_code_for_secret(totp_secret),
        }))
        .send()
        .await
        .expect("signin request failed");
    resp.headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(';').next())
        .map(|s| s.trim().to_string())
        .expect("no cookie in signin response")
}

async fn owner(ctx: &AdminTestContext) -> String {
    sign_in(ctx, &ctx.user_email, &ctx.user_password, &ctx.totp_secret_b32).await
}
async fn admin(ctx: &AdminTestContext) -> String {
    sign_in(ctx, &ctx.admin_email, &ctx.admin_password, &ctx.admin_totp_secret_b32).await
}
async fn viewer(ctx: &AdminTestContext) -> String {
    sign_in(ctx, &ctx.viewer_email, &ctx.viewer_password, &ctx.viewer_totp_secret_b32).await
}

async fn patch_project(ctx: &AdminTestContext, cookie: &str, body: serde_json::Value) -> u16 {
    ctx.client
        .patch(ctx.url(&format!("/admin/v1/projects/{PROJECT}")))
        .header("Cookie", cookie)
        .json(&body)
        .send()
        .await
        .expect("PATCH project failed")
        .status()
        .as_u16()
}

async fn invite(ctx: &AdminTestContext, cookie: &str, email: &str, role: &str) -> u16 {
    ctx.client
        .post(ctx.url("/admin/v1/members/invite"))
        .header("Cookie", cookie)
        .json(&serde_json::json!({"email": email, "role": role}))
        .send()
        .await
        .expect("invite failed")
        .status()
        .as_u16()
}

// ── #59 ───────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn viewer_cannot_patch_project() {
    let ctx = AdminTestContext::new().await;
    let cookie = viewer(&ctx).await;
    assert_eq!(patch_project(&ctx, &cookie, serde_json::json!({"name": "pwned"})).await, 403);

    let name: Option<String> = sqlx::query_scalar("SELECT name FROM projects WHERE id = $1")
        .bind(PROJECT)
        .fetch_one(&ctx.pool)
        .await
        .expect("read name");
    assert_ne!(name.as_deref(), Some("pwned"), "Viewer PATCH must not mutate the project");
}

#[tokio::test]
async fn admin_patch_with_unknown_status_returns_422() {
    let ctx = AdminTestContext::new().await;
    let cookie = admin(&ctx).await;
    assert_eq!(patch_project(&ctx, &cookie, serde_json::json!({"status": "bogus"})).await, 422);
    assert_eq!(patch_project(&ctx, &cookie, serde_json::json!({"status": "suspended"})).await, 200);
}

#[tokio::test]
async fn owner_cannot_reactivate_project_during_billing_hold() {
    let ctx = AdminTestContext::new().await;
    let cookie = owner(&ctx).await;
    let account_id = Uuid::parse_str(&ctx.account_id).expect("account uuid");

    sqlx::query(
        "INSERT INTO subscriptions (account_id, status) VALUES ($1, 'past_due') \
         ON CONFLICT (account_id) DO UPDATE SET status = 'past_due'",
    )
    .bind(account_id)
    .execute(&ctx.pool)
    .await
    .expect("seed past_due subscription");
    sqlx::query("UPDATE projects SET status = 'suspended' WHERE id = $1")
        .bind(PROJECT)
        .execute(&ctx.pool)
        .await
        .expect("suspend project");

    assert_eq!(patch_project(&ctx, &cookie, serde_json::json!({"status": "active"})).await, 409);

    let status: String = sqlx::query_scalar("SELECT status FROM projects WHERE id = $1")
        .bind(PROJECT)
        .fetch_one(&ctx.pool)
        .await
        .expect("read status");
    assert_eq!(status, "suspended");
}

// ── #60 ───────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn admin_cannot_invite_owner() {
    let ctx = AdminTestContext::new().await;
    let cookie = admin(&ctx).await;
    assert_eq!(invite(&ctx, &cookie, "boss@example.com", "Owner").await, 403);
}

#[tokio::test]
async fn invite_with_unknown_role_returns_422() {
    let ctx = AdminTestContext::new().await;
    let cookie = owner(&ctx).await;
    assert_eq!(invite(&ctx, &cookie, "x@example.com", "superuser").await, 422);
    assert_eq!(invite(&ctx, &cookie, "x@example.com", "owner").await, 422);
}

#[tokio::test]
async fn invite_with_invalid_email_returns_422() {
    let ctx = AdminTestContext::new().await;
    let cookie = owner(&ctx).await;
    for bad in ["", "no-at-sign", "a b@example.com", "a@b@example.com"] {
        assert_eq!(invite(&ctx, &cookie, bad, "Viewer").await, 422, "email {bad:?}");
    }
}

#[tokio::test]
async fn owner_can_invite_admin() {
    let ctx = AdminTestContext::new().await;
    let cookie = owner(&ctx).await;
    // invite_member's existing contract is 202 Accepted (email is async).
    assert_eq!(invite(&ctx, &cookie, "new-admin@example.com", "Admin").await, 202);
}

#[tokio::test]
async fn admin_cannot_promote_self_to_owner() {
    let ctx = AdminTestContext::new().await;
    let cookie = admin(&ctx).await;
    let status = ctx
        .client
        .patch(ctx.url(&format!("/admin/v1/members/{}/role", ctx.admin_id)))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({"role": "Owner"}))
        .send()
        .await
        .expect("change role failed")
        .status()
        .as_u16();
    assert_eq!(status, 403);
}
