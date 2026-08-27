//! Slice 05 — The Identical History Mechanism Protects Collection-Group
//! Rules Too (US-05, ADR-035). LAST slice of `security-rules-operations`.
//!
//! Acceptance criteria verified here (feature-delta.md US-05):
//!   AC-17-171: `group_access_rules` redefinitions are captured, attributed,
//!              and retrievable — identical mechanism to Slices 01/02/04,
//!              applied to the group table.
//!   AC-17-172: a collection-group rule's history is structurally
//!              independent of any SAME-NAMED exact-path rule's own
//!              history — defining BOTH an exact-path (read) rule AND a
//!              group rule on the SAME collection id/path, with DIFFERENT
//!              condition histories, retrieving one never returns entries
//!              from the other.
//!   AC-17-173: a collection-group rule can be restored to any prior entry
//!              via the identical restore-is-just-define-again mechanism —
//!              proven with a real `RunQuery(all_descendants = true)` call,
//!              mirroring Slice 04's own AC-17-170 shape, applied to the
//!              collection-group path.
//!
//! Driving ports: Admin HTTP :9090 `POST/GET .../group_access_rules[...]`
//! (AC-17-171/172, via `SecurityRulesAdminContext`) and gRPC :8080
//! `RunQuery` (AC-17-173, via `SecurityRulesFullContext` — the same real
//! composition root sr02/swp02/scgr02 use).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    group_access_rule_history_rows, mint_client_identity_token, now_unix, run_query,
    SecurityRulesAdminContext, SecurityRulesFullContext,
};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-171: group-rule redefinitions are captured, attributed, retrievable
// — identical mechanism to Slices 01/02/04, applied to the group table
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: project `trailmark-prod` exists, `journal_entries` has no group
///          rule defined yet, Nora is signed in as Admin
///   When:  Nora defines a group rule, then redefines it with a different
///          condition
///   Then:  BOTH successful calls each produce a matching
///          `group_access_rule_history` entry (condition, Nora's real
///          `account_id`, a capture time) — retrievable via the new history
///          endpoint, newest first
///
/// AC-17-171
///
/// @walking_skeleton @driving_port @real-io @US-05 @AC-17-171
#[tokio::test]
async fn group_rule_redefinitions_are_captured_attributed_and_retrievable() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("nora@trailmark.example", "Admin").await;
    ctx.insert_project("trailmark-prod").await;

    let first_condition = "request.auth.uid == resource.data.owner_id";
    let second_condition = "true";

    for condition in [first_condition, second_condition] {
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
            .expect("define request failed");
        assert_eq!(resp.status().as_u16(), 200, "define/redefine must succeed");
    }

    // Direct row proof (mirrors AC-17-168's own real-domain-observable shape).
    let rows = group_access_rule_history_rows(&ctx, "trailmark-prod", "journal_entries").await;
    assert_eq!(
        rows.len(),
        2,
        "AC-17-171: define + redefine must capture exactly two group-rule history entries"
    );
    assert_eq!(rows[0].condition_source, first_condition);
    assert_eq!(rows[1].condition_source, second_condition);
    assert_eq!(rows[0].actor_account_id, ctx.account_id);
    assert_eq!(rows[1].actor_account_id, ctx.account_id);

    // Retrieval-endpoint proof (mirrors AC-17-168's own shape).
    let resp = ctx
        .client
        .get(ctx.url(
            "/admin/v1/projects/trailmark-prod/group_access_rules/journal_entries/history",
        ))
        .header("Cookie", &cookie)
        .send()
        .await
        .expect("history request failed");
    assert_eq!(resp.status().as_u16(), 200, "AC-17-171: history retrieval must succeed");

    let body: serde_json::Value = resp.json().await.expect("parse history response");
    let history = body["history"].as_array().expect("history must be an array");
    assert_eq!(history.len(), 2, "AC-17-171: must return every captured entry");
    assert_eq!(
        history[0]["condition"], second_condition,
        "AC-17-171: newest entry (the redefine) must be first"
    );
    assert_eq!(
        history[1]["condition"], first_condition,
        "AC-17-171: oldest entry (the first-ever definition) must be last"
    );
    assert_eq!(
        history[0]["actor_account_id"],
        ctx.account_id.to_string(),
        "AC-17-171: each entry must show the acting account"
    );
    assert!(
        history[0]["captured_at"].is_string(),
        "AC-17-171: each entry must show a capture timestamp"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-172: a collection-group rule's history is structurally independent
// of any same-named exact-path rule's own history
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` on `trailmark-prod` gets BOTH an exact-path
///          read rule (`access_rules`) AND a group rule
///          (`group_access_rules`) defined, each redefined once, with
///          DIFFERENT condition histories
///   When:  Alex retrieves the exact-path rule's history and the group
///          rule's history separately
///   Then:  each retrieval returns ONLY its own table's entries — never the
///          other's condition text
///
/// AC-17-172
///
/// @driving_port @real-io @US-05 @AC-17-172
#[tokio::test]
async fn group_rule_history_and_exact_path_rule_history_are_structurally_independent() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let exact_condition_a = "request.auth.uid == resource.data.owner_id";
    let exact_condition_b = "request.auth != null";
    let group_condition_a = "true";
    let group_condition_b = "request.auth.uid == resource.data.owner_id";

    for condition in [exact_condition_a, exact_condition_b] {
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
            .expect("exact-path rule define request failed");
        assert_eq!(resp.status().as_u16(), 200);
    }
    for condition in [group_condition_a, group_condition_b] {
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
            .expect("group rule define request failed");
        assert_eq!(resp.status().as_u16(), 200);
    }

    let exact_resp = ctx
        .client
        .get(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/journal_entries/history"))
        .header("Cookie", &cookie)
        .send()
        .await
        .expect("exact-path history request failed");
    let exact_body: serde_json::Value = exact_resp.json().await.expect("parse exact-path history");
    let exact_history = exact_body["history"].as_array().expect("history must be an array");

    let group_resp = ctx
        .client
        .get(ctx.url(
            "/admin/v1/projects/trailmark-prod/group_access_rules/journal_entries/history",
        ))
        .header("Cookie", &cookie)
        .send()
        .await
        .expect("group history request failed");
    let group_body: serde_json::Value = group_resp.json().await.expect("parse group history");
    let group_history = group_body["history"].as_array().expect("history must be an array");

    assert_eq!(exact_history.len(), 2, "AC-17-172: exact-path history must have its own two entries");
    assert_eq!(group_history.len(), 2, "AC-17-172: group history must have its own two entries");

    let exact_conditions: Vec<&str> =
        exact_history.iter().map(|e| e["condition"].as_str().unwrap()).collect();
    let group_conditions: Vec<&str> =
        group_history.iter().map(|e| e["condition"].as_str().unwrap()).collect();

    assert!(
        exact_conditions.contains(&exact_condition_a) && exact_conditions.contains(&exact_condition_b),
        "AC-17-172: exact-path history must contain exactly the exact-path rule's own conditions"
    );
    assert!(
        group_conditions.contains(&group_condition_a) && group_conditions.contains(&group_condition_b),
        "AC-17-172: group history must contain exactly the group rule's own conditions"
    );
    assert!(
        !exact_conditions.iter().any(|c| group_conditions.contains(c) && !exact_conditions.contains(c)),
        "AC-17-172: exact-path history must never contain a group-rule-only condition, got {exact_conditions:?}"
    );
    assert!(
        !group_conditions.iter().any(|c| exact_conditions.contains(c) && !group_conditions.contains(c)),
        "AC-17-172: group history must never contain an exact-path-rule-only condition, got {group_conditions:?}"
    );
    // group_condition_b and exact_condition_a share the SAME text on
    // purpose (both "request.auth.uid == resource.data.owner_id") — the
    // real non-interference proof is each table's ROW COUNT (2 each, never
    // 3+) and each table's EXCLUSIVE-only conditions
    // (exact_condition_b/group_condition_a) landing in exactly one side.
    assert!(
        exact_conditions.contains(&exact_condition_b) && !group_conditions.contains(&exact_condition_b),
        "AC-17-172: exact-path-only condition must never appear in group history"
    );
    assert!(
        group_conditions.contains(&group_condition_a) && !exact_conditions.contains(&group_condition_a),
        "AC-17-172: group-only condition must never appear in exact-path history"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-173: a collection-group rule can be restored to any prior entry via
// the identical restore-is-just-define-again mechanism — real domain proof
// via RunQuery
// ─────────────────────────────────────────────────────────────────────────────

/// Calls the EXISTING `define_group_access_rule` admin action against
/// `journal_entries` on `ctx.project_id` — used for the initial definition,
/// the redefinition, AND the restore (all three ARE the identical action,
/// mirrors Slice 04's own `define_flagged_content_write_rule` precedent
/// exactly).
async fn define_journal_entries_group_rule(ctx: &SecurityRulesFullContext, cookie: &str, condition: &str) {
    let resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/group_access_rules",
            ctx.project_id
        )))
        .header("Cookie", cookie)
        .json(&serde_json::json!({
            "collection_id": "journal_entries",
            "condition": condition,
        }))
        .send()
        .await
        .expect("define request failed");
    assert_eq!(resp.status().as_u16(), 200, "define/redefine/restore must succeed");
}

/// Journey (mirrors AC-17-170's own shape, applied to the collection-group
/// path):
///   Given: `journal_entries`'s group rule is defined with condition A
///          (ownership-required: `request.auth.uid ==
///          resource.data.owner_id`), then redefined to condition B (`true`,
///          open) — Dana issues an unfiltered `collectionGroup` query
///   When:  Alex restores the rule using condition A's exact text
///   Then:  Dana's real unfiltered `RunQuery(all_descendants = true)` call
///          is rejected again (missing the required ownership filter) —
///          identical evaluated behavior to condition A restored
///
/// AC-17-173
///
/// @walking_skeleton @driving_port @real-io @US-05 @AC-17-173
#[tokio::test]
async fn restoring_a_group_rules_prior_condition_restores_identical_evaluated_behavior() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sro05-restore-group").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    let condition_a = "request.auth.uid == resource.data.owner_id";
    let condition_b = "true";

    // Condition A active: Dana's unfiltered collection-group query is
    // rejected (missing the required ownership-equality filter).
    define_journal_entries_group_rule(&ctx, &cookie, condition_a).await;
    let under_a = run_query(&ctx, "journal_entries", true, &[], Some(&danas_token)).await;
    assert!(
        under_a.is_err(),
        "precondition: under condition A, Dana's unfiltered group query must be rejected"
    );

    // Redefine to condition B: Dana's unfiltered query is now admitted.
    define_journal_entries_group_rule(&ctx, &cookie, condition_b).await;
    let under_b = run_query(&ctx, "journal_entries", true, &[], Some(&danas_token)).await;
    assert!(
        under_b.is_ok(),
        "precondition: under redefined condition B, Dana's unfiltered group query must be \
         allowed: {:?}",
        under_b.err()
    );

    // Restore: redefine with condition A's EXACT text.
    define_journal_entries_group_rule(&ctx, &cookie, condition_a).await;
    let restored = run_query(&ctx, "journal_entries", true, &[], Some(&danas_token)).await;
    let err = restored.expect_err(
        "AC-17-173: after restoring condition A's exact text, Dana's unfiltered group query \
         must be rejected again",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-173: restored behavior must be identical to condition A's original evaluated \
         behavior (PermissionDenied), got {:?}",
        err.code()
    );
}
