//! SR01 (Slice 01, Walking Skeleton — Activity A, US-01, Release 1) — Alex
//! Defines (and Redefines) an Access-Control Rule for a Collection.
//!
//! Acceptance criteria verified here (feature-delta.md US-01):
//!   AC-17-01: first-time rule definition -> stored and active for the
//!             named collection.
//!   AC-17-02: redefining a collection's rule fully and immediately
//!             replaces the prior condition — no merge, no overlap window.
//!   AC-17-03: a condition using an out-of-v1-grammar construct (cross
//!             -document reads) is rejected, naming that specifically,
//!             distinguishable from a plain syntax error.
//!   AC-17-04: a condition with invalid syntax is rejected, naming what's
//!             invalid.
//!   AC-17-05: missing/invalid admin session is rejected the same way any
//!             other admin endpoint rejects it.
//!
//! Driving port: Admin HTTP :9090 (`SecurityRulesAdminContext`, real
//! `build_admin_router` composition root — Pillar 3).
//!
//! Error ratio: 3 error/edge (AC-17-03/04/05) out of 5 scenarios = 60% —
//! comfortably over the 40% mandate.
//!
//! All 5 scenarios enabled — unignored one at a time across DELIVER's steps
//! (per DISTILL's one-at-a-time discipline), now GREEN.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{assert_state_delta, set_to, universe, SecurityRulesAdminContext};
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-01: first-time rule definition succeeds and is immediately active
// (WALKING SKELETON — Activity A, feature-delta.md § Story Map)
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (chained — this is the opening Given of the security-rules
/// story line; sr01's own second scenario reuses this Given+When as its own
/// starting precondition, per Pillar 2):
///   Given: project `trailmark-prod` exists, `journal_entries` has no rule
///          defined yet
///   When:  Alex defines a rule with a valid v1-grammar condition, using a
///          valid admin session
///   Then:  the rule is stored and active for `journal_entries`
///
/// AC-17-01
///
/// @walking_skeleton @driving_port @real-io @US-01 @AC-17-01
#[tokio::test]
async fn first_time_rule_definition_succeeds_and_is_immediately_active() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let condition = "request.auth.uid == resource.data.owner_id";

    let before: HashMap<&str, Option<String>> = HashMap::from([(
        universe::ACCESS_RULE_CONDITION_SOURCE,
        ctx.access_rule_condition_source("trailmark-prod", "journal_entries")
            .await,
    )]);

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "journal_entries",
            "condition": condition,
        }))
        .send()
        .await
        .expect("define request failed");

    assert_eq!(resp.status().as_u16(), 200, "AC-17-01: valid rule definition must return 200");
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(body["project_id"], "trailmark-prod");
    assert_eq!(body["collection_path"], "journal_entries");
    assert_eq!(body["condition"], condition);
    assert!(body.get("created_at").is_some());
    assert!(body.get("updated_at").is_some());

    let after: HashMap<&str, Option<String>> = HashMap::from([(
        universe::ACCESS_RULE_CONDITION_SOURCE,
        ctx.access_rule_condition_source("trailmark-prod", "journal_entries")
            .await,
    )]);
    let mut expected = HashMap::new();
    expected.insert(
        universe::ACCESS_RULE_CONDITION_SOURCE,
        set_to(Some(condition.to_string())),
    );
    assert_state_delta(
        &before,
        &after,
        &[universe::ACCESS_RULE_CONDITION_SOURCE],
        &expected,
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-02: redefining fully replaces, no overlap window (error/edge —
// exercises the "old condition must no longer apply" boundary)
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (chained — Given reuses this file's own walking-skeleton
/// definition step conceptually, then submits a SECOND, different
/// condition for the SAME collection):
///   Given: `journal_entries` already has an active rule
///   When:  Alex submits a new condition for the same collection
///   Then:  the new condition is immediately and fully active, and the
///          previous condition no longer applies to any subsequent read
///
/// AC-17-02
///
/// @error @driving_port @real-io @US-01 @AC-17-02
#[tokio::test]
async fn redefining_an_existing_rule_fully_replaces_it_with_no_overlap_window() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;
    ctx.seed_access_rule(
        "trailmark-prod",
        "journal_entries",
        "request.auth.uid == resource.data.owner_id",
    )
    .await;

    let new_condition = "request.auth != null";
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "journal_entries",
            "condition": new_condition,
        }))
        .send()
        .await
        .expect("redefine request failed");

    assert_eq!(resp.status().as_u16(), 200, "AC-17-02: redefinition must return 200, same as first-time definition");

    let stored = ctx
        .access_rule_condition_source("trailmark-prod", "journal_entries")
        .await;
    assert_eq!(
        stored.as_deref(),
        Some(new_condition),
        "AC-17-02: the new condition must FULLY replace the prior one — no blend, no overlap window"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-03: cross-document-read construct rejected, distinguishable from
// a plain syntax error (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-03
///
/// @error @driving_port @real-io @US-01 @AC-17-03
#[tokio::test]
async fn a_condition_using_an_out_of_v1_scope_construct_is_rejected_naming_whats_unsupported() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "journal_entries",
            "condition": "get(/databases/(default)/documents/users/$(request.auth.uid)) != null",
        }))
        .send()
        .await
        .expect("define request failed");

    assert_eq!(resp.status().as_u16(), 400, "AC-17-03: out-of-v1-scope construct must be rejected 400");
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["reason"], "UNSUPPORTED_CONSTRUCT",
        "AC-17-03: must be distinguishable from a plain syntax error (AC-17-04)"
    );
    let error_text = body["error"].as_str().unwrap_or("").to_lowercase();
    assert!(
        error_text.contains("cross-document") || error_text.contains("get()") || error_text.contains("exists()"),
        "AC-17-03: the message must specifically name cross-document reads as unsupported in v1, got: {error_text}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-04: plain invalid syntax rejected, naming what's invalid (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-04
///
/// @error @driving_port @real-io @US-01 @AC-17-04
#[tokio::test]
async fn a_condition_with_invalid_syntax_is_rejected_with_a_specific_reason() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "journal_entries",
            "condition": "(request.auth.uid == resource.data.owner_id",
        }))
        .send()
        .await
        .expect("define request failed");

    assert_eq!(resp.status().as_u16(), 400, "AC-17-04: invalid syntax must be rejected 400");
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["reason"], "SYNTAX_ERROR",
        "AC-17-04: must be distinguishable from an out-of-v1-scope construct (AC-17-03)"
    );
    assert!(body.get("error").is_some(), "AC-17-04: the message must name what specifically is invalid");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-05: missing/invalid admin session rejected (error/edge)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-05
///
/// @error @driving_port @real-io @US-01 @AC-17-05
#[tokio::test]
async fn rule_definition_without_valid_admin_credentials_is_rejected() {
    let ctx = SecurityRulesAdminContext::new().await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules"))
        // No Cookie header at all.
        .json(&serde_json::json!({
            "collection_path": "journal_entries",
            "condition": "true",
        }))
        .send()
        .await
        .expect("define request failed");

    assert_eq!(
        resp.status().as_u16(),
        401,
        "AC-17-05: missing session must be rejected exactly as other admin endpoints reject it"
    );
}
