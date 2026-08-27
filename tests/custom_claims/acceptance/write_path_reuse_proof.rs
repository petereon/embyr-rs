//! Slice 03 (US-03, ADR-034) — The Identical Mechanism Gates Writes, For
//! Free.
//!
//! Proves Resolution 4's central hypothesis (feature-delta.md § Resolution
//! 4, § Slice 03 Learning Hypothesis): `AuthContext`'s uniform construction
//! across every already-shipped call site means the SAME `Operand::
//! AuthTokenClaim` extension Slice 02 shipped for `handle_get_document`
//! reaches `handle_create_document`/`handle_update_document`/
//! `handle_delete_document`'s existing `evaluate()` call sites automatically
//! — the ONLY change this slice makes is wiring `verified_identity.claims`
//! into those 3 write-path `AuthContext` construction call sites (mirroring
//! `handle_get_document`'s own Slice 02 wiring exactly), zero new
//! `embyr-core::access_control` logic.
//!
//! Acceptance criteria verified here (feature-delta.md Slice 03):
//!   AC-17-144: the identical `request.auth.token.<claim>` operand, used in
//!              a `write_access_rules` condition, correctly gates
//!              `UpdateDocument` — a caller whose claim satisfies the
//!              condition succeeds, one whose claim doesn't fails.
//!   AC-17-145: a `GetDocument` claim-based read rule and a
//!              `write_access_rules` claim-based write rule, independently
//!              authored on the SAME collection, are evaluated
//!              independently — a caller satisfying ONE but not the other
//!              gets the expected mixed outcome (mirrors
//!              `security-rules-write-path`'s own AC-17-43 read/write
//!              independence convention).
//!
//! Driving port: gRPC :8080 `GetDocument`/`UpdateDocument` (via
//! `SecurityRulesFullContext` + this feature's own
//! `create_document`/`update_document`/`seed_write_access_rule_full`/
//! `mint_client_identity_token_with_claims` helpers — Pillar 3, real
//! Postgres, real gRPC, real Ed25519-signed tokens, no mocks).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    create_document, mint_client_identity_token, mint_client_identity_token_with_claims,
    now_unix, seed_write_access_rule_full, string_field, update_document, SecurityRulesFullContext,
};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

fn resource_name(ctx: &SecurityRulesFullContext, collection: &str, doc_id: &str) -> String {
    format!(
        "projects/{}/databases/(default)/documents/{}/{}",
        ctx.project_id, collection, doc_id
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-144: a caller whose claim satisfies the write rule's condition
// succeeds.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `flagged_content` has a WRITE rule
///          `request.auth.token.is_moderator == true` — the IDENTICAL
///          operand Slice 02 shipped for `GetDocument`, now used in
///          `write_access_rules` instead
///   And:   Priya Nair holds a verified identity with claim
///          `is_moderator: true`
///   When:  Priya calls `updateDoc()` on a `flagged_content` document
///   Then:  the update succeeds
///
/// AC-17-144
///
/// @driving_port @real-io @US-03 @AC-17-144
#[tokio::test]
async fn a_caller_whose_claim_satisfies_the_write_rule_succeeds() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cc03-allow").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;

    // Seed the document BEFORE the write rule exists, unauthenticated — the
    // write rule below must not gate this setup call.
    let mut fields = std::collections::HashMap::new();
    fields.insert("text".to_string(), string_field("reported content"));
    create_document(&ctx, "flagged_content", "cc03-allow-doc", fields.clone(), None)
        .await
        .expect("seed flagged_content document");

    seed_write_access_rule_full(&ctx, "flagged_content", "request.auth.token.is_moderator == true")
        .await;

    let priyas_token = mint_client_identity_token_with_claims(
        &signing_key,
        "priya-nair",
        &ctx.project_id,
        now_unix() + 3600,
        serde_json::json!({"is_moderator": true}),
    );

    let resp = update_document(
        &ctx,
        &resource_name(&ctx, "flagged_content", "cc03-allow-doc"),
        fields,
        Some(&priyas_token),
    )
    .await;

    assert!(
        resp.is_ok(),
        "AC-17-144: a moderator's claim-satisfying updateDoc must succeed: {:?}",
        resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-144: a caller whose claim does not satisfy the write rule's
// condition is denied.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `flagged_content` has a WRITE rule
///          `request.auth.token.is_moderator == true`
///   And:   Dana Kim holds a verified identity with NO `is_moderator` claim
///   When:  Dana calls `updateDoc()` on the same `flagged_content` document
///   Then:  the update is denied with PermissionDenied, attributable to the
///          write rule
///
/// AC-17-144
///
/// @error @driving_port @real-io @US-03 @AC-17-144
#[tokio::test]
async fn a_caller_whose_claim_does_not_satisfy_the_write_rule_is_denied() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cc03-deny").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;

    let mut fields = std::collections::HashMap::new();
    fields.insert("text".to_string(), string_field("reported content"));
    create_document(&ctx, "flagged_content", "cc03-deny-doc", fields.clone(), None)
        .await
        .expect("seed flagged_content document");

    seed_write_access_rule_full(&ctx, "flagged_content", "request.auth.token.is_moderator == true")
        .await;

    // Dana's token carries no claims at all — the fail-closed trigger,
    // mirroring Slice 02's own denial domain example.
    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    let resp = update_document(
        &ctx,
        &resource_name(&ctx, "flagged_content", "cc03-deny-doc"),
        fields,
        Some(&danas_token),
    )
    .await;

    let err = resp.expect_err(
        "AC-17-144: a caller with no is_moderator claim must be denied, not allowed",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-144: denial must be attributable to the write rule (PermissionDenied), got {:?}",
        err.code()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-145: a GetDocument claim-based read rule and a write_access_rules
// claim-based write rule, independently authored on the SAME collection,
// are evaluated independently — a caller satisfying ONE but not the other
// gets the expected mixed outcome.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `flagged_content` has an independently-authored READ rule
///          (`request.auth.token.is_moderator == true`) AND an
///          independently-authored WRITE rule
///          (`request.auth.token.is_editor == true`)
///   And:   Priya Nair's token carries `is_moderator: true` only (no
///          `is_editor`); Jordan Lee's token carries `is_editor: true` only
///          (no `is_moderator`)
///   When:  each calls `getDoc()` then `updateDoc()` on the same document
///   Then:  Priya can read but not write; Jordan can write but not read —
///          the two rule tables never cross-govern each other
///
/// AC-17-145
///
/// @driving_port @real-io @US-03 @AC-17-145
#[tokio::test]
async fn read_and_write_claim_rules_on_the_same_collection_are_evaluated_independently() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cc03-independence").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;

    let mut fields = std::collections::HashMap::new();
    fields.insert("text".to_string(), string_field("reported content"));
    create_document(&ctx, "flagged_content", "cc03-independence-doc", fields.clone(), None)
        .await
        .expect("seed flagged_content document");

    // Independently authored: a READ rule (access_rules) and a WRITE rule
    // (write_access_rules), different tables, different claims, same
    // collection.
    ctx.seed_access_rule("flagged_content", "request.auth.token.is_moderator == true")
        .await;
    seed_write_access_rule_full(&ctx, "flagged_content", "request.auth.token.is_editor == true")
        .await;

    let priyas_token = mint_client_identity_token_with_claims(
        &signing_key,
        "priya-nair",
        &ctx.project_id,
        now_unix() + 3600,
        serde_json::json!({"is_moderator": true}),
    );
    let jordans_token = mint_client_identity_token_with_claims(
        &signing_key,
        "jordan-lee",
        &ctx.project_id,
        now_unix() + 3600,
        serde_json::json!({"is_editor": true}),
    );

    let doc_name = resource_name(&ctx, "flagged_content", "cc03-independence-doc");

    let priyas_read = ctx.get_document(&doc_name, Some(&priyas_token)).await;
    let priyas_write =
        update_document(&ctx, &doc_name, fields.clone(), Some(&priyas_token)).await;
    let jordans_read = ctx.get_document(&doc_name, Some(&jordans_token)).await;
    let jordans_write = update_document(&ctx, &doc_name, fields, Some(&jordans_token)).await;

    assert!(
        priyas_read.is_ok(),
        "AC-17-145: Priya (is_moderator) must be able to READ via the independent read rule: {:?}",
        priyas_read.err()
    );
    assert_eq!(
        priyas_write.expect_err(
            "AC-17-145: Priya (no is_editor) must be DENIED write — the read rule must not govern writes"
        ).code(),
        tonic::Code::PermissionDenied
    );
    assert_eq!(
        jordans_read.expect_err(
            "AC-17-145: Jordan (no is_moderator) must be DENIED read — the write rule must not govern reads"
        ).code(),
        tonic::Code::PermissionDenied
    );
    assert!(
        jordans_write.is_ok(),
        "AC-17-145: Jordan (is_editor) must be able to WRITE via the independent write rule: {:?}",
        jordans_write.err()
    );
}
