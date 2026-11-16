// @driving_port @real-io
//! account-deletion — DELETE /admin/v1/account.
//! docs/feature/account-deletion/feature-delta.md — AC-ADL-01..06, 09..11.
//! CI only (testcontainers Postgres; one scenario uses the real Stripe test-mode key).

#[path = "../common/mod.rs"]
mod common;
use common::AdminTestContext;
use serde_json::{json, Value};

const ACCOUNT_NAME: &str = "Test Account"; // seeded by AdminTestContext

// ── helpers ──────────────────────────────────────────────────────────────────

async fn sign_in(ctx: &AdminTestContext, email: &str, password: &str, code: String) -> (u16, Option<String>) {
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/auth/signin"))
        .json(&json!({"email": email, "password": password, "totp_code": code}))
        .send()
        .await
        .expect("signin failed");
    let cookie = resp
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(';').next())
        .map(|s| s.trim().to_string());
    (resp.status().as_u16(), cookie)
}

async fn owner(ctx: &AdminTestContext) -> String {
    sign_in(ctx, &ctx.user_email, &ctx.user_password, ctx.totp_code_now()).await.1.expect("owner cookie")
}
async fn admin(ctx: &AdminTestContext) -> String {
    sign_in(ctx, &ctx.admin_email, &ctx.admin_password, ctx.admin_totp_code_now()).await.1.expect("admin cookie")
}
async fn viewer(ctx: &AdminTestContext) -> String {
    sign_in(ctx, &ctx.viewer_email, &ctx.viewer_password, ctx.viewer_totp_code_now()).await.1.expect("viewer cookie")
}

fn body(ctx: &AdminTestContext) -> Value {
    json!({"password": ctx.user_password, "totp_code": ctx.totp_code_now(), "confirm_account_name": ACCOUNT_NAME})
}

async fn delete_account(ctx: &AdminTestContext, auth: (&str, &str), body: Value) -> (u16, Option<String>, Value) {
    let resp = ctx
        .client
        .delete(ctx.url("/admin/v1/account"))
        .header(auth.0, auth.1)
        .json(&body)
        .send()
        .await
        .expect("delete failed");
    let status = resp.status().as_u16();
    let set_cookie = resp.headers().get("set-cookie").and_then(|v| v.to_str().ok()).map(str::to_string);
    (status, set_cookie, resp.json().await.unwrap_or(Value::Null))
}

async fn list_projects_status(ctx: &AdminTestContext, cookie: &str) -> u16 {
    ctx.client
        .get(ctx.url("/admin/v1/projects"))
        .header("Cookie", cookie)
        .send()
        .await
        .expect("list projects")
        .status()
        .as_u16()
}

async fn count(ctx: &AdminTestContext, sql: &str) -> i64 {
    sqlx::query_scalar(sql)
        .bind(uuid::Uuid::parse_str(&ctx.account_id).unwrap())
        .fetch_one(&ctx.pool)
        .await
        .expect("count query")
}

async fn account_deleted(ctx: &AdminTestContext) -> bool {
    count(ctx, "SELECT count(*) FROM accounts WHERE id = $1 AND deleted_at IS NOT NULL").await == 1
}

async fn seed_sdk_key(ctx: &AdminTestContext) {
    sqlx::query(
        "INSERT INTO sdk_api_keys (project_id, name, key_hash, prefix) \
         VALUES ('test-project-seeded-for-account', 'k', $1, 'embyr_sdk_x')",
    )
    .bind(blake3::hash(b"sdk-key-1").as_bytes().to_vec())
    .execute(&ctx.pool)
    .await
    .expect("sdk key");
}

async fn seed_admin_key(ctx: &AdminTestContext, plaintext: &str) {
    sqlx::query(
        "INSERT INTO admin_api_keys (account_id, name, key_hash, prefix, role, member_id) \
         VALUES ($1, 'k', $2, 'embyr_adm_x', 'Owner', $3)",
    )
    .bind(uuid::Uuid::parse_str(&ctx.account_id).unwrap())
    .bind(blake3::hash(plaintext.as_bytes()).as_bytes().to_vec())
    .bind(uuid::Uuid::parse_str(&ctx.user_id).unwrap())
    .execute(&ctx.pool)
    .await
    .expect("admin key");
}

async fn seed_subscription(ctx: &AdminTestContext, stripe_subscription_id: &str) {
    sqlx::query(
        "INSERT INTO subscriptions (account_id, plan, status, stripe_subscription_id) \
         VALUES ($1, 'pro', 'active', $2)",
    )
    .bind(uuid::Uuid::parse_str(&ctx.account_id).unwrap())
    .bind(stripe_subscription_id)
    .execute(&ctx.pool)
    .await
    .expect("subscription");
}

// ── AC-ADL-01/05/06/09/10: Owner deletes; everything is cut off ───────────────

#[tokio::test]
async fn owner_deletion_cuts_off_the_whole_account_and_a_second_delete_is_401() {
    let ctx = AdminTestContext::new().await;
    let owner_cookie = owner(&ctx).await;
    let admin_cookie = admin(&ctx).await;
    seed_sdk_key(&ctx).await;
    seed_admin_key(&ctx, "embyr_adm_to_be_revoked").await;
    sqlx::query(
        "INSERT INTO invitations (account_id, email, role, expires_at) \
         VALUES ($1, 'pending@example.com', 'Viewer', now() + interval '1 day')",
    )
    .bind(uuid::Uuid::parse_str(&ctx.account_id).unwrap())
    .execute(&ctx.pool)
    .await
    .expect("invitation");

    let (status, set_cookie, resp) = delete_account(&ctx, ("Cookie", &owner_cookie), body(&ctx)).await;

    // AC-ADL-01
    assert_eq!(status, 202, "{resp}");
    assert!(set_cookie.expect("set-cookie").contains("Max-Age=0"));
    let deleted_at = chrono::DateTime::parse_from_rfc3339(resp["deleted_at"].as_str().unwrap()).unwrap();
    let purge_after = chrono::DateTime::parse_from_rfc3339(resp["purge_after"].as_str().unwrap()).unwrap();
    assert_eq!((purge_after - deleted_at).num_hours(), 168);
    assert_eq!(resp["projects_deleted"], 1); // the seeded 'deleted' project was already deleted
    assert_eq!(resp["subscription"], "none");

    // AC-ADL-05
    assert!(account_deleted(&ctx).await);
    assert_eq!(
        count(&ctx, "SELECT count(*) FROM projects WHERE account_id = $1 AND status <> 'deleted'").await,
        0
    );
    assert_eq!(
        count(
            &ctx,
            "SELECT count(*) FROM projects WHERE account_id = $1 \
             AND id = 'test-project-seeded-for-account' AND deleted_at IS NOT NULL"
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &ctx,
            "SELECT count(*) FROM sdk_api_keys WHERE revoked_at IS NULL \
             AND project_id IN (SELECT id FROM projects WHERE account_id = $1)"
        )
        .await,
        0
    );
    assert_eq!(count(&ctx, "SELECT count(*) FROM sessions WHERE account_id = $1 AND expires_at > now()").await, 0);
    assert_eq!(count(&ctx, "SELECT count(*) FROM admin_api_keys WHERE account_id = $1 AND revoked_at IS NULL").await, 0);
    assert_eq!(
        count(&ctx, "SELECT count(*) FROM invitations WHERE account_id = $1 AND accepted_at IS NULL AND revoked_at IS NULL").await,
        0
    );

    // AC-ADL-06 + AC-ADL-09: every former session (the Admin's too) is dead; nobody can sign in.
    assert_eq!(list_projects_status(&ctx, &owner_cookie).await, 401);
    assert_eq!(list_projects_status(&ctx, &admin_cookie).await, 401);
    let (status, cookie) = sign_in(&ctx, &ctx.user_email, &ctx.user_password, ctx.totp_code_now()).await;
    assert_eq!((status, cookie), (401, None));
    let (status, _) = sign_in(&ctx, &ctx.admin_email, &ctx.admin_password, ctx.admin_totp_code_now()).await;
    assert_eq!(status, 401);

    // AC-ADL-10: second delete with the (now invalid) session is 401.
    let (status, _, _) = delete_account(&ctx, ("Cookie", &owner_cookie), body(&ctx)).await;
    assert_eq!(status, 401);
}

// ── AC-ADL-01: recovery code is an accepted second factor ─────────────────────

#[tokio::test]
async fn owner_can_delete_with_a_recovery_code() {
    let ctx = AdminTestContext::new().await;
    let cookie = owner(&ctx).await;
    let (status, _, resp) = delete_account(
        &ctx,
        ("Cookie", &cookie),
        json!({"password": ctx.user_password, "recovery_code": "RECOV-0001", "confirm_account_name": ACCOUNT_NAME}),
    )
    .await;
    assert_eq!(status, 202, "{resp}");
    assert!(account_deleted(&ctx).await);
}

// ── AC-ADL-02: only an Owner with a user session may delete ───────────────────

#[tokio::test]
async fn admin_viewer_and_bearer_key_are_refused_and_nothing_changes() {
    let ctx = AdminTestContext::new().await;
    seed_admin_key(&ctx, "embyr_adm_owner_bearer").await;

    let admin_cookie = admin(&ctx).await;
    let viewer_cookie = viewer(&ctx).await;
    let admin_body = json!({"password": ctx.admin_password, "totp_code": ctx.admin_totp_code_now(), "confirm_account_name": ACCOUNT_NAME});
    let viewer_body = json!({"password": ctx.viewer_password, "totp_code": ctx.viewer_totp_code_now(), "confirm_account_name": ACCOUNT_NAME});

    assert_eq!(delete_account(&ctx, ("Cookie", &admin_cookie), admin_body).await.0, 403);
    assert_eq!(delete_account(&ctx, ("Cookie", &viewer_cookie), viewer_body).await.0, 403);
    // Owner-role Bearer admin key: no user session to re-authenticate.
    assert_eq!(
        delete_account(&ctx, ("Authorization", "Bearer embyr_adm_owner_bearer"), body(&ctx)).await.0,
        403
    );

    assert!(!account_deleted(&ctx).await);
    assert_eq!(list_projects_status(&ctx, &admin_cookie).await, 200);
}

// ── AC-ADL-02/03: bad credentials change nothing; lockout still applies ───────

#[tokio::test]
async fn wrong_name_password_or_code_changes_nothing_and_lockout_applies() {
    let ctx = AdminTestContext::new().await;
    let cookie = owner(&ctx).await;

    // Right credentials, wrong typed name: 400.
    let mut b = body(&ctx);
    b["confirm_account_name"] = json!("not the name");
    assert_eq!(delete_account(&ctx, ("Cookie", &cookie), b).await.0, 400);

    // Wrong password: signin's own body.
    let mut b = body(&ctx);
    b["password"] = json!("wrong-password");
    let (status, _, resp) = delete_account(&ctx, ("Cookie", &cookie), b).await;
    assert_eq!((status, resp["message"].as_str()), (401, Some("Invalid credentials")));

    // Missing second factor.
    let (status, _, resp) = delete_account(
        &ctx,
        ("Cookie", &cookie),
        json!({"password": ctx.user_password, "confirm_account_name": ACCOUNT_NAME}),
    )
    .await;
    assert_eq!((status, resp["message"].as_str()), (401, Some("Invalid or expired code")));

    // Three wrong codes, then even the right code is locked out (429): not a lockout bypass.
    for _ in 0..3 {
        let mut b = body(&ctx);
        b["totp_code"] = json!(ctx.totp_code_wrong());
        let (status, _, resp) = delete_account(&ctx, ("Cookie", &cookie), b).await;
        assert_eq!((status, resp["message"].as_str()), (401, Some("Invalid or expired code")));
    }
    assert_eq!(delete_account(&ctx, ("Cookie", &cookie), body(&ctx)).await.0, 429);

    assert!(!account_deleted(&ctx).await);
    assert_eq!(list_projects_status(&ctx, &cookie).await, 200);
}

// ── AC-ADL-04: Stripe failure -> 502 and nothing is deleted ───────────────────

#[tokio::test]
async fn stripe_failure_returns_502_and_deletes_nothing() {
    // Harness gateway holds a placeholder key: real Stripe rejects the cancel call.
    let ctx = AdminTestContext::new().await;
    seed_subscription(&ctx, "sub_does_not_matter").await;
    let cookie = owner(&ctx).await;

    let (status, _, _) = delete_account(&ctx, ("Cookie", &cookie), body(&ctx)).await;

    assert_eq!(status, 502);
    assert!(!account_deleted(&ctx).await);
    assert_eq!(
        count(&ctx, "SELECT count(*) FROM projects WHERE account_id = $1 AND status = 'active'").await,
        1
    );
    assert_eq!(list_projects_status(&ctx, &cookie).await, 200);
    assert_eq!(
        count(&ctx, "SELECT count(*) FROM subscriptions WHERE account_id = $1 AND status = 'active'").await,
        1
    );
}

// ── AC-ADL-04: real Stripe — cancel, and retry after a partial failure ────────

#[tokio::test]
async fn real_stripe_subscription_is_cancelled_and_retry_after_partial_failure_converges() {
    use embyr_server::adapters::stripe_gateway::StripeGateway;

    let key = std::env::var("STRIPE_SECRET_KEY")
        .expect("STRIPE_SECRET_KEY (Stripe test-mode key) is required, as for card_payments_backend tests");
    let gateway = StripeGateway::new(key.clone());
    let customer = gateway.get_or_create_customer(uuid::Uuid::new_v4()).await.expect("customer");
    let sub = gateway
        .upsert_subscription(&customer, embyr_core::admin::SubscriptionPlan::Pro)
        .await
        .expect("subscription");

    let ctx = AdminTestContext::new_with_stripe_key(&key).await;
    seed_subscription(&ctx, &sub.stripe_subscription_id).await;
    let cookie = owner(&ctx).await;

    // Crash between the Stripe call and the local transaction: Stripe is already cancelled.
    gateway.cancel_subscription(&sub.stripe_subscription_id).await.expect("pre-cancel");

    // The retried request treats "already cancelled" as success and completes the delete.
    let (status, _, resp) = delete_account(&ctx, ("Cookie", &cookie), body(&ctx)).await;
    assert_eq!(status, 202, "{resp}");
    assert_eq!(resp["subscription"], "cancelled");
    assert!(account_deleted(&ctx).await);
    assert_eq!(
        count(&ctx, "SELECT count(*) FROM subscriptions WHERE account_id = $1 AND status = 'canceled'").await,
        1
    );
}
