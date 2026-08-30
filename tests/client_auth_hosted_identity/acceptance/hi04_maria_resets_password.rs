//! HI04 (Slice 04, US-04) — Maria Resets Her Forgotten Password Without
//! Contacting Alex.
//!
//! Acceptance criteria verified here
//! (slice-04-maria-resets-forgotten-password.md):
//!   AC-18-14: reset-request (`accounts:sendOobCode`) always returns the
//!             IDENTICAL generic response regardless of email registration.
//!   AC-18-15: full round trip — request reset, extract the mailed token,
//!             confirm with a strong new password, new password works for a
//!             subsequent `accounts:signInWithPassword`, old password no
//!             longer does.
//!   AC-18-16: expired token -> RESET_TOKEN_EXPIRED, distinguishable from a
//!             malformed/unknown token -> RESET_TOKEN_INVALID.
//!   AC-18-17: a reset token can be consumed at most once.
//!
//! AC-18-18 ("send" step uses IEmailSender) is a source-level audit, not a
//! runtime-testable criterion — see the crafter's own commit message.
//!
//! Driving ports: REST :8081 (signup precondition + sendOobCode +
//! resetPassword + signInWithPassword) — mirrors hi02/hi03.
//!
//! Error ratio: 2 error/edge (AC-18-16 covers 2 sub-cases, AC-18-17) out of 4
//! behaviors = 50%.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::HostedIdentityFullContext;

async fn sign_up(
    ctx: &HostedIdentityFullContext,
    email: &str,
    password: &str,
) -> (u16, serde_json::Value) {
    let url = ctx.rest_url(&format!(
        "/v1/projects/{}/accounts:signUp?key={}",
        ctx.project_id, ctx.api_key
    ));
    let resp = reqwest::Client::new()
        .post(url)
        .json(&serde_json::json!({"email": email, "password": password}))
        .send()
        .await
        .expect("sign_up request failed");
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.expect("sign_up body must be JSON");
    (status, body)
}

async fn sign_in(
    ctx: &HostedIdentityFullContext,
    email: &str,
    password: &str,
) -> (u16, serde_json::Value) {
    let url = ctx.rest_url(&format!(
        "/v1/projects/{}/accounts:signInWithPassword?key={}",
        ctx.project_id, ctx.api_key
    ));
    let resp = reqwest::Client::new()
        .post(url)
        .json(&serde_json::json!({"email": email, "password": password}))
        .send()
        .await
        .expect("sign_in request failed");
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.expect("sign_in body must be JSON");
    (status, body)
}

async fn send_oob_code(ctx: &HostedIdentityFullContext, email: &str) -> (u16, serde_json::Value) {
    let url = ctx.rest_url(&format!(
        "/v1/projects/{}/accounts:sendOobCode?key={}",
        ctx.project_id, ctx.api_key
    ));
    let resp = reqwest::Client::new()
        .post(url)
        .json(&serde_json::json!({"email": email}))
        .send()
        .await
        .expect("sendOobCode request failed");
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.expect("sendOobCode body must be JSON");
    (status, body)
}

async fn reset_password(
    ctx: &HostedIdentityFullContext,
    oob_code: &str,
    new_password: &str,
) -> (u16, serde_json::Value) {
    let url = ctx.rest_url(&format!(
        "/v1/projects/{}/accounts:resetPassword?key={}",
        ctx.project_id, ctx.api_key
    ));
    let resp = reqwest::Client::new()
        .post(url)
        .json(&serde_json::json!({"oobCode": oob_code, "newPassword": new_password}))
        .send()
        .await
        .expect("resetPassword request failed");
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.expect("resetPassword body must be JSON");
    (status, body)
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-18-14: reset-request is byte-identical for a registered vs. unregistered
// email — the core oracle-protection requirement
// ─────────────────────────────────────────────────────────────────────────────

/// AC-18-14
///
/// @driving_port @real-io @US-04 @AC-18-14
#[tokio::test]
async fn reset_request_returns_the_byte_identical_response_regardless_of_registration() {
    let ctx = HostedIdentityFullContext::new("trailmark-prod-hi04-oracle").await;
    ctx.enable_hosted_identity().await;
    let (signup_status, _) =
        sign_up(&ctx, "maria@trailmark.example", "correct-horse-battery").await;
    assert_eq!(signup_status, 201, "precondition: signup must succeed");

    let registered = send_oob_code(&ctx, "maria@trailmark.example").await;
    let unregistered = send_oob_code(&ctx, "never-signed-up@trailmark.example").await;

    assert_eq!(registered.0, 200, "AC-18-14: reset-request must always be 200");
    assert_eq!(
        registered, unregistered,
        "AC-18-14: registered and unregistered emails must produce a byte-identical \
         response (status + body), proving no account-enumeration oracle exists"
    );
    assert_eq!(registered.1["message"], "if this account exists, a reset was sent");

    // Only the REGISTERED email actually gets a reset mailed (AC-18-18).
    assert_eq!(
        ctx.captured_emails.sent_count(),
        1,
        "AC-18-18: exactly one email sent, for the registered address only"
    );
    assert_eq!(ctx.captured_emails.last_email().unwrap().to, "maria@trailmark.example");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-18-15: full round trip — reset succeeds, new password works, old
// password stops working
// ─────────────────────────────────────────────────────────────────────────────

/// AC-18-15
///
/// @driving_port @real-io @US-04 @AC-18-15
#[tokio::test]
async fn reset_confirm_updates_the_password_and_the_old_one_stops_working() {
    let ctx = HostedIdentityFullContext::new("trailmark-prod-hi04-roundtrip").await;
    ctx.enable_hosted_identity().await;
    let (signup_status, _) =
        sign_up(&ctx, "maria@trailmark.example", "correct-horse-battery").await;
    assert_eq!(signup_status, 201, "precondition: signup must succeed");

    let (oob_status, _) = send_oob_code(&ctx, "maria@trailmark.example").await;
    assert_eq!(oob_status, 200, "precondition: reset-request must succeed");

    // Only the raw token's BLAKE3 hash is ever persisted — the test reads it
    // back from the captured mailed email (first line of the body, per this
    // handler's own body_text shape).
    let mailed = ctx
        .captured_emails
        .last_email()
        .expect("reset-request must have mailed a token");
    let raw_token = mailed.body.lines().next().expect("email body must contain the token");

    let (confirm_status, confirm_body) =
        reset_password(&ctx, raw_token, "new-correct-horse-battery").await;
    assert_eq!(confirm_status, 200, "AC-18-15: valid reset-confirm must return 200");
    assert_eq!(confirm_body["email"], "maria@trailmark.example");

    let (new_pw_status, _) =
        sign_in(&ctx, "maria@trailmark.example", "new-correct-horse-battery").await;
    assert_eq!(
        new_pw_status, 200,
        "AC-18-15: the NEW password must work for a subsequent signin"
    );

    let (old_pw_status, _) = sign_in(&ctx, "maria@trailmark.example", "correct-horse-battery").await;
    assert_eq!(
        old_pw_status, 401,
        "AC-18-15: the OLD password must no longer work"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-18-16: expired token distinguishable from malformed/unknown token
// ─────────────────────────────────────────────────────────────────────────────

/// AC-18-16
///
/// @error @driving_port @real-io @US-04 @AC-18-16
#[tokio::test]
async fn expired_token_is_distinguishable_from_a_malformed_or_unknown_token() {
    let ctx = HostedIdentityFullContext::new("trailmark-prod-hi04-expired").await;
    ctx.enable_hosted_identity().await;
    let (signup_status, _) =
        sign_up(&ctx, "maria@trailmark.example", "correct-horse-battery").await;
    assert_eq!(signup_status, 201, "precondition: signup must succeed");

    // Seed an already-expired row directly (can't wait an hour in a test).
    let expired_raw_token = "already-expired-reset-token";
    ctx.seed_reset_token(
        "maria@trailmark.example",
        expired_raw_token,
        chrono::Utc::now() - chrono::Duration::hours(1),
    )
    .await;

    let (expired_status, expired_body) =
        reset_password(&ctx, expired_raw_token, "new-correct-horse-battery").await;
    assert_eq!(expired_status, 401, "AC-18-16: expired token must be rejected");
    assert_eq!(expired_body["reason"], "RESET_TOKEN_EXPIRED");

    let (unknown_status, unknown_body) =
        reset_password(&ctx, "never-issued-token", "new-correct-horse-battery").await;
    assert_eq!(unknown_status, 401, "AC-18-16: unknown token must be rejected");
    assert_eq!(
        unknown_body["reason"], "RESET_TOKEN_INVALID",
        "AC-18-16: unknown/malformed token must be a DIFFERENT reason than expired"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-18-17: a reset token can be consumed at most once
// ─────────────────────────────────────────────────────────────────────────────

/// AC-18-17
///
/// @error @driving_port @real-io @US-04 @AC-18-17
#[tokio::test]
async fn a_reset_token_can_be_consumed_at_most_once() {
    let ctx = HostedIdentityFullContext::new("trailmark-prod-hi04-singleuse").await;
    ctx.enable_hosted_identity().await;
    let (signup_status, _) =
        sign_up(&ctx, "maria@trailmark.example", "correct-horse-battery").await;
    assert_eq!(signup_status, 201, "precondition: signup must succeed");
    let (oob_status, _) = send_oob_code(&ctx, "maria@trailmark.example").await;
    assert_eq!(oob_status, 200, "precondition: reset-request must succeed");
    let raw_token = ctx
        .captured_emails
        .last_email()
        .expect("reset-request must have mailed a token")
        .body
        .lines()
        .next()
        .expect("email body must contain the token")
        .to_string();

    let (first_status, _) = reset_password(&ctx, &raw_token, "new-correct-horse-battery").await;
    assert_eq!(first_status, 200, "AC-18-17: first consumption must succeed");

    let (second_status, second_body) =
        reset_password(&ctx, &raw_token, "yet-another-new-password").await;
    assert_eq!(
        second_status, 401,
        "AC-18-17: a SECOND use of the same token must be rejected"
    );
    assert_eq!(second_body["reason"], "RESET_TOKEN_INVALID");
}
