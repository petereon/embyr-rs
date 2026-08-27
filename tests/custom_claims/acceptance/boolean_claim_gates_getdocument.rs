//! Slice 02 (US-02, ADR-034) — A Boolean-Claim Rule Gates a GetDocument
//! Read.
//!
//! Acceptance criteria verified here (feature-delta.md US-02):
//!   AC-17-141: a rule referencing `request.auth.token.<claim>` with a
//!              boolean claim value correctly allows a caller whose real
//!              minted token carries that claim as `true`, and denies a
//!              caller whose token lacks it.
//!   AC-17-142: a rule combining a claim check with existing grammar
//!              (`&&`/`||`) evaluates correctly.
//!   AC-17-143: a claim-to-resource-field comparison correctly allows/denies
//!              via ordinary `FieldValue` equality.
//!
//! Driving port: gRPC :8080 `GetDocument` (via `SecurityRulesFullContext` +
//! this feature's own `create_document`/`mint_client_identity_token_with_claims`
//! helpers — Pillar 3, real Postgres, real gRPC, real Ed25519-signed
//! tokens, no mocks).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    create_document, mint_client_identity_token, mint_client_identity_token_with_claims,
    now_unix, string_field, SecurityRulesFullContext,
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
// AC-17-141: a caller whose claim satisfies the rule's condition succeeds.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `flagged_content` has a rule `request.auth.token.is_moderator == true`
///   And:   Priya Nair holds a verified identity with claim `is_moderator: true`
///   When:  Priya calls `getDoc()` on a `flagged_content` document
///   Then:  the read succeeds
///
/// AC-17-141
///
/// @driving_port @real-io @US-02 @AC-17-141
#[tokio::test]
async fn a_caller_whose_claim_satisfies_the_rule_succeeds() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cc02-allow").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule("flagged_content", "request.auth.token.is_moderator == true")
        .await;

    let mut fields = std::collections::HashMap::new();
    fields.insert("text".to_string(), string_field("reported content"));
    create_document(&ctx, "flagged_content", "cc02-allow-doc", fields, None)
        .await
        .expect("seed flagged_content document");

    let priyas_token = mint_client_identity_token_with_claims(
        &signing_key,
        "priya-nair",
        &ctx.project_id,
        now_unix() + 3600,
        serde_json::json!({"is_moderator": true}),
    );

    let resp = ctx
        .get_document(&resource_name(&ctx, "flagged_content", "cc02-allow-doc"), Some(&priyas_token))
        .await;

    assert!(
        resp.is_ok(),
        "AC-17-141: a moderator's claim-satisfying getDoc must succeed: {:?}",
        resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-141: a caller whose claim does not satisfy the rule's condition is
// denied.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `flagged_content` has a rule `request.auth.token.is_moderator == true`
///   And:   Dana Kim holds a verified identity with NO `is_moderator` claim
///   When:  Dana calls `getDoc()` on the same `flagged_content` document
///   Then:  the read is denied with PermissionDenied, attributable to the rule
///
/// AC-17-141
///
/// @error @driving_port @real-io @US-02 @AC-17-141
#[tokio::test]
async fn a_caller_whose_claim_does_not_satisfy_the_rule_is_denied() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cc02-deny").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule("flagged_content", "request.auth.token.is_moderator == true")
        .await;

    let mut fields = std::collections::HashMap::new();
    fields.insert("text".to_string(), string_field("reported content"));
    create_document(&ctx, "flagged_content", "cc02-deny-doc", fields, None)
        .await
        .expect("seed flagged_content document");

    // Dana's token carries no claims at all — the OTHER fail-closed trigger
    // from the identical claim (`AuthTokenClaim`), not a different claim
    // value, mirroring US-02's own Domain Example 3.
    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    let resp = ctx
        .get_document(&resource_name(&ctx, "flagged_content", "cc02-deny-doc"), Some(&danas_token))
        .await;

    let err = resp.expect_err(
        "AC-17-141: a caller with no is_moderator claim must be denied, not allowed",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-141: denial must be attributable to the rule (PermissionDenied), got {:?}",
        err.code()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-142: a rule combining a claim check with existing ownership grammar
// (||) evaluates correctly — satisfied via the ownership branch, not the
// claim branch.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `flagged_content` has a rule
///          `request.auth.token.is_moderator == true || request.auth.uid == resource.data.owner_id`
///   And:   Maria Santos owns a `flagged_content` document but has no
///          `is_moderator` claim
///   When:  Maria calls `getDoc()` on her own document
///   Then:  the read succeeds, satisfied by the ownership branch
///
/// AC-17-142
///
/// @driving_port @real-io @US-02 @AC-17-142
#[tokio::test]
async fn a_rule_combining_a_claim_check_with_ownership_grammar_evaluates_correctly() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cc02-or").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule(
        "flagged_content",
        "request.auth.token.is_moderator == true || request.auth.uid == resource.data.owner_id",
    )
    .await;

    let mut fields = std::collections::HashMap::new();
    fields.insert("owner_id".to_string(), string_field("maria-santos"));
    create_document(&ctx, "flagged_content", "cc02-or-doc", fields, None)
        .await
        .expect("seed flagged_content document");

    // Maria's token carries NO is_moderator claim — the claim disjunct
    // cannot satisfy this rule for her; only the ownership disjunct can.
    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let resp = ctx
        .get_document(&resource_name(&ctx, "flagged_content", "cc02-or-doc"), Some(&marias_token))
        .await;

    assert!(
        resp.is_ok(),
        "AC-17-142: the document owner must be admitted via the ownership disjunct, \
         even with no moderator claim: {:?}",
        resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-143: a claim-to-resource-field comparison (attribute-based access)
// correctly allows/denies via ordinary FieldValue equality.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `support_tickets` has a rule
///          `request.auth.token.department == resource.data.department`
///   And:   Jordan Lee holds a verified identity with claim `department: "billing"`
///   And:   a `support_tickets` document has `department: "billing"`
///   When:  Jordan calls `getDoc()` on that document
///   Then:  the read succeeds
///
/// AC-17-143
///
/// @driving_port @real-io @US-02 @AC-17-143
#[tokio::test]
async fn a_claim_compared_to_a_resource_field_gates_attribute_based_access() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cc02-abac-allow").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule(
        "support_tickets",
        "request.auth.token.department == resource.data.department",
    )
    .await;

    let mut fields = std::collections::HashMap::new();
    fields.insert("department".to_string(), string_field("billing"));
    create_document(&ctx, "support_tickets", "cc02-abac-allow-doc", fields, None)
        .await
        .expect("seed support_tickets document");

    let jordans_token = mint_client_identity_token_with_claims(
        &signing_key,
        "jordan-lee",
        &ctx.project_id,
        now_unix() + 3600,
        serde_json::json!({"department": "billing"}),
    );

    let resp = ctx
        .get_document(
            &resource_name(&ctx, "support_tickets", "cc02-abac-allow-doc"),
            Some(&jordans_token),
        )
        .await;

    assert!(
        resp.is_ok(),
        "AC-17-143: a matching department claim/field must allow: {:?}",
        resp.err()
    );
}

/// Journey:
///   Given: `support_tickets` has a rule
///          `request.auth.token.department == resource.data.department`
///   And:   Jordan Lee holds a verified identity with claim `department: "billing"`
///   And:   a DIFFERENT `support_tickets` document has `department: "engineering"`
///   When:  Jordan calls `getDoc()` on that document
///   Then:  the read is denied
///
/// AC-17-143 (deny half — "allows/denies" per the AC's own wording)
///
/// @error @driving_port @real-io @US-02 @AC-17-143
#[tokio::test]
async fn a_mismatched_claim_to_resource_field_comparison_is_denied() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cc02-abac-deny").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule(
        "support_tickets",
        "request.auth.token.department == resource.data.department",
    )
    .await;

    let mut fields = std::collections::HashMap::new();
    fields.insert("department".to_string(), string_field("engineering"));
    create_document(&ctx, "support_tickets", "cc02-abac-deny-doc", fields, None)
        .await
        .expect("seed support_tickets document");

    let jordans_token = mint_client_identity_token_with_claims(
        &signing_key,
        "jordan-lee",
        &ctx.project_id,
        now_unix() + 3600,
        serde_json::json!({"department": "billing"}),
    );

    let resp = ctx
        .get_document(
            &resource_name(&ctx, "support_tickets", "cc02-abac-deny-doc"),
            Some(&jordans_token),
        )
        .await;

    let err = resp.expect_err(
        "AC-17-143: a mismatched department claim/field must deny, not allow",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-143: denial must be attributable to the rule (PermissionDenied), got {:?}",
        err.code()
    );
}
