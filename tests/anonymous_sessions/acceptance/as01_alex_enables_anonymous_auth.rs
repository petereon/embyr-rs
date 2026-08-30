//! AS01 (Slice 01, US-01) — Alex Enables Anonymous Authentication For
//! Trailmark.
//!
//! Acceptance criteria verified here
//! (slice-01-alex-enables-anonymous-auth.md, ADR-043):
//!   AC-20-01: valid first-time enablement -> 201; `anonymous_signing_keys`
//!             row exists; no signing material in the response.
//!   AC-20-02: re-enablement of an already-enabled project -> 201
//!             (ADR-043 Decision 3: always-201, mirrors
//!             `enable_hosted_identity`'s own convention, NOT
//!             `register_oauth_provider`'s 201-vs-200 dance); no duplicate
//!             row; signing key material byte-identical before/after.
//!   AC-20-03: missing/invalid admin session -> 401.
//!   AC-20-04: enablement for a non-existent/deleted project -> 404.
//!
//! Plus a Viewer-role boundary scenario (403), mirroring
//! `oauth_providers.rs`'s/`hosted_identity.rs`'s own established precedent.
//!
//! Driving port: Admin HTTP :9090 (`AnonymousIdentityAdminContext`, real
//! `build_admin_router` composition root). Real Postgres via testcontainers
//! (@wiring_e2e-tier real-I/O) — a single golden walkthrough per scenario is
//! this codebase's own established convention for this exact test class
//! (mirrors `hi01`/`op01`'s identical single-example shape; no property
//! framing fits "Alex enables anonymous auth for trailmark-prod" any better
//! than the sibling files' own established shape).
//!
//! Error ratio: 3 error/edge (AC-20-03/04 + Viewer) out of 5 scenarios = 60%.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::AnonymousIdentityAdminContext;

// ─────────────────────────────────────────────────────────────────────────────
// AC-20-01: valid first-time enablement activates anonymous sign-in
// (WALKING SKELETON)
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (chained — opening Given of this feature's story line; AC-20-02
/// reuses this enablement as its own precondition):
///   Given: project `trailmark-prod` exists, anonymous auth not yet enabled
///   When:  Alex enables anonymous auth using a valid admin session
///   Then:  a signing-key row is created; no signing material is echoed back
///
/// AC-20-01
///
/// @walking_skeleton @driving_port @real-io @US-01 @AC-20-01
#[tokio::test]
async fn alex_enables_anonymous_auth_and_a_signing_key_is_created() {
    let ctx = AnonymousIdentityAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    assert!(
        !ctx.signing_key_row_exists("trailmark-prod").await,
        "precondition: no signing key before enablement"
    );

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/anonymous_identity/enable"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("enable request failed");

    assert_eq!(
        resp.status().as_u16(),
        201,
        "AC-20-01: valid first-time enablement must return 201"
    );
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(body["project_id"], "trailmark-prod");
    assert_eq!(body["algorithm"], "EdDSA");
    assert!(body.get("created_at").is_some());
    assert!(
        body.get("public_key").is_none() && body.get("private_key_enc").is_none(),
        "AC-20-01: no signing material of any kind must appear in the response"
    );

    assert!(
        ctx.signing_key_row_exists("trailmark-prod").await,
        "AC-20-01: anonymous_signing_keys row must exist after enablement"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-20-02: idempotent re-enablement — no error, no duplicate signing key
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (chained — Given reuses this file's own walking-skeleton
/// enablement, then re-enables):
///   Given: `trailmark-prod` already has anonymous auth enabled
///   When:  Alex submits another enable request for the same project
///   Then:  the request succeeds (201) with no error, and no new signing
///          key is generated (byte-identical material, one row)
///
/// AC-20-02
///
/// @driving_port @real-io @US-01 @AC-20-02
#[tokio::test]
async fn reenabling_an_already_enabled_project_is_idempotent() {
    let ctx = AnonymousIdentityAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let first = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/anonymous_identity/enable"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("first enable request failed");
    assert_eq!(
        first.status().as_u16(),
        201,
        "first enablement must succeed"
    );
    let first_material = ctx
        .signing_key_material("trailmark-prod")
        .await
        .expect("signing key row must exist after first enablement");

    let second = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/anonymous_identity/enable"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("second enable request failed");
    assert_eq!(
        second.status().as_u16(),
        201,
        "AC-20-02: re-enablement must succeed, no error"
    );

    let second_material = ctx
        .signing_key_material("trailmark-prod")
        .await
        .expect("signing key row must still exist after re-enablement");
    assert_eq!(
        first_material, second_material,
        "AC-20-02: re-enablement must never regenerate the signing key — \
         public_key/private_key_enc must be byte-identical"
    );
    assert_eq!(
        ctx.signing_key_row_count("trailmark-prod").await,
        1,
        "AC-20-02: no duplicate row must be created"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-20-03: missing/invalid admin session (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-20-03
///
/// @error @driving_port @real-io @US-01 @AC-20-03
#[tokio::test]
async fn enablement_without_a_valid_session_is_rejected_same_as_any_other_admin_endpoint() {
    let ctx = AnonymousIdentityAdminContext::new().await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/anonymous_identity/enable"))
        // No Cookie header at all.
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("enable request failed");

    assert_eq!(
        resp.status().as_u16(),
        401,
        "AC-20-03: missing session must be rejected exactly as other admin endpoints reject it"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-20-04: enablement against a non-existent/deleted project (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-20-04
///
/// @error @driving_port @real-io @US-01 @AC-20-04
#[tokio::test]
async fn enablement_against_a_non_existent_project_is_rejected_as_not_found() {
    let ctx = AnonymousIdentityAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    // trailmark-staging-old is never inserted.

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-staging-old/anonymous_identity/enable"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("enable request failed");

    assert_eq!(
        resp.status().as_u16(),
        404,
        "AC-20-04: non-existent project must return 404"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Boundary: a Viewer (not Owner/Admin) cannot enable anonymous auth
// ─────────────────────────────────────────────────────────────────────────────

/// Boundary scenario — role gate mirrors `oauth_providers.rs`'s/
/// `hosted_identity.rs`'s identical Owner/Admin-only precedent (this
/// codebase's established convention for every admin write action in this
/// Identity track).
///
/// @error @driving_port @real-io @US-01
#[tokio::test]
async fn a_viewer_role_cannot_enable_anonymous_auth() {
    let ctx = AnonymousIdentityAdminContext::new().await;
    let cookie = ctx.seed_session("viewer@trailmark.example", "Viewer").await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/anonymous_identity/enable"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("enable request failed");

    assert_eq!(
        resp.status().as_u16(),
        403,
        "Viewer role must not be able to enable anonymous auth"
    );
}
