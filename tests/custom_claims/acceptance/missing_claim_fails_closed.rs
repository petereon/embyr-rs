//! Slice 04 (US-04, ADR-034) — A Missing Claim Fails Closed.
//!
//! Pure proof-obligation slice (feature-delta.md Slice 04 / § Learning
//! Hypothesis): `resolve_field_value`'s `Operand::AuthTokenClaim` arm
//! (`crates/embyr-core/src/access_control/mod.rs`) already implements both
//! fail-closed collapses this slice proves — `auth.ok_or(FieldMissing)?`
//! first (anonymous session, AC-17-147), then
//! `auth.claims.get(key).ok_or(FieldMissing)` (verified identity, absent
//! key, AC-17-146) — both short-circuiting the WHOLE evaluation to `Deny`
//! via `eval_bool`'s existing `?` propagation, unmodified since Slice 02.
//! This file adds no new production code; it proves the mechanism Slice 02
//! already shipped behaves as ADR-034 documents, against real sessions.
//!
//! Acceptance criteria verified here (feature-delta.md US-04):
//!   AC-17-146: a real caller whose token verifies successfully but does NOT
//!              carry the claim a rule references is denied, never crashes.
//!   AC-17-147: an anonymous caller (no client-identity token presented at
//!              all) against the SAME claim-referencing rule is denied,
//!              identically to how an anonymous caller is denied against a
//!              uid-referencing rule (mirrors `security_rules`'s own
//!              AC-17-11 assertion shape, sr03_anonymous_session_evaluated_
//!              as_null_auth.rs).
//!
//! A third behavior — a claim present but with the WRONG `FieldValue`
//! variant (string `"true"` vs. the rule's boolean literal `true`) — is
//! included as its own test (Slice 04's own § IN Scope, "type-mismatch
//! behavior"). It is NOT a variation of AC-17-146/147: it never reaches
//! `FieldMissing` at all — `resolve_field_value` returns
//! `Ok(FieldValue::String("true"))` successfully, and denial comes from the
//! ordinary `FieldValue::PartialEq` fallthrough in `compare_operands`
//! returning `false` (a string never equals a bool). Genuinely distinct
//! code path from the other two, so it earns its own test rather than being
//! folded into either.
//!
//! Driving port: gRPC :8080 `GetDocument` (via `SecurityRulesFullContext` +
//! this feature's own `mint_client_identity_token`/
//! `mint_client_identity_token_with_claims` helpers — Pillar 3, real
//! Postgres, real gRPC, real Ed25519-signed tokens, no mocks).
//!
//! Test Budget: 3 behaviors x 2 = 6 max. 3 tests written (1 per behavior) —
//! all happy-path-of-the-deny-shape, no input-variation parametrization
//! needed per behavior.

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
// AC-17-146: a real caller whose token verifies successfully but does NOT
// carry the claim a rule references is denied, never crashes.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `flagged_content` has a rule `request.auth.token.is_moderator == true`
///   And:   Maria Santos holds a verified identity (real Ed25519-signed
///          token, verifies successfully) with NO `is_moderator` claim at
///          all
///   When:  Maria calls `getDoc()` on a `flagged_content` document
///   Then:  the read is denied, never a crash
///
/// AC-17-146
///
/// @error @driving_port @real-io @US-04 @AC-17-146
#[tokio::test]
async fn a_caller_with_no_such_claim_on_their_token_is_denied_never_crashes() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cc04-missingclaim").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule("flagged_content", "request.auth.token.is_moderator == true")
        .await;

    let mut fields = std::collections::HashMap::new();
    fields.insert("text".to_string(), string_field("reported content"));
    create_document(&ctx, "flagged_content", "cc04-missingclaim-doc", fields, None)
        .await
        .expect("seed flagged_content document");

    // Maria's token verifies successfully (real signature, real sub/aud/exp)
    // but carries no claims at all — the verified-identity, absent-key
    // fail-closed branch (`auth.claims.get(key).ok_or(FieldMissing)`).
    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let resp = ctx
        .get_document(
            &resource_name(&ctx, "flagged_content", "cc04-missingclaim-doc"),
            Some(&marias_token),
        )
        .await;

    let err = resp.expect_err(
        "AC-17-146: a verified caller with no is_moderator claim must be denied, not crash",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-146: denial must be attributable to the rule (PermissionDenied), never Internal/Unknown, got {:?}",
        err.code()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-147: an anonymous caller (no client-identity token presented at
// all) against a claim-referencing rule is denied, identically to how an
// anonymous caller is denied against a uid-referencing rule.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `flagged_content` has a rule `request.auth.token.is_moderator == true`
///   When:  a session with no verified identity at all (no client-identity
///          token presented) calls `getDoc()` on a `flagged_content`
///          document
///   Then:  the read is denied, using the identical anonymous-session
///          fail-closed path `security_rules`'s own AC-17-11 already
///          established for a uid-referencing rule
///
/// AC-17-147
///
/// @error @driving_port @real-io @US-04 @AC-17-147
#[tokio::test]
async fn an_anonymous_caller_against_a_claim_referencing_rule_is_denied_identically_to_a_uid_referencing_rule(
) {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cc04-anon").await;
    ctx.seed_access_rule("flagged_content", "request.auth.token.is_moderator == true")
        .await;

    let mut fields = std::collections::HashMap::new();
    fields.insert("text".to_string(), string_field("reported content"));
    create_document(&ctx, "flagged_content", "cc04-anon-doc", fields, None)
        .await
        .expect("seed flagged_content document");

    // No client-identity token presented at all — mirrors sr03's own
    // AC-17-11 anonymous-denial precedent exactly (same `None`, same
    // assertion shape).
    let resp = ctx
        .get_document(&resource_name(&ctx, "flagged_content", "cc04-anon-doc"), None)
        .await;

    let err = resp.expect_err(
        "AC-17-147: an anonymous caller against a claim-referencing rule must be denied",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-147: denial must be attributable to the rule (PermissionDenied), got {:?}",
        err.code()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Slice 04 § IN Scope, type-mismatch: a claim present but with the WRONG
// `FieldValue` variant resolves via ordinary `PartialEq`, denies without
// any special-casing or crash. Distinct code path from AC-17-146/147 — it
// never reaches `FieldMissing`.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `flagged_content` has a rule `request.auth.token.is_moderator == true`
///   And:   a caller's token carries `is_moderator` as a STRING (`"true"`),
///          not a boolean
///   When:  that caller calls `getDoc()` on a `flagged_content` document
///   Then:  the read is denied — `FieldValue::String("true") !=
///          FieldValue::Boolean(true)` under ordinary `PartialEq`, no
///          crash, no special-casing
///
/// (Slice 04 § IN Scope — type-mismatch behavior)
///
/// @error @driving_port @real-io @US-04
#[tokio::test]
async fn a_claim_present_with_the_wrong_field_value_type_is_denied_via_ordinary_equality() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cc04-typemismatch").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule("flagged_content", "request.auth.token.is_moderator == true")
        .await;

    let mut fields = std::collections::HashMap::new();
    fields.insert("text".to_string(), string_field("reported content"));
    create_document(&ctx, "flagged_content", "cc04-typemismatch-doc", fields, None)
        .await
        .expect("seed flagged_content document");

    // `is_moderator` is present, but its JSON value is a string, not a
    // boolean — resolve_field_value succeeds (Ok(FieldValue::String)), the
    // FieldMissing path is never entered; denial comes purely from
    // FieldValue::PartialEq.
    let callers_token = mint_client_identity_token_with_claims(
        &signing_key,
        "someone-with-string-claim",
        &ctx.project_id,
        now_unix() + 3600,
        serde_json::json!({"is_moderator": "true"}),
    );

    let resp = ctx
        .get_document(
            &resource_name(&ctx, "flagged_content", "cc04-typemismatch-doc"),
            Some(&callers_token),
        )
        .await;

    let err = resp.expect_err(
        "type-mismatch: a string-valued is_moderator claim must deny against the boolean literal rule, not crash",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "type-mismatch: denial must be attributable to the rule (PermissionDenied), got {:?}",
        err.code()
    );
}
