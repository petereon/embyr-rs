//! HI01 (Slice 01, US-01) — Alex Enables Hosted Email/Password Identity For
//! Trailmark.
//!
//! Acceptance criteria verified here (slice-01-alex-enables-hosted-identity.md):
//!   AC-18-01: valid enablement (session + correct api_key) -> 201; row
//!             created; no signing material of any kind in the response.
//!   AC-18-02: second enablement on an already-enabled project -> idempotent
//!             success; SAME stored public_key/private_key_enc (no
//!             regeneration, no re-encryption).
//!   AC-18-03: missing/invalid admin session -> 401.
//!   AC-18-04: enablement for a non-existent/deleted project -> 404.
//!   AC-18-05: correct session, wrong/stale api_key -> 401
//!             {"reason": "INVALID_API_KEY"}.
//!   AC-18-06: backend_mode=agent project -> 403
//!             {"reason": "HOSTED_IDENTITY_UNAVAILABLE_FOR_BACKEND_MODE"}.
//!
//! Plus a Viewer-role boundary scenario (403), mirroring
//! `client_identity.rs`'s own precedent (ca01's identical convention).
//!
//! Driving port: Admin HTTP :9090 (`HostedIdentityAdminContext`, real
//! `build_admin_router` composition root).
//!
//! Error ratio: 5 error/edge (AC-18-03/04/05/06 + Viewer) out of 7 scenarios
//! = 71%.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{assert_state_delta, set_to, universe, HostedIdentityAdminContext};
use std::collections::HashMap;

const API_KEY: &str = "trailmark-prod-real-api-key";

// ─────────────────────────────────────────────────────────────────────────────
// AC-18-01: valid enablement succeeds and never echoes signing material back
// (WALKING SKELETON)
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (chained — opening Given of this feature's story line; AC-18-02
/// reuses this enablement as its own precondition, Pillar 2):
///   Given: project `trailmark-prod` exists (`direct_pg`), no hosted identity
///          signing key yet
///   When:  Alex enables hosted identity using a valid admin session and the
///          project's own correct api_key
///   Then:  a signing key row is created, and the response confirms success
///          WITHOUT any signing material
///
/// AC-18-01
///
/// @walking_skeleton @driving_port @real-io @US-01 @AC-18-01
#[tokio::test]
async fn alex_enables_hosted_identity_and_no_signing_material_is_echoed_back() {
    let ctx = HostedIdentityAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod", "direct_pg", API_KEY)
        .await;

    let before: HashMap<&str, bool> = HashMap::from([(
        universe::SIGNING_KEY_ROW_EXISTS,
        ctx.signing_key_row_exists("trailmark-prod").await,
    )]);

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/hosted_identity/enable"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({"api_key": API_KEY}))
        .send()
        .await
        .expect("enable request failed");

    assert_eq!(
        resp.status().as_u16(),
        201,
        "AC-18-01: valid enablement must return 201"
    );
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");

    assert_eq!(body["project_id"], "trailmark-prod");
    assert_eq!(body["algorithm"], "EdDSA");
    assert!(body.get("created_at").is_some());
    assert!(
        body.get("public_key").is_none()
            && body.get("private_key_enc").is_none()
            && body.get("key").is_none()
            && body.get("fingerprint").is_none(),
        "AC-18-01: no signing material of any kind must appear in the response"
    );

    let after: HashMap<&str, bool> = HashMap::from([(
        universe::SIGNING_KEY_ROW_EXISTS,
        ctx.signing_key_row_exists("trailmark-prod").await,
    )]);
    let mut expected = HashMap::new();
    expected.insert(universe::SIGNING_KEY_ROW_EXISTS, set_to(true));
    assert_state_delta(
        &before,
        &after,
        &[universe::SIGNING_KEY_ROW_EXISTS],
        &expected,
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-18-02: second enablement is idempotent — no regeneration, no re-encryption
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (chained — Given reuses this file's own walking-skeleton
/// enablement, then attempts a SECOND enablement for the same project):
///   Given: `trailmark-prod` already has hosted identity enabled
///   When:  Alex enables it again, same api_key
///   Then:  the call succeeds (not an error), and the stored key material is
///          byte-identical to the first call's — proving no regeneration
///
/// AC-18-02
///
/// @driving_port @real-io @US-01 @AC-18-02
#[tokio::test]
async fn second_enablement_is_idempotent_and_reuses_the_same_signing_key() {
    let ctx = HostedIdentityAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod", "direct_pg", API_KEY)
        .await;

    let first = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/hosted_identity/enable"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({"api_key": API_KEY}))
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
        .post(ctx.url("/admin/v1/projects/trailmark-prod/hosted_identity/enable"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({"api_key": API_KEY}))
        .send()
        .await
        .expect("second enable request failed");
    assert!(
        second.status().is_success(),
        "AC-18-02: a second enablement must succeed, not error — got {}",
        second.status()
    );
    let second_material = ctx
        .signing_key_material("trailmark-prod")
        .await
        .expect("signing key row must still exist after second enablement");

    assert_eq!(
        first_material, second_material,
        "AC-18-02: the second call's stored public_key/private_key_enc must be \
         byte-identical to the first call's — no regeneration, no re-encryption"
    );

    let row_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM hosted_identity_signing_keys WHERE project_id = $1",
    )
    .bind("trailmark-prod")
    .fetch_one(&ctx.pool)
    .await
    .expect("count rows");
    assert_eq!(row_count, 1, "AC-18-02: no duplicate row must be created");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-18-03: missing/invalid admin session (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-18-03
///
/// @error @driving_port @real-io @US-01 @AC-18-03
#[tokio::test]
async fn enablement_without_a_valid_session_is_rejected_same_as_any_other_admin_endpoint() {
    let ctx = HostedIdentityAdminContext::new().await;
    ctx.insert_project("trailmark-prod", "direct_pg", API_KEY)
        .await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/hosted_identity/enable"))
        // No Cookie header at all.
        .json(&serde_json::json!({"api_key": API_KEY}))
        .send()
        .await
        .expect("enable request failed");

    assert_eq!(
        resp.status().as_u16(),
        401,
        "AC-18-03: missing session must be rejected exactly as other admin endpoints reject it"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-18-04: enablement against a non-existent/deleted project (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-18-04
///
/// @error @driving_port @real-io @US-01 @AC-18-04
#[tokio::test]
async fn enablement_against_a_non_existent_project_is_rejected_as_not_found() {
    let ctx = HostedIdentityAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    // trailmark-staging-old is never inserted.

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-staging-old/hosted_identity/enable"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({"api_key": API_KEY}))
        .send()
        .await
        .expect("enable request failed");

    assert_eq!(
        resp.status().as_u16(),
        404,
        "AC-18-04: non-existent project must return 404"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-18-05: wrong/stale api_key in the body (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-18-05
///
/// @error @driving_port @real-io @US-01 @AC-18-05
#[tokio::test]
async fn enablement_with_a_wrong_api_key_is_rejected_and_never_silently_accepted() {
    let ctx = HostedIdentityAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod", "direct_pg", API_KEY)
        .await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/hosted_identity/enable"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({"api_key": "stale-or-wrong-api-key"}))
        .send()
        .await
        .expect("enable request failed");

    assert_eq!(
        resp.status().as_u16(),
        401,
        "AC-18-05: wrong api_key must return 401"
    );
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(body["reason"], "INVALID_API_KEY");
    assert!(
        !ctx.signing_key_row_exists("trailmark-prod").await,
        "AC-18-05: a wrong api_key must never result in a stored signing key"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-18-06: backend_mode=agent project is structurally refused (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-18-06
///
/// @error @driving_port @real-io @US-01 @AC-18-06
#[tokio::test]
async fn enablement_for_an_agent_backend_project_is_refused_outright() {
    let ctx = HostedIdentityAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-agent-proj", "agent", API_KEY)
        .await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-agent-proj/hosted_identity/enable"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({"api_key": API_KEY}))
        .send()
        .await
        .expect("enable request failed");

    assert_eq!(
        resp.status().as_u16(),
        403,
        "AC-18-06: backend_mode=agent must return 403"
    );
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["reason"],
        "HOSTED_IDENTITY_UNAVAILABLE_FOR_BACKEND_MODE"
    );
    assert!(
        !ctx.signing_key_row_exists("trailmark-agent-proj").await,
        "AC-18-06: an agent-backend project must never get a stored signing key"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Boundary: a Viewer (not Owner/Admin) cannot enable hosted identity
// ─────────────────────────────────────────────────────────────────────────────

/// Boundary scenario — role gate mirrors
/// `client_identity.rs::register_client_identity_credential`'s Owner/Admin-
/// only precedent (this codebase's established convention for every admin
/// write action).
///
/// @error @driving_port @real-io @US-01
#[tokio::test]
async fn a_viewer_role_cannot_enable_hosted_identity() {
    let ctx = HostedIdentityAdminContext::new().await;
    let cookie = ctx.seed_session("viewer@trailmark.example", "Viewer").await;
    ctx.insert_project("trailmark-prod", "direct_pg", API_KEY)
        .await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/hosted_identity/enable"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({"api_key": API_KEY}))
        .send()
        .await
        .expect("enable request failed");

    assert_eq!(
        resp.status().as_u16(),
        403,
        "Viewer role must not be able to enable hosted identity"
    );
}
