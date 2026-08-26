//! security-rules-collection-group-rules Slice 05 (US-05, ADR-032) —
//! GetDocument, Writes, and Non-Group Queries Remain Governed Exclusively by
//! Existing Exact-Path Rules.
//!
//! Pure regression-proof slice (per its own slice doc, § OUT Scope): no new
//! production logic is expected — Slices 01–04 already built the
//! disjoint-table, disjoint-call-site-branch design (Resolution 1). This
//! slice proves that design is structurally, not merely conventionally,
//! correct — mirroring `security-rules-query-path`'s own Slice 06 and
//! `security-rules-write-path`'s own Slice 06 discipline exactly. Confirmed
//! directly (not assumed) by reading `handle_get_document`,
//! `handle_create_document`/`handle_update_document`/`handle_delete_document`,
//! and the non-group (`else`) arm of `handle_run_query`: NONE of them
//! reference `group_access_rules`/`get_group_access_rule` anywhere.
//!
//! The strongest possible contrast fixture is used throughout: `journal_entries`
//! carries BOTH an active exact-path rule (`request.auth.uid ==
//! resource.data.owner_id`) AND a group rule with a DELIBERATELY
//! WEAKER/DIFFERENT condition (`request.auth != null`, later redefined to
//! `false` — deny-all, the strongest possible content change) simultaneously.
//!
//! Acceptance criteria verified here (feature-delta.md
//! `security-rules-collection-group-rules`, slice-05-other-surfaces-unaffected.md):
//!   AC-17-93: GetDocument unmodified — governed exclusively by the
//!             exact-path rule, regardless of the group rule's existence or
//!             content.
//!   AC-17-94: non-group RunQuery (`all_descendants = false`) unmodified —
//!             same guarantee.
//!   AC-17-95: writes (create/update/delete) on `journal_entries` (top-level)
//!             and on `expeditions/trek-2026/journal_entries` (nested) behave
//!             per write-path's own default (no write rule defined ⇒
//!             unrestricted writes, ADR-030 Resolution 1/AC-17-42) —
//!             completely independent of both the exact-path READ rule and
//!             the group rule.
//!   AC-17-96: the group rule's CONTENT has zero observable effect — proven
//!             by a caller who satisfies the group rule's weaker condition
//!             but not the exact-path rule's stricter condition still being
//!             rejected, AND by redefining the group rule's condition to
//!             `false` (deny-all) leaving GetDocument/non-group-RunQuery
//!             outcomes byte-for-byte unchanged.
//!
//! Driving ports: gRPC :8080 `GetDocument`/`RunQuery`/`CreateDocument`/
//! `UpdateDocument`/`DeleteDocument`, admin HTTP :9090 (group-rule
//! redefinition) — all via the real production composition root
//! (`SecurityRulesFullContext`).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    create_document, delete_document, mint_client_identity_token, now_unix, run_query,
    seed_group_access_rule_full, string_field, update_document, SecurityRulesFullContext,
};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-93 + AC-17-96: GetDocument is governed exclusively by the exact-path
// rule. Dana satisfies the group rule's weaker condition (`request.auth !=
// null` — she IS signed in) but not the exact-path rule's ownership
// condition; if the group rule's weaker condition had ANY bearing on
// GetDocument, she would be admitted. Redefining the group rule's content to
// deny-all afterward leaves the outcome byte-for-byte unchanged.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has BOTH an active exact-path rule
///          (`request.auth.uid == resource.data.owner_id`) AND a group rule
///          with a deliberately WEAKER condition (`request.auth != null`)
///   When:  Dana (signed in, but not the document's owner) calls GetDocument
///          on Maria's document
///   Then:  Dana is denied — the group rule's weaker condition has zero
///          bearing on GetDocument's own decision
///   When:  the group rule is redefined to `false` (deny-all)
///   Then:  Maria's own GetDocument still succeeds — the group rule's
///          content, even fully deny-all, has zero effect on GetDocument
///
/// AC-17-93, AC-17-96
///
/// @driving_port @real-io @US-05 @AC-17-93 @AC-17-96 @security-critical
#[tokio::test]
async fn get_document_governed_exclusively_by_exact_path_rule_despite_weaker_group_rule() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-scgr05-getdoc").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;

    ctx.seed_access_rule(
        "journal_entries",
        "request.auth.uid == resource.data.owner_id",
    )
    .await;
    seed_group_access_rule_full(&ctx, "journal_entries", "request.auth != null").await;

    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);
    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );
    let marias_doc = ctx.document_resource_name("journal_entries", "maria-doc");

    let dana_result = ctx.get_document(&marias_doc, Some(&danas_token)).await;
    assert_eq!(
        dana_result
            .expect_err(
                "AC-17-93: Dana satisfies only the group rule's weaker condition, not the \
                 exact-path rule's ownership condition — GetDocument must deny her, proving the \
                 group rule has zero bearing here"
            )
            .code(),
        tonic::Code::PermissionDenied
    );

    let maria_result = ctx.get_document(&marias_doc, Some(&marias_token)).await;
    assert!(
        maria_result.is_ok(),
        "AC-17-93: Maria, who satisfies the exact-path rule, must still be admitted, got: {:?}",
        maria_result.err()
    );

    // AC-17-96: redefine the group rule's content to deny-all — the
    // strongest possible content change — and prove GetDocument is
    // unaffected.
    let redefine_resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/group_access_rules",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({"collection_id": "journal_entries", "condition": "false"}))
        .send()
        .await
        .expect("redefine group-rule request failed");
    assert_eq!(redefine_resp.status().as_u16(), 200);

    let maria_after = ctx.get_document(&marias_doc, Some(&marias_token)).await;
    assert!(
        maria_after.is_ok(),
        "AC-17-96: redefining the group rule's content to deny-all must have zero effect on \
         GetDocument, got: {:?}",
        maria_after.err()
    );
    let dana_after = ctx.get_document(&marias_doc, Some(&danas_token)).await;
    assert_eq!(
        dana_after
            .expect_err("AC-17-96: Dana's rejection (by the exact-path rule alone) must also be unchanged")
            .code(),
        tonic::Code::PermissionDenied
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-94 + AC-17-96: non-group RunQuery is governed exclusively by the
// exact-path rule. Dana is signed in (satisfies the group rule's weaker
// condition) and issues a non-group query filtered to someone ELSE's uid — a
// filter that does not satisfy the exact-path ownership-equality rule. If
// the group rule's weaker condition leaked into non-group RunQuery's
// decision, this would be admitted; it must instead be rejected.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: the same dual-rule `journal_entries` fixture as above
///   When:  Dana (signed in) issues a non-group `RunQuery` filtered by
///          `owner_id == "maria-santos"` — someone else's uid
///   Then:  the query is rejected — the exact-path ownership-equality rule
///          alone governs, unaffected by the group rule's weaker condition
///   When:  the group rule is redefined to `false` (deny-all)
///   Then:  Maria's own compliant non-group query still succeeds unaffected
///
/// AC-17-94, AC-17-96
///
/// @driving_port @real-io @US-05 @AC-17-94 @AC-17-96 @security-critical
#[tokio::test]
async fn non_group_run_query_governed_exclusively_by_exact_path_rule_despite_weaker_group_rule() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-scgr05-nongroupquery").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;

    ctx.seed_access_rule(
        "journal_entries",
        "request.auth.uid == resource.data.owner_id",
    )
    .await;
    seed_group_access_rule_full(&ctx, "journal_entries", "request.auth != null").await;

    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);
    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let dana_result = run_query(
        &ctx,
        "journal_entries",
        false,
        &[("owner_id", "maria-santos")],
        Some(&danas_token),
    )
    .await;
    assert_eq!(
        dana_result
            .expect_err(
                "AC-17-94: Dana is signed in (satisfies the group rule's weaker condition) but \
                 her filter is bound to someone else's uid, not satisfying the exact-path rule — \
                 the non-group query must be rejected, proving the group rule has zero bearing"
            )
            .code(),
        tonic::Code::PermissionDenied
    );

    let maria_result = run_query(
        &ctx,
        "journal_entries",
        false,
        &[("owner_id", "maria-santos")],
        Some(&marias_token),
    )
    .await;
    assert!(
        maria_result.is_ok(),
        "AC-17-94: Maria, who satisfies the exact-path rule, must still be admitted, got: {:?}",
        maria_result.err()
    );

    // AC-17-96: redefine the group rule's content to deny-all; the
    // non-group outcome must be byte-for-byte unchanged.
    let redefine_resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/group_access_rules",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({"collection_id": "journal_entries", "condition": "false"}))
        .send()
        .await
        .expect("redefine group-rule request failed");
    assert_eq!(redefine_resp.status().as_u16(), 200);

    let maria_after = run_query(
        &ctx,
        "journal_entries",
        false,
        &[("owner_id", "maria-santos")],
        Some(&marias_token),
    )
    .await;
    assert!(
        maria_after.is_ok(),
        "AC-17-96: redefining the group rule's content to deny-all must have zero effect on \
         non-group RunQuery, got: {:?}",
        maria_after.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-95: writes (create/update/delete) on `journal_entries` (top-level)
// AND on `expeditions/trek-2026/journal_entries` (nested) behave per
// write-path's own default — no write rule is defined for either path, so
// writes remain fully unrestricted (ADR-030 Resolution 1/AC-17-42) —
// completely independent of both the exact-path READ rule and the (deny-all,
// strongest-contrast) group rule active on the SAME collection id.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has an active exact-path READ rule AND an
///          active group rule set to `false` (deny-all — the strongest
///          possible contrast); NO write rule is defined for `journal_entries`
///          or for `expeditions/trek-2026/journal_entries`
///   When:  an anonymous caller (satisfying neither rule) creates, updates,
///          and deletes a document at the top-level path, and creates
///          (via upsert) and deletes a document at the nested path
///   Then:  every write succeeds unrestricted — write-path never consults
///          `access_rules` or `group_access_rules`
///
/// AC-17-95
///
/// @driving_port @real-io @US-05 @AC-17-95 @security-critical
#[tokio::test]
async fn writes_on_top_level_and_nested_journal_entries_unaffected_by_read_rule_and_group_rule() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-scgr05-writes").await;

    ctx.seed_access_rule(
        "journal_entries",
        "request.auth.uid == resource.data.owner_id",
    )
    .await;
    seed_group_access_rule_full(&ctx, "journal_entries", "false").await;

    let mut fields = HashMap::new();
    fields.insert("owner_id".to_string(), string_field("nobody-in-particular"));

    // Top-level journal_entries: real CreateDocument/UpdateDocument/
    // DeleteDocument, anonymous caller, no write rule defined for this
    // collection.
    create_document(&ctx, "journal_entries", "scgr05-write-doc", fields.clone(), None)
        .await
        .expect(
            "AC-17-95: create must succeed unrestricted — no write rule is defined, and neither \
             the exact-path READ rule nor the (deny-all) group rule govern writes",
        );
    let top_level_name = format!(
        "projects/{}/databases/(default)/documents/journal_entries/scgr05-write-doc",
        ctx.project_id
    );

    let mut updated_fields = HashMap::new();
    updated_fields.insert("owner_id".to_string(), string_field("still-nobody"));
    update_document(&ctx, &top_level_name, updated_fields, None)
        .await
        .expect("AC-17-95: update must succeed unrestricted");

    delete_document(&ctx, &top_level_name, None)
        .await
        .expect("AC-17-95: delete must succeed unrestricted");

    // Nested expeditions/trek-2026/journal_entries: create-via-upsert +
    // delete (CreateDocument cannot target a nested path — see this
    // feature's own common module doc comment on `seed_document_at_path`).
    let nested_name = format!(
        "projects/{}/databases/(default)/documents/expeditions/trek-2026/journal_entries/scgr05-nested-write-doc",
        ctx.project_id
    );
    update_document(&ctx, &nested_name, fields, None)
        .await
        .expect("AC-17-95: create-via-upsert on the nested path must succeed unrestricted");
    delete_document(&ctx, &nested_name, None)
        .await
        .expect("AC-17-95: delete on the nested path must succeed unrestricted");
}
