//! LD01 (Slice 01, Walking Skeleton, US-01) — Alex Lists the Documents in a
//! Collection Without Writing a Query.
//!
//! Acceptance criteria verified here (slice-01-list-documents.md):
//!   AC-01-01/AC-01-02: an explicit `page_size` returns at most that many
//!             documents per page, `next_page_token` present iff more
//!             remain; presenting a previously-returned token returns the
//!             remaining documents, never repeating or skipping any.
//!   AC-01-03: an empty `collection_id` returns documents from every
//!             collection directly under `parent`, not just one.
//!   AC-01-04: a collection with zero documents returns an empty
//!             `documents` array and an empty `next_page_token`, no error.
//!   AC-01-05: an empty `parent` is rejected with `InvalidArgument`.
//!   AC-01-06: a suspended project's `ListDocumentsRequest` is rejected
//!             with `permission_denied` before any query runs.
//!   Domain Example (default page_size): an unspecified `page_size` uses
//!             the server default (100), returning every document in one
//!             page when the collection is smaller than that.
//!   Explicit task requirement (per-document read-path access-rule
//!             evaluation, mirrors `handle_batch_get_documents`'s own
//!             pattern): a document the caller is not authorized to read
//!             is silently excluded from the listing, never aborting the
//!             whole call.
//!
//! Every scenario below uses a NESTED `parent`
//! (`users/{id}`) to also prove the nested-parent-path-is-correctly-
//! incorporated requirement — `parent`'s own document-path prefix is joined
//! with `collection_id`, unlike the confirmed-buggy agent-side RPC
//! (ADR-050 Findings 1/2), which this feature never routes through.
//!
//! Driving port: gRPC :8080 `ListDocuments` (via `SecurityRulesFullContext`
//! — the same `embyr_server::start_test_server` composition root the
//! existing regression suite uses, Pillar 3).
//!
//! Test Budget: 7 behaviors (paginated single-collection listing;
//! collection_id-empty fan-out; empty-collection no-error; default
//! page_size; empty-parent rejection; suspended-project rejection;
//! per-document access-rule denial) x 2 = 14 max. 7 written (one per
//! behavior, no variation-inflation).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    create_document, list_documents, mint_client_identity_token, nested_parent, now_unix,
    root_parent, string_field, SecurityRulesFullContext,
};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use std::collections::HashMap;

fn owner_fields(owner: &str) -> HashMap<String, embyr_proto::firestore::Value> {
    HashMap::from([("owner_id".to_string(), string_field(owner))])
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-01-01 / AC-01-02: documents in a NESTED collection are listed across
// pages, in order, no repeats or skips (WALKING SKELETON — happy path).
// ─────────────────────────────────────────────────────────────────────────────

/// AC-01-01, AC-01-02
///
/// @walking_skeleton @driving_port @real-io @US-01 @AC-01-01 @AC-01-02
#[tokio::test]
async fn documents_in_a_nested_collection_are_listed_across_pages_with_no_repeats_or_skips() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ld01-paginated").await;

    // Nested collection_path, mirroring the real SDK shape:
    // "users/maria-santos-a1b2/trip_entries" — created via the flat
    // create_document helper (its own collection_id IS the full
    // collection_path, per this codebase's established convention).
    let collection_path = "users/maria-santos-a1b2/trip_entries";
    let mut created_names = Vec::new();
    for doc_id in ["yosemite-2024", "banff-2024", "patagonia-2025"] {
        let doc = create_document(
            &ctx,
            collection_path,
            doc_id,
            owner_fields("maria-santos"),
            None,
        )
        .await
        .expect("create trip entry")
        .into_inner();
        created_names.push(doc.name);
    }

    let parent = nested_parent(&ctx, "users/maria-santos-a1b2");

    let page1 = list_documents(&ctx, &parent, "trip_entries", 2, "", None)
        .await
        .expect("AC-01-01: page 1 must succeed");
    assert_eq!(page1.documents.len(), 2, "AC-01-01: page 1 must contain exactly page_size documents");
    assert!(
        !page1.next_page_token.is_empty(),
        "AC-01-01: next_page_token must be present when more documents remain"
    );

    let page2 = list_documents(&ctx, &parent, "trip_entries", 2, &page1.next_page_token, None)
        .await
        .expect("AC-01-02: page 2 must succeed");
    assert_eq!(page2.documents.len(), 1, "AC-01-02: page 2 must contain the remaining document");
    assert!(
        page2.next_page_token.is_empty(),
        "AC-01-02: next_page_token must be empty on the last page"
    );

    let mut seen: Vec<String> = page1.documents.iter().chain(page2.documents.iter()).map(|d| d.name.clone()).collect();
    seen.sort();
    let mut expected = created_names;
    expected.sort();
    assert_eq!(seen, expected, "AC-01-02: every document must appear exactly once across pages");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-01-03: omitting collection_id lists documents from every collection
// directly under a NESTED parent.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-01-03
///
/// @driving_port @real-io @US-01 @AC-01-03
#[tokio::test]
async fn omitting_collection_id_lists_documents_across_every_collection_under_the_parent() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ld01-fanout").await;

    let trip = create_document(
        &ctx,
        "users/maria-santos-a1b2/trip_entries",
        "yosemite-2024",
        owner_fields("maria-santos"),
        None,
    )
    .await
    .expect("create trip entry")
    .into_inner();
    let payment = create_document(
        &ctx,
        "users/maria-santos-a1b2/payment_methods",
        "visa-ending-1234",
        owner_fields("maria-santos"),
        None,
    )
    .await
    .expect("create payment method")
    .into_inner();

    let parent = nested_parent(&ctx, "users/maria-santos-a1b2");
    let response = list_documents(&ctx, &parent, "", 100, "", None)
        .await
        .expect("AC-01-03: empty collection_id must succeed");

    let names: Vec<&str> = response.documents.iter().map(|d| d.name.as_str()).collect();
    assert!(names.contains(&trip.name.as_str()), "AC-01-03: must include the trip_entries document");
    assert!(names.contains(&payment.name.as_str()), "AC-01-03: must include the payment_methods document");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-01-04: a collection with zero documents returns an empty result, no
// error.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-01-04
///
/// @driving_port @real-io @US-01 @AC-01-04
#[tokio::test]
async fn an_empty_collection_returns_no_documents_and_no_error() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ld01-empty").await;

    let parent = nested_parent(&ctx, "users/brand-new-user");
    let response = list_documents(&ctx, &parent, "trip_entries", 10, "", None)
        .await
        .expect("AC-01-04: an empty collection must not error");

    assert!(response.documents.is_empty(), "AC-01-04: documents must be empty");
    assert!(response.next_page_token.is_empty(), "AC-01-04: next_page_token must be empty");
}

// ─────────────────────────────────────────────────────────────────────────────
// Domain Example: an unspecified page_size uses the server default (100),
// returning every document in one page.
// ─────────────────────────────────────────────────────────────────────────────

/// Domain Example (US-01) — default page_size
///
/// @driving_port @real-io @US-01
#[tokio::test]
async fn the_default_page_size_returns_all_documents_in_a_single_page_when_unspecified() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ld01-default-size").await;

    for doc_id in ["a", "b", "c"] {
        create_document(&ctx, "trip_entries_default", doc_id, owner_fields("maria-santos"), None)
            .await
            .expect("create doc");
    }

    let parent = root_parent(&ctx);
    // page_size = 0 (proto3 default int32) — "not specified" per SPEC.md.
    let response = list_documents(&ctx, &parent, "trip_entries_default", 0, "", None)
        .await
        .expect("default page_size must succeed");

    assert_eq!(response.documents.len(), 3, "all 3 documents must fit in the default page size (100)");
    assert!(response.next_page_token.is_empty(), "no next_page_token when everything fits in one page");
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-01-05: an empty parent is rejected with InvalidArgument.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-01-05
///
/// @error @driving_port @real-io @US-01 @AC-01-05
#[tokio::test]
async fn an_empty_parent_is_rejected_with_invalid_argument() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ld01-empty-parent").await;

    let result = list_documents(&ctx, "", "trip_entries", 10, "", None).await;

    let err = result.expect_err("AC-01-05: an empty parent must be rejected");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-01-06: a suspended project's ListDocumentsRequest is rejected before
// any query runs, matching every other RPC.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-01-06
///
/// @error @driving_port @real-io @US-01 @AC-01-06
#[tokio::test]
async fn a_suspended_projects_list_documents_request_is_rejected() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ld01-suspended").await;

    sqlx::query("UPDATE projects SET status = 'suspended' WHERE id = $1")
        .bind(&ctx.project_id)
        .execute(&ctx.sys_pool)
        .await
        .expect("suspend project");

    let parent = root_parent(&ctx);
    let result = list_documents(&ctx, &parent, "trip_entries", 10, "", None).await;

    let err = result.expect_err("AC-01-06: a suspended project's ListDocuments must be rejected");
    assert_eq!(err.code(), tonic::Code::PermissionDenied);
}

// ─────────────────────────────────────────────────────────────────────────────
// Explicit task requirement: per-document read-path access-rule evaluation
// (mirrors handle_batch_get_documents's own pattern) — a document the
// caller is not authorized to read is silently excluded, never aborting
// the whole call.
// ─────────────────────────────────────────────────────────────────────────────

/// Per-document access-rule denial
///
/// @driving_port @real-io @US-01
#[tokio::test]
async fn a_document_the_caller_is_not_authorized_to_read_is_excluded_from_the_listing() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ld01-denied").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule("trip_entries", "request.auth.uid == resource.data.owner_id")
        .await;

    let marias_token =
        mint_client_identity_token(&signing_key, "maria-santos", &ctx.project_id, now_unix() + 3600);

    let marias_doc = create_document(
        &ctx,
        "trip_entries",
        "marias-trip",
        owner_fields("maria-santos"),
        Some(&marias_token),
    )
    .await
    .expect("create maria's document")
    .into_inner();
    let danas_doc = create_document(
        &ctx,
        "trip_entries",
        "danas-trip",
        owner_fields("dana-kim"),
        Some(&marias_token),
    )
    .await
    .expect("create dana's document")
    .into_inner();

    let parent = root_parent(&ctx);
    let response = list_documents(&ctx, &parent, "trip_entries", 100, "", Some(&marias_token))
        .await
        .expect("a denied document must not abort the whole call");

    let names: Vec<&str> = response.documents.iter().map(|d| d.name.as_str()).collect();
    assert!(names.contains(&marias_doc.name.as_str()), "maria's own document must be included");
    assert!(!names.contains(&danas_doc.name.as_str()), "dana's document must be excluded, not visible to maria");
}
