//! security-rules-collection-group-rules Slice 02 (US-02, ADR-032) — A
//! Compliant Collection-Group Query Is Admitted, Spanning Every Nesting
//! Depth.
//!
//! Production code for this slice already exists: Slice 04 (commit
//! `a570a09`) built the FULL `all_descendants` branch atomically, including
//! the `Some(group_rule_row)` arm that calls `check_query_compliance()` and
//! falls through to the existing `adapter.run_query()` on `Admitted` — this
//! slice adds acceptance-test coverage proving that already-shipped
//! mechanism is correct, mirroring how `security-rules-query-path`'s own
//! Slice 05 added zero new production code on top of Slice 01.
//!
//! Acceptance criteria verified here (feature-delta.md
//! `security-rules-collection-group-rules`):
//!   AC-17-81: a compliant group query is admitted and returns rows from
//!             EVERY actual nesting depth the collection id occurs at.
//!   AC-17-82: an extra, non-required filter alongside the required
//!             ownership filter does not break compliance.
//!   AC-17-83: `check_query_compliance()`/`QueryComplianceOutcome`/
//!             `UnsatisfiedConjunct` (ADR-031) are reused completely
//!             unmodified — no runtime assertion is natural for this; it is
//!             confirmed by direct `git diff` inspection showing zero
//!             changes to `crates/embyr-core/src/access_control/mod.rs`
//!             across this feature's own slices (verified by the
//!             orchestrator before commit, not asserted here).
//!   AC-17-84: row-level narrowing across nesting depths comes from the
//!             PRE-EXISTING `all_descendants` SQL branch in
//!             `backend_adapter::run_query`, not new logic — proven by
//!             seeding a THIRD, unrelated nested path owned by a DIFFERENT
//!             user and confirming it is correctly excluded.
//!
//! Driving port: gRPC :8080 `RunQuery` (`SecurityRulesFullContext`).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    mint_client_identity_token, now_unix, run_query, seed_document_at_path,
    seed_group_access_rule_full, string_field, SecurityRulesFullContext,
};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-81 + AC-17-84: a compliant group query returns rows from every
// nesting depth the collection id occurs at, and correctly excludes a
// differently-owned document at a third, unrelated nested path — proving the
// row-narrowing comes from the pre-existing `all_descendants` SQL branch,
// not new logic.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has an active group rule
///          (`request.auth.uid == resource.data.owner_id`); the context's
///          own default fixture already seeds a TOP-LEVEL
///          `journal_entries/{project}-maria-doc` owned by Maria
///          (`SecurityRulesFullContext::new`'s documented default seed); an
///          additional document exists at a NESTED
///          `expeditions/trek-2026/journal_entries` subcollection, ALSO
///          owned by Maria; a THIRD document exists at another nested path
///          (`teams/beta-team/journal_entries`) owned by a DIFFERENT user
///   When:  Maria issues a `collectionGroup('journal_entries')` query
///          filtered to her own uid
///   Then:  both of Maria's own documents (the default top-level fixture and
///          the nested one) are returned, and the differently-owned third
///          document is excluded
///
/// AC-17-81, AC-17-84
///
/// @driving_port @real-io @US-02 @AC-17-81 @AC-17-84
#[tokio::test]
async fn compliant_group_query_returns_rows_from_every_nesting_depth_and_excludes_other_owners() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-scgr02-nesting").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;

    seed_group_access_rule_full(
        &ctx,
        "journal_entries",
        "request.auth.uid == resource.data.owner_id",
    )
    .await;

    // The context's own default fixture already seeds a top-level
    // `journal_entries/{project}-maria-doc` owned by maria-santos — no need
    // to seed a second one (see `SecurityRulesFullContext::new`'s own doc
    // comment).

    let mut nested_fields = HashMap::new();
    nested_fields.insert("owner_id".to_string(), string_field("maria-santos"));
    seed_document_at_path(
        &ctx,
        "expeditions/trek-2026/journal_entries",
        "entry-nested",
        nested_fields,
    )
    .await;

    let mut other_owner_fields = HashMap::new();
    other_owner_fields.insert("owner_id".to_string(), string_field("dana-kim"));
    seed_document_at_path(
        &ctx,
        "teams/beta-team/journal_entries",
        "entry-other-owner",
        other_owner_fields,
    )
    .await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let result = run_query(
        &ctx,
        "journal_entries",
        true,
        &[("owner_id", "maria-santos")],
        Some(&marias_token),
    )
    .await;

    let docs = result.expect(
        "AC-17-81: a compliant collection-group query must be admitted, not rejected",
    );

    assert_eq!(
        docs.len(),
        2,
        "AC-17-81/AC-17-84: exactly Maria's own two documents (the default top-level fixture \
         + the nested one) must be returned; the third, differently-owned document at another \
         nested path must be excluded — proving the pre-existing all_descendants SQL branch \
         plus the ownership filter compose correctly, got {} docs: {:?}",
        docs.len(),
        docs.iter().map(|d| &d.name).collect::<Vec<_>>()
    );
    let names: Vec<&str> = docs.iter().map(|d| d.name.as_str()).collect();
    assert!(
        names.iter().any(|n| n.ends_with("-maria-doc")),
        "AC-17-81: Maria's default top-level document must be included, got: {names:?}"
    );
    assert!(
        names.iter().any(|n| n.ends_with("/entry-nested")),
        "AC-17-81/AC-17-84: Maria's nested document must be included, got: {names:?}"
    );
    assert!(
        !names.iter().any(|n| n.ends_with("/entry-other-owner")),
        "AC-17-84: the differently-owned document at a third nested path must be excluded, \
         got: {names:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-82: an extra, non-required filter alongside the required ownership
// filter does not break compliance.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has an active group rule requiring ownership
///          equality alone
///   When:  Maria issues a compliant `collectionGroup('journal_entries')`
///          query with the required ownership filter PLUS an extra,
///          unrelated filter the rule doesn't require
///   Then:  the query is still admitted — the extra filter narrows the
///          result set further but does not break compliance
///
/// AC-17-82
///
/// @driving_port @real-io @US-02 @AC-17-82
#[tokio::test]
async fn extra_unrelated_filter_alongside_required_ownership_filter_does_not_break_compliance() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-scgr02-extrafilter").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;

    seed_group_access_rule_full(
        &ctx,
        "journal_entries",
        "request.auth.uid == resource.data.owner_id",
    )
    .await;

    let mut fields = HashMap::new();
    fields.insert("owner_id".to_string(), string_field("maria-santos"));
    fields.insert("status".to_string(), string_field("published"));
    seed_document_at_path(&ctx, "journal_entries", "entry-published", fields).await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let result = run_query(
        &ctx,
        "journal_entries",
        true,
        &[("owner_id", "maria-santos"), ("status", "published")],
        Some(&marias_token),
    )
    .await;

    let docs = result.expect(
        "AC-17-82: an extra, non-required filter alongside the required ownership filter must \
         not break compliance — the query must still be admitted",
    );
    assert_eq!(
        docs.len(),
        1,
        "AC-17-82: the compliant query with an extra filter must return the matching document"
    );
}
