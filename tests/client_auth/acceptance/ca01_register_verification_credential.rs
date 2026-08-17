//! CA01 (Slice 01, Walking Skeleton half A, US-01, Release 1) — Alex
//! Registers Trailmark's Verification Credential.
//!
//! Acceptance criteria verified here (feature-delta.md US-01):
//!   AC-16-01: valid registration -> 201; credential stored+active; no raw
//!             material in the response.
//!   AC-16-02: malformed verification material -> 400, naming what's wrong.
//!   AC-16-03: missing/invalid admin Bearer -> 401 (session-cookie auth here
//!             — "invalid credential" means no/garbage session cookie).
//!   AC-16-04: registering when a credential already exists -> 409, directs
//!             to rotation (US-03).
//!   AC-16-05: registering for a non-existent/deleted project -> 404.
//!
//! Driving port: Admin HTTP :9090 (`ClientAuthAdminContext`, real
//! `build_admin_router` composition root — Pillar 3).
//!
//! Error ratio: 4 error/edge (AC-16-02/03/04/05) out of 6 scenarios = 67% —
//! comfortably over the 40% mandate.
//!
//! All scenarios `#[ignore]` except the walking-skeleton scenario — RED,
//! enable one at a time in DELIVER (per DISTILL's one-at-a-time discipline).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{assert_state_delta, set_to, universe, ClientAuthAdminContext};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────────────────────
// AC-16-01: first registration succeeds and never echoes the raw material back
// (WALKING SKELETON — Activity A, feature-delta.md § Story Map)
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (chained — this is the opening Given of the client-auth story
/// line; every later scenario in ca02-ca05 reuses this credential-exists
/// precondition, per Pillar 2):
///   Given: project `trailmark-prod` exists, no verification credential yet
///   When:  Alex registers verification material using a valid admin session
///   Then:  the credential is stored and active, and the response confirms
///          success WITHOUT including the raw material
///
/// AC-16-01
///
/// @walking_skeleton @driving_port @real-io @US-01 @AC-16-01
#[tokio::test]
async fn alex_registers_trailmarks_verification_credential_and_no_raw_material_is_echoed_back() {
    let ctx = ClientAuthAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let signing_key = SigningKey::generate(&mut OsRng);
    let public_key_b64 = common::public_key_b64(&signing_key);

    let before: HashMap<&str, bool> = HashMap::from([(
        universe::CREDENTIAL_ROW_EXISTS,
        ctx.credential_row_exists("trailmark-prod").await,
    )]);

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/client_identity_credential"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({"public_key": public_key_b64}))
        .send()
        .await
        .expect("register request failed");

    assert_eq!(resp.status().as_u16(), 201, "AC-16-01: valid registration must return 201");
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");

    assert_eq!(body["project_id"], "trailmark-prod");
    assert_eq!(body["algorithm"], "EdDSA");
    assert!(body.get("fingerprint").is_some(), "AC-16-01: response must include a fingerprint");
    assert!(body.get("created_at").is_some());
    assert!(
        body.get("public_key").is_none() && body.get("key").is_none(),
        "AC-16-01: the raw verification material must NEVER appear in the response"
    );
    let body_str = body.to_string();
    assert!(
        !body_str.contains(&public_key_b64),
        "AC-16-01: raw key material must not be echoed back anywhere in the response body"
    );

    let after: HashMap<&str, bool> = HashMap::from([(
        universe::CREDENTIAL_ROW_EXISTS,
        ctx.credential_row_exists("trailmark-prod").await,
    )]);
    let mut expected = HashMap::new();
    expected.insert(universe::CREDENTIAL_ROW_EXISTS, set_to(true));
    assert_state_delta(&before, &after, &[universe::CREDENTIAL_ROW_EXISTS], &expected);
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-16-02: malformed verification material (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-16-02
///
/// @error @driving_port @real-io @US-01 @AC-16-02
#[tokio::test]
async fn registration_with_wrong_length_key_material_is_rejected_naming_whats_wrong() {
    let ctx = ClientAuthAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    // A copy-paste truncation — 16 bytes instead of the required 32.
    let truncated_key_b64 =
        base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, [0u8; 16]);

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/client_identity_credential"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({"public_key": truncated_key_b64}))
        .send()
        .await
        .expect("register request failed");

    assert_eq!(resp.status().as_u16(), 400, "AC-16-02: malformed material must return 400");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-16-03: missing/invalid admin credential (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-16-03
///
/// @error @driving_port @real-io @US-01 @AC-16-03
#[tokio::test]
async fn registration_without_a_valid_session_is_rejected_same_as_any_other_admin_endpoint() {
    let ctx = ClientAuthAdminContext::new().await;
    ctx.insert_project("trailmark-prod").await;

    let signing_key = SigningKey::generate(&mut OsRng);
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/client_identity_credential"))
        // No Cookie header at all.
        .json(&serde_json::json!({"public_key": common::public_key_b64(&signing_key)}))
        .send()
        .await
        .expect("register request failed");

    assert_eq!(
        resp.status().as_u16(),
        401,
        "AC-16-03: missing session must be rejected exactly as other admin endpoints reject it"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-16-04: duplicate registration is rejected, directs to rotation (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (chained — Given reuses this file's own walking-skeleton
/// registration step, then attempts a SECOND registration):
///   Given: `trailmark-prod` already has an active verification credential
///   When:  Alex submits another registration request for the same project
///   Then:  the request is rejected (409), directed to the rotation action
///
/// AC-16-04
///
/// @error @driving_port @real-io @US-01 @AC-16-04
#[tokio::test]
async fn registering_a_second_credential_for_an_already_registered_project_is_rejected_not_overwritten() {
    let ctx = ClientAuthAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let first_key = SigningKey::generate(&mut OsRng);
    ctx.seed_credential("trailmark-prod", &first_key.verifying_key().to_bytes())
        .await;

    let second_key = SigningKey::generate(&mut OsRng);
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/client_identity_credential"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({"public_key": common::public_key_b64(&second_key)}))
        .send()
        .await
        .expect("register request failed");

    assert_eq!(resp.status().as_u16(), 409, "AC-16-04: duplicate registration must return 409");
    let body: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
    let body_str = body.to_string().to_lowercase();
    assert!(
        body_str.contains("rotat"),
        "AC-16-04: the 409 response must direct the caller to the rotation action"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-16-05: registration against a non-existent/deleted project (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-16-05
///
/// @error @driving_port @real-io @US-01 @AC-16-05
#[tokio::test]
async fn registration_against_a_non_existent_project_is_rejected_as_not_found() {
    let ctx = ClientAuthAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    // trailmark-staging-old is never inserted.

    let signing_key = SigningKey::generate(&mut OsRng);
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-staging-old/client_identity_credential"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({"public_key": common::public_key_b64(&signing_key)}))
        .send()
        .await
        .expect("register request failed");

    assert_eq!(resp.status().as_u16(), 404, "AC-16-05: non-existent project must return 404");
}

// ─────────────────────────────────────────────────────────────────────────────
// Boundary: a Viewer (not Owner/Admin) cannot register a credential
// ─────────────────────────────────────────────────────────────────────────────

/// Boundary scenario — role gate mirrors `sdk_keys.rs::create_sdk_key`'s
/// Owner/Admin-only precedent (feature-delta.md Reuse Analysis).
///
/// @error @driving_port @real-io @US-01
#[tokio::test]
async fn a_viewer_role_cannot_register_a_verification_credential() {
    let ctx = ClientAuthAdminContext::new().await;
    let cookie = ctx.seed_session("viewer@trailmark.example", "Viewer").await;
    ctx.insert_project("trailmark-prod").await;

    let signing_key = SigningKey::generate(&mut OsRng);
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/client_identity_credential"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({"public_key": common::public_key_b64(&signing_key)}))
        .send()
        .await
        .expect("register request failed");

    assert_eq!(resp.status().as_u16(), 403, "Viewer role must not be able to register a credential");
}
