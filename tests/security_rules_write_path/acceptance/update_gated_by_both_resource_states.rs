//! Slice 03 (US-03, ADR-030) — An Update Is Gated by the Write Rule Using
//! Both the Existing and Proposed Document.
//!
//! Acceptance criteria verified here (feature-delta.md US-03):
//!   AC-17-30: an update where the caller owns the existing document and the
//!             proposed new document does not change the protected field
//!             succeeds.
//!   AC-17-31: an update where the caller does not own the existing document
//!             is denied.
//!   AC-17-32: an update where the caller owns the existing document but the
//!             proposed new document changes the protected field is denied
//!             — the two-value (old vs. new) comparison this slice exists
//!             to prove.
//!   AC-17-33: an update evaluation fails closed if the referenced field is
//!             missing from either the existing OR the proposed document
//!             (two sub-cases).
//!   AC-17-34: a denied update's response does not reveal whether the
//!             target document existed (mirrors `security-rules`' own
//!             AC-17-10 exactly).
//!   (regression, not separately numbered) an update against a collection
//!             with NO write rule succeeds exactly as before.
//!
//! Driving port: gRPC :8080 `UpdateDocument` (via `SecurityRulesFullContext`
//! + this file's own `update_document`/`seed_write_access_rule_full`
//! helpers — Pillar 3, real Postgres, real gRPC, no mocks).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    mint_client_identity_token, now_unix, seed_write_access_rule_full, string_field,
    update_document, SecurityRulesFullContext,
};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

/// The immutable-field write rule this slice exists to prove: the caller
/// must own the EXISTING document, AND the proposed new document must not
/// change the owner field — the two-value (old vs. new) comparison
/// (grounded in Trailmark's `journal_entries`/`owner_id` domain, matching
/// prior slices).
const IMMUTABLE_OWNER_RULE: &str =
    "resource.data.owner_id == request.resource.data.owner_id && request.auth.uid == resource.data.owner_id";

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-30 (simplest — start here): an update where the caller owns the
// existing document and the proposed new document does not change the
// protected field succeeds.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has the immutable-owner WRITE rule
///   And:   the seed document `maria-doc` is owned by `maria-santos`
///   And:   Maria Santos (the owner) holds a verified identity
///   When:  Maria calls `updateDoc()` proposing the SAME `owner_id`
///          (`maria-santos`, unchanged) on her own document
///   Then:  the update succeeds
///
/// AC-17-30
///
/// @driving_port @real-io @US-03 @AC-17-30
#[tokio::test]
async fn an_update_by_the_owner_that_preserves_the_protected_field_succeeds() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-swp03-allow").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    seed_write_access_rule_full(&ctx, "journal_entries", IMMUTABLE_OWNER_RULE).await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let mut fields = std::collections::HashMap::new();
    fields.insert("owner_id".to_string(), string_field("maria-santos"));

    let resp = update_document(
        &ctx,
        &ctx.document_resource_name("journal_entries", "maria-doc"),
        fields,
        Some(&marias_token),
    )
    .await;

    assert!(
        resp.is_ok(),
        "AC-17-30: an update by the owner that does not change the protected field must succeed: {:?}",
        resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-31: an update where the caller does not own the existing document is
// denied.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has a WRITE rule requiring the caller to own
///          the existing document AND not change its `owner_id`
///   And:   the seed document `maria-doc` is owned by `maria-santos`
///   And:   Dana Kim (NOT the owner) holds a verified identity
///   When:  Dana calls `updateDoc()` proposing the SAME `owner_id`
///          (`maria-santos`, unchanged) on Maria's document
///   Then:  the update is denied — Dana does not own the existing document
///
/// AC-17-31
///
/// @error @driving_port @real-io @US-03 @AC-17-31
#[tokio::test]
async fn an_update_by_a_non_owner_of_the_existing_document_is_denied() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-swp03-nonowner").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    seed_write_access_rule_full(&ctx, "journal_entries", IMMUTABLE_OWNER_RULE).await;

    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    let mut fields = std::collections::HashMap::new();
    fields.insert("owner_id".to_string(), string_field("maria-santos"));

    let resp = update_document(
        &ctx,
        &ctx.document_resource_name("journal_entries", "maria-doc"),
        fields,
        Some(&danas_token),
    )
    .await;

    let err = resp.expect_err(
        "AC-17-31: an update by a caller who does not own the existing document must be denied",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-31: denial must be attributable to the write rule (PermissionDenied), got {:?}",
        err.code()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-32: an update where the caller owns the existing document but the
// proposed new document changes the protected field is denied — the
// two-value (old vs. new) comparison this feature exists to prove.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has the immutable-owner WRITE rule
///   And:   the seed document `maria-doc` is owned by `maria-santos`
///   And:   Maria Santos (the OWNER) holds a verified identity
///   When:  Maria calls `updateDoc()` proposing a DIFFERENT `owner_id`
///          (`dana-kim`) on her own document
///   Then:  the update is denied — she owns the document, but the proposed
///          new document changes the protected field
///
/// AC-17-32
///
/// @error @driving_port @real-io @US-03 @AC-17-32
#[tokio::test]
async fn an_update_that_changes_the_protected_field_is_denied_even_by_the_owner() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-swp03-changeowner").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    seed_write_access_rule_full(&ctx, "journal_entries", IMMUTABLE_OWNER_RULE).await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let mut fields = std::collections::HashMap::new();
    fields.insert("owner_id".to_string(), string_field("dana-kim"));

    let resp = update_document(
        &ctx,
        &ctx.document_resource_name("journal_entries", "maria-doc"),
        fields,
        Some(&marias_token),
    )
    .await;

    let err = resp.expect_err(
        "AC-17-32: an update that changes the protected field must be denied, even by the owner \
         of the existing document — the two-value old-vs-new comparison",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-32: denial must be attributable to the write rule (PermissionDenied), got {:?}",
        err.code()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-33: an update evaluation fails closed if the referenced field is
// missing from either the existing OR the proposed document (two
// sub-cases).
// ─────────────────────────────────────────────────────────────────────────────

/// Journey (sub-case A — missing from `resource.data`):
///   Given: `journal_entries` has the immutable-owner WRITE rule
///   And:   the seed document `no-owner-doc` has NO `owner_id` field at all
///   And:   Maria Santos holds a verified identity
///   When:  Maria calls `updateDoc()` proposing an `owner_id`
///   Then:  the update is denied (fail-closed) — `resource.data.owner_id`
///          cannot resolve on the existing document
///
/// AC-17-33 (sub-case A)
///
/// @error @driving_port @real-io @US-03 @AC-17-33
#[tokio::test]
async fn an_update_fails_closed_when_the_field_is_missing_from_the_existing_document() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-swp03-missingold").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    seed_write_access_rule_full(&ctx, "journal_entries", IMMUTABLE_OWNER_RULE).await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let mut fields = std::collections::HashMap::new();
    fields.insert("owner_id".to_string(), string_field("maria-santos"));

    let resp = update_document(
        &ctx,
        &ctx.document_resource_name("journal_entries", "no-owner-doc"),
        fields,
        Some(&marias_token),
    )
    .await;

    let err = resp.expect_err(
        "AC-17-33: an update must fail closed when the referenced field is missing from the \
         EXISTING document, never a crash",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-33: must fail CLOSED (PermissionDenied) — never Internal (a crash), got {:?}",
        err.code()
    );
}

/// Journey (sub-case B — missing from `request.resource.data`):
///   Given: `journal_entries` has the immutable-owner WRITE rule
///   And:   the seed document `maria-doc` is owned by `maria-santos`
///   And:   Maria Santos (the owner) holds a verified identity
///   When:  Maria calls `updateDoc()` proposing a document with NO
///          `owner_id` field at all
///   Then:  the update is denied (fail-closed) —
///          `request.resource.data.owner_id` cannot resolve on the proposed
///          document
///
/// AC-17-33 (sub-case B)
///
/// @error @driving_port @real-io @US-03 @AC-17-33
#[tokio::test]
async fn an_update_fails_closed_when_the_field_is_missing_from_the_proposed_document() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-swp03-missingnew").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    seed_write_access_rule_full(&ctx, "journal_entries", IMMUTABLE_OWNER_RULE).await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let mut fields = std::collections::HashMap::new();
    fields.insert("unrelated_field".to_string(), string_field("some-value"));

    let resp = update_document(
        &ctx,
        &ctx.document_resource_name("journal_entries", "maria-doc"),
        fields,
        Some(&marias_token),
    )
    .await;

    let err = resp.expect_err(
        "AC-17-33: an update must fail closed when the referenced field is missing from the \
         PROPOSED document, never a crash",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-33: must fail CLOSED (PermissionDenied) — never Internal (a crash), got {:?}",
        err.code()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-34: a denied update's response does not reveal whether the target
// document existed (mirrors `security-rules`' own AC-17-10 exactly).
// ─────────────────────────────────────────────────────────────────────────────

/// Locked security-observable behavior (ADR-030 § Existence non-leakage,
/// reusing ADR-029's mechanism verbatim). Compares Dana's denied update of
/// Maria's REAL (existing, wrong-owner) document against Dana's denied
/// update of a document ID that was NEVER seeded at all — both must produce
/// a byte-identical `PermissionDenied`, with no distinguishing detail
/// (status code AND message), mirroring
/// `sr02_signed_in_read_gated_by_rule::a_denied_read_never_reveals_whether_the_target_document_exists`.
///
/// AC-17-34
///
/// @error @driving_port @real-io @US-03 @AC-17-34 @security-regression
#[tokio::test]
async fn a_denied_update_never_reveals_whether_the_target_document_exists() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-swp03-nonleak").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    seed_write_access_rule_full(&ctx, "journal_entries", IMMUTABLE_OWNER_RULE).await;

    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    let mut fields_existing = std::collections::HashMap::new();
    fields_existing.insert("owner_id".to_string(), string_field("maria-santos"));
    let resp_existing_wrong_owner = update_document(
        &ctx,
        &ctx.document_resource_name("journal_entries", "maria-doc"),
        fields_existing,
        Some(&danas_token),
    )
    .await;

    let mut fields_nonexistent = std::collections::HashMap::new();
    fields_nonexistent.insert("owner_id".to_string(), string_field("dana-kim"));
    let resp_nonexistent = update_document(
        &ctx,
        &ctx.nonexistent_document_resource_name("journal_entries"),
        fields_nonexistent,
        Some(&danas_token),
    )
    .await;

    let err_existing = resp_existing_wrong_owner
        .expect_err("AC-17-34: wrong-owner update of an EXISTING document must be denied");
    let err_nonexistent = resp_nonexistent.expect_err(
        "AC-17-34: update of a NON-EXISTENT document must ALSO be denied (not NotFound)",
    );

    assert_eq!(
        err_existing.code(),
        err_nonexistent.code(),
        "AC-17-34: existence non-leakage — both denials must carry the IDENTICAL gRPC status code"
    );
    assert_eq!(
        err_existing.code(),
        tonic::Code::PermissionDenied,
        "AC-17-34: must be PermissionDenied, never NotFound (that would leak non-existence)"
    );
    assert_eq!(
        err_existing.message(),
        err_nonexistent.message(),
        "AC-17-34: existence non-leakage — both denials must carry the IDENTICAL message, \
         with no detail distinguishing 'wrong owner' from 'no such document'"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Regression guardrail (not one of the 5 numbered ACs above, but essential
// coverage — mirrors every prior slice's own "no rule = unaffected" case,
// e.g. Slice 02's `a_create_against_a_collection_with_no_write_rule_succeeds_unchanged`):
// an update against a collection with NO write rule succeeds exactly as
// before this feature existed.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `app_config` has NO write rule defined at all
///   When:  an UNAUTHENTICATED update (no client-identity token) is made
///          against its seed document `config-doc`
///   Then:  the update succeeds — the structural no-rule-defined guardrail
///          means `get_write_access_rule` returns `None` and
///          `evaluate()`/write-rule gating (including the pre-write fetch)
///          is never invoked at all
///
/// (regression guardrail, no dedicated AC number)
///
/// @driving_port @real-io @US-03
#[tokio::test]
async fn an_update_against_a_collection_with_no_write_rule_succeeds_unchanged() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-swp03-norule").await;

    let mut fields = std::collections::HashMap::new();
    fields.insert("enabled".to_string(), string_field("false"));

    let resp = update_document(
        &ctx,
        &ctx.document_resource_name("app_config", "config-doc"),
        fields,
        None,
    )
    .await;

    assert!(
        resp.is_ok(),
        "an update against a collection with no write rule defined must succeed unchanged: {:?}",
        resp.err()
    );
}
