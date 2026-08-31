//! FT03 (Slice 03, US-03) — Alex's Array Fields Update Without Duplicates.
//! Final slice of firestore-field-transforms: after this, all 6 real
//! transform kinds (`serverTimestamp`, `increment`/`maximum`/`minimum`,
//! `appendMissingElements`/`removeAllFromArray`) are real.
//!
//! Acceptance criteria verified here (slice-03-array-membership.md):
//!   AC-03-01: `arrayUnion` adds elements not already present (structural
//!             equality via `FieldValue::PartialEq`).
//!   AC-03-02: `arrayUnion` is idempotent — re-adding an already-present
//!             element produces no duplicate.
//!   AC-03-03: `arrayRemove` removes ALL matching occurrences, not just the
//!             first.
//!   AC-03-04: `arrayUnion` against a missing field creates it as the
//!             incoming array.
//!   AC-03-05: `arrayRemove` against a missing field is a no-op and does NOT
//!             create the field.
//!   (ADR-053 Escalation 2): array-kind transforms NEVER populate
//!             `WriteResult.transform_results` — distinct from Slice 01/02.
//!
//! Driving port: gRPC :8080 `Commit` (via `SecurityRulesFullContext`).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    append_missing_elements_transform, begin_transaction, commit_writes, create_document,
    remove_all_from_array_transform, string_array_field, string_field, transform_write,
    SecurityRulesFullContext,
};

fn string_array_values(doc: &embyr_proto::firestore::Document, key: &str) -> Vec<String> {
    match doc.fields.get(key).and_then(|v| v.value_type.clone()) {
        Some(embyr_proto::firestore::value::ValueType::ArrayValue(arr)) => arr
            .values
            .into_iter()
            .map(|v| match v.value_type {
                Some(embyr_proto::firestore::value::ValueType::StringValue(s)) => s,
                other => panic!("expected StringValue array element, got {other:?}"),
            })
            .collect(),
        other => panic!("expected ArrayValue for {key}, got {other:?}"),
    }
}

fn document_name(ctx: &SecurityRulesFullContext, document_id: &str) -> String {
    format!(
        "projects/{}/databases/(default)/documents/trip_entries/{document_id}",
        ctx.project_id
    )
}

/// AC-03-01/AC-03-02 — a single `arrayUnion` call re-adding an already-present
/// element AND a genuinely-new element in the same transform: only the new
/// element is appended, in order, no duplicate for the already-present one.
///
/// @driving_port @real-io @US-03 @AC-03-01 @AC-03-02
#[tokio::test]
async fn array_union_adds_only_genuinely_new_elements_in_order() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ft03-union-existing").await;
    create_document(
        &ctx,
        "trip_entries",
        "kilimanjaro-trek",
        std::collections::HashMap::from([(
            "sharedWithUserIds".to_string(),
            string_array_field(&["u-diego"]),
        )]),
        None,
    )
    .await
    .expect("seed document must be created");
    let name = document_name(&ctx, "kilimanjaro-trek");

    let txn = begin_transaction(&ctx).await;
    commit_writes(
        &ctx,
        vec![transform_write(
            &name,
            vec![append_missing_elements_transform(
                "sharedWithUserIds",
                vec![string_field("u-diego"), string_field("u-priya")],
            )],
        )],
        txn,
        None,
    )
    .await
    .expect("AC-03-01/02: arrayUnion must succeed");

    let doc = ctx.get_document(&name, None).await.unwrap().into_inner();
    assert_eq!(
        string_array_values(&doc, "sharedWithUserIds"),
        vec!["u-diego".to_string(), "u-priya".to_string()],
        "AC-03-01/02: u-diego stays (no duplicate), u-priya appended in order"
    );
}

/// AC-03-04 — `arrayUnion` against a missing field creates it as the
/// incoming array.
///
/// @driving_port @real-io @US-03 @AC-03-04
#[tokio::test]
async fn array_union_creates_missing_field_as_incoming_array() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ft03-union-missing").await;
    create_document(&ctx, "trip_entries", "new-trip", std::collections::HashMap::new(), None)
        .await
        .expect("seed document must be created");
    let name = document_name(&ctx, "new-trip");

    let txn = begin_transaction(&ctx).await;
    commit_writes(
        &ctx,
        vec![transform_write(
            &name,
            vec![append_missing_elements_transform(
                "sharedWithUserIds",
                vec![string_field("u-diego")],
            )],
        )],
        txn,
        None,
    )
    .await
    .expect("AC-03-04: arrayUnion against a missing field must succeed");

    let doc = ctx.get_document(&name, None).await.unwrap().into_inner();
    assert_eq!(
        string_array_values(&doc, "sharedWithUserIds"),
        vec!["u-diego".to_string()],
        "AC-03-04: missing field created as the incoming array"
    );
}

/// `appendMissingElements` against a non-array existing value is rejected
/// (ADR-052 § Decision 5a residual, by direct analogy to the numeric
/// non-numeric-target rule).
///
/// @error @driving_port @real-io @US-03
#[tokio::test]
async fn array_union_against_non_array_existing_value_is_rejected() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ft03-union-nonarray").await;
    create_document(
        &ctx,
        "trip_entries",
        "kilimanjaro-trek",
        std::collections::HashMap::from([(
            "sharedWithUserIds".to_string(),
            string_field("not-an-array"),
        )]),
        None,
    )
    .await
    .expect("seed document must be created");
    let name = document_name(&ctx, "kilimanjaro-trek");

    let txn = begin_transaction(&ctx).await;
    let result = commit_writes(
        &ctx,
        vec![transform_write(
            &name,
            vec![append_missing_elements_transform(
                "sharedWithUserIds",
                vec![string_field("u-diego")],
            )],
        )],
        txn,
        None,
    )
    .await;

    let err = result.expect_err("arrayUnion against a non-array target must be rejected");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);

    let doc = ctx.get_document(&name, None).await.unwrap().into_inner();
    assert_eq!(
        doc.fields.get("sharedWithUserIds").and_then(|v| v.value_type.clone()),
        Some(embyr_proto::firestore::value::ValueType::StringValue(
            "not-an-array".to_string()
        )),
        "the document's field must be unchanged"
    );
}

/// AC-03-03 — `arrayRemove` removes ALL matching occurrences of a given
/// element, not just the first (a pre-existing duplicate from unrelated
/// legacy data).
///
/// @driving_port @real-io @US-03 @AC-03-03
#[tokio::test]
async fn array_remove_removes_all_matching_occurrences() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ft03-remove-duplicates").await;
    create_document(
        &ctx,
        "trip_entries",
        "kilimanjaro-trek",
        std::collections::HashMap::from([(
            "sharedWithUserIds".to_string(),
            string_array_field(&["u-diego", "u-priya", "u-diego"]),
        )]),
        None,
    )
    .await
    .expect("seed document must be created");
    let name = document_name(&ctx, "kilimanjaro-trek");

    let txn = begin_transaction(&ctx).await;
    commit_writes(
        &ctx,
        vec![transform_write(
            &name,
            vec![remove_all_from_array_transform(
                "sharedWithUserIds",
                vec![string_field("u-diego")],
            )],
        )],
        txn,
        None,
    )
    .await
    .expect("AC-03-03: arrayRemove must succeed");

    let doc = ctx.get_document(&name, None).await.unwrap().into_inner();
    assert_eq!(
        string_array_values(&doc, "sharedWithUserIds"),
        vec!["u-priya".to_string()],
        "AC-03-03: both u-diego occurrences removed, not just the first"
    );
}

/// AC-03-05 — `arrayRemove` against a missing field is a no-op and does NOT
/// create the field.
///
/// @driving_port @real-io @US-03 @AC-03-05
#[tokio::test]
async fn array_remove_against_missing_field_is_a_no_op() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ft03-remove-missing").await;
    create_document(&ctx, "trip_entries", "new-trip", std::collections::HashMap::new(), None)
        .await
        .expect("seed document must be created");
    let name = document_name(&ctx, "new-trip");

    let txn = begin_transaction(&ctx).await;
    commit_writes(
        &ctx,
        vec![transform_write(
            &name,
            vec![remove_all_from_array_transform(
                "sharedWithUserIds",
                vec![string_field("u-diego")],
            )],
        )],
        txn,
        None,
    )
    .await
    .expect("AC-03-05: arrayRemove against a missing field must succeed as a no-op");

    let doc = ctx.get_document(&name, None).await.unwrap().into_inner();
    assert!(
        doc.fields.get("sharedWithUserIds").is_none(),
        "AC-03-05: a no-op must not create the field"
    );
}

/// `removeAllFromArray` against a non-array existing value is rejected
/// (same residual as `appendMissingElements`).
///
/// @error @driving_port @real-io @US-03
#[tokio::test]
async fn array_remove_against_non_array_existing_value_is_rejected() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ft03-remove-nonarray").await;
    create_document(
        &ctx,
        "trip_entries",
        "kilimanjaro-trek",
        std::collections::HashMap::from([(
            "sharedWithUserIds".to_string(),
            string_field("not-an-array"),
        )]),
        None,
    )
    .await
    .expect("seed document must be created");
    let name = document_name(&ctx, "kilimanjaro-trek");

    let txn = begin_transaction(&ctx).await;
    let result = commit_writes(
        &ctx,
        vec![transform_write(
            &name,
            vec![remove_all_from_array_transform(
                "sharedWithUserIds",
                vec![string_field("u-diego")],
            )],
        )],
        txn,
        None,
    )
    .await;

    let err = result.expect_err("arrayRemove against a non-array target must be rejected");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
}

/// ADR-053 Escalation 2 — neither `appendMissingElements` nor
/// `removeAllFromArray` populates `WriteResult.transform_results`, even
/// though the array actually changed in both cases — distinguishing this
/// slice's behavior from Slice 01/02, which both DID populate it. This is
/// the one thing most likely to get silently inverted by mistake.
///
/// @driving_port @real-io @US-03
#[tokio::test]
async fn array_transforms_never_populate_transform_results() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ft03-no-transform-results").await;
    create_document(
        &ctx,
        "trip_entries",
        "kilimanjaro-trek",
        std::collections::HashMap::from([(
            "sharedWithUserIds".to_string(),
            string_array_field(&["u-diego"]),
        )]),
        None,
    )
    .await
    .expect("seed document must be created");
    let name = document_name(&ctx, "kilimanjaro-trek");

    let txn = begin_transaction(&ctx).await;
    let union_response = commit_writes(
        &ctx,
        vec![transform_write(
            &name,
            vec![append_missing_elements_transform(
                "sharedWithUserIds",
                vec![string_field("u-priya")],
            )],
        )],
        txn,
        None,
    )
    .await
    .expect("arrayUnion must succeed")
    .into_inner();
    assert!(
        union_response.write_results[0].transform_results.is_empty(),
        "appendMissingElements must NEVER populate transform_results, even though the array changed"
    );

    let txn = begin_transaction(&ctx).await;
    let remove_response = commit_writes(
        &ctx,
        vec![transform_write(
            &name,
            vec![remove_all_from_array_transform(
                "sharedWithUserIds",
                vec![string_field("u-priya")],
            )],
        )],
        txn,
        None,
    )
    .await
    .expect("arrayRemove must succeed")
    .into_inner();
    assert!(
        remove_response.write_results[0].transform_results.is_empty(),
        "removeAllFromArray must NEVER populate transform_results, even though the array changed"
    );
}
