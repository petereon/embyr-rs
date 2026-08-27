//! Slice 01 (Walking Skeleton) — A Rule's Every State Is Captured,
//! Attributed, and Timestamped (US-01, ADR-035).
//!
//! Acceptance criteria verified here (feature-delta.md US-01):
//!   AC-17-156: every successful `define_access_rule` call (first-time AND
//!              redefine) captures a history entry — the condition just
//!              made active, the acting admin's real `account_id`, and a
//!              capture time.
//!   AC-17-157: a rule's first-ever definition produces EXACTLY ONE history
//!              entry (not zero, not two).
//!   AC-17-158: two successive redefinitions in quick succession produce two
//!              distinct, correctly-ordered entries (`id`-based ordering,
//!              not timestamp-based).
//!
//! AC-17-159 (existing `access_rules`/`define_access_rule` behavior
//! completely unmodified) is proven by running the EXISTING, untouched
//! `security_rules_sr01_define_and_redefine_rule` target — not a new test
//! here (see DELIVER report).
//!
//! Driving port: Admin HTTP :9090 (`SecurityRulesAdminContext`, reused from
//! `security-rules`' own fixture module — the SAME `POST .../access_rules`
//! route this feature's history capture is fused into, ADR-035 § Decision —
//! Capture Mechanism Placement).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{access_rule_history_rows, SecurityRulesAdminContext};

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-156: every successful define/redefine captures a history entry
// (condition, actor account_id, capture time)
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: project `trailmark-prod` exists, `journal_entries` has no rule
///          defined yet, Nora is signed in as Admin
///   When:  Nora defines a rule, then redefines it with a different
///          condition
///   Then:  BOTH successful calls each produce a matching history entry —
///          the condition just made active, Nora's real `account_id`, and
///          a capture time
///
/// AC-17-156
///
/// @walking_skeleton @driving_port @real-io @US-01 @AC-17-156
#[tokio::test]
async fn every_successful_define_or_redefine_captures_a_history_entry_with_condition_actor_and_time(
) {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("nora@trailmark.example", "Admin").await;
    ctx.insert_project("trailmark-prod").await;

    let first_condition = "request.auth.uid == resource.data.owner_id";
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "journal_entries",
            "condition": first_condition,
        }))
        .send()
        .await
        .expect("define request failed");
    assert_eq!(resp.status().as_u16(), 200, "first-time definition must succeed");

    let rows = access_rule_history_rows(&ctx, "trailmark-prod", "journal_entries").await;
    assert_eq!(rows.len(), 1, "AC-17-156: first-time definition must capture exactly one entry");
    assert_eq!(rows[0].condition_source, first_condition, "AC-17-156: captured condition must match the one just made active");
    assert_eq!(rows[0].actor_account_id, ctx.account_id, "AC-17-156: captured actor must be the real acting admin's account_id");

    let second_condition = "request.auth != null";
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "journal_entries",
            "condition": second_condition,
        }))
        .send()
        .await
        .expect("redefine request failed");
    assert_eq!(resp.status().as_u16(), 200, "redefinition must succeed");

    let rows = access_rule_history_rows(&ctx, "trailmark-prod", "journal_entries").await;
    assert_eq!(rows.len(), 2, "AC-17-156: redefinition must ALSO capture a history entry — total is now two");
    let redefine_entry = &rows[1];
    assert_eq!(redefine_entry.condition_source, second_condition, "AC-17-156: redefine's captured condition must be the NEW value just made active");
    assert_eq!(redefine_entry.actor_account_id, ctx.account_id, "AC-17-156: captured actor must be the real acting admin's account_id on redefine too");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-157: a rule's first-ever definition produces EXACTLY ONE history
// entry (not zero, not two)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-157
///
/// @driving_port @real-io @US-01 @AC-17-157
#[tokio::test]
async fn a_rules_first_ever_definition_produces_exactly_one_history_entry() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "trail_guides",
            "condition": "true",
        }))
        .send()
        .await
        .expect("define request failed");
    assert_eq!(resp.status().as_u16(), 200);

    let rows = access_rule_history_rows(&ctx, "trailmark-prod", "trail_guides").await;
    assert_eq!(
        rows.len(),
        1,
        "AC-17-157: first-ever definition must produce exactly one history entry, got {}",
        rows.len()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-158: two successive redefinitions in quick succession produce two
// distinct, correctly-ordered entries (id-based ordering)
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (mirrors US-01's own domain example — Nora redefines
/// `journal_entries`'s rule twice in rapid succession):
///   Given: `journal_entries` already has an active rule
///   When:  Nora submits two DIFFERENT conditions back-to-back, with no
///          delay between them
///   Then:  exactly two NEW history entries exist, in the correct
///          submission order, ordered by the monotonic `id` column
///
/// AC-17-158
///
/// @driving_port @real-io @US-01 @AC-17-158
#[tokio::test]
async fn two_rapid_successive_redefinitions_produce_two_distinct_correctly_ordered_entries() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("nora@trailmark.example", "Admin").await;
    ctx.insert_project("trailmark-prod").await;
    ctx.seed_access_rule(
        "trailmark-prod",
        "journal_entries",
        "request.auth.uid == resource.data.owner_id",
    )
    .await;

    let redefine_a = "request.auth != null";
    let redefine_b = "true";

    for condition in [redefine_a, redefine_b] {
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
            .expect("redefine request failed");
        assert_eq!(resp.status().as_u16(), 200);
    }

    let rows = access_rule_history_rows(&ctx, "trailmark-prod", "journal_entries").await;
    assert_eq!(
        rows.len(),
        2,
        "AC-17-158: two rapid successive redefinitions must produce exactly two history entries"
    );
    assert!(
        rows[0].id < rows[1].id,
        "AC-17-158: entries must be correctly ordered by the monotonic id column"
    );
    assert_eq!(rows[0].condition_source, redefine_a, "AC-17-158: the first redefinition's entry must be ordered first");
    assert_eq!(rows[1].condition_source, redefine_b, "AC-17-158: the second redefinition's entry must be ordered second");
}
