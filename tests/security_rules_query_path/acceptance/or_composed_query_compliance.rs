//! Slice 02 (US-02, firestore-or-filter-support) — OR-Composed Query
//! Security Compliance.
//!
//! Acceptance criteria verified here (feature-delta.md US-02):
//!   AC-OR-04: an OR query where only ONE branch binds the ownership-equality
//!             constraint is REJECTED — the other branch could match a
//!             document that does not satisfy the rule.
//!   AC-OR-05: an OR query where EVERY branch independently binds the
//!             ownership-equality constraint is ADMITTED, and returns the
//!             correctly-scoped result set.
//!
//! This is the security-critical half of firestore-or-filter-support:
//! `filter_binds_field_to_uid`'s own `CompositeOr` arm uses `.all()`, not
//! `.any()` (the semantics `Composite`'s own AND arm correctly uses) — using
//! `.any()` here would be a real access-control bypass, since a document
//! matching an OR filter need only satisfy ONE branch.
//!
//! Domain example (mirrors `and_composed_query_compliance.rs`'s own
//! `trip_photos` grounding): a `journal_entries`-style collection with rule
//! `request.auth.uid == resource.data.author_id`.
//!
//! Driving port: gRPC :8080 `RunQuery` (via `SecurityRulesFullContext` +
//! `run_query_raw` — Pillar 3, real Postgres, real gRPC, no mocks).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    create_document, equality_filter, mint_client_identity_token, now_unix, not_equal_filter,
    run_query_raw, string_field, SecurityRulesFullContext,
};
use ed25519_dalek::SigningKey;
use embyr_proto::firestore::structured_query::{
    composite_filter::Operator as CompositeOp, filter::FilterType, CompositeFilter, Filter,
};
use rand_core::OsRng;

const JOURNAL_ENTRIES_RULE: &str = "request.auth.uid == resource.data.author_id";

fn or_filter(branches: Vec<Filter>) -> Filter {
    Filter {
        filter_type: Some(FilterType::CompositeFilter(CompositeFilter {
            op: CompositeOp::Or as i32,
            filters: branches,
        })),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-OR-04: an OR query where only ONE branch proves ownership is REJECTED.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has an ownership-equality READ rule
///   And:   Maria Santos holds a verified identity
///   When:  Maria calls `RunQuery` with `Filter.or(author_id == "maria-santos",
///          status == "published")` — only the FIRST branch proves ownership
///   Then:  the query is REJECTED — a document could match via the `status ==
///          "published"` branch alone, without satisfying the rule
///
/// AC-OR-04
///
/// @error @driving_port @real-io @US-02 @AC-OR-04
#[tokio::test]
async fn an_or_query_where_only_one_branch_proves_ownership_is_rejected() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-or04-partial").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule("journal_entries", JOURNAL_ENTRIES_RULE).await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let filter = or_filter(vec![
        equality_filter("author_id", "maria-santos"),
        equality_filter("status", "published"),
    ]);

    let result = run_query_raw(
        &ctx,
        "journal_entries",
        Some(filter),
        Some(&marias_token),
        &ctx.api_key,
    )
    .await;

    let err = result.expect_err(
        "AC-OR-04: an OR query where only one branch proves ownership must be rejected — the \
         other branch could match a document that does not satisfy the rule",
    );
    assert_eq!(
        err.code(),
        tonic::Code::PermissionDenied,
        "AC-OR-04: rejection must be attributable to the rule (PermissionDenied), got {:?}: {}",
        err.code(),
        err.message()
    );
    assert!(
        err.message().contains("[OWNERSHIP_FILTER_MISSING]") && err.message().contains("author_id"),
        "AC-OR-04: rejection must name the unmet ownership conjunct specifically, got: {}",
        err.message()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-OR-05: an OR query where EVERY branch independently proves ownership is
// ADMITTED and returns the correctly-scoped result set.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has the same ownership-equality READ rule
///   And:   Maria Santos holds a verified identity and authored a seeded entry
///   When:  Maria calls `RunQuery` with `Filter.or(author_id == "maria-santos",
///          author_id == "maria-santos")` — a degenerate but valid case where
///          BOTH branches independently prove ownership
///   Then:  the query is ADMITTED and returns the seeded document
///
/// AC-OR-05
///
/// @driving_port @real-io @US-02 @AC-OR-05
#[tokio::test]
async fn an_or_query_where_every_branch_proves_ownership_is_admitted() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-or05-full").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule("journal_entries", JOURNAL_ENTRIES_RULE).await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let mut fields = std::collections::HashMap::new();
    fields.insert("author_id".to_string(), string_field("maria-santos"));
    fields.insert("title".to_string(), string_field("patagonia-trip-notes"));
    create_document(&ctx, "journal_entries", "or05-entry-doc", fields, Some(&marias_token))
        .await
        .expect("seed journal_entries document");

    let filter = or_filter(vec![
        equality_filter("author_id", "maria-santos"),
        equality_filter("author_id", "maria-santos"),
    ]);

    let admitted = run_query_raw(
        &ctx,
        "journal_entries",
        Some(filter),
        Some(&marias_token),
        &ctx.api_key,
    )
    .await
    .expect("AC-OR-05: an OR query where every branch proves ownership must be admitted");

    assert_eq!(admitted.len(), 1, "expected the seeded journal entry document");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-OR-04 (nested case): an OR branch that ITSELF AND-composes with the
// ownership filter also independently proves ownership — proving `.all()`
// recurses correctly through nested AND-inside-OR, not just bare field
// filters.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has the same ownership-equality READ rule
///   When:  Maria calls `RunQuery` with `Filter.or(author_id == "maria-santos",
///          And(author_id == "maria-santos", status == "draft"))` — the
///          SECOND branch is itself an AND that includes the ownership
///          filter alongside an unrelated one
///   Then:  the query is ADMITTED — both branches, recursively, prove
///          ownership
///
/// AC-OR-05 (nested AND-inside-OR case)
///
/// @driving_port @real-io @US-02 @AC-OR-05
#[tokio::test]
async fn an_or_branch_that_and_composes_with_ownership_still_proves_ownership() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-or05-nested").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule("journal_entries", JOURNAL_ENTRIES_RULE).await;

    let marias_token = mint_client_identity_token(
        &signing_key,
        "maria-santos",
        &ctx.project_id,
        now_unix() + 3600,
    );

    let mut fields = std::collections::HashMap::new();
    fields.insert("author_id".to_string(), string_field("maria-santos"));
    fields.insert("status".to_string(), string_field("draft"));
    create_document(&ctx, "journal_entries", "or05-nested-doc", fields, Some(&marias_token))
        .await
        .expect("seed journal_entries document");

    let and_branch = Filter {
        filter_type: Some(FilterType::CompositeFilter(CompositeFilter {
            op: CompositeOp::And as i32,
            filters: vec![
                equality_filter("author_id", "maria-santos"),
                equality_filter("status", "draft"),
            ],
        })),
    };
    let filter = or_filter(vec![equality_filter("author_id", "maria-santos"), and_branch]);

    let admitted = run_query_raw(
        &ctx,
        "journal_entries",
        Some(filter),
        Some(&marias_token),
        &ctx.api_key,
    )
    .await
    .expect(
        "AC-OR-05 (nested): an OR branch that itself AND-composes with the ownership filter \
         must still independently prove ownership",
    );

    assert_eq!(admitted.len(), 1, "expected the seeded journal entry document");
}
