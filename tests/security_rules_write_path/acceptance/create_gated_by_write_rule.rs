//! Slice 02 (US-02, ADR-030) — A Create Is Gated by the Write Rule Using
//! Only the Proposed New Document.
//!
//! Acceptance criteria verified here (feature-delta.md US-02):
//!   AC-17-26: a create whose proposed new document satisfies the write
//!             rule succeeds.
//!   AC-17-27: a create whose proposed new document fails the write rule is
//!             denied, attributable to the rule.
//!   AC-17-28: a write rule referencing `resource.data.<field>` evaluates
//!             that reference as fail-closed/absent during create, since no
//!             document exists yet — never a crash. This is the
//!             acceptance-level proof that the SAME fail-closed mechanism
//!             `security-rules` AC-17-09 already proves for "field present
//!             on the wrong document" also correctly handles "field
//!             referenced on a document that doesn't exist yet" — via the
//!             exact `evaluate()` function, no create-time special case.
//!   AC-17-29: a write rule not based on ownership correctly allows a
//!             signed-in caller to create a document (mirrors
//!             `security-rules`' own AC-17-08 precedent).
//!   (regression, not separately numbered) a create against a collection
//!             with NO write rule defined succeeds exactly as before this
//!             feature existed — the short-circuit guardrail (ADR-030 §
//!             Decision — Composition, "None -> proceed to the existing
//!             adapter.create_document(...) call, completely unmodified").
//!
//! Driving port: gRPC :8080 `CreateDocument` (via `SecurityRulesFullContext`
//! + this file's own `create_document`/`seed_write_access_rule_full`
//! helpers — Pillar 3, real Postgres, real gRPC, no mocks).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    create_document, mint_client_identity_token, now_unix, seed_write_access_rule_full,
    string_field, SecurityRulesFullContext,
};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-26: a create whose proposed new document satisfies the write rule
// succeeds.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has a WRITE rule requiring
///          `request.resource.data.owner_id == request.auth.uid`
///   And:   Maria Santos holds a verified identity
///   When:  Maria calls `createDoc()` proposing a new document whose
///          `owner_id` IS her own uid
///   Then:  the create succeeds
///
/// AC-17-26
///
/// @driving_port @real-io @US-02 @AC-17-26
#[tokio::test]
async fn a_create_whose_proposed_document_satisfies_the_write_rule_succeeds() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-swp02-allow").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    seed_write_access_rule_full(
        &ctx,
        "journal_entries",
        "request.resource.data.owner_id == request.auth.uid",
    )
    .await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let mut fields = std::collections::HashMap::new();
    fields.insert("owner_id".to_string(), string_field("maria-santos"));

    let resp = create_document(
        &ctx,
        "journal_entries",
        "swp02-allow-doc",
        fields,
        Some(&marias_token),
    )
    .await;

    assert!(
        resp.is_ok(),
        "AC-17-26: a create whose proposed owner_id matches the caller must succeed: {:?}",
        resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-28: a write rule referencing the OLD `resource.data.<field>`
// operand evaluates that reference as fail-closed/absent during create,
// since no document exists yet — never a crash. Acceptance-level proof of
// this slice's own Learning Hypothesis: the SAME fail-closed mechanism
// `security-rules` AC-17-09 already proves for "field present on the wrong
// document" also handles "field referenced on a document that doesn't
// exist yet" — via the exact `evaluate()` function (verified at the unit
// level too, `resource_field_reference_on_a_nonexistent_create_time_document_denies_ac_17_28`
// in `embyr_core::access_control::tests`), not a create-time special case.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has a WRITE rule requiring
///          `request.auth.uid == resource.data.owner_id` — the OLD
///          `resource.data` operand, which only ever made sense pre-write-path
///          for an EXISTING document
///   And:   Maria Santos holds a verified identity
///   When:  Maria calls `createDoc()` — no document exists yet, so
///          `resource.data.owner_id` cannot resolve
///   Then:  the create is denied (fail-closed), never `Internal` (a crash)
///
/// AC-17-28
///
/// @error @driving_port @real-io @US-02 @AC-17-28
#[tokio::test]
async fn a_write_rule_referencing_the_old_resource_data_operand_fails_closed_on_create() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-swp02-oldop").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    seed_write_access_rule_full(
        &ctx,
        "journal_entries",
        "request.auth.uid == resource.data.owner_id",
    )
    .await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let mut fields = std::collections::HashMap::new();
    fields.insert("owner_id".to_string(), string_field("maria-santos"));

    let resp = create_document(
        &ctx,
        "journal_entries",
        "swp02-oldop-doc",
        fields,
        Some(&marias_token),
    )
    .await;

    let err = resp.expect_err(
        "AC-17-28: a write rule referencing resource.data.<field> must deny at create-time \
         (no document exists yet), not succeed",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-28: must fail CLOSED (PermissionDenied) — never Internal (a crash), got {:?}",
        err.code()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-29: a write rule not based on ownership correctly allows a
// signed-in caller to create a document (mirrors `security-rules`' own
// AC-17-08 precedent — proves the grammar isn't owner-equality-only).
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `trail_guides` has a WRITE rule requiring only
///          `request.auth != null` (no ownership check at all)
///   And:   Dana Kim holds a verified identity
///   When:  Dana calls `createDoc()` proposing a new document with ANY
///          fields
///   Then:  the create succeeds — the rule allows any signed-in caller
///
/// AC-17-29
///
/// @driving_port @real-io @US-02 @AC-17-29
#[tokio::test]
async fn a_non_ownership_write_rule_allows_any_signed_in_caller_to_create() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-swp02-authreq").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    seed_write_access_rule_full(&ctx, "trail_guides", "request.auth != null").await;

    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    let mut fields = std::collections::HashMap::new();
    fields.insert("title".to_string(), string_field("New Trail Guide"));

    let resp = create_document(
        &ctx,
        "trail_guides",
        "swp02-authreq-doc",
        fields,
        Some(&danas_token),
    )
    .await;

    assert!(
        resp.is_ok(),
        "AC-17-29: any signed-in caller must be allowed by a non-ownership write rule: {:?}",
        resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Regression guardrail (not one of the 4 numbered ACs above, but essential
// coverage for this slice, proven immediately rather than deferred — mirrors
// every prior slice's own "no rule = unaffected" case, e.g. AC-17-42/ADR-030
// § Decision — Composition step 2): a create against a collection with NO
// write rule defined succeeds exactly as before this feature existed.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `app_config` has NO write rule defined at all
///   When:  an UNAUTHENTICATED create (no client-identity token) is made
///          against it
///   Then:  the create succeeds — the structural no-rule-defined guardrail
///          means `get_write_access_rule` returns `None` and
///          `evaluate()`/write-rule gating is never invoked at all
///
/// (regression guardrail, no dedicated AC number)
///
/// @driving_port @real-io @US-02
#[tokio::test]
async fn a_create_against_a_collection_with_no_write_rule_succeeds_unchanged() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-swp02-norule").await;

    let mut fields = std::collections::HashMap::new();
    fields.insert("enabled".to_string(), string_field("true"));

    let resp = create_document(&ctx, "app_config", "swp02-norule-doc", fields, None).await;

    assert!(
        resp.is_ok(),
        "a create against a collection with no write rule defined must succeed unchanged: {:?}",
        resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-27: a create whose proposed new document fails the write rule is
// denied, attributable to the rule (error/edge).
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has a WRITE rule requiring
///          `request.resource.data.owner_id == request.auth.uid`
///   And:   Maria Santos holds a verified identity
///   When:  Maria calls `createDoc()` proposing a new document whose
///          `owner_id` is NOT her own uid
///   Then:  the create is denied, attributable to the write rule
///
/// AC-17-27
///
/// @error @driving_port @real-io @US-02 @AC-17-27
#[tokio::test]
async fn a_create_whose_proposed_document_fails_the_write_rule_is_denied() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-swp02-deny").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    seed_write_access_rule_full(
        &ctx,
        "journal_entries",
        "request.resource.data.owner_id == request.auth.uid",
    )
    .await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let mut fields = std::collections::HashMap::new();
    fields.insert("owner_id".to_string(), string_field("dana-kim"));

    let resp = create_document(
        &ctx,
        "journal_entries",
        "swp02-deny-doc",
        fields,
        Some(&marias_token),
    )
    .await;

    let err = resp.expect_err(
        "AC-17-27: a create whose proposed owner_id does not match the caller must be denied",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-27: denial must be attributable to the write rule (PermissionDenied), got {:?}",
        err.code()
    );
}
