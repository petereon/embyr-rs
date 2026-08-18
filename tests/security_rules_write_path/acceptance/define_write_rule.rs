//! security-rules-write-path Slice 01 (US-01, ADR-030) — Alex Defines (and
//! Redefines) an Independent Write Rule for a Collection.
//!
//! Storage + admin-API surface only (this slice does NOT implement write-
//! time evaluation — that is Slices 02-04). Acceptance criteria verified
//! here (feature-delta.md `security-rules-write-path`):
//!   AC-17-20: first-time write-rule definition is stored and active.
//!   AC-17-21: redefining fully replaces, no merge/overlap window.
//!   AC-17-22: write-rule define/redefine has ZERO observable effect on the
//!             same collection's READ rule, and vice versa (AC-17-43's own
//!             acceptance-level proof — this feature's single highest
//!             -consequence architectural guarantee).
//!   AC-17-23 (partial): arbitrary valid v1-grammar text is accepted and
//!             stored via the EXISTING, unmodified `parse_condition`.
//!   AC-17-24: an out-of-v1-grammar construct is rejected with the same
//!             distinguishable UNSUPPORTED_CONSTRUCT/SYNTAX_ERROR taxonomy
//!             `security-rules` already established.
//!   AC-17-25: missing admin credential is rejected consistently with
//!             existing admin-endpoint behavior.
//!
//! Driving port: Admin HTTP :9090 (`SecurityRulesAdminContext`, real
//! `build_admin_router` composition root, reused unmodified from
//! `security-rules`' own fixture — Pillar 3).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{assert_state_delta, set_to, unchanged};
use common::{
    seed_write_access_rule, write_access_rule_condition_source, SecurityRulesAdminContext,
};
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-20: first-time write-rule definition succeeds and is immediately
// active for the named collection.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: project `trailmark-prod` exists, `journal_entries` has no write
///          rule defined yet
///   When:  Alex defines a write rule with a valid v1-grammar condition,
///          using a valid admin session
///   Then:  the write rule is stored and active for `journal_entries`, in
///          `write_access_rules` — a table entirely independent of
///          `access_rules`
///
/// AC-17-20
///
/// @driving_port @real-io @US-01 @AC-17-20
#[tokio::test]
async fn first_time_write_rule_definition_is_stored_and_active() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let condition = "request.auth.uid == resource.data.owner_id";

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/write_access_rules"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "journal_entries",
            "condition": condition,
        }))
        .send()
        .await
        .expect("define write-rule request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-17-20: a valid first-time write-rule definition must return 200"
    );
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(body["project_id"], "trailmark-prod");
    assert_eq!(body["collection_path"], "journal_entries");
    assert_eq!(body["condition"], condition);
    assert!(body.get("created_at").is_some());
    assert!(body.get("updated_at").is_some());

    let stored =
        write_access_rule_condition_source(&ctx, "trailmark-prod", "journal_entries").await;
    assert_eq!(
        stored.as_deref(),
        Some(condition),
        "AC-17-20: the condition must be persisted in write_access_rules"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-21: redefining a write rule fully and immediately replaces the
// prior condition — no merge, no overlap window.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` already has an active WRITE rule
///   When:  Alex submits a new condition for the same collection's write
///          rule
///   Then:  the new condition is immediately and fully active, and the
///          previous condition no longer applies
///
/// AC-17-21
///
/// @error @driving_port @real-io @US-01 @AC-17-21
#[tokio::test]
async fn redefining_an_existing_write_rule_fully_replaces_it_with_no_overlap_window() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;
    seed_write_access_rule(
        &ctx,
        "trailmark-prod",
        "journal_entries",
        "request.auth.uid == resource.data.owner_id",
    )
    .await;

    let new_condition = "request.auth != null";
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/write_access_rules"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "journal_entries",
            "condition": new_condition,
        }))
        .send()
        .await
        .expect("redefine write-rule request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-17-21: redefinition must return 200, same as first-time definition"
    );

    let stored =
        write_access_rule_condition_source(&ctx, "trailmark-prod", "journal_entries").await;
    assert_eq!(
        stored.as_deref(),
        Some(new_condition),
        "AC-17-21: the new condition must FULLY replace the prior one — no blend, no overlap window"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-22 (== AC-17-43's acceptance-level proof): defining/redefining a
// WRITE rule has ZERO observable effect on the SAME collection's READ rule,
// and vice versa — independent conditions, independently upsertable. This
// feature's single highest-consequence architectural guarantee.
// ─────────────────────────────────────────────────────────────────────────────

const READ_CONDITION: &str = "read_rule.condition_source";
const WRITE_CONDITION: &str = "write_rule.condition_source";

/// Journey (both directions in one behavior — independence is symmetric):
///   Given: `journal_entries` has BOTH an active READ rule and an active
///          WRITE rule, with two DIFFERENT conditions
///   When:  Alex redefines the WRITE rule alone
///   Then:  the READ rule's stored condition is byte-for-byte unchanged
///   When:  Alex then redefines the READ rule alone (existing
///          `/access_rules` endpoint, untouched by this feature)
///   Then:  the WRITE rule's stored condition (just redefined above) is
///          byte-for-byte unchanged
///
/// AC-17-22 / AC-17-43
///
/// @driving_port @real-io @US-01 @AC-17-22 @AC-17-43
#[tokio::test]
async fn write_rule_and_read_rule_are_fully_independent_in_both_directions() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let original_read_condition = "resource.data.title == resource.data.title";
    let original_write_condition = "request.auth.uid == resource.data.owner_id";
    ctx.seed_access_rule("trailmark-prod", "journal_entries", original_read_condition)
        .await;
    seed_write_access_rule(
        &ctx,
        "trailmark-prod",
        "journal_entries",
        original_write_condition,
    )
    .await;

    // ── Direction 1: redefine WRITE -> READ must be untouched ──
    let before_1: HashMap<&str, Option<String>> = HashMap::from([
        (
            READ_CONDITION,
            ctx.access_rule_condition_source("trailmark-prod", "journal_entries")
                .await,
        ),
        (
            WRITE_CONDITION,
            write_access_rule_condition_source(&ctx, "trailmark-prod", "journal_entries").await,
        ),
    ]);

    let new_write_condition = "request.auth != null";
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/write_access_rules"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "journal_entries",
            "condition": new_write_condition,
        }))
        .send()
        .await
        .expect("define write-rule request failed");
    assert_eq!(resp.status().as_u16(), 200);

    let after_1: HashMap<&str, Option<String>> = HashMap::from([
        (
            READ_CONDITION,
            ctx.access_rule_condition_source("trailmark-prod", "journal_entries")
                .await,
        ),
        (
            WRITE_CONDITION,
            write_access_rule_condition_source(&ctx, "trailmark-prod", "journal_entries").await,
        ),
    ]);
    let mut expected_1 = HashMap::new();
    expected_1.insert(
        WRITE_CONDITION,
        set_to(Some(new_write_condition.to_string())),
    );
    expected_1.insert(READ_CONDITION, unchanged());
    assert_state_delta(
        &before_1,
        &after_1,
        &[READ_CONDITION, WRITE_CONDITION],
        &expected_1,
    );

    // ── Direction 2: redefine READ (existing endpoint) -> WRITE must be untouched ──
    let before_2 = after_1;

    let new_read_condition = "true";
    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/access_rules"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "journal_entries",
            "condition": new_read_condition,
        }))
        .send()
        .await
        .expect("define read-rule request failed");
    assert_eq!(resp.status().as_u16(), 200);

    let after_2: HashMap<&str, Option<String>> = HashMap::from([
        (
            READ_CONDITION,
            ctx.access_rule_condition_source("trailmark-prod", "journal_entries")
                .await,
        ),
        (
            WRITE_CONDITION,
            write_access_rule_condition_source(&ctx, "trailmark-prod", "journal_entries").await,
        ),
    ]);
    let mut expected_2 = HashMap::new();
    expected_2.insert(READ_CONDITION, set_to(Some(new_read_condition.to_string())));
    expected_2.insert(WRITE_CONDITION, unchanged());
    assert_state_delta(
        &before_2,
        &after_2,
        &[READ_CONDITION, WRITE_CONDITION],
        &expected_2,
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-23 (partial, this slice only): arbitrary valid v1-grammar text is
// accepted and stored via the EXISTING, unmodified `parse_condition` — the
// `request.resource.data.<field>` grammar extension itself is Slice 02's
// job, NOT this slice's (no `Operand::RequestResourceField` implemented
// here).
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: project `trailmark-prod` exists
///   When:  Alex defines a write rule using a condition that exercises `!`,
///          `&&`, `||`, parenthesized grouping, and a bare `true` literal —
///          NOT just the simple ownership-equality shape earlier tests use
///   Then:  it is accepted and stored verbatim, proving the write-rule
///          endpoint reuses the FULL v1 grammar `parse_condition` already
///          supports, not a narrower hand-picked subset
///
/// AC-17-23 (partial)
///
/// @driving_port @real-io @US-01 @AC-17-23
#[tokio::test]
async fn arbitrary_valid_v1_grammar_condition_is_accepted_and_stored_verbatim() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let condition =
        "!(request.auth == null) && (resource.data.owner_id == request.auth.uid || true)";

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/write_access_rules"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "journal_entries",
            "condition": condition,
        }))
        .send()
        .await
        .expect("define write-rule request failed");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "AC-17-23: arbitrary valid v1-grammar text (!, &&, ||, parens, true) must be accepted"
    );
    let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
    assert_eq!(body["condition"], condition);

    let stored =
        write_access_rule_condition_source(&ctx, "trailmark-prod", "journal_entries").await;
    assert_eq!(stored.as_deref(), Some(condition));
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-24: a construct outside the v1 grammar is rejected with the SAME
// distinguishable UNSUPPORTED_CONSTRUCT/SYNTAX_ERROR taxonomy
// `security-rules` already established (input variations of the SAME
// "rejected with distinguishable taxonomy" behavior — parametrized, per
// Mandate 5, rather than split into two ACs' worth of separate tests).
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-24
///
/// @error @driving_port @real-io @US-01 @AC-17-24
#[tokio::test]
async fn a_condition_outside_the_v1_grammar_is_rejected_with_the_established_taxonomy() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let cases: &[(&str, &str)] = &[
        (
            // Recognized-but-out-of-v1-scope shape.
            "get(/databases/(default)/documents/users/$(request.auth.uid)) != null",
            "UNSUPPORTED_CONSTRUCT",
        ),
        (
            // Plain invalid syntax (unbalanced parens).
            "(request.auth.uid == resource.data.owner_id",
            "SYNTAX_ERROR",
        ),
    ];

    for (condition, expected_reason) in cases {
        let resp = ctx
            .client
            .post(ctx.url("/admin/v1/projects/trailmark-prod/write_access_rules"))
            .header("Cookie", &cookie)
            .json(&serde_json::json!({
                "collection_path": "journal_entries",
                "condition": condition,
            }))
            .send()
            .await
            .expect("define write-rule request failed");

        assert_eq!(
            resp.status().as_u16(),
            400,
            "AC-17-24: '{condition}' must be rejected 400"
        );
        let body: serde_json::Value = resp.json().await.expect("response body must be JSON");
        assert_eq!(
            body["reason"], *expected_reason,
            "AC-17-24: '{condition}' must be tagged {expected_reason}"
        );
        assert!(body.get("error").is_some());

        let stored =
            write_access_rule_condition_source(&ctx, "trailmark-prod", "journal_entries").await;
        assert!(
            stored.is_none(),
            "AC-17-24: a rejected condition must never be persisted"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-25: missing or invalid admin credential is rejected, consistent
// with existing admin-endpoint behavior (may already be GREEN via existing
// session_auth_middleware — confirmed by testing, not assumed).
// ─────────────────────────────────────────────────────────────────────────────

/// AC-17-25
///
/// @error @driving_port @real-io @US-01 @AC-17-25
#[tokio::test]
async fn write_rule_definition_without_valid_admin_credentials_is_rejected() {
    let ctx = SecurityRulesAdminContext::new().await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/write_access_rules"))
        // No Cookie header, no Authorization Bearer header at all.
        .json(&serde_json::json!({
            "collection_path": "journal_entries",
            "condition": "true",
        }))
        .send()
        .await
        .expect("define write-rule request failed");

    assert_eq!(
        resp.status().as_u16(),
        401,
        "AC-17-25: missing admin credential must be rejected exactly as other admin endpoints reject it"
    );

    let stored =
        write_access_rule_condition_source(&ctx, "trailmark-prod", "journal_entries").await;
    assert!(
        stored.is_none(),
        "AC-17-25: an unauthenticated request must never write a rule"
    );
}

/// Journey:
///   Given: a valid, signed-in session with the Viewer role (below Admin)
///   When:  that session attempts to define a write rule
///   Then:  rejected 403 — Viewer is insufficient, distinct from the 401
///          "no credential at all" case above (mirrors AC-17-05's own
///          precedent for the read-rule endpoint).
///
/// AC-17-25
///
/// @error @driving_port @real-io @US-01 @AC-17-25
#[tokio::test]
async fn write_rule_definition_by_a_viewer_role_session_is_rejected_403() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("dana@trailmark.example", "Viewer").await;
    ctx.insert_project("trailmark-prod").await;

    let resp = ctx
        .client
        .post(ctx.url("/admin/v1/projects/trailmark-prod/write_access_rules"))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "journal_entries",
            "condition": "true",
        }))
        .send()
        .await
        .expect("define write-rule request failed");

    assert_eq!(
        resp.status().as_u16(),
        403,
        "AC-17-25: Viewer role must be rejected 403 — insufficient role, distinct from 401 (no credential)"
    );
}
