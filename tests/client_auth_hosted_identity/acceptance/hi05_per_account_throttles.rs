//! HI05 — security finding X5: per-(project, email) throttles on the
//! end-user REST surface. `signInWithPassword` (credential stuffing) and
//! `sendOobCode` (email bombing) were bounded only by the project-wide bucket.
//!
//! Burst sizes (sign-in 5, reset 3) with slow refill: assertions depend only
//! on request COUNT, never on a wall-clock window.
//!
//! Driving port: REST :8081.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::HostedIdentityFullContext;

async fn post(
    ctx: &HostedIdentityFullContext,
    action: &str,
    body: serde_json::Value,
) -> (u16, serde_json::Value) {
    let url = ctx.rest_url(&format!(
        "/v1/projects/{}/accounts:{action}?key={}",
        ctx.project_id, ctx.api_key
    ));
    let resp = reqwest::Client::new()
        .post(url)
        .json(&body)
        .send()
        .await
        .unwrap_or_else(|e| panic!("{action} request failed: {e}"));
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.expect("body must be JSON");
    (status, body)
}

async fn sign_in(ctx: &HostedIdentityFullContext, email: &str, password: &str) -> (u16, serde_json::Value) {
    post(ctx, "signInWithPassword", serde_json::json!({"email": email, "password": password})).await
}

async fn send_oob_code(ctx: &HostedIdentityFullContext, email: &str) -> (u16, serde_json::Value) {
    post(ctx, "sendOobCode", serde_json::json!({"email": email})).await
}

/// Statuses of `n` consecutive wrong-password sign-ins for `email`.
async fn failed_signins(ctx: &HostedIdentityFullContext, email: &str, n: usize) -> Vec<u16> {
    let mut statuses = Vec::new();
    for _ in 0..n {
        statuses.push(sign_in(ctx, email, "wrong-password-123").await.0);
    }
    statuses
}

#[tokio::test]
async fn credential_stuffing_one_account_is_throttled_even_with_the_right_password_and_leaks_no_existence() {
    let ctx = HostedIdentityFullContext::new("trailmark-prod-hi05-signin").await;
    ctx.enable_hosted_identity().await;
    let (signup, _) = post(
        &ctx,
        "signUp",
        serde_json::json!({"email": "maria@trailmark.example", "password": "correct-horse-battery"}),
    )
    .await;
    assert_eq!(signup, 201, "precondition: signup must succeed");

    // Registered and unregistered emails behave identically: 5 oracle-protected
    // 401s, then 429 — the throttle must not reveal which account exists.
    let registered = failed_signins(&ctx, "maria@trailmark.example", 6).await;
    let unregistered = failed_signins(&ctx, "ghost@trailmark.example", 6).await;
    assert_eq!(registered, vec![401, 401, 401, 401, 401, 429]);
    assert_eq!(unregistered, registered, "throttle must be identical for unknown emails");

    // Throttled means throttled: the correct password no longer gets through,
    // and a case variant of the email shares the same bucket.
    let (status, body) = sign_in(&ctx, "MARIA@trailmark.example", "correct-horse-battery").await;
    assert_eq!(status, 429, "a successful attempt must also be refused while throttled");
    assert_eq!(body["error"]["status"], "RESOURCE_EXHAUSTED");
    assert_eq!(body["error"]["code"], 429);

    // Another account in the same project is unaffected.
    let (status, _) = sign_in(&ctx, "other@trailmark.example", "wrong-password-123").await;
    assert_eq!(status, 401, "throttle is per account, not project-wide");
}

#[tokio::test]
async fn password_reset_email_bombing_one_address_is_throttled_identically_for_unknown_emails() {
    let ctx = HostedIdentityFullContext::new("trailmark-prod-hi05-oob").await;
    ctx.enable_hosted_identity().await;
    let (signup, _) = post(
        &ctx,
        "signUp",
        serde_json::json!({"email": "maria@trailmark.example", "password": "correct-horse-battery"}),
    )
    .await;
    assert_eq!(signup, 201, "precondition: signup must succeed");

    let mut registered = Vec::new();
    let mut unregistered = Vec::new();
    for _ in 0..4 {
        registered.push(send_oob_code(&ctx, "maria@trailmark.example").await);
        unregistered.push(send_oob_code(&ctx, "ghost@trailmark.example").await);
    }

    let statuses: Vec<u16> = registered.iter().map(|(s, _)| *s).collect();
    assert_eq!(statuses, vec![200, 200, 200, 429]);
    assert_eq!(unregistered, registered, "responses must be byte-identical for unknown emails");
    assert_eq!(registered[3].1["error"]["status"], "RESOURCE_EXHAUSTED");
    assert_eq!(ctx.captured_emails.sent_count(), 3, "throttled requests must not send mail");
}
