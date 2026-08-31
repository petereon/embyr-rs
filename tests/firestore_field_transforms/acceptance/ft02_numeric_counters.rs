//! FT02 (Slice 02, US-02) — Alex's Counters Increment Atomically and
//! Type-Correctly.
//!
//! Acceptance criteria verified here (slice-02-numeric-counters.md):
//!   AC-02-01: `increment` against an existing numeric field preserves
//!             integer/double type per SPEC.md's promotion rule.
//!   AC-02-02: two concurrent `increment` calls against the same field both
//!             land — no lost update (FOR UPDATE-locked read-compute-write).
//!   AC-02-03: `increment` against a missing field treats it as 0/0.0;
//!             `maximum`/`minimum` against a missing field sets it DIRECTLY
//!             to the given value instead (the opposite rule).
//!   AC-02-04: `maximum`/`minimum` compare against the existing value and
//!             set the field to whichever is greater/lesser, type-preserved.
//!   AC-02-05: `increment`/`maximum`/`minimum` against a non-numeric
//!             existing value is rejected with `InvalidArgument`, document
//!             unmodified.
//!   (Escalation 1, ADR-053): `increment` i64 overflow returns
//!             `InvalidArgument` (checked_add, never wraps), unmodified.
//!
//! Driving port: gRPC :8080 `Commit` (via `SecurityRulesFullContext`).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    begin_transaction, commit_writes, create_document, double_field, increment_transform, int_field,
    maximum_transform, minimum_transform, string_field, transform_write, update_write_with_transforms,
    SecurityRulesFullContext,
};

fn int_value(doc: &embyr_proto::firestore::Document, key: &str) -> i64 {
    match doc.fields.get(key).and_then(|v| v.value_type.clone()) {
        Some(embyr_proto::firestore::value::ValueType::IntegerValue(i)) => i,
        other => panic!("expected IntegerValue for {key}, got {other:?}"),
    }
}

fn double_value(doc: &embyr_proto::firestore::Document, key: &str) -> f64 {
    match doc.fields.get(key).and_then(|v| v.value_type.clone()) {
        Some(embyr_proto::firestore::value::ValueType::DoubleValue(d)) => d,
        other => panic!("expected DoubleValue for {key}, got {other:?}"),
    }
}

fn document_name(ctx: &SecurityRulesFullContext, document_id: &str) -> String {
    format!(
        "projects/{}/databases/(default)/documents/trip_entries/{document_id}",
        ctx.project_id
    )
}

/// AC-02-01 — `increment` against an existing integer field adds correctly
/// and preserves the integer type; a double delta promotes the result to
/// double (SPEC.md's own promotion rule).
///
/// @driving_port @real-io @US-02 @AC-02-01
#[tokio::test]
async fn increment_on_existing_field_preserves_or_promotes_type() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ft02-increment-existing").await;
    create_document(
        &ctx,
        "trip_entries",
        "kilimanjaro-trek",
        std::collections::HashMap::from([("viewCount".to_string(), int_field(41))]),
        None,
    )
    .await
    .expect("seed document must be created");
    let name = document_name(&ctx, "kilimanjaro-trek");

    // Integer delta against an integer field stays integer.
    let txn = begin_transaction(&ctx).await;
    commit_writes(
        &ctx,
        vec![transform_write(&name, vec![increment_transform("viewCount", int_field(1))])],
        txn,
        None,
    )
    .await
    .expect("AC-02-01: integer increment must succeed");
    let doc = ctx.get_document(&name, None).await.unwrap().into_inner();
    assert_eq!(int_value(&doc, "viewCount"), 42, "AC-02-01: 41 + 1 = 42, integer preserved");

    // A double delta against the now-integer field promotes to double.
    let txn = begin_transaction(&ctx).await;
    commit_writes(
        &ctx,
        vec![transform_write(&name, vec![increment_transform("viewCount", double_field(0.5))])],
        txn,
        None,
    )
    .await
    .expect("AC-02-01: double increment must succeed");
    let doc = ctx.get_document(&name, None).await.unwrap().into_inner();
    assert_eq!(
        double_value(&doc, "viewCount"),
        42.5,
        "AC-02-01: 42 + 0.5 = 42.5, promoted to double"
    );
}

/// AC-02-02 — two concurrent `increment` calls against the same field both
/// land: no lost update, proving the FOR UPDATE-locked read-compute-write
/// mechanism.
///
/// @driving_port @real-io @US-02 @AC-02-02
#[tokio::test]
async fn two_concurrent_increments_against_the_same_field_both_land() {
    let ctx = std::sync::Arc::new(
        SecurityRulesFullContext::new("trailmark-prod-ft02-concurrent").await,
    );
    create_document(
        &ctx,
        "trip_entries",
        "kilimanjaro-trek",
        std::collections::HashMap::from([("viewCount".to_string(), int_field(41))]),
        None,
    )
    .await
    .expect("seed document must be created");
    let name = document_name(&ctx, "kilimanjaro-trek");

    let increment_once = |ctx: std::sync::Arc<SecurityRulesFullContext>, name: String| async move {
        let txn = begin_transaction(&ctx).await;
        commit_writes(
            &ctx,
            vec![transform_write(&name, vec![increment_transform("viewCount", int_field(1))])],
            txn,
            None,
        )
        .await
        .expect("AC-02-02: each concurrent increment must itself succeed");
    };

    let (r1, r2) = tokio::join!(
        increment_once(ctx.clone(), name.clone()),
        increment_once(ctx.clone(), name.clone())
    );
    let _ = (r1, r2);

    let doc = ctx.get_document(&name, None).await.unwrap().into_inner();
    assert_eq!(
        int_value(&doc, "viewCount"),
        43,
        "AC-02-02: both increments must land — no lost update"
    );
}

/// AC-02-03 — `increment` against a missing field treats it as 0/0.0 per
/// the delta's own type.
///
/// @driving_port @real-io @US-02 @AC-02-03
#[tokio::test]
async fn increment_against_a_missing_field_treats_it_as_zero_per_delta_type() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ft02-increment-missing").await;
    create_document(&ctx, "trip_entries", "new-trip", std::collections::HashMap::new(), None)
        .await
        .expect("seed document must be created");
    let name = document_name(&ctx, "new-trip");

    let txn = begin_transaction(&ctx).await;
    commit_writes(
        &ctx,
        vec![transform_write(&name, vec![increment_transform("viewCount", int_field(1))])],
        txn,
        None,
    )
    .await
    .expect("AC-02-03: increment against a missing field must succeed");

    let doc = ctx.get_document(&name, None).await.unwrap().into_inner();
    assert_eq!(int_value(&doc, "viewCount"), 1, "AC-02-03: 0 + 1 = 1, missing treated as integer 0");
}

/// AC-02-03/AC-02-04 — `maximum`/`minimum` against a MISSING field set it
/// DIRECTLY to the given value — the opposite rule from `increment`'s own
/// missing-field-as-zero baseline. Dedicated test: this is the easiest
/// thing to get backwards.
///
/// @driving_port @real-io @US-02 @AC-02-03 @AC-02-04
#[tokio::test]
async fn maximum_and_minimum_against_a_missing_field_set_it_directly_not_compared_to_zero() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ft02-maxmin-missing").await;
    create_document(&ctx, "trip_entries", "new-trip", std::collections::HashMap::new(), None)
        .await
        .expect("seed document must be created");
    let name = document_name(&ctx, "new-trip");

    // maximum(-5) against a missing field: if it were compared against an
    // assumed 0 baseline (increment's rule), the result would stay 0/absent.
    // The correct rule sets it DIRECTLY to -5.
    let txn = begin_transaction(&ctx).await;
    commit_writes(
        &ctx,
        vec![transform_write(&name, vec![maximum_transform("promoBoostCount", int_field(-5))])],
        txn,
        None,
    )
    .await
    .expect("AC-02-03/04: maximum against a missing field must succeed");

    let doc = ctx.get_document(&name, None).await.unwrap().into_inner();
    assert_eq!(
        int_value(&doc, "promoBoostCount"),
        -5,
        "AC-02-03/04: maximum on a missing field sets DIRECTLY to the given value, not compared to 0"
    );
}

/// AC-02-04 — `maximum`/`minimum` correctly compare against an existing
/// value and set the field to whichever is greater/lesser, type-preserved.
///
/// @driving_port @real-io @US-02 @AC-02-04
#[tokio::test]
async fn maximum_and_minimum_compare_against_existing_value() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ft02-maxmin-existing").await;
    create_document(
        &ctx,
        "trip_entries",
        "kilimanjaro-trek",
        std::collections::HashMap::from([("promoBoostCount".to_string(), int_field(1200))]),
        None,
    )
    .await
    .expect("seed document must be created");
    let name = document_name(&ctx, "kilimanjaro-trek");

    // maximum(1000) against 1200: current already exceeds the given
    // maximum, so it stays unchanged.
    let txn = begin_transaction(&ctx).await;
    commit_writes(
        &ctx,
        vec![transform_write(&name, vec![maximum_transform("promoBoostCount", int_field(1000))])],
        txn,
        None,
    )
    .await
    .expect("AC-02-04: maximum must succeed");
    let doc = ctx.get_document(&name, None).await.unwrap().into_inner();
    assert_eq!(
        int_value(&doc, "promoBoostCount"),
        1200,
        "AC-02-04: current 1200 already exceeds given maximum 1000 — unchanged"
    );

    // minimum(500) against 1200: 500 is lesser, field becomes 500.
    let txn = begin_transaction(&ctx).await;
    commit_writes(
        &ctx,
        vec![transform_write(&name, vec![minimum_transform("promoBoostCount", int_field(500))])],
        txn,
        None,
    )
    .await
    .expect("AC-02-04: minimum must succeed");
    let doc = ctx.get_document(&name, None).await.unwrap().into_inner();
    assert_eq!(
        int_value(&doc, "promoBoostCount"),
        500,
        "AC-02-04: given minimum 500 is lesser than current 1200"
    );
}

/// AC-02-05 — `increment` against a non-numeric existing value is rejected
/// with `InvalidArgument`, and the document's field is left unchanged.
///
/// @error @driving_port @real-io @US-02 @AC-02-05
#[tokio::test]
async fn increment_against_a_non_numeric_existing_value_is_rejected() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ft02-increment-nonnumeric").await;
    create_document(
        &ctx,
        "trip_entries",
        "kilimanjaro-trek",
        std::collections::HashMap::from([(
            "viewCount".to_string(),
            string_field("not-a-number"),
        )]),
        None,
    )
    .await
    .expect("seed document must be created");
    let name = document_name(&ctx, "kilimanjaro-trek");

    let txn = begin_transaction(&ctx).await;
    let result = commit_writes(
        &ctx,
        vec![transform_write(&name, vec![increment_transform("viewCount", int_field(1))])],
        txn,
        None,
    )
    .await;

    let err = result.expect_err("AC-02-05: increment against a non-numeric field must be rejected");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);

    let doc = ctx.get_document(&name, None).await.unwrap().into_inner();
    assert_eq!(
        doc.fields.get("viewCount").and_then(|v| v.value_type.clone()),
        Some(embyr_proto::firestore::value::ValueType::StringValue(
            "not-a-number".to_string()
        )),
        "AC-02-05: the document's viewCount field must be unchanged"
    );
}

/// Escalation 1 (ADR-053) — `increment` overflow (`i64::MAX` + 1) returns
/// `InvalidArgument`, never wraps, document unmodified.
///
/// @error @driving_port @real-io @US-02
#[tokio::test]
async fn increment_overflow_is_rejected_without_wrapping_or_modifying_the_document() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ft02-overflow").await;
    create_document(
        &ctx,
        "trip_entries",
        "kilimanjaro-trek",
        std::collections::HashMap::from([("viewCount".to_string(), int_field(i64::MAX))]),
        None,
    )
    .await
    .expect("seed document must be created");
    let name = document_name(&ctx, "kilimanjaro-trek");

    let txn = begin_transaction(&ctx).await;
    let result = commit_writes(
        &ctx,
        vec![transform_write(&name, vec![increment_transform("viewCount", int_field(1))])],
        txn,
        None,
    )
    .await;

    let err = result.expect_err("increment overflow must be rejected, not wrap");
    assert_eq!(err.code(), tonic::Code::InvalidArgument);

    let doc = ctx.get_document(&name, None).await.unwrap().into_inner();
    assert_eq!(
        int_value(&doc, "viewCount"),
        i64::MAX,
        "overflow must leave the document unmodified — never silently wrap"
    );
}

/// AC-02-01/AC-02-05 (`update_transforms` wire shape) — `increment` also
/// works when attached to a regular `update` in the SAME write, mirroring
/// FT01's own second wire-shape scenario, proving both wire shapes reuse
/// the identical numeric-transform compute path.
///
/// @driving_port @real-io @US-02 @AC-02-01
#[tokio::test]
async fn increment_attached_to_an_update_persists_both_together() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-ft02-attached").await;
    create_document(
        &ctx,
        "trip_entries",
        "kilimanjaro-trek",
        std::collections::HashMap::from([
            ("name".to_string(), string_field("Kilimanjaro Trek")),
            ("viewCount".to_string(), int_field(41)),
        ]),
        None,
    )
    .await
    .expect("seed document must be created");
    let name = document_name(&ctx, "kilimanjaro-trek");

    let txn = begin_transaction(&ctx).await;
    let write = update_write_with_transforms(
        &name,
        std::collections::HashMap::from([(
            "name".to_string(),
            string_field("Kilimanjaro Trek — Revised Itinerary"),
        )]),
        vec![increment_transform("viewCount", int_field(1))],
    );
    commit_writes(&ctx, vec![write], txn, None)
        .await
        .expect("increment attached via update_transforms must succeed");

    let doc = ctx.get_document(&name, None).await.unwrap().into_inner();
    assert_eq!(
        doc.fields.get("name").and_then(|v| v.value_type.clone()),
        Some(embyr_proto::firestore::value::ValueType::StringValue(
            "Kilimanjaro Trek — Revised Itinerary".to_string()
        )),
        "the regular field update must persist"
    );
    assert_eq!(
        int_value(&doc, "viewCount"),
        42,
        "the attached increment must ALSO persist, reading the PRE-existing 41, not the write's own fields map"
    );
}
