//! SR05 (Slice 05, US-05, Release 2) — Alex Tests a Rule Against Concrete
//! Examples Before Publishing It.
//!
//! Not walking-skeleton-level (per DISCUSS § Story Map, Release 2 comes
//! after Release 1's Walking Skeleton; the DISTILL dispatch instructions
//! scope `@walking_skeleton` to Slices 01-04 only).
//!
//! Acceptance criteria verified here (feature-delta.md US-05):
//!   AC-17-17: simulating a candidate rule against a synthetic identity/
//!             document pair returns the same allow/deny outcome real
//!             evaluation would produce for that exact pair.
//!   AC-17-18: simulating a rule has zero effect on live/published
//!             traffic — a live collection's real reads are evaluated only
//!             against its published rule, never a simulated one.
//!   AC-17-19: simulation supports the anonymous (no synthetic identity)
//!             case, matching US-03's real anonymous-evaluation semantics
//!             exactly.
//!
//! Driving ports: Admin HTTP :9090 (`SecurityRulesAdminContext`, scenarios
//! 1/2/4) + Admin HTTP :9090 AND gRPC :8080 together
//! (`SecurityRulesFullContext`, scenario 3 — AC-17-18 needs BOTH the
//! simulate action and a REAL getDoc call on the SAME composition root to
//! prove zero cross-effect).
//!
//! ADR-029 § Simulation shares the exact evaluation routine: every scenario
//! here exercises `embyr_core::access_control::{parse_condition, evaluate}`
//! via `admin::handlers::access_rules::simulate_access_rule` — the IDENTICAL
//! function real enforcement (sr02/sr03) calls, never a second,
//! independently-maintained copy.
//!
//! Error ratio: 3 edge (bug-surfacing, zero-live-effect, anonymous case)
//! out of 4 = 75%.
//!
//! One scenario enabled at a time (RED scaffold discipline, ADR-025 D2).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{mint_client_identity_token, now_unix, SecurityRulesAdminContext, SecurityRulesFullContext};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-17: simulating a matching identity/document pair returns the
// correct (allow) outcome
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (chained — this file's own opening Given; scenario 2 below
/// reuses this file's admin-context + simulate-request shape, per Pillar
/// 2):
///   Given: Alex holds a candidate rule and a synthetic identity/document
///          pair that should satisfy it
///   When:  Alex calls the simulation action with the candidate rule and
///          the synthetic pair
///   Then:  the response shows "allow," matching what real evaluation
///          would produce for that pair
///
/// AC-17-17
///
/// @driving_port @real-io @US-05 @AC-17-17
#[tokio::test]
async fn simulating_a_valid_candidate_rule_against_a_matching_pair_returns_the_correct_outcome() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/simulate"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "condition": "request.auth.uid == resource.data.owner_id",
            "auth": {"uid": "test-user-001"},
            "resource": {"owner_id": "test-user-001"},
        }))
        .send()
        .await
        .expect("simulate request failed");

    assert_eq!(resp.status().as_u16(), 200, "AC-17-17: a valid simulation request must return 200");
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["outcome"], "allow",
        "AC-17-17: a matching identity/document pair must simulate as 'allow'"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-17: simulation surfaces an over-permissive rule bug before publishing
// (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-17 (bug-surfacing case)
///
/// @error @driving_port @real-io @US-05 @AC-17-17
#[tokio::test]
#[ignore = "one-scenario-at-a-time RED discipline — DELIVER unskips per step"]
async fn simulation_surfaces_an_over_permissive_rule_bug_before_publishing() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    // Alex mistakenly wrote `!=` instead of `==` — the equality direction
    // is reversed (feature-delta.md § Journey, Failure modes), so a
    // MISMATCHED pair evaluates to "allow" instead of the "deny" Alex
    // expected — exactly the over-permissive bug simulation exists to
    // catch before it reaches Maria or Dana.
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/simulate"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "condition": "request.auth.uid != resource.data.owner_id",
            "auth": {"uid": "test-user-002"},
            "resource": {"owner_id": "test-user-001"},
        }))
        .send()
        .await
        .expect("simulate request failed");

    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["outcome"], "allow",
        "AC-17-17: the reversed-equality bug must surface as 'allow' — the discrepancy from \
         Alex's expectation ('deny') IS the bug simulation is meant to catch pre-publish"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-18: simulation has zero effect on live traffic (error/edge, guardrail)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-18
///
/// @error @driving_port @real-io @US-05 @AC-17-18
#[tokio::test]
#[ignore = "one-scenario-at-a-time RED discipline — DELIVER unskips per step"]
async fn simulation_has_zero_effect_on_live_traffic() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sr05-liveunaffected").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    // journal_entries has an ACTIVE, PUBLISHED rule (Maria owns her own doc).
    ctx.seed_access_rule(
        "journal_entries",
        "request.auth.uid == resource.data.owner_id",
    )
    .await;
    let marias_token =
        mint_client_identity_token(&signing_key, "maria-santos", &ctx.project_id, now_unix() + 3600);

    // Real caller succeeds against the PUBLISHED rule before simulation.
    let before = ctx
        .get_document(&ctx.document_resource_name("journal_entries", "maria-doc"), Some(&marias_token))
        .await;
    assert!(before.is_ok(), "precondition: Maria's real read must succeed before simulation");

    // Alex simulates a COMPLETELY DIFFERENT candidate rule (unconditional
    // deny) with synthetic data — this must never touch `access_rules` or
    // any live document.
    let simulate_resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/access_rules/simulate",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "condition": "false",
            "auth": {"uid": "test-user-999"},
            "resource": {"owner_id": "someone-else"},
        }))
        .send()
        .await
        .expect("simulate request failed");
    assert_eq!(simulate_resp.status().as_u16(), 200);

    // Real callers' getDoc calls continue to be evaluated against the
    // PUBLISHED rule, completely unaffected by the simulation just run.
    let after = ctx
        .get_document(&ctx.document_resource_name("journal_entries", "maria-doc"), Some(&marias_token))
        .await;
    assert!(
        after.is_ok(),
        "AC-17-18: Maria's real read must STILL succeed after the simulation — simulation must \
         have zero effect on live traffic: {:?}",
        after.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-19: simulation supports the anonymous case identically to real
// evaluation (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-19
///
/// @error @driving_port @real-io @US-05 @AC-17-19
#[tokio::test]
#[ignore = "one-scenario-at-a-time RED discipline — DELIVER unskips per step"]
async fn simulation_supports_the_anonymous_case_identically_to_real_evaluation() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    // No "auth" field at all — represents an anonymous caller, identical
    // in shape to real evaluation's `Option<AuthContext>` (US-03's own
    // `request.auth == null` semantics).
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/simulate"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "condition": "request.auth != null",
        }))
        .send()
        .await
        .expect("simulate request failed");

    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["outcome"], "deny",
        "AC-17-19: simulating with no synthetic identity must match what a real anonymous \
         caller receives under US-03 (denied by an auth-required rule)"
    );
}
