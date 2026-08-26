//! Slice 02 (US-02, ADR-033) — A Rule-Protected Collection's Initial
//! Snapshot Honors the Caller's Own Query Filter.
//!
//! Fixes a currently-shipped, project-wide bug: `handle_add_target`'s own
//! `domain_query.filter` is hardcoded `None` — the client's own
//! `StructuredQuery.where_` clause is never extracted or consulted. Every
//! Listen subscription's initial snapshot today returns the ENTIRE
//! unfiltered collection, regardless of what filter the client's SDK query
//! specified. This slice fixes it by extracting the full `StructuredQuery`
//! (not merely `from[0].collection_id`) and reusing `translate_filter()` —
//! the EXACT function `RunQuery` already uses — to build
//! `domain_query.filter`.
//!
//! Acceptance criteria verified here (slice-02-initial-snapshot-honors-filter.md):
//!   AC-17-109: a Listen subscription's initial snapshot narrows by every
//!              filter specified in the client's own `StructuredQuery.where_`,
//!              matching `RunQuery`'s own filter-honoring behavior for an
//!              identical filter shape and identical seeded documents.
//!   AC-17-110: composite AND filters are honored in FULL (2+ conjuncts),
//!              not partially applied.
//!   AC-17-111: a subscription specifying NO filter at all continues to
//!              return the full, unfiltered collection — unchanged from
//!              pre-feature behavior (a regression guard, not a new
//!              capability).
//!   AC-17-112: filter translation reuses `translate_filter()` — this is a
//!              structural/reuse property (no second, independently
//!              -maintained filter-translation path), verified via `git
//!              diff` confirming `translate_filter`'s own implementation
//!              body is unchanged (only its visibility widened), NOT via a
//!              runtime assertion here. Asserting AST/source shape in a
//!              test would itself be a banned "AST-shape test"
//!              (Testing Theater Pattern) — the correct proof is the code
//!              review artifact (this feature's commit message), not a
//!              test. AC-17-109/110's own oracle-comparison design below
//!              (Listen's narrowed snapshot vs `RunQuery`'s own, run
//!              against IDENTICAL filters and seeded documents) is the
//!              strongest available BEHAVIORAL evidence that no divergent,
//!              second filter-translation path exists: two independently
//!              -composed call sites feeding the SAME function necessarily
//!              agree.
//!
//! Driving port: gRPC :8080 `Listen` (via `SecurityRulesFullContext` +
//! this feature's own `open_listen_stream_filtered`/`collect_initial_snapshot`
//! helpers) cross-checked against gRPC :8080 `RunQuery` (via
//! `security_rules_query_path`'s own `run_query` helper) — Pillar 3, real
//! Postgres, real gRPC, no mocks.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    collect_initial_snapshot, create_document, equality_where_filter, open_listen_stream,
    open_listen_stream_filtered, run_query, string_field, SecurityRulesFullContext,
};

use std::collections::HashMap;

fn owner_doc(owner_id: &str) -> HashMap<String, embyr_proto::firestore::Value> {
    let mut fields = HashMap::new();
    fields.insert("owner_id".to_string(), string_field(owner_id));
    fields
}

fn owner_status_doc(owner_id: &str, status: &str) -> HashMap<String, embyr_proto::firestore::Value> {
    let mut fields = owner_doc(owner_id);
    fields.insert("status".to_string(), string_field(status));
    fields
}

fn sorted(mut names: Vec<String>) -> Vec<String> {
    names.sort();
    names
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-109: a Listen subscription's initial snapshot narrows by the
// client's own single-field-equality filter, matching RunQuery's own
// filter-honoring behavior for an identical filter shape and identical
// seeded documents.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `journal_entries` has the context's own default-seeded document
///          (owned by Maria, see `SecurityRulesFullContext::new`'s own doc
///          comment) plus 2 MORE documents owned by Maria and 1 owned by
///          Dana, no access rule defined (this slice is filter-mechanics
///          only — compliance-checking is Slice 03's own job)
///   When:  a Listen subscription's `AddTarget` carries a filter
///          `owner_id == "maria-santos"`
///   Then:  the initial snapshot contains ONLY Maria's 3 documents (the
///          default fixture + the 2 seeded here) — the EXACT SAME set
///          `RunQuery` returns for the identical filter against the
///          identical seeded documents
///
/// AC-17-109
///
/// @driving_port @real-io @US-02 @AC-17-109
#[tokio::test]
async fn initial_snapshot_narrows_by_the_callers_single_field_filter() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-srr02-single").await;

    create_document(&ctx, "journal_entries", "srr02-maria-1", owner_doc("maria-santos"), None)
        .await
        .expect("seed maria doc 1");
    create_document(&ctx, "journal_entries", "srr02-maria-2", owner_doc("maria-santos"), None)
        .await
        .expect("seed maria doc 2");
    create_document(&ctx, "journal_entries", "srr02-dana-1", owner_doc("dana-kim"), None)
        .await
        .expect("seed dana doc");

    let filter = equality_where_filter(&[("owner_id", "maria-santos")]);
    let mut stream =
        open_listen_stream_filtered(&ctx, "journal_entries", filter).await;
    let listen_names = sorted(collect_initial_snapshot(&mut stream).await);

    let run_query_docs = run_query(&ctx, "journal_entries", &[("owner_id", "maria-santos")], None)
        .await
        .expect("RunQuery with the identical filter should succeed");
    let run_query_names = sorted(run_query_docs.into_iter().map(|d| d.name).collect());

    assert_eq!(
        listen_names.len(),
        3,
        "AC-17-109: expected exactly Maria's 3 documents (default fixture + 2 seeded here) in \
         the initial snapshot, got: {listen_names:?}"
    );
    assert_eq!(
        listen_names, run_query_names,
        "AC-17-109: Listen's initial snapshot must match RunQuery's own filter-honoring \
         behavior for an identical filter and identical seeded documents"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-110: composite AND filters (2+ conjuncts) are honored in FULL, not
// partially applied.
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `trip_photos` has one document matching BOTH conjuncts
///          (`owner_id == "maria-santos"` AND `status == "active"`), one
///          matching only the owner conjunct, and one matching only the
///          status conjunct
///   When:  a Listen subscription's `AddTarget` carries the composite AND
///          filter (both conjuncts)
///   Then:  the initial snapshot contains ONLY the document matching BOTH
///          conjuncts — the EXACT SAME set `RunQuery` returns for the
///          identical composite filter
///
/// AC-17-110
///
/// @driving_port @real-io @US-02 @AC-17-110
#[tokio::test]
async fn initial_snapshot_honors_composite_and_filter_in_full() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-srr02-composite").await;

    create_document(
        &ctx,
        "trip_photos",
        "srr02-both-match",
        owner_status_doc("maria-santos", "active"),
        None,
    )
    .await
    .expect("seed doc matching both conjuncts");
    create_document(
        &ctx,
        "trip_photos",
        "srr02-owner-only",
        owner_status_doc("maria-santos", "archived"),
        None,
    )
    .await
    .expect("seed doc matching owner conjunct only");
    create_document(
        &ctx,
        "trip_photos",
        "srr02-status-only",
        owner_status_doc("dana-kim", "active"),
        None,
    )
    .await
    .expect("seed doc matching status conjunct only");

    let filter = equality_where_filter(&[("owner_id", "maria-santos"), ("status", "active")]);
    let mut stream = open_listen_stream_filtered(&ctx, "trip_photos", filter).await;
    let listen_names = sorted(collect_initial_snapshot(&mut stream).await);

    let run_query_docs = run_query(
        &ctx,
        "trip_photos",
        &[("owner_id", "maria-santos"), ("status", "active")],
        None,
    )
    .await
    .expect("RunQuery with the identical composite filter should succeed");
    let run_query_names = sorted(run_query_docs.into_iter().map(|d| d.name).collect());

    assert_eq!(
        listen_names.len(),
        1,
        "AC-17-110: expected exactly the ONE document matching BOTH conjuncts, got: {listen_names:?}"
    );
    assert!(
        listen_names[0].ends_with("srr02-both-match"),
        "AC-17-110: expected the doubly-matching document, got: {listen_names:?}"
    );
    assert_eq!(
        listen_names, run_query_names,
        "AC-17-110: Listen's initial snapshot must match RunQuery's own composite-AND-filter \
         behavior for an identical filter and identical seeded documents"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-17-111: a subscription specifying NO filter at all continues to return
// the full, unfiltered collection — unchanged from pre-feature behavior (a
// regression guard, not a new capability).
// ─────────────────────────────────────────────────────────────────────────────

/// Journey:
///   Given: `app_config` has the context's own default-seeded document (see
///          `SecurityRulesFullContext::new`'s own doc comment) plus 3 MORE
///          documents with varying fields
///   When:  a Listen subscription's `AddTarget` carries NO `where_` filter
///   Then:  the initial snapshot contains all 4 documents (the default
///          fixture + the 3 seeded here) — an absent filter is not itself a
///          bug, this slice must not over-narrow
///
/// AC-17-111
///
/// @driving_port @real-io @US-02 @AC-17-111
#[tokio::test]
async fn initial_snapshot_stays_unfiltered_when_the_caller_specifies_no_filter() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-srr02-nofilter").await;

    create_document(&ctx, "app_config", "srr02-cfg-1", owner_doc("maria-santos"), None)
        .await
        .expect("seed cfg doc 1");
    create_document(&ctx, "app_config", "srr02-cfg-2", owner_doc("dana-kim"), None)
        .await
        .expect("seed cfg doc 2");
    create_document(&ctx, "app_config", "srr02-cfg-3", owner_doc("anyone"), None)
        .await
        .expect("seed cfg doc 3");

    let mut stream = open_listen_stream(&ctx, "app_config").await;
    let listen_names = collect_initial_snapshot(&mut stream).await;

    assert_eq!(
        listen_names.len(),
        4,
        "AC-17-111: an unfiltered subscription must continue returning the FULL collection \
         (default fixture + 3 seeded here), got: {listen_names:?}"
    );
}
