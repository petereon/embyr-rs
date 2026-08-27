//! Slice 02 (Walking Skeleton) — Alex Sees Exactly What a Rule Used to Say,
//! By Whom, and When (US-02, ADR-035).
//!
//! Acceptance criteria verified here (feature-delta.md US-02):
//!   AC-17-160: history retrieval returns every captured entry, newest
//!              first, each showing its condition, acting account, and
//!              timestamp.
//!   AC-17-161: a collection with no rule ever defined returns an empty
//!              history list, not an error.
//!   AC-17-162: history retrieval is available to any authenticated project
//!              member regardless of role (Viewer included).
//!   AC-17-163: a request without a valid admin session is rejected 401.
//!
//! Driving port: Admin HTTP :9090, GET .../access_rules/:collection_path/history
//! (ADR-035 § Decision — Admin Surface). Reuses `SecurityRulesAdminContext`
//! (Slice 01's own fixture pattern) — real System DB rows, real HTTP calls
//! through `POST .../access_rules` to seed history, never fixture-inserted
//! history rows.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::SecurityRulesAdminContext;

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-160: returns every captured entry, newest first, with
// condition/actor/timestamp
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Alex defines `journal_entries`'s rule, then redefines it with a
///          different condition — both real, successive `define_access_rule`
///          calls
///   When:  Alex retrieves `journal_entries`'s history
///   Then:  both entries come back, newest first, each with its own
///          condition, Alex's real `account_id`, and a timestamp
///
/// AC-17-160
///
/// @walking_skeleton @driving_port @real-io @US-02 @AC-17-160
#[tokio::test]
async fn history_retrieval_returns_every_entry_newest_first_with_condition_actor_and_timestamp() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let first_condition = "request.auth.uid == resource.data.owner_id";
    let second_condition = "request.auth != null";

    for condition in [first_condition, second_condition] {
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
        assert_eq!(resp.status().as_u16(), 200, "seeding define request must succeed");
    }

    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/journal_entries/history"))
        .header("Cookie", &cookie)
        .send()
        .await
        .expect("history request failed");
    assert_eq!(resp.status().as_u16(), 200, "AC-17-160: history retrieval must succeed");

    let body: serde_json::Value = resp.json().await.expect("parse history response");
    let history = body["history"].as_array().expect("history must be an array");
    assert_eq!(
        history.len(),
        2,
        "AC-17-160: must return every captured entry, got {}",
        history.len()
    );
    assert_eq!(
        history[0]["condition"], second_condition,
        "AC-17-160: newest entry (the most recent redefine) must be first"
    );
    assert_eq!(
        history[1]["condition"], first_condition,
        "AC-17-160: oldest entry (the first-ever definition) must be last"
    );
    assert_eq!(
        history[0]["actor_account_id"],
        ctx.account_id.to_string(),
        "AC-17-160: each entry must show the acting account"
    );
    assert!(
        history[0]["captured_at"].is_string(),
        "AC-17-160: each entry must show a capture timestamp"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-161: no rule ever defined returns an empty list, not an error
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-161
///
/// @driving_port @real-io @US-02 @AC-17-161
#[tokio::test]
async fn a_collection_with_no_rule_ever_defined_returns_an_empty_history_list() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/never_defined/history"))
        .header("Cookie", &cookie)
        .send()
        .await
        .expect("history request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-17-161: a collection with no rule ever defined must be 200, never an error"
    );
    let body: serde_json::Value = resp.json().await.expect("parse history response");
    assert_eq!(
        body["history"].as_array().expect("history must be an array").len(),
        0,
        "AC-17-161: history must be an empty list"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-162: any authenticated project member, Viewer included, can retrieve
// history
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: Alex (Owner) has already defined `journal_entries`'s rule
///   When:  Dana, signed in as Viewer, retrieves `journal_entries`'s history
///   Then:  the retrieval succeeds — read access is not gated by role
///
/// AC-17-162
///
/// @driving_port @real-io @US-02 @AC-17-162
#[tokio::test]
async fn a_viewer_role_session_can_retrieve_history() {
    let ctx = SecurityRulesAdminContext::new().await;
    let owner_cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules"))
        .header("Cookie", &owner_cookie)
        .json(&serde_json::json!({
            "collection_path": "journal_entries",
            "condition": "true",
        }))
        .send()
        .await
        .expect("define request failed");
    assert_eq!(resp.status().as_u16(), 200, "seeding define request must succeed");

    let viewer_cookie = ctx.seed_session("dana@trailmark.example", "Viewer").await;
    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/journal_entries/history"))
        .header("Cookie", &viewer_cookie)
        .send()
        .await
        .expect("history request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-17-162: a Viewer-role session must be able to retrieve history"
    );
    let body: serde_json::Value = resp.json().await.expect("parse history response");
    assert_eq!(
        body["history"].as_array().expect("history must be an array").len(),
        1,
        "AC-17-162: Viewer must see the same real history an Owner would"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-163: missing/invalid admin session rejected 401
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-163
///
/// @error @driving_port @real-io @US-02 @AC-17-163
#[tokio::test]
async fn history_retrieval_without_valid_admin_session_is_rejected_401() {
    let ctx = SecurityRulesAdminContext::new().await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .get(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/journal_entries/history"))
        // No Cookie header at all.
        .send()
        .await
        .expect("history request failed");

    assert_eq!(
        resp.status().as_u16(),
        401,
        "AC-17-163: missing session must be rejected exactly as other admin endpoints reject it"
    );
}
