//! security-rules-collection-group-rules Slice 01 (US-01, ADR-032) — Alex
//! Defines (and Redefines) an Independent Collection-Group Rule for a
//! Collection ID.
//!
//! Storage + admin-API surface only (this slice does NOT implement
//! collection-group query-time enforcement — that is Slices 02-04).
//! Acceptance criteria verified here (feature-delta.md
//! `security-rules-collection-group-rules`):
//!   AC-17-77: first-time group-rule definition is stored and active.
//!   AC-17-78: redefining fully replaces, no merge/overlap window.
//!   AC-17-79: group-rule define/redefine has ZERO observable effect on the
//!             same collection id's exact-path `access_rules` row, and vice
//!             versa (this feature's single highest-consequence storage
//!             -independence guarantee).
//!   AC-17-80: a `/`-containing collection id is rejected with a
//!             distinguishable reason, before ever reaching the DB.
//!
//! Driving port: Admin HTTP :9090 (`SecurityRulesAdminContext`, real
//! `build_admin_router` composition root, reused unmodified from
//! `security-rules`'/`security-rules-write-path`'s own fixture — Pillar 3).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    assert_state_delta, group_access_rule_condition_source, seed_group_access_rule, set_to,
    unchanged, SecurityRulesAdminContext,
};
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-77: first-time group-rule definition succeeds and is immediately
// active for the named collection id.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: project `trailmark-prod` exists, `journal_entries` has an active
///          exact-path rule from `security-rules`, and no group rule yet
///   When:  Alex defines a group rule for collection id `journal_entries`
///          with a valid decidable-shape condition, using a valid admin
///          session
///   Then:  the group rule is stored and active for collection id
///          `journal_entries`
///
/// AC-17-77
///
/// @driving_port @real-io @US-01 @AC-17-77
#[tokio::test]
async fn first_time_group_rule_definition_is_stored_and_active() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;
    ctx.seed_access_rule(
        "trailmark-prod",
        "journal_entries",
        "request.auth.uid == resource.data.owner_id",
    )
    .await;

    let condition = "request.auth.uid == resource.data.owner_id";

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/group_access_rules"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_id": "journal_entries",
            "condition": condition,
        }))
        .send()
        .await
        .expect("define group-rule request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-17-77: a valid first-time group-rule definition must return 200"
    );
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(body["project_id"], "trailmark-prod");
    assert_eq!(body["collection_id"], "journal_entries");
    assert_eq!(body["condition"], condition);
    assert!(body.get("created_at").is_some());
    assert!(body.get("updated_at").is_some());

    let stored =
        group_access_rule_condition_source(&ctx, "trailmark-prod", "journal_entries").await;
    assert_eq!(
        stored.as_deref(),
        Some(condition),
        "AC-17-77: the condition must be persisted in group_access_rules"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-78: redefining a group rule fully and immediately replaces the
// prior condition — no merge, no overlap window.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` already has an active group rule
///   When:  Alex submits a new condition for the same collection id's group
///          rule
///   Then:  the new condition is immediately and fully active, and the
///          previous condition no longer applies
///
/// AC-17-78
///
/// @error @driving_port @real-io @US-01 @AC-17-78
#[tokio::test]
async fn redefining_an_existing_group_rule_fully_replaces_it_with_no_overlap_window() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;
    seed_group_access_rule(
        &ctx,
        "trailmark-prod",
        "journal_entries",
        "request.auth.uid == resource.data.owner_id",
    )
    .await;

    let new_condition = "request.auth != null";
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/group_access_rules"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_id": "journal_entries",
            "condition": new_condition,
        }))
        .send()
        .await
        .expect("redefine group-rule request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-17-78: redefinition must return 200, same as first-time definition"
    );

    let stored =
        group_access_rule_condition_source(&ctx, "trailmark-prod", "journal_entries").await;
    assert_eq!(
        stored.as_deref(),
        Some(new_condition),
        "AC-17-78: the new condition must FULLY replace the prior one — no blend, no overlap window"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-79: defining/redefining a GROUP rule has ZERO observable effect on
// the SAME collection id's exact-path `access_rules` row, and vice versa —
// independent conditions, independently upsertable, DIFFERENT conditions
// throughout. This feature's single highest-consequence storage-
// independence guarantee.
// ─────────────────────────────────────────────────────────────────────────────

const EXACT_PATH_CONDITION: &str = "exact_path_rule.condition_source";
const GROUP_CONDITION: &str = "group_rule.condition_source";

/// Journey (both directions in one behavior — independence is symmetric):
///   Given: `journal_entries` has BOTH an active exact-path rule
///          (`access_rules`) and an active group rule
///          (`group_access_rules`), with two DIFFERENT conditions
///   When:  Alex redefines the GROUP rule alone
///   Then:  the exact-path rule's stored condition is byte-for-byte
///          unchanged
///   When:  Alex then redefines the exact-path rule alone (existing
///          `/access_rules` endpoint, untouched by this feature)
///   Then:  the group rule's stored condition (just redefined above) is
///          byte-for-byte unchanged
///
/// AC-17-79
///
/// @driving_port @real-io @US-01 @AC-17-79
#[tokio::test]
async fn group_rule_and_exact_path_rule_are_fully_independent_in_both_directions() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let original_exact_path_condition = "resource.data.title == resource.data.title";
    let original_group_condition = "request.auth.uid == resource.data.owner_id";
    ctx.seed_access_rule(
        "trailmark-prod",
        "journal_entries",
        original_exact_path_condition,
    )
    .await;
    seed_group_access_rule(
        &ctx,
        "trailmark-prod",
        "journal_entries",
        original_group_condition,
    )
    .await;

    // ── Direction 1: redefine GROUP -> exact-path must be untouched ──
    let before_1: HashMap<&str, Option<String>> = HashMap::from([
        (
            EXACT_PATH_CONDITION,
            ctx.access_rule_condition_source("trailmark-prod", "journal_entries")
                .await,
        ),
        (
            GROUP_CONDITION,
            group_access_rule_condition_source(&ctx, "trailmark-prod", "journal_entries").await,
        ),
    ]);

    let new_group_condition = "request.auth != null";
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/group_access_rules"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_id": "journal_entries",
            "condition": new_group_condition,
        }))
        .send()
        .await
        .expect("define group-rule request failed");
    assert_eq!(resp.status().as_u16(), 200);

    let after_1: HashMap<&str, Option<String>> = HashMap::from([
        (
            EXACT_PATH_CONDITION,
            ctx.access_rule_condition_source("trailmark-prod", "journal_entries")
                .await,
        ),
        (
            GROUP_CONDITION,
            group_access_rule_condition_source(&ctx, "trailmark-prod", "journal_entries").await,
        ),
    ]);
    let mut expected_1 = HashMap::new();
    expected_1.insert(GROUP_CONDITION, set_to(Some(new_group_condition.to_string())));
    expected_1.insert(EXACT_PATH_CONDITION, unchanged());
    assert_state_delta(
        &before_1,
        &after_1,
        &[EXACT_PATH_CONDITION, GROUP_CONDITION],
        &expected_1,
    );

    // ── Direction 2: redefine exact-path (existing endpoint) -> GROUP must be untouched ──
    let before_2 = after_1;

    let new_exact_path_condition = "true";
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "journal_entries",
            "condition": new_exact_path_condition,
        }))
        .send()
        .await
        .expect("define exact-path rule request failed");
    assert_eq!(resp.status().as_u16(), 200);

    let after_2: HashMap<&str, Option<String>> = HashMap::from([
        (
            EXACT_PATH_CONDITION,
            ctx.access_rule_condition_source("trailmark-prod", "journal_entries")
                .await,
        ),
        (
            GROUP_CONDITION,
            group_access_rule_condition_source(&ctx, "trailmark-prod", "journal_entries").await,
        ),
    ]);
    let mut expected_2 = HashMap::new();
    expected_2.insert(
        EXACT_PATH_CONDITION,
        set_to(Some(new_exact_path_condition.to_string())),
    );
    expected_2.insert(GROUP_CONDITION, unchanged());
    assert_state_delta(
        &before_2,
        &after_2,
        &[EXACT_PATH_CONDITION, GROUP_CONDITION],
        &expected_2,
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-80: a collection id containing a `/` is rejected as invalid for a
// group-rule definition, with a distinguishable reason — before ever
// reaching the DB (the DB CHECK constraint is a separate, independent
// defense-in-depth layer, proven at the adapter level in
// `crates/embyr-server/src/adapters/system_db.rs`'s own test module, not
// here).
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: project `trailmark-prod` exists
///   When:  Alex submits a group-rule definition with collection id
///          `expeditions/journal_entries`
///   Then:  the request is rejected 400 with a distinguishable reason naming
///          that a collection-group id must be a bare identifier, not a
///          path, and nothing is persisted
///
/// AC-17-80
///
/// @error @driving_port @real-io @US-01 @AC-17-80
#[tokio::test]
async fn a_collection_id_containing_a_path_separator_is_rejected() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/group_access_rules"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_id": "expeditions/journal_entries",
            "condition": "request.auth.uid == resource.data.owner_id",
        }))
        .send()
        .await
        .expect("define group-rule request failed");

    assert_eq!(
        resp.status().as_u16(),
        400,
        "AC-17-80: a '/'-containing collection id must be rejected 400"
    );
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(
        body["reason"], "INVALID_COLLECTION_ID",
        "AC-17-80: rejection must carry a distinguishable reason, separate from SYNTAX_ERROR/UNSUPPORTED_CONSTRUCT"
    );
    assert!(body.get("error").is_some());

    let stored =
        group_access_rule_condition_source(&ctx, "trailmark-prod", "expeditions/journal_entries")
            .await;
    assert!(
        stored.is_none(),
        "AC-17-80: a rejected collection id must never be persisted"
    );
}
