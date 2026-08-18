//! Slice 04 (US-04, ADR-030) — A Delete Is Gated by the Write Rule Using
//! Only the Existing Document.
//!
//! Acceptance criteria verified here (feature-delta.md US-04):
//!   AC-17-35: a delete where the caller owns the existing document
//!             succeeds.
//!   AC-17-36: a delete where the caller does not own the existing document
//!             is denied.
//!   AC-17-37: a delete evaluation fails closed on a missing referenced
//!             field, never crashes.
//!   AC-17-38: a denied delete's response does not reveal whether the
//!             target document existed (mirrors AC-17-34/AC-17-10 exactly).
//!   (regression, not separately numbered) a delete against a collection
//!             with NO write rule succeeds exactly as before.
//!
//! Driving port: gRPC :8080 `DeleteDocument` (via `SecurityRulesFullContext`
//! + this file's own `delete_document`/`seed_write_access_rule_full`
//! helpers — Pillar 3, real Postgres, real gRPC, no mocks).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    delete_document, mint_client_identity_token, now_unix, seed_write_access_rule_full,
    SecurityRulesFullContext,
};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

/// The ownership write rule this slice exists to prove: the caller must own
/// the EXISTING document — `resource.data.<field>` only, no
/// `request.resource.data` reference (a delete has no proposed new
/// document).
const OWNER_RULE: &str = "request.auth.uid == resource.data.owner_id";

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-35: a delete where the caller owns the existing document succeeds.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has a WRITE rule requiring the caller to own
///          the existing document (`resource.data.owner_id`)
///   And:   the seed document `maria-doc` is owned by `maria-santos`
///   And:   Maria Santos (the owner) holds a verified identity
///   When:  Maria calls `deleteDoc()` on her own document
///   Then:  the delete succeeds
///
/// AC-17-35
///
/// @driving_port @real-io @US-04 @AC-17-35
#[tokio::test]
async fn a_delete_by_the_owner_of_the_existing_document_succeeds() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-swp04-allow").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    seed_write_access_rule_full(&ctx, "journal_entries", OWNER_RULE).await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let resp = delete_document(
        &ctx,
        &ctx.document_resource_name("journal_entries", "maria-doc"),
        Some(&marias_token),
    )
    .await;

    assert!(
        resp.is_ok(),
        "AC-17-35: a delete by the owner of the existing document must succeed: {:?}",
        resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-37: a delete evaluation fails closed on a missing referenced field,
// never crashes.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has a WRITE rule requiring the caller to own
///          the existing document (`resource.data.owner_id`)
///   And:   the seed document `no-owner-doc` has NO `owner_id` field at all
///   And:   Maria Santos holds a verified identity
///   When:  Maria calls `deleteDoc()` on the fieldless document
///   Then:  the delete is denied (fail-closed) — `resource.data.owner_id`
///          cannot resolve, never a crash
///
/// AC-17-37
///
/// @error @driving_port @real-io @US-04 @AC-17-37
#[tokio::test]
async fn a_delete_fails_closed_when_the_referenced_field_is_missing() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-swp04-missingfield").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    seed_write_access_rule_full(&ctx, "journal_entries", OWNER_RULE).await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let resp = delete_document(
        &ctx,
        &ctx.document_resource_name("journal_entries", "no-owner-doc"),
        Some(&marias_token),
    )
    .await;

    let err = resp.expect_err(
        "AC-17-37: a delete must fail closed when the referenced field is missing from the \
         existing document, never a crash",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-37: must fail CLOSED (PermissionDenied) — never Internal (a crash), got {:?}",
        err.code()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-38: a denied delete's response does not reveal whether the target
// document existed (mirrors `security-rules-write-path`'s own AC-17-34 /
// `security-rules`' own AC-17-10 exactly).
// ─────────────────────────────────────────────────────────────────────────────

/// Locked security-observable behavior (ADR-030 § Existence non-leakage,
/// reusing ADR-029's mechanism verbatim). Compares Dana's denied delete of
/// Maria's REAL (existing, wrong-owner) document against Dana's denied
/// delete of a document ID that was NEVER seeded at all — both must produce
/// a byte-identical `PermissionDenied`, with no distinguishing detail
/// (status code AND message).
///
/// AC-17-38
///
/// @error @driving_port @real-io @US-04 @AC-17-38 @security-regression
#[tokio::test]
async fn a_denied_delete_never_reveals_whether_the_target_document_exists() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-swp04-nonleak").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    seed_write_access_rule_full(&ctx, "journal_entries", OWNER_RULE).await;

    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    let resp_existing_wrong_owner = delete_document(
        &ctx,
        &ctx.document_resource_name("journal_entries", "maria-doc"),
        Some(&danas_token),
    )
    .await;

    let resp_nonexistent = delete_document(
        &ctx,
        &ctx.nonexistent_document_resource_name("journal_entries"),
        Some(&danas_token),
    )
    .await;

    let err_existing = resp_existing_wrong_owner
        .expect_err("AC-17-38: wrong-owner delete of an EXISTING document must be denied");
    let err_nonexistent = resp_nonexistent.expect_err(
        "AC-17-38: delete of a NON-EXISTENT document must ALSO be denied (not NotFound)",
    );

    assert_eq!(
        err_existing.code(),
        err_nonexistent.code(),
        "AC-17-38: existence non-leakage — both denials must carry the IDENTICAL gRPC status code"
    );
    assert_eq!(
        err_existing.code(),
        tonic::Code::PermissionDenied,
        "AC-17-38: must be PermissionDenied, never NotFound (that would leak non-existence)"
    );
    assert_eq!(
        err_existing.message(),
        err_nonexistent.message(),
        "AC-17-38: existence non-leakage — both denials must carry the IDENTICAL message, \
         with no detail distinguishing 'wrong owner' from 'no such document'"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-36: a delete where the caller does not own the existing document is
// denied.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has a WRITE rule requiring the caller to own
///          the existing document (`resource.data.owner_id`)
///   And:   the seed document `maria-doc` is owned by `maria-santos`
///   And:   Dana Kim (NOT the owner) holds a verified identity
///   When:  Dana calls `deleteDoc()` on Maria's document
///   Then:  the delete is denied — Dana does not own the existing document
///
/// AC-17-36
///
/// @error @driving_port @real-io @US-04 @AC-17-36
#[tokio::test]
async fn a_delete_by_a_non_owner_of_the_existing_document_is_denied() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-swp04-nonowner").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    seed_write_access_rule_full(&ctx, "journal_entries", OWNER_RULE).await;

    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    let resp = delete_document(
        &ctx,
        &ctx.document_resource_name("journal_entries", "maria-doc"),
        Some(&danas_token),
    )
    .await;

    let err = resp.expect_err(
        "AC-17-36: a delete by a caller who does not own the existing document must be denied",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-17-36: denial must be attributable to the write rule (PermissionDenied), got {:?}",
        err.code()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Regression guardrail (not one of the 4 numbered ACs above, but essential
// coverage — mirrors every prior slice's own "no rule = unaffected" case,
// e.g. Slice 03's `an_update_against_a_collection_with_no_write_rule_succeeds_unchanged`):
// a delete against a collection with NO write rule succeeds exactly as
// before.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `app_config` has NO write rule defined at all
///   When:  an UNAUTHENTICATED delete (no client-identity token) is made
///          against its seed document `config-doc`
///   Then:  the delete succeeds — the structural no-rule-defined guardrail
///          means `get_write_access_rule` returns `None` and
///          `evaluate()`/write-rule gating (including the pre-write fetch)
///          is never invoked at all
///
/// (regression guardrail, no dedicated AC number)
///
/// @driving_port @real-io @US-04
#[tokio::test]
async fn a_delete_against_a_collection_with_no_write_rule_succeeds_unchanged() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-swp04-norule").await;

    let resp = delete_document(
        &ctx,
        &ctx.document_resource_name("app_config", "config-doc"),
        None,
    )
    .await;

    assert!(
        resp.is_ok(),
        "a delete against a collection with no write rule defined must succeed unchanged: {:?}",
        resp.err()
    );
}
