//! BW01 (Slice 01, Walking Skeleton, US-01) — Alex's Bulk Import Succeeds
//! When Every Row Is Well-Formed.
//!
//! Acceptance criteria verified here (slice-01-all-writes-succeed.md):
//!   AC-01-01: N well-formed writes return `write_results`/`status` arrays
//!             of exactly N entries, positionally aligned.
//!   AC-01-02: every well-formed write is applied and readable via
//!             `GetDocument` immediately after the response is returned.
//!   AC-01-03: a successful write's `status[i]` entry is `Status{code: 0}`
//!             (OK) — the wire encoding for SPEC.md's own "null" shorthand
//!             (ADR-048 § Decision 5).
//!   AC-01-04: an empty `BatchWriteRequest.writes` returns an immediate
//!             response with empty `write_results`/`status` arrays, no error.
//!   AC-01-05: a suspended project's `BatchWriteRequest` is rejected with
//!             `permission_denied` before any write is attempted.
//!
//! Driving port: gRPC :8080 `BatchWrite` (via `SecurityRulesFullContext` —
//! the same `embyr_server::start_test_server` composition root the existing
//! regression suite uses, Pillar 3).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{batch_write, string_field, update_write, SecurityRulesFullContext};

// ─────────────────────────────────────────────────────────────────────────────
// AC-01-01/AC-01-02/AC-01-03: a batch of well-formed writes across
// `trip_entries` all succeed independently, positionally aligned, each
// `status[i]` is OK, and every document is readable immediately after
// (WALKING SKELETON — happy path).
// ─────────────────────────────────────────────────────────────────────────────

/// AC-01-01, AC-01-02, AC-01-03
///
/// @walking_skeleton @driving_port @real-io @US-01 @AC-01-01 @AC-01-02 @AC-01-03
#[tokio::test]
async fn a_batch_of_well_formed_writes_all_succeed_independently() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-bw01-happy").await;

    let mut names = Vec::new();
    let mut writes = Vec::new();
    for i in 0..3 {
        let name = format!(
            "projects/{}/databases/(default)/documents/trip_entries/trip-{i}",
            ctx.project_id
        );
        names.push(name.clone());
        writes.push(update_write(
            &name,
            std::collections::HashMap::from([(
                "owner_id".to_string(),
                string_field("maria-santos"),
            )]),
        ));
    }

    let response = batch_write(&ctx, writes, None)
        .await
        .expect("AC-01-01: a batch of well-formed writes must succeed");

    assert_eq!(
        response.write_results.len(),
        3,
        "AC-01-01: write_results must contain exactly one entry per write"
    );
    assert_eq!(
        response.status.len(),
        3,
        "AC-01-01: status must contain exactly one entry per write"
    );
    for status in &response.status {
        assert_eq!(
            status.code, 0,
            "AC-01-03: a successful write's status[i] must be OK (code 0)"
        );
    }

    for name in &names {
        let doc = ctx.get_document(name, None).await.expect(
            "AC-01-02: every well-formed write must be readable via GetDocument immediately after",
        );
        assert_eq!(&doc.into_inner().name, name);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Degenerate single-write batch — behaves identically in outcome to a
// one-write Commit, exactly one populated write_results entry and one OK
// status entry.
// ─────────────────────────────────────────────────────────────────────────────

/// Domain Example 2 (US-01) — single-write "bulk" import
///
/// @driving_port @real-io @US-01
#[tokio::test]
async fn a_single_write_batch_behaves_like_a_degenerate_bulk_import() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-bw01-single").await;

    let name = format!(
        "projects/{}/databases/(default)/documents/trip_entries/solo-trip",
        ctx.project_id
    );
    let write = update_write(
        &name,
        std::collections::HashMap::from([("owner_id".to_string(), string_field("maria-santos"))]),
    );

    let response = batch_write(&ctx, vec![write], None)
        .await
        .expect("a single-write batch must succeed");

    assert_eq!(response.write_results.len(), 1);
    assert_eq!(response.status.len(), 1);
    assert_eq!(response.status[0].code, 0);
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-01-04: an empty batch returns immediately with no error
// ─────────────────────────────────────────────────────────────────────────────

/// AC-01-04
///
/// @driving_port @real-io @US-01 @AC-01-04
#[tokio::test]
async fn an_empty_batch_returns_immediately_with_empty_arrays_and_no_error() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-bw01-empty").await;

    let response = batch_write(&ctx, vec![], None)
        .await
        .expect("AC-01-04: an empty batch must not error");

    assert!(response.write_results.is_empty());
    assert!(response.status.is_empty());
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-01-05: a suspended project's BatchWriteRequest is rejected before any
// write is attempted, matching every other RPC.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-01-05
///
/// @error @driving_port @real-io @US-01 @AC-01-05
#[tokio::test]
async fn a_suspended_projects_batch_write_is_rejected_before_any_write_is_attempted() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-bw01-suspended").await;

    sqlx::query("UPDATE projects SET status = 'suspended' WHERE id = $1")
        .bind(&ctx.project_id)
        .execute(&ctx.sys_pool)
        .await
        .expect("suspend project");

    let name = format!(
        "projects/{}/databases/(default)/documents/trip_entries/should-not-apply",
        ctx.project_id
    );
    let write = update_write(
        &name,
        std::collections::HashMap::from([("owner_id".to_string(), string_field("maria-santos"))]),
    );

    let result = batch_write(&ctx, vec![write], None).await;

    let err = result.expect_err("AC-01-05: a suspended project's BatchWrite must be rejected");
    assert_eq!(err.code(), tonic::Code::PermissionDenied);
}
