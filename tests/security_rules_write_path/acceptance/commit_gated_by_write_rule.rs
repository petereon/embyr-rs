//! Bug fix (2026-08-30) — `handle_commit` (the `Commit` RPC, used for
//! transactional commits AND ordinary multi-document batch writes) never
//! evaluated write-path security rules before applying writes, unlike
//! `handle_create_document`/`handle_update_document`/`handle_delete_document`
//! (all correct, all covered by sibling slices in this same directory).
//!
//! Regression scenarios proven here:
//!   - a `Commit` write that violates its collection's write rule is denied,
//!     and the target document is left unchanged (the bypass this fix
//!     closes).
//!   - a `Commit` BATCH is rejected WHOLESALE when ANY one write in it
//!     violates its own collection's write rule — mirroring `Commit`'s own
//!     pre-existing all-or-nothing atomicity contract. The write that would
//!     have been allowed on its own must NOT be applied either.
//!   - a `Commit` write that satisfies the write rule succeeds (no
//!     regression on the allowed path).
//!   - a `Commit` write against a collection with NO write rule defined
//!     succeeds exactly as before this feature existed (no regression on
//!     the unrestricted path).
//!
//! Driving port: gRPC :8080 `Commit` (via `SecurityRulesFullContext` + this
//! directory's own `commit_writes`/`update_write`/`seed_write_access_rule_full`
//! helpers — Pillar 3, real Postgres, real gRPC, no mocks).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    begin_transaction, commit_writes, mint_client_identity_token, now_unix,
    seed_write_access_rule_full, string_field, update_write, SecurityRulesFullContext,
};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;

/// The ownership write rule this file exercises — the caller must own the
/// EXISTING document (grounded in Trailmark's `journal_entries`/`owner_id`
/// domain, matching sibling slices in this directory).
const OWNERSHIP_RULE: &str = "request.auth.uid == resource.data.owner_id";

/// Read a string field back off a `Document` response — the observable this
/// file's atomicity/unchanged assertions need.
fn field_string(doc: &embyr_proto::firestore::Document, key: &str) -> Option<String> {
    doc.fields.get(key).and_then(|v| match &v.value_type {
        Some(embyr_proto::firestore::value::ValueType::StringValue(s)) => Some(s.clone()),
        _ => None,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// A Commit write that violates its collection's write rule is denied, and
// the target document is left unchanged.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has an ownership WRITE rule
///   And:   the seed document `maria-doc` is owned by `maria-santos`
///   And:   Dana Kim (NOT the owner) holds a verified identity
///   When:  Dana calls `Commit` with a single write proposing a NEW
///          `owner_id` (`dana-kim`) on Maria's document
///   Then:  the commit is denied, attributable to the write rule
///   And:   the document is left unchanged (still owned by `maria-santos`)
///
/// @error @driving_port @real-io @bugfix
#[tokio::test]
async fn a_commit_write_that_violates_the_write_rule_is_denied() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-commit-deny").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    seed_write_access_rule_full(&ctx, "journal_entries", OWNERSHIP_RULE).await;

    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    let target = ctx.document_resource_name("journal_entries", "maria-doc");
    let mut fields = std::collections::HashMap::new();
    fields.insert("owner_id".to_string(), string_field("dana-kim"));

    let txn = begin_transaction(&ctx).await;
    let resp = commit_writes(
        &ctx,
        vec![update_write(&target, fields)],
        txn,
        Some(&danas_token),
    )
    .await;

    let err = resp.expect_err(
        "a Commit write that violates the write rule must be denied — this is the exact bypass \
         the bug fix closes",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "denial must be attributable to the write rule (PermissionDenied), got {:?}",
        err.code()
    );

    let doc = ctx
        .get_document(&target, None)
        .await
        .expect("the denied write must not have been applied — document must still exist")
        .into_inner();
    assert_eq!(
        field_string(&doc, "owner_id").as_deref(),
        Some("maria-santos"),
        "the denied Commit write must leave the target document unchanged"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// A Commit batch is rejected WHOLESALE when ANY one write in it violates its
// own collection's write rule — mirroring Commit's own all-or-nothing
// atomicity contract. The write that would have been allowed on its own
// must NOT be applied either.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has an ownership WRITE rule
///   And:   `trail_guides` has NO write rule defined at all
///   And:   Dana Kim (NOT the owner of `maria-doc`) holds a verified identity
///   When:  Dana calls `Commit` with a batch of TWO writes: (1) an update to
///          `trail_guides/guide-doc` that would be ALLOWED on its own (no
///          rule at all), and (2) an update to `journal_entries/maria-doc`
///          that is DENIED (Dana does not own it)
///   Then:  the ENTIRE commit is rejected, attributable to the write rule
///   And:   `trail_guides/guide-doc` is left UNCHANGED — proving the batch
///          did not partially apply
///
/// @error @driving_port @real-io @bugfix @atomicity
#[tokio::test]
async fn a_commit_batch_is_rejected_wholesale_when_any_write_violates_its_rule() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-commit-atomic").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    seed_write_access_rule_full(&ctx, "journal_entries", OWNERSHIP_RULE).await;

    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    let allowed_target = ctx.document_resource_name("trail_guides", "guide-doc");
    let mut allowed_fields = std::collections::HashMap::new();
    allowed_fields.insert("title".to_string(), string_field("Modified By Dana"));

    let denied_target = ctx.document_resource_name("journal_entries", "maria-doc");
    let mut denied_fields = std::collections::HashMap::new();
    denied_fields.insert("owner_id".to_string(), string_field("dana-kim"));

    let txn = begin_transaction(&ctx).await;
    let resp = commit_writes(
        &ctx,
        vec![
            update_write(&allowed_target, allowed_fields),
            update_write(&denied_target, denied_fields),
        ],
        txn,
        Some(&danas_token),
    )
    .await;

    let err = resp.expect_err(
        "a Commit batch containing ANY write that violates its own collection's write rule must \
         be rejected wholesale, before any write in the batch is applied"
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "denial must be attributable to the write rule (PermissionDenied), got {:?}",
        err.code()
    );

    let doc = ctx
        .get_document(&allowed_target, None)
        .await
        .expect("the otherwise-allowed write must not have been applied — document must still exist")
        .into_inner();
    assert_eq!(
        field_string(&doc, "title").as_deref(),
        Some("Guide"),
        "the otherwise-allowed write in a rejected batch must NOT have been applied — Commit's \
         atomicity contract requires all-or-nothing"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// A Commit write that satisfies the write rule succeeds (no regression on
// the allowed path).
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has an ownership WRITE rule
///   And:   the seed document `maria-doc` is owned by `maria-santos`
///   And:   Maria Santos (the owner) holds a verified identity
///   When:  Maria calls `Commit` with a single write on her own document
///   Then:  the commit succeeds
///
/// @driving_port @real-io @bugfix
#[tokio::test]
async fn a_commit_write_that_satisfies_the_write_rule_succeeds() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-commit-allow").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    seed_write_access_rule_full(&ctx, "journal_entries", OWNERSHIP_RULE).await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let target = ctx.document_resource_name("journal_entries", "maria-doc");
    let mut fields = std::collections::HashMap::new();
    fields.insert("owner_id".to_string(), string_field("maria-santos"));

    let txn = begin_transaction(&ctx).await;
    let resp = commit_writes(
        &ctx,
        vec![update_write(&target, fields)],
        txn,
        Some(&marias_token),
    )
    .await;

    assert!(
        resp.is_ok(),
        "a Commit write that satisfies the write rule must succeed: {:?}",
        resp.err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Regression guardrail: a Commit write against a collection with NO write
// rule defined succeeds exactly as before this feature existed.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `app_config` has NO write rule defined at all
///   When:  an UNAUTHENTICATED Commit (no client-identity token) is made
///          against its seed document `config-doc`
///   Then:  the commit succeeds unchanged
///
/// @driving_port @real-io @bugfix
#[tokio::test]
async fn a_commit_write_against_a_collection_with_no_write_rule_succeeds_unchanged() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-commit-norule").await;

    let target = ctx.document_resource_name("app_config", "config-doc");
    let mut fields = std::collections::HashMap::new();
    fields.insert("enabled".to_string(), string_field("false"));

    let txn = begin_transaction(&ctx).await;
    let resp = commit_writes(&ctx, vec![update_write(&target, fields)], txn, None).await;

    assert!(
        resp.is_ok(),
        "a Commit write against a collection with no write rule defined must succeed unchanged: {:?}",
        resp.err()
    );
}
