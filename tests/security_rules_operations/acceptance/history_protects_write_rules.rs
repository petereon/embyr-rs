//! Slice 04 — The Identical History Mechanism Protects Write Rules Too
//! (US-04, ADR-035).
//!
//! Acceptance criteria verified here (feature-delta.md US-04):
//!   AC-17-168: `write_access_rules` redefinitions are captured, attributed,
//!              and retrievable — identical mechanism to Slices 01/02,
//!              applied to the write table.
//!   AC-17-169: a collection's READ-rule history and WRITE-rule history are
//!              structurally independent — defining BOTH a read rule and a
//!              write rule (with DIFFERENT condition histories) on the SAME
//!              collection, retrieving one never returns entries from the
//!              other.
//!   AC-17-170: a write rule can be restored to any prior entry via the
//!              identical restore-is-just-define-again mechanism — proven
//!              with a real `CreateDocument` call, mirroring Slice 03's own
//!              AC-17-164 shape, applied to write-path.
//!
//! Driving ports: Admin HTTP :9090 `POST/GET .../write_access_rules[...]`
//! (AC-17-168/169, via `SecurityRulesAdminContext`) and gRPC :8080
//! `CreateDocument` (AC-17-170, via `SecurityRulesFullContext` — the same
//! real composition root `sr02`/`swp02` use).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    create_document, mint_client_identity_token, now_unix, string_field,
    write_access_rule_history_rows, SecurityRulesAdminContext, SecurityRulesFullContext,
};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-168: write-rule redefinitions are captured, attributed, retrievable
// — identical mechanism to Slices 01/02, applied to the write table
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: project `trailmark-prod` exists, `flagged_content` has no write
///          rule defined yet, Nora is signed in as Admin
///   When:  Nora defines a write rule, then redefines it with a different
///          condition
///   Then:  BOTH successful calls each produce a matching
///          `write_access_rule_history` entry (condition, Nora's real
///          `account_id`, a capture time) — retrievable via the new history
///          endpoint, newest first
///
/// AC-17-168
///
/// @walking_skeleton @driving_port @real-io @US-04 @AC-17-168
#[tokio::test]
async fn write_rule_redefinitions_are_captured_attributed_and_retrievable() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("nora@trailmark.example", "Admin").await;
    ctx.insert_project("trailmark-prod").await;

    let first_condition = "request.resource.data.owner_id == request.auth.uid";
    let second_condition = "request.auth != null";

    for condition in [first_condition, second_condition] {
        let resp = ctx
            .client
            .post(ctx.url("/admin/v1/projects/trailmark-prod/write_access_rules"))
            .header("Cookie", &cookie)
            .json(&serde_json::json!({
                "collection_path": "flagged_content",
                "condition": condition,
            }))
            .send()
            .await
            .expect("define request failed");
        assert_eq!(resp.status().as_u16(), 200, "define/redefine must succeed");
    }

    // Direct row proof (mirrors AC-17-156's own real-domain-observable shape).
    let rows = write_access_rule_history_rows(&ctx, "trailmark-prod", "flagged_content").await;
    assert_eq!(
        rows.len(),
        2,
        "AC-17-168: define + redefine must capture exactly two write-rule history entries"
    );
    assert_eq!(rows[0].condition_source, first_condition);
    assert_eq!(rows[1].condition_source, second_condition);
    assert_eq!(rows[0].actor_account_id, ctx.account_id);
    assert_eq!(rows[1].actor_account_id, ctx.account_id);

    // Retrieval-endpoint proof (mirrors AC-17-160's own shape).
    let resp = ctx
        .client
        .get(ctx.url(
            "/admin/v1/projects/trailmark-prod/write_access_rules/flagged_content/history",
        ))
        .header("Cookie", &cookie)
        .send()
        .await
        .expect("history request failed");
    assert_eq!(resp.status().as_u16(), 200, "AC-17-168: history retrieval must succeed");

    let body: serde_json::Value = resp.json().await.expect("parse history response");
    let history = body["history"].as_array().expect("history must be an array");
    assert_eq!(history.len(), 2, "AC-17-168: must return every captured entry");
    assert_eq!(
        history[0]["condition"], second_condition,
        "AC-17-168: newest entry (the redefine) must be first"
    );
    assert_eq!(
        history[1]["condition"], first_condition,
        "AC-17-168: oldest entry (the first-ever definition) must be last"
    );
    assert_eq!(
        history[0]["actor_account_id"],
        ctx.account_id.to_string(),
        "AC-17-168: each entry must show the acting account"
    );
    assert!(
        history[0]["captured_at"].is_string(),
        "AC-17-168: each entry must show a capture timestamp"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-169: a collection's read-rule history and write-rule history are
// structurally independent
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` on `trailmark-prod` gets BOTH a read rule
///          (`access_rules`) AND a write rule (`write_access_rules`)
///          defined, each redefined once, with DIFFERENT condition
///          histories
///   When:  Alex retrieves the read-rule history and the write-rule history
///          separately
///   Then:  each retrieval returns ONLY its own table's entries — never the
///          other's condition text
///
/// AC-17-169
///
/// @driving_port @real-io @US-04 @AC-17-169
#[tokio::test]
async fn read_rule_history_and_write_rule_history_are_structurally_independent() {
    let ctx = SecurityRulesAdminContext::new().await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    ctx.insert_project("trailmark-prod").await;

    let read_condition_a = "request.auth.uid == resource.data.owner_id";
    let read_condition_b = "request.auth != null";
    let write_condition_a = "request.resource.data.owner_id == request.auth.uid";
    let write_condition_b = "true";

    for condition in [read_condition_a, read_condition_b] {
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
            .expect("read-rule define request failed");
        assert_eq!(resp.status().as_u16(), 200);
    }
    for condition in [write_condition_a, write_condition_b] {
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
            .expect("write-rule define request failed");
        assert_eq!(resp.status().as_u16(), 200);
    }

    let read_resp = ctx
        .client
        .get(ctx.url("/admin/v1/projects/trailmark-prod/access_rules/journal_entries/history"))
        .header("Cookie", &cookie)
        .send()
        .await
        .expect("read-rule history request failed");
    let read_body: serde_json::Value = read_resp.json().await.expect("parse read history");
    let read_history = read_body["history"].as_array().expect("history must be an array");

    let write_resp = ctx
        .client
        .get(ctx.url(
            "/admin/v1/projects/trailmark-prod/write_access_rules/journal_entries/history",
        ))
        .header("Cookie", &cookie)
        .send()
        .await
        .expect("write-rule history request failed");
    let write_body: serde_json::Value = write_resp.json().await.expect("parse write history");
    let write_history = write_body["history"].as_array().expect("history must be an array");

    assert_eq!(read_history.len(), 2, "AC-17-169: read history must have its own two entries");
    assert_eq!(write_history.len(), 2, "AC-17-169: write history must have its own two entries");

    let read_conditions: Vec<&str> =
        read_history.iter().map(|e| e["condition"].as_str().unwrap()).collect();
    let write_conditions: Vec<&str> =
        write_history.iter().map(|e| e["condition"].as_str().unwrap()).collect();

    assert!(
        read_conditions.contains(&read_condition_a) && read_conditions.contains(&read_condition_b),
        "AC-17-169: read history must contain exactly the read-rule's own conditions"
    );
    assert!(
        write_conditions.contains(&write_condition_a) && write_conditions.contains(&write_condition_b),
        "AC-17-169: write history must contain exactly the write-rule's own conditions"
    );
    assert!(
        !read_conditions.iter().any(|c| write_conditions.contains(c)),
        "AC-17-169: read history must never contain a write-rule-only condition, got {read_conditions:?}"
    );
    assert!(
        !write_conditions.iter().any(|c| read_conditions.contains(c)),
        "AC-17-169: write history must never contain a read-rule-only condition, got {write_conditions:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-170: a write rule can be restored to any prior entry via the
// identical restore-is-just-define-again mechanism — real domain proof via
// CreateDocument
// ─────────────────────────────────────────────────────────────────────────────

/// Calls the EXISTING `define_write_access_rule` admin action against
/// `flagged_content` on `ctx.project_id` — used for the initial definition,
/// the redefinition, AND the restore (all three ARE the identical action,
/// mirrors Slice 03's own `define_journal_entries_rule` precedent exactly).
async fn define_flagged_content_write_rule(ctx: &SecurityRulesFullContext, cookie: &str, condition: &str) {
    let resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/write_access_rules",
            ctx.project_id
        )))
        .header("Cookie", cookie)
        .json(&serde_json::json!({
            "collection_path": "flagged_content",
            "condition": condition,
        }))
        .send()
        .await
        .expect("define request failed");
    assert_eq!(resp.status().as_u16(), 200, "define/redefine/restore must succeed");
}

/// Journey (mirrors AC-17-164's own shape, applied to write-path):
///   Given: `flagged_content`'s write rule is defined with condition A
///          (ownership-required: `request.resource.data.owner_id ==
///          request.auth.uid`), then redefined to condition B (any
///          signed-in caller) — Dana proposes a document she does NOT own
///          (`owner_id` set to someone else)
///   When:  Alex restores the rule using condition A's exact text
///   Then:  a real `CreateDocument` call by Dana, proposing the SAME
///          not-her-own `owner_id`, is denied again — identical evaluated
///          behavior to condition A restored
///
/// AC-17-170
///
/// @walking_skeleton @driving_port @real-io @US-04 @AC-17-170
#[tokio::test]
async fn restoring_a_write_rules_prior_condition_restores_identical_evaluated_behavior() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-sro04-restore-write").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    let condition_a = "request.resource.data.owner_id == request.auth.uid";
    let condition_b = "request.auth != null";

    let mut not_danas_fields = std::collections::HashMap::new();
    not_danas_fields.insert("owner_id".to_string(), string_field("maria-santos"));

    // Condition A active: Dana's create proposing someone else's owner_id is denied.
    define_flagged_content_write_rule(&ctx, &cookie, condition_a).await;
    let under_a = create_document(
        &ctx,
        "flagged_content",
        "sro04-doc-a",
        not_danas_fields.clone(),
        Some(&danas_token),
    )
    .await;
    assert!(
        under_a.is_err(),
        "precondition: under condition A, Dana's non-owned create must be denied"
    );

    // Redefine to condition B: Dana's create is now allowed regardless of owner_id.
    define_flagged_content_write_rule(&ctx, &cookie, condition_b).await;
    let under_b = create_document(
        &ctx,
        "flagged_content",
        "sro04-doc-b",
        not_danas_fields.clone(),
        Some(&danas_token),
    )
    .await;
    assert!(
        under_b.is_ok(),
        "precondition: under redefined condition B, Dana's create must be allowed: {:?}",
        under_b.err()
    );

    // Restore: redefine with condition A's EXACT text.
    define_flagged_content_write_rule(&ctx, &cookie, condition_a).await;
    let restored = create_document(
        &ctx,
        "flagged_content",
        "sro04-doc-restored",
        not_danas_fields,
        Some(&danas_token),
    )
    .await;
    let err = restored.expect_err(
        "AC-17-170: after restoring condition A's exact text, Dana's non-owned create must be denied again",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-170: restored behavior must be identical to condition A's original evaluated \
         behavior (PermissionDenied), got {:?}",
        err.code()
    );
}
