//! ALC01 (Slice 01, Walking Skeleton, LAST/ONLY slice, US-01) — Alex
//! Enumerates a Document's Subcollections Against an Agent-Mode Project
//! (ADR-059).
//!
//! Acceptance criteria verified here (slice-01-enumerate-subcollections.md):
//!   AC-01: returns exactly the correct, distinct set of child collection
//!          IDs for a document with subcollections.
//!   AC-02: returns an empty list (not an error) for a document with no
//!          subcollections.
//!   AC-03: pagination returns the complete set with no duplicates or gaps
//!          — exercised at the exact 100/101 boundary worked through in
//!          ADR-059 § Decision 3 (the boundary `AgentBackendAdapter`'s own
//!          page-flattening loop must get right, not a rounder number).
//!   AC-04: subcollections are correctly scoped to their own parent
//!          document, never conflated with a sibling document's own
//!          identically-named subcollection.
//!   AC-05: exercised against a real `embyr-agent` binary and real Postgres
//!          (every test below uses `AgentModeWriteStreamingContext`, WS
//!          Strategy B — no mocked transport).
//!
//! Driving port: gRPC :8080 `ListCollectionIds` (client-facing RPC, already
//! shipped by `firestore-list-rpcs`/ADR-051 — unchanged by this feature).
//! This feature's own new code (proto RPC + agent handler +
//! `AgentBackendAdapter` override) is exercised transitively: the existing
//! handler now resolves to a real `AgentBackendAdapter::list_collection_ids`
//! implementation for `backend_mode=agent` instead of the inherited
//! default-error body.
//!
//! Test Budget: 4 behaviors (correct distinct set; empty-no-error;
//! pagination completeness at the 100/101 boundary; sibling-parent scoping)
//! x 2 = 8 max. 4 written — one per behavior, matching this feature's own
//! Domain Examples 1-4 in feature-delta.md, no variation-inflation.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{list_collection_ids, nested_parent, seed_documents, AgentModeWriteStreamingContext};

/// AC-01: `patients/p-204` has 3 subcollections (`visits`, `labResults`,
/// `medications`); the call returns exactly those 3 IDs, distinct.
///
/// @driving_port @real-io @US-01 @AC-01
#[tokio::test]
async fn returns_exactly_the_distinct_set_of_child_collection_ids() {
    let ctx = AgentModeWriteStreamingContext::new("alc01-happy-path").await;

    seed_documents(
        &ctx,
        "patients/p-204",
        &[
            ("visits".to_string(), "v-1"),
            ("labResults".to_string(), "l-1"),
            ("medications".to_string(), "m-1"),
        ],
    )
    .await;

    let parent = nested_parent(&ctx, "patients/p-204");
    let response = list_collection_ids(&ctx, &parent, 10, "")
        .await
        .expect("AC-01: ListCollectionIds must succeed against an agent-mode project");

    let mut ids = response.collection_ids.clone();
    ids.sort();
    let mut expected = vec!["visits".to_string(), "labResults".to_string(), "medications".to_string()];
    expected.sort();
    assert_eq!(ids, expected, "AC-01: must return exactly the document's real subcollections");
    assert!(response.next_page_token.is_empty(), "AC-01: single page, no next_page_token");
}

/// AC-02: `patients/p-209` is a newly-created patient record with no
/// subcollections — the call returns an empty list, not an error.
///
/// @driving_port @real-io @US-01 @AC-02
#[tokio::test]
async fn a_document_with_no_subcollections_returns_an_empty_list_not_an_error() {
    let ctx = AgentModeWriteStreamingContext::new("alc01-empty").await;

    // patients/p-209 itself is never created — an absent parent document
    // still has zero subcollections, which must not error (AC-02's own
    // domain example: "a newly-created patient record with no
    // subcollections yet").
    let parent = nested_parent(&ctx, "patients/p-209");
    let response = list_collection_ids(&ctx, &parent, 10, "")
        .await
        .expect("AC-02: a document with no subcollections must not error");

    assert!(response.collection_ids.is_empty(), "AC-02: collection_ids must be empty");
    assert!(response.next_page_token.is_empty(), "AC-02: next_page_token must be empty");
}

/// AC-03: `patients/p-204` has 101 distinct subcollections. The first call
/// (`page_size=100`) returns exactly 100 IDs plus a `next_page_token`; a
/// follow-up call with that token returns the remaining 1, with no overlap
/// and no gap across the two pages — the exact 100/101 boundary worked
/// through in ADR-059 § Decision 3 (Alternative A's naive single-round-trip
/// forward is demonstrated incorrect at precisely this boundary).
///
/// @driving_port @real-io @US-01 @AC-03
#[tokio::test]
async fn pagination_returns_the_complete_set_with_no_duplicates_or_gaps_at_the_100_101_boundary() {
    let ctx = AgentModeWriteStreamingContext::new("alc01-pagination-boundary").await;

    let subcollections: Vec<(String, &str)> =
        (0..101).map(|i| (format!("sub{i:03}"), "doc-1")).collect();
    seed_documents(&ctx, "patients/p-204", &subcollections).await;

    let parent = nested_parent(&ctx, "patients/p-204");

    let page1 = list_collection_ids(&ctx, &parent, 100, "")
        .await
        .expect("AC-03: page 1 must succeed");
    assert_eq!(page1.collection_ids.len(), 100, "AC-03: page 1 must contain exactly page_size IDs");
    assert!(!page1.next_page_token.is_empty(), "AC-03: next_page_token must be present when more remain");

    let page2 = list_collection_ids(&ctx, &parent, 100, &page1.next_page_token)
        .await
        .expect("AC-03: page 2 must succeed");
    assert_eq!(page2.collection_ids.len(), 1, "AC-03: page 2 must contain exactly the remaining ID");
    assert!(page2.next_page_token.is_empty(), "AC-03: next_page_token must be empty on the last page");

    let mut seen: Vec<String> = page1
        .collection_ids
        .iter()
        .chain(page2.collection_ids.iter())
        .cloned()
        .collect();
    seen.sort();
    let mut expected: Vec<String> = subcollections.iter().map(|(id, _)| id.clone()).collect();
    expected.sort();
    assert_eq!(seen, expected, "AC-03: every subcollection ID must appear exactly once across pages");
}

/// AC-04: `patients/p-204` has subcollection `visits`; a DIFFERENT document
/// `patients/p-205` also has its own separate `visits` subcollection. The
/// call for `p-204` returns `visits` exactly once, never conflated with
/// `p-205`'s own `visits`.
///
/// @driving_port @real-io @US-01 @AC-04
#[tokio::test]
async fn subcollections_are_scoped_to_their_own_parent_document_never_conflated_with_a_sibling() {
    let ctx = AgentModeWriteStreamingContext::new("alc01-sibling-scoping").await;

    seed_documents(&ctx, "patients/p-204", &[("visits".to_string(), "v-1")]).await;
    seed_documents(&ctx, "patients/p-205", &[("visits".to_string(), "v-1")]).await;

    let parent = nested_parent(&ctx, "patients/p-204");
    let response = list_collection_ids(&ctx, &parent, 10, "")
        .await
        .expect("AC-04: ListCollectionIds must succeed");

    let occurrences = response.collection_ids.iter().filter(|c| c.as_str() == "visits").count();
    assert_eq!(
        occurrences, 1,
        "AC-04: 'visits' must appear exactly once for p-204, never conflated with p-205's own 'visits'"
    );
}
