//! CA03 (Slice 03, US-03, Release 2) — Alex Rotates Trailmark's Verification
//! Credential Without Breaking Signed-In Users.
//!
//! Acceptance criteria verified here (feature-delta.md US-03):
//!   AC-16-10: valid rotation activates the new credential; new tokens verify.
//!   AC-16-11: tokens under the immediately-previous credential still verify
//!             during the rotation window.
//!   AC-16-12: tokens under a credential older than the window are rejected
//!             (as Malformed — no separate "stale" error class, ADR-025).
//!   AC-16-13: rotation without valid admin credentials is rejected 401.
//!
//! Driving ports: Admin HTTP :9090 (rotate) + REST :8081 (sign-in, to prove
//! the dual-generation window empirically) — both via
//! `ClientAuthFullContext`'s single composition root, so a rotate-then-verify
//! journey does not require switching test harnesses.
//!
//! Error ratio: 2 error/edge (AC-16-12/13) out of 4 = 50%.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{mint_client_identity_token, now_unix, ClientAuthFullContext};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

async fn sign_in(ctx: &ClientAuthFullContext, token: &str) -> (u16, serde_json::Value) {
    let resp = reqwest::Client::new()
        .post(ctx.rest_url(&format!(
            "/v1/projects/{}/accounts:signInWithCustomToken",
            ctx.project_id
        )))
        .json(&serde_json::json!({"token": token}))
        .send()
        .await
        .expect("sign-in request failed");
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
    (status, body)
}

/// Rotate `ctx.project_id`'s verification credential to `new_key`, via the
/// session cookie in `cookie` (or none, when `cookie` is `None` — AC-16-13's
/// unauthenticated-rotation scenario). Shared by every scenario in this file
/// — identical POST shape at every call site.
async fn rotate(ctx: &ClientAuthFullContext, cookie: Option<&str>, new_key: &SigningKey) -> u16 {
    let mut req = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/client_identity_credential/rotate",
            ctx.project_id
        )))
        .json(&serde_json::json!({"public_key": common::public_key_b64(new_key)}));
    if let Some(cookie) = cookie {
        req = req.header("Cookie", cookie);
    }
    req.send()
        .await
        .expect("rotate request failed")
        .status()
        .as_u16()
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-16-10: valid rotation activates the new credential for new tokens
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (chained — Given reuses this file's own rotation step across
/// scenarios via the shared `rotate` helper below, per Pillar 2):
///   Given: `trailmark-prod` has an active verification credential
///   When:  Alex submits a rotation request with new verification material
///   Then:  the new credential becomes active; new tokens verify successfully
///
/// AC-16-10
///
/// @driving_port @real-io @US-03 @AC-16-10
#[tokio::test]
async fn a_valid_rotation_activates_the_new_credential_for_new_tokens() {
    let ctx = ClientAuthFullContext::new("trailmark-prod-ca03-basic").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let original_key = SigningKey::generate(&mut OsRng);
    ctx.seed_credential(&original_key.verifying_key().to_bytes())
        .await;

    let new_key = SigningKey::generate(&mut OsRng);
    let rotate_status = rotate(&ctx, Some(&cookie), &new_key).await;
    assert_eq!(
        rotate_status, 200,
        "AC-16-10: valid rotation must return 200"
    );

    let new_token =
        mint_client_identity_token(&new_key, "maria-santos", &ctx.project_id, now_unix() + 3600);
    let (status, _) = sign_in(&ctx, &new_token).await;
    assert_eq!(
        status, 200,
        "AC-16-10: a token minted under the NEW credential must verify"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-16-11: immediately-previous-credential tokens still verify during the window
// ─────────────────────────────────────────────────────────────────────────────

/// AC-16-11
///
/// @driving_port @real-io @US-03 @AC-16-11
#[tokio::test]
async fn a_token_minted_under_the_immediately_previous_credential_still_verifies_during_the_window()
{
    let ctx = ClientAuthFullContext::new("trailmark-prod-ca03-prev").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let original_key = SigningKey::generate(&mut OsRng);
    ctx.seed_credential(&original_key.verifying_key().to_bytes())
        .await;

    // Maria's session, signed under the ORIGINAL credential, minted just
    // before Alex's rotation (US-03 Domain Example 1: "Maria's session,
    // signed in at 1:50pm under the old credential").
    let marias_token = mint_client_identity_token(
        &original_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let new_key = SigningKey::generate(&mut OsRng);
    rotate(&ctx, Some(&cookie), &new_key).await;

    let (status, _) = sign_in(&ctx, &marias_token).await;
    assert_eq!(
        status, 200,
        "AC-16-11: Maria's token, minted under the immediately-previous credential, must still verify"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-16-12: a credential from 2+ rotations ago is rejected as no longer valid (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

/// A token minted under a credential from THREE rotations ago (US-03 Domain
/// Example 3 / Edge Case 2: only the two most recent credentials remain
/// valid — mirroring the D5 dual-hash-window shape, not an unbounded
/// history). ADR-025: rejected via the ordinary Malformed path — no
/// separate "stale credential" error class.
///
/// @error @driving_port @real-io @US-03 @AC-16-12
#[tokio::test]
async fn a_token_signed_under_a_credential_two_rotations_ago_is_rejected_as_no_longer_valid() {
    let ctx = ClientAuthFullContext::new("trailmark-prod-ca03-stale").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let stale_key = SigningKey::generate(&mut OsRng);
    ctx.seed_credential(&stale_key.verifying_key().to_bytes())
        .await;
    let stale_token = mint_client_identity_token(
        &stale_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    // Two rotations: stale_key -> intermediate_key -> final_key. After both,
    // {final_key, intermediate_key} are the only valid generations —
    // stale_key is neither current nor previous.
    let intermediate_key = SigningKey::generate(&mut OsRng);
    rotate(&ctx, Some(&cookie), &intermediate_key).await;

    let final_key = SigningKey::generate(&mut OsRng);
    rotate(&ctx, Some(&cookie), &final_key).await;

    let (status, body) = sign_in(&ctx, &stale_token).await;
    assert_eq!(
        status, 400,
        "AC-16-12: a two-rotations-ago token must be rejected"
    );
    assert_eq!(
        body["reason"], "MALFORMED_TOKEN",
        "ADR-025: no separate 'stale credential' error class — rejected via the ordinary Malformed path"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-16-13: rotation without valid admin credentials is rejected 401 (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-16-13
///
/// @error @driving_port @real-io @US-03 @AC-16-13
#[tokio::test]
async fn rotation_without_a_valid_session_is_rejected() {
    let ctx = ClientAuthFullContext::new("trailmark-prod-ca03-unauth").await;
    let original_key = SigningKey::generate(&mut OsRng);
    ctx.seed_credential(&original_key.verifying_key().to_bytes())
        .await;

    let new_key = SigningKey::generate(&mut OsRng);
    // No Cookie header at all.
    let status = rotate(&ctx, None, &new_key).await;

    assert_eq!(
        status, 401,
        "AC-16-13: rotation without a valid session must return 401"
    );
}

/// Boundary scenario — role gate mirrors ca01's identical Viewer-cannot-
/// register precedent, but exercised against rotate's own `session.role <
/// Role::Admin` check (a separate source occurrence from register's; a
/// mutation-testing pass found rotate's copy of this guard under-covered).
///
/// @error @driving_port @real-io @US-03 @AC-16-13
#[tokio::test]
async fn a_viewer_role_cannot_rotate_a_verification_credential() {
    let ctx = ClientAuthFullContext::new("trailmark-prod-ca03-viewer").await;
    let cookie = ctx.seed_session("viewer@trailmark.example", "Viewer").await;
    let original_key = SigningKey::generate(&mut OsRng);
    ctx.seed_credential(&original_key.verifying_key().to_bytes())
        .await;

    let new_key = SigningKey::generate(&mut OsRng);
    let status = rotate(&ctx, Some(&cookie), &new_key).await;

    assert_eq!(
        status, 403,
        "AC-16-13: Viewer role must not be able to rotate a credential"
    );
}

/// Boundary scenario — an Admin-role session (exactly the gate's threshold,
/// Role::Admin = 2) must succeed at rotation, not just Owner (=3). A
/// mutation-testing pass found `session.role < Role::Admin` mutable to
/// `<=` with no test failing: that mutant incorrectly rejects Admin-exactly
/// sessions (Admin <= Admin is true) while every existing rotate scenario
/// used Owner, which passes under either operator. Only an exact-Admin
/// session distinguishes `<` from `<=`.
///
/// @driving_port @real-io @US-03 @AC-16-13
#[tokio::test]
async fn an_admin_role_session_can_rotate_a_verification_credential() {
    let ctx = ClientAuthFullContext::new("trailmark-prod-ca03-admin").await;
    let cookie = ctx.seed_session("admin@trailmark.example", "Admin").await;
    let original_key = SigningKey::generate(&mut OsRng);
    ctx.seed_credential(&original_key.verifying_key().to_bytes())
        .await;

    let new_key = SigningKey::generate(&mut OsRng);
    let status = rotate(&ctx, Some(&cookie), &new_key).await;

    assert_eq!(
        status, 200,
        "AC-16-13: an Admin-role session (the gate's exact threshold) must be able to rotate"
    );
}
