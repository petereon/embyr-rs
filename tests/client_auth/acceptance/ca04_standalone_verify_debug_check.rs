//! CA04 (Slice 04, US-04, Release 2) — Alex Verifies a Minted Token Resolves
//! to the Right End User Before Shipping.
//!
//! Acceptance criteria verified here (feature-delta.md US-04):
//!   AC-16-14: verifying a valid token returns the resolved identity+expiry;
//!             does NOT itself establish a live signed-in session.
//!   AC-16-15: the verify check surfaces the IDENTICAL rejection-reason
//!             taxonomy used by real sign-in (US-02) — malformed/expired/
//!             wrong-project — so Alex can trust it as a faithful
//!             pre-production diagnostic (shared-artifact integrity, the
//!             HIGH integration risk DISCUSS flagged).
//!
//! Driving port: Admin HTTP :9090 (`ClientAuthAdminContext` — debug-verify is
//! session-auth, any role, read-only by construction).
//!
//! Error ratio: 2 error/edge (expired + wrong-project) out of 4 = 50%.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{mint_client_identity_token, now_unix, ClientAuthAdminContext};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

// ─────────────────────────────────────────────────────────────────────────────
// AC-16-14: valid token -> resolved identity + expiry, no live session created
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (chained — Given reuses ca01's credential-registration precondition
/// conceptually; this file hand-seeds per Slice 04's own allowance):
///   Given: Alex holds a valid, unexpired token minted against
///          `trailmark-staging`'s registered credential
///   When:  Alex calls the standalone verify check with that token
///   Then:  the response shows the resolved identity + expiry, no live
///          session is created (verified via `sessions` row count, a
///          port-exposed observable — the debug-verify endpoint never
///          writes to that table)
///
/// AC-16-14
///
/// @driving_port @real-io @US-04 @AC-16-14
#[tokio::test]
#[ignore = "RED — this feature's own Walking Skeleton lives in ca01/ca02; enable ca04 after those are GREEN"]
async fn verifying_a_valid_token_returns_resolved_identity_and_creates_no_live_session() {
    let ctx = ClientAuthAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-staging").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_credential("trailmark-staging", &signing_key.verifying_key().to_bytes())
        .await;
    let token = mint_client_identity_token(&signing_key, "test-user-001", "trailmark-staging", now_unix() + 3600);

    let sessions_before: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions")
        .fetch_one(&ctx.pool)
        .await
        .unwrap_or(0);

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-staging/client_identity_credential/verify"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({"token": token}))
        .send()
        .await
        .expect("verify request failed");

    assert_eq!(resp.status().as_u16(), 200, "AC-16-14: valid token verify must return 200");
    let body: serde_json::Value = resp.json().await.expect("verify response must be JSON");
    assert_eq!(body["end_user_id"], "test-user-001");
    assert!(body.get("expires_at").is_some());

    let sessions_after: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions")
        .fetch_one(&ctx.pool)
        .await
        .unwrap_or(0);
    assert_eq!(
        sessions_before, sessions_after,
        "AC-16-14: the debug-verify check must never create a live session (sessions row count unchanged, \
         excluding Alex's own admin session seeded above)"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-16-15: verify surfaces the IDENTICAL taxonomy real sign-in uses (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

/// Alex's minting code has a bug and embeds `trailmark-prod` as the project
/// instead of `trailmark-staging` (US-04 Domain Example 2). The verify check
/// must surface the SAME project-mismatch reason a real sign-in attempt
/// (US-02) would.
///
/// @error @driving_port @real-io @US-04 @AC-16-15
#[tokio::test]
#[ignore = "RED — enable after ca01/ca02's walking skeletons are GREEN"]
async fn verifying_a_token_with_a_project_mismatch_surfaces_the_same_reason_as_real_signin() {
    let ctx = ClientAuthAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-staging").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_credential("trailmark-staging", &signing_key.verifying_key().to_bytes())
        .await;
    // Minted against trailmark-staging's key, but embeds trailmark-prod as `aud`.
    let buggy_token = mint_client_identity_token(&signing_key, "test-user-001", "trailmark-prod", now_unix() + 3600);

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-staging/client_identity_credential/verify"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({"token": buggy_token}))
        .send()
        .await
        .expect("verify request failed");

    assert_eq!(resp.status().as_u16(), 400, "AC-16-15: mismatched-project token must be rejected");
    let body: serde_json::Value = resp.json().await.expect("verify response must be JSON");
    assert_eq!(
        body["reason"], "PROJECT_MISMATCH",
        "AC-16-15: debug-verify must surface the IDENTICAL reason real sign-in (US-02) would"
    );
}

/// Alex calls the verify check with a token that expired five minutes ago
/// (US-04 Domain Example 3).
///
/// @error @driving_port @real-io @US-04 @AC-16-15
#[tokio::test]
#[ignore = "RED — enable after ca01/ca02's walking skeletons are GREEN"]
async fn verifying_an_expired_token_surfaces_the_expiry_reason_distinguishable_from_others() {
    let ctx = ClientAuthAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-staging").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_credential("trailmark-staging", &signing_key.verifying_key().to_bytes())
        .await;
    let expired_token =
        mint_client_identity_token(&signing_key, "test-user-001", "trailmark-staging", now_unix() - 300);

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-staging/client_identity_credential/verify"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({"token": expired_token}))
        .send()
        .await
        .expect("verify request failed");

    assert_eq!(resp.status().as_u16(), 400, "AC-16-15: expired token must be rejected");
    let body: serde_json::Value = resp.json().await.expect("verify response must be JSON");
    assert_eq!(body["reason"], "TOKEN_EXPIRED");
}

/// Boundary — verifying against a project with no credential registered at
/// all (all four ACs' shared precondition-boundary case).
///
/// @error @driving_port @real-io @US-04
#[tokio::test]
#[ignore = "RED — enable after ca01/ca02's walking skeletons are GREEN"]
async fn verifying_with_no_credential_registered_for_the_project_is_rejected_not_a_crash() {
    let ctx = ClientAuthAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-never-registered").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    let token = mint_client_identity_token(
        &signing_key,
        "test-user-001",
        "trailmark-never-registered",
        now_unix() + 3600,
    );

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-never-registered/client_identity_credential/verify"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({"token": token}))
        .send()
        .await
        .expect("verify request failed");

    assert!(
        resp.status().is_client_error(),
        "verifying against a project with no registered credential must be a clean rejection, not a 5xx crash"
    );
}
