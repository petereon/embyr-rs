//! BG01 (Slice 01, Walking Skeleton, US-01) — Alex Resolves a Batch of
//! Document References in One Round Trip.
//!
//! Acceptance criteria verified here (slice-01-batch-fetch.md):
//!   AC-01-01: a batch of the caller's own documents, spanning two
//!             collections, all resolve `found` in one round trip.
//!   AC-01-02: a document that no longer exists resolves `missing`,
//!             alongside `found` results for the rest of the batch — never
//!             an error.
//!   AC-01-03: a document the caller is not authorized to read resolves
//!             `missing` for THAT document only (ADR-042 — per-document
//!             denial, batch never aborted); every other document in the
//!             batch resolves normally.
//!   AC-01-04: a collection with no access rule defined is unaffected —
//!             resolves unrestricted, exactly as `GetDocument` already does.
//!   AC-01-05: an empty `documents` list is rejected `InvalidArgument`
//!             before any per-document work begins.
//!   AC-01-06: a batch mixing document names from two different projects is
//!             rejected `InvalidArgument`.
//!   DDD-BGD-14: a batch exceeding 1000 documents is rejected
//!             `InvalidArgument` before any per-document work begins
//!             (DESIGN-added scope, orchestrator-confirmed).
//!
//! Driving port: gRPC :8080 `BatchGetDocuments` (via `SecurityRulesFullContext`
//! — the same `embyr_server::start_test_server` composition root the
//! existing regression suite uses, Pillar 3).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    batch_get_documents, create_document, delete_document, mint_client_identity_token, now_unix,
    string_field, BatchItem, SecurityRulesFullContext,
};
use ed25519_dalek::SigningKey;
use rand_core::OsRng;
use std::collections::HashMap;

fn owner_fields(owner: &str) -> HashMap<String, embyr_proto::firestore::Value> {
    HashMap::from([("owner_id".to_string(), string_field(owner))])
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-01-01: a batch of the caller's own documents across two collections all
// resolve, found, in one round trip (WALKING SKELETON — happy path)
// ─────────────────────────────────────────────────────────────────────────────

/// AC-01-01
///
/// @walking_skeleton @driving_port @real-io @US-01 @AC-01-01
#[tokio::test]
async fn a_batch_of_the_callers_own_documents_across_two_collections_all_resolve_found() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-bg01-happy").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule(
        "trip_entries",
        "request.auth.uid == resource.data.owner_id",
    )
    .await;
    ctx.seed_access_rule("expenses", "request.auth.uid == resource.data.owner_id")
        .await;

    let marias_token =
        mint_client_identity_token(&signing_key, "maria-santos", &ctx.project_id, now_unix() + 3600);

    let mut names = Vec::new();
    for i in 0..3 {
        let doc = create_document(
            &ctx,
            "trip_entries",
            &format!("trip-{i}"),
            owner_fields("maria-santos"),
            Some(&marias_token),
        )
        .await
        .expect("create trip entry")
        .into_inner();
        names.push(doc.name);
    }
    for i in 0..2 {
        let doc = create_document(
            &ctx,
            "expenses",
            &format!("expense-{i}"),
            owner_fields("maria-santos"),
            Some(&marias_token),
        )
        .await
        .expect("create expense")
        .into_inner();
        names.push(doc.name);
    }

    let items = batch_get_documents(&ctx, &names, Some(&marias_token))
        .await
        .expect("AC-01-01: batch of the caller's own documents must succeed");

    assert_eq!(
        items.len(),
        5,
        "AC-01-01: every requested document must produce exactly one response item"
    );
    for name in &names {
        assert!(
            items.contains(&BatchItem::Found(name.clone())),
            "AC-01-01: {name} must resolve `found`"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-01-02: a document that no longer exists resolves `missing`, alongside
// `found` results for the rest of the batch — never an error
// ─────────────────────────────────────────────────────────────────────────────

/// AC-01-02
///
/// @driving_port @real-io @US-01 @AC-01-02
#[tokio::test]
async fn a_document_that_no_longer_exists_resolves_missing_alongside_found_results() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-bg01-missing").await;

    let mut names = Vec::new();
    for i in 0..4 {
        let doc = create_document(
            &ctx,
            "daily_logs",
            &format!("log-{i}"),
            HashMap::new(),
            None,
        )
        .await
        .expect("create daily log")
        .into_inner();
        names.push(doc.name);
    }
    let deleted_name = names[3].clone();
    delete_document(&ctx, &deleted_name, None)
        .await
        .expect("delete the fourth daily log");

    let items = batch_get_documents(&ctx, &names, None)
        .await
        .expect("AC-01-02: a batch with one deleted document must still succeed, never error");

    assert_eq!(items.len(), 4);
    assert_eq!(
        items.iter().filter(|i| matches!(i, BatchItem::Found(_))).count(),
        3,
        "AC-01-02: the 3 still-existing documents must resolve `found`"
    );
    assert!(
        items.contains(&BatchItem::Missing(deleted_name)),
        "AC-01-02: the deleted document must resolve `missing`"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-01-03: a document the caller is not authorized to read resolves
// `missing` for THAT document only — ADR-042, the primary decision of this
// feature. The batch is never aborted; every other document resolves
// normally.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-01-03 (ADR-042)
///
/// @error @driving_port @real-io @US-01 @AC-01-03 @security-regression
#[tokio::test]
async fn a_document_the_caller_is_not_authorized_to_read_resolves_missing_without_affecting_the_rest_of_the_batch(
) {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-bg01-denied").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule(
        "journal_entries",
        "request.auth.uid == resource.data.owner_id",
    )
    .await;

    let marias_token =
        mint_client_identity_token(&signing_key, "maria-santos", &ctx.project_id, now_unix() + 3600);
    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    let maria_doc = create_document(
        &ctx,
        "journal_entries",
        "maria-entry",
        owner_fields("maria-santos"),
        Some(&marias_token),
    )
    .await
    .expect("create Maria's document")
    .into_inner();

    let mut dana_names = Vec::new();
    for i in 0..2 {
        let doc = create_document(
            &ctx,
            "journal_entries",
            &format!("dana-entry-{i}"),
            owner_fields("dana-kim"),
            Some(&danas_token),
        )
        .await
        .expect("create Dana's document")
        .into_inner();
        dana_names.push(doc.name);
    }

    let mut request_names = dana_names.clone();
    request_names.push(maria_doc.name.clone());

    let items = batch_get_documents(&ctx, &request_names, Some(&danas_token))
        .await
        .expect(
            "AC-01-03: the batch must succeed as a whole — a single denied \
             document must never abort it (ADR-042)",
        );

    assert_eq!(items.len(), 3);
    for name in &dana_names {
        assert!(
            items.contains(&BatchItem::Found(name.clone())),
            "AC-01-03: Dana's own document {name} must still resolve `found`"
        );
    }
    assert!(
        items.contains(&BatchItem::Missing(maria_doc.name)),
        "AC-01-03: Maria's document must resolve `missing` for Dana, \
         indistinguishable from a non-existent document"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-01-04: a collection with no access rule defined is unaffected —
// resolves unrestricted, exactly as `GetDocument` already does
// ─────────────────────────────────────────────────────────────────────────────

/// AC-01-04
///
/// @driving_port @real-io @US-01 @AC-01-04
#[tokio::test]
async fn a_collection_with_no_access_rule_defined_resolves_every_document_unrestricted() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-bg01-norule").await;

    let mut names = Vec::new();
    for i in 0..2 {
        let doc = create_document(
            &ctx,
            "daily_logs",
            &format!("unruled-{i}"),
            HashMap::new(),
            None,
        )
        .await
        .expect("create daily log")
        .into_inner();
        names.push(doc.name);
    }

    // Anonymous caller (no client identity token) — a ruleless collection
    // must resolve unrestricted for ANY caller, mirroring GetDocument.
    let items = batch_get_documents(&ctx, &names, None)
        .await
        .expect("AC-01-04: an unruled collection must resolve unrestricted");

    assert_eq!(items.len(), 2);
    for name in &names {
        assert!(items.contains(&BatchItem::Found(name.clone())));
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-01-05: an empty `documents` list is rejected before any work begins
// ─────────────────────────────────────────────────────────────────────────────

/// AC-01-05
///
/// @error @driving_port @real-io @US-01 @AC-01-05
#[tokio::test]
async fn an_empty_batch_request_is_rejected_before_any_work_begins() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-bg01-empty").await;

    let result = batch_get_documents(&ctx, &[], None).await;

    let err = result.expect_err("AC-01-05: an empty documents list must be rejected");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-01-06: a batch mixing document names from two different projects is
// rejected before any per-document work begins
// ─────────────────────────────────────────────────────────────────────────────

/// AC-01-06
///
/// @error @driving_port @real-io @US-01 @AC-01-06
#[tokio::test]
async fn a_batch_mixing_documents_from_two_different_projects_is_rejected() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-bg01-crossproj").await;

    let doc = create_document(&ctx, "daily_logs", "own-doc", HashMap::new(), None)
        .await
        .expect("create own daily log")
        .into_inner();

    let foreign_name =
        "projects/some-other-project/databases/(default)/documents/daily_logs/foreign-doc"
            .to_string();

    let result = batch_get_documents(&ctx, &[doc.name, foreign_name], None).await;

    let err = result.expect_err("AC-01-06: a cross-project batch must be rejected");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
}

// ─────────────────────────────────────────────────────────────────────────────
// DDD-BGD-14: a batch exceeding 1000 documents is rejected before any
// per-document work begins (DESIGN-added scope, orchestrator-confirmed)
// ─────────────────────────────────────────────────────────────────────────────

/// DDD-BGD-14
///
/// @error @driving_port @real-io @US-01 @DDD-BGD-14
#[tokio::test]
async fn a_batch_exceeding_one_thousand_documents_is_rejected() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-bg01-cap").await;

    let names: Vec<String> = (0..1001)
        .map(|i| {
            format!(
                "projects/{}/databases/(default)/documents/daily_logs/doc-{i}",
                ctx.project_id
            )
        })
        .collect();

    let result = batch_get_documents(&ctx, &names, None).await;

    let err = result.expect_err("DDD-BGD-14: a batch over 1000 documents must be rejected");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
}
