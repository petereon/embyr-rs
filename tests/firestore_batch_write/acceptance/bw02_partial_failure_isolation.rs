//! BW02 (Slice 02, US-02) — A Failing Write in Alex's Bulk Import Doesn't
//! Block Its Siblings.
//!
//! Acceptance criteria verified here (slice-02-partial-failure-isolation.md):
//!   AC-02-01: a precondition-violating write never rolls back, blocks, or
//!             otherwise affects any sibling write in the same batch.
//!   AC-02-02: a failing write's `status[i]` is non-null with a specific,
//!             actionable code/message.
//!   AC-02-03: a batch where every write fails still returns a normal
//!             `BatchWriteResponse`, no top-level RPC error.
//!   AC-02-04: failure position (first, middle, last) does not change
//!             isolation behavior for sibling writes.
//!   AC-02-05: a failing write's `write_results[i]` entry is always present
//!             (an empty placeholder), never absent.
//!
//! REGRESSION-RISK NOTE (per slice-02's own carried recommendation): the
//! first scenario below is the one most likely to catch a future refactor
//! that accidentally collapses the per-write `begin_transaction`+
//! `commit_transaction` loop in `handle_batch_write` back into a single
//! `commit_transaction` call over the whole batch — which would silently
//! reintroduce `Commit`'s own all-or-nothing semantics and make this test
//! fail (write 1/3 would roll back alongside write 2's own precondition
//! failure).
//!
//! Driving port: gRPC :8080 `BatchWrite` (via `SecurityRulesFullContext`).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    batch_write, create_document, string_field, update_write, update_write_requiring_not_exists,
    SecurityRulesFullContext,
};

// ─────────────────────────────────────────────────────────────────────────────
// AC-02-01/AC-02-02: a precondition-violating write fails without affecting
// its siblings; the failing write's own status is non-null and specific.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-02-01, AC-02-02
///
/// @driving_port @real-io @US-02 @AC-02-01 @AC-02-02
#[tokio::test]
async fn a_precondition_violating_write_fails_without_affecting_its_siblings() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-bw02-isolation").await;

    // An already-existing trip_entries document (a prior partial import).
    let existing = create_document(
        &ctx,
        "trip_entries",
        "already-imported",
        std::collections::HashMap::from([("owner_id".to_string(), string_field("maria-santos"))]),
        None,
    )
    .await
    .expect("seed already-existing document")
    .into_inner();

    let new_name_0 = format!(
        "projects/{}/databases/(default)/documents/trip_entries/trip-0",
        ctx.project_id
    );
    let new_name_2 = format!(
        "projects/{}/databases/(default)/documents/trip_entries/trip-2",
        ctx.project_id
    );

    let writes = vec![
        update_write(
            &new_name_0,
            std::collections::HashMap::from([(
                "owner_id".to_string(),
                string_field("maria-santos"),
            )]),
        ),
        // Duplicate of the already-existing document — violates
        // `current_document.exists = false`.
        update_write_requiring_not_exists(
            &existing.name,
            std::collections::HashMap::from([(
                "owner_id".to_string(),
                string_field("maria-santos"),
            )]),
        ),
        update_write(
            &new_name_2,
            std::collections::HashMap::from([(
                "owner_id".to_string(),
                string_field("maria-santos"),
            )]),
        ),
    ];

    let response = batch_write(&ctx, writes, None)
        .await
        .expect("AC-02-03: BatchWrite itself must not error, even with a failing write inside");

    assert_eq!(response.status.len(), 3);
    assert_eq!(
        response.status[0].code, 0,
        "AC-02-01: write 1 (before the failure) must commit unaffected"
    );
    assert_ne!(
        response.status[1].code, 0,
        "AC-02-02: the duplicate write's own status must be non-null"
    );
    assert!(
        !response.status[1].message.is_empty(),
        "AC-02-02: the failing write's status must carry an actionable message"
    );
    assert_eq!(
        response.status[2].code, 0,
        "AC-02-01: write 3 (after the failure) must commit unaffected"
    );

    for name in [&new_name_0, &new_name_2] {
        ctx.get_document(name, None).await.unwrap_or_else(|_| {
            panic!("AC-02-01: sibling write {name} must be readable via GetDocument")
        });
    }

    let unchanged = ctx
        .get_document(&existing.name, None)
        .await
        .expect("AC-02-01: the already-existing document must be unaffected")
        .into_inner();
    assert_eq!(
        unchanged.fields, existing.fields,
        "AC-02-01: the already-existing document must be unchanged by the failed duplicate write"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-02-03: the degenerate all-fail case — no top-level RPC error, every
// status entry non-null, no document modified.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-02-03
///
/// @driving_port @real-io @US-02 @AC-02-03
#[tokio::test]
async fn no_top_level_error_is_returned_even_when_every_write_in_the_batch_fails() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-bw02-allfail").await;

    let mut existing_docs = Vec::new();
    for i in 0..3 {
        let doc = create_document(
            &ctx,
            "trip_entries",
            &format!("already-imported-{i}"),
            std::collections::HashMap::from([(
                "owner_id".to_string(),
                string_field("maria-santos"),
            )]),
            None,
        )
        .await
        .expect("seed already-existing document")
        .into_inner();
        existing_docs.push(doc);
    }

    let writes = existing_docs
        .iter()
        .map(|doc| {
            update_write_requiring_not_exists(
                &doc.name,
                std::collections::HashMap::from([(
                    "owner_id".to_string(),
                    string_field("maria-santos"),
                )]),
            )
        })
        .collect();

    let response = batch_write(&ctx, writes, None)
        .await
        .expect("AC-02-03: the RPC call itself must succeed even when every write fails");

    assert_eq!(response.status.len(), 3);
    for status in &response.status {
        assert_ne!(status.code, 0, "AC-02-03: every status entry must be non-null");
    }

    for doc in &existing_docs {
        let unchanged = ctx
            .get_document(&doc.name, None)
            .await
            .expect("document must still be readable")
            .into_inner();
        assert_eq!(
            unchanged.fields, doc.fields,
            "AC-02-03: no document may be modified when every write fails"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-02-04: failure position within the batch does not affect sibling
// isolation — the FIRST write fails, the remaining two (not middle/last)
// succeed.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-02-04
///
/// @driving_port @real-io @US-02 @AC-02-04
#[tokio::test]
async fn failure_position_within_the_batch_does_not_affect_sibling_isolation() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-bw02-position").await;

    let existing = create_document(
        &ctx,
        "trip_entries",
        "already-imported",
        std::collections::HashMap::from([("owner_id".to_string(), string_field("maria-santos"))]),
        None,
    )
    .await
    .expect("seed already-existing document")
    .into_inner();

    let name_1 = format!(
        "projects/{}/databases/(default)/documents/trip_entries/trip-1",
        ctx.project_id
    );
    let name_2 = format!(
        "projects/{}/databases/(default)/documents/trip_entries/trip-2",
        ctx.project_id
    );

    let writes = vec![
        // FIRST write fails.
        update_write_requiring_not_exists(
            &existing.name,
            std::collections::HashMap::from([(
                "owner_id".to_string(),
                string_field("maria-santos"),
            )]),
        ),
        update_write(
            &name_1,
            std::collections::HashMap::from([(
                "owner_id".to_string(),
                string_field("maria-santos"),
            )]),
        ),
        update_write(
            &name_2,
            std::collections::HashMap::from([(
                "owner_id".to_string(),
                string_field("maria-santos"),
            )]),
        ),
    ];

    let response = batch_write(&ctx, writes, None)
        .await
        .expect("BatchWrite must not error");

    assert_ne!(
        response.status[0].code, 0,
        "AC-02-04: the first write's own failure must be reported"
    );
    assert_eq!(
        response.status[1].code, 0,
        "AC-02-04: write 2 must commit unaffected by write 1's own failure"
    );
    assert_eq!(
        response.status[2].code, 0,
        "AC-02-04: write 3 must commit unaffected by write 1's own failure"
    );

    for name in [&name_1, &name_2] {
        ctx.get_document(name, None)
            .await
            .unwrap_or_else(|_| panic!("{name} must be readable via GetDocument"));
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AC-02-05: a failed write's write_results[i] entry is always present (an
// empty placeholder), never absent — the positional-alignment invariant
// established in US-01 still holds under failure.
// ─────────────────────────────────────────────────────────────────────────────

/// AC-02-05
///
/// @driving_port @real-io @US-02 @AC-02-05
#[tokio::test]
async fn a_failed_writes_write_results_entry_is_an_empty_placeholder_not_absent() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-bw02-placeholder").await;

    let existing = create_document(
        &ctx,
        "trip_entries",
        "already-imported",
        std::collections::HashMap::from([("owner_id".to_string(), string_field("maria-santos"))]),
        None,
    )
    .await
    .expect("seed already-existing document")
    .into_inner();

    let name_0 = format!(
        "projects/{}/databases/(default)/documents/trip_entries/trip-0",
        ctx.project_id
    );

    let writes = vec![
        update_write(
            &name_0,
            std::collections::HashMap::from([(
                "owner_id".to_string(),
                string_field("maria-santos"),
            )]),
        ),
        update_write_requiring_not_exists(
            &existing.name,
            std::collections::HashMap::from([(
                "owner_id".to_string(),
                string_field("maria-santos"),
            )]),
        ),
    ];

    let response = batch_write(&ctx, writes, None)
        .await
        .expect("BatchWrite must not error");

    assert_eq!(
        response.write_results.len(),
        2,
        "AC-02-05: write_results must preserve one entry per input write, even under failure"
    );
    assert_ne!(response.status[1].code, 0, "write 2 must have failed");
    assert_eq!(
        response.write_results[1].update_time, None,
        "AC-02-05: a failed write's write_results[i] must be an empty placeholder"
    );
    assert!(
        response.write_results[1].transform_results.is_empty(),
        "AC-02-05: a failed write's write_results[i] must be an empty placeholder"
    );
}
