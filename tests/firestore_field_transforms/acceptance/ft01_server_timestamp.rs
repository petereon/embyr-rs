//! FT01 (Slice 01, Walking Skeleton, US-01) — Alex's `serverTimestamp()`
//! Actually Persists.
//!
//! Acceptance criteria verified here (slice-01-server-timestamp.md):
//!   AC-01-01: a standalone `serverTimestamp()` transform write persists a
//!             real server-generated timestamp, readable via `GetDocument`.
//!   AC-01-02: a `serverTimestamp()` transform attached to a regular
//!             `update` (via `update_transforms`) persists BOTH the regular
//!             field change and the transformed field in the same write.
//!   AC-01-03: a `serverTimestamp()` transform creates the target field if
//!             it does not already exist (covered by the same standalone
//!             write as AC-01-01 — the target document does not exist yet).
//!   AC-01-04: `WriteResult.transform_results` contains the computed
//!             timestamp value.
//!   AC-01-05: an unsupported `ServerValue` is rejected with
//!             `InvalidArgument`, and the document is left unmodified.
//!
//! Driving port: gRPC :8080 `Commit` (via `SecurityRulesFullContext` — the
//! same `embyr_server::start_test_server` composition root the existing
//! regression suite uses, Pillar 3).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    begin_transaction, commit_writes, create_document, server_timestamp_transform, string_field,
    transform_write, unsupported_server_value_transform, update_write_with_transforms,
    SecurityRulesFullContext,
};

fn timestamp_field(doc: &embyr_proto::firestore::Document, key: &str) -> (i64, i32) {
    let value = doc
        .fields
        .get(key)
        .unwrap_or_else(|| panic!("field {key} must be present"));
    match &value.value_type {
        Some(embyr_proto::firestore::value::ValueType::TimestampValue(ts)) => {
            (ts.seconds, ts.nanos)
        }
        other => panic!("expected TimestampValue for {key}, got {other:?}"),
    }
}

/// AC-01-01, AC-01-03, AC-01-04 — a standalone transform on a document that
/// does not exist yet both creates the document AND persists a real server
/// timestamp, reported back in `WriteResult.transform_results`.
///
/// @walking_skeleton @driving_port @real-io @US-01 @AC-01-01 @AC-01-03 @AC-01-04
#[tokio::test]
async fn a_standalone_server_timestamp_transform_creates_and_persists_a_real_timestamp() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ft01-standalone").await;
    let name = format!(
        "projects/{}/databases/(default)/documents/trip_entries/kilimanjaro-trek",
        ctx.project_id
    );

    let before = chrono::Utc::now().timestamp();
    let txn = begin_transaction(&ctx).await;
    let response = commit_writes(
        &ctx,
        vec![transform_write(&name, vec![server_timestamp_transform("updatedAt")])],
        txn,
        None,
    )
    .await
    .expect("AC-01-01: a standalone serverTimestamp() transform must succeed")
    .into_inner();
    let after = chrono::Utc::now().timestamp();

    assert_eq!(
        response.write_results.len(),
        1,
        "AC-01-01: exactly one WriteResult for the one write"
    );
    let transform_results = &response.write_results[0].transform_results;
    assert_eq!(
        transform_results.len(),
        1,
        "AC-01-04: transform_results must contain exactly one entry"
    );
    let (result_secs, _) = match &transform_results[0].value_type {
        Some(embyr_proto::firestore::value::ValueType::TimestampValue(ts)) => (ts.seconds, ts.nanos),
        other => panic!("AC-01-04: transform_results entry must be a Timestamp, got {other:?}"),
    };
    assert!(
        (before - 2..=after + 5).contains(&result_secs),
        "AC-01-04: transform_results timestamp {result_secs} must be close to commit time [{before}, {after}]"
    );

    let doc = ctx
        .get_document(&name, None)
        .await
        .expect("AC-01-03: the document must now exist")
        .into_inner();
    let (persisted_secs, _) = timestamp_field(&doc, "updatedAt");
    assert!(
        (before - 2..=after + 5).contains(&persisted_secs),
        "AC-01-01/AC-01-03: persisted updatedAt {persisted_secs} must be close to commit time [{before}, {after}]"
    );
}

/// AC-01-02 — a `serverTimestamp()` transform attached to a regular
/// `update` (`update_transforms`) persists BOTH the regular field change
/// AND the transformed field, from the same single write.
///
/// @driving_port @real-io @US-01 @AC-01-02
#[tokio::test]
async fn a_server_timestamp_transform_attached_to_an_update_persists_both_together() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ft01-combined").await;
    create_document(
        &ctx,
        "trip_entries",
        "kilimanjaro-trek",
        std::collections::HashMap::from([("name".to_string(), string_field("Kilimanjaro Trek"))]),
        None,
    )
    .await
    .expect("seed document must be created");
    let name = format!(
        "projects/{}/databases/(default)/documents/trip_entries/kilimanjaro-trek",
        ctx.project_id
    );

    let before = chrono::Utc::now().timestamp();
    let txn = begin_transaction(&ctx).await;
    let write = update_write_with_transforms(
        &name,
        std::collections::HashMap::from([(
            "name".to_string(),
            string_field("Kilimanjaro Trek — Revised Itinerary"),
        )]),
        vec![server_timestamp_transform("updatedAt")],
    );
    commit_writes(&ctx, vec![write], txn, None)
        .await
        .expect("AC-01-02: an update carrying update_transforms must succeed");
    let after = chrono::Utc::now().timestamp();

    let doc = ctx
        .get_document(&name, None)
        .await
        .expect("document must be readable")
        .into_inner();

    assert_eq!(
        doc.fields.get("name").and_then(|v| v.value_type.clone()),
        Some(embyr_proto::firestore::value::ValueType::StringValue(
            "Kilimanjaro Trek — Revised Itinerary".to_string()
        )),
        "AC-01-02: the regular field update must persist"
    );
    let (persisted_secs, _) = timestamp_field(&doc, "updatedAt");
    assert!(
        (before - 2..=after + 5).contains(&persisted_secs),
        "AC-01-02: the attached transform must ALSO persist, in the same write"
    );
}

/// AC-01-05 — an unsupported `ServerValue` is rejected with
/// `InvalidArgument`, and the document is left completely unmodified.
///
/// @error @driving_port @real-io @US-01 @AC-01-05
#[tokio::test]
async fn an_unsupported_server_value_is_rejected_without_modifying_the_document() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ft01-unsupported").await;
    create_document(
        &ctx,
        "trip_entries",
        "kilimanjaro-trek",
        std::collections::HashMap::from([("name".to_string(), string_field("Kilimanjaro Trek"))]),
        None,
    )
    .await
    .expect("seed document must be created");
    let name = format!(
        "projects/{}/databases/(default)/documents/trip_entries/kilimanjaro-trek",
        ctx.project_id
    );

    let txn = begin_transaction(&ctx).await;
    let result = commit_writes(
        &ctx,
        vec![transform_write(&name, vec![unsupported_server_value_transform("updatedAt")])],
        txn,
        None,
    )
    .await;

    let err = result.expect_err("AC-01-05: an unsupported ServerValue must be rejected");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);

    let doc = ctx
        .get_document(&name, None)
        .await
        .expect("document must still be readable")
        .into_inner();
    assert!(
        !doc.fields.contains_key("updatedAt"),
        "AC-01-05: the document must be left unmodified — no updatedAt field"
    );
    assert_eq!(
        doc.fields.get("name").and_then(|v| v.value_type.clone()),
        Some(embyr_proto::firestore::value::ValueType::StringValue(
            "Kilimanjaro Trek".to_string()
        )),
        "AC-01-05: pre-existing fields must be unchanged"
    );
}
