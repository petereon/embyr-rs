//! go-live P4 — several aggregations per `RunAggregationQuery` request
//! (what the Firebase SDK's `getAggregateFromServer({count, sum, avg})`
//! sends), `Count.up_to`, and alias resolution.
//!
//! Driving port: gRPC :8080 `RunAggregationQuery` (real Postgres, real gRPC).
//! Pure alias/SQL-encoder logic is unit-tested in embyr-core / embyr-pg-storage.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    create_document, integer_field, mint_client_identity_token, now_unix, run_aggregations,
    string_field, SecurityRulesFullContext,
};
use ed25519_dalek::SigningKey;
use embyr_proto::firestore::{
    structured_aggregation_query::aggregation::{Avg, Count, Operator as Op, Sum},
    structured_query::FieldReference,
    value::ValueType,
};
use rand_core::OsRng;
use std::collections::HashMap;

fn field(path: &str) -> Option<FieldReference> {
    Some(FieldReference { field_path: path.to_string() })
}

/// Maria owns three `trip_expenses` docs with `amount_cents` 1000, 2000, 500.
async fn seeded() -> (SecurityRulesFullContext, String) {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-agg-multi").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes())
        .await;
    ctx.seed_access_rule("trip_expenses", "request.auth.uid == resource.data.owner_id")
        .await;
    let token = mint_client_identity_token(&signing_key, "maria-santos", &ctx.project_id, now_unix() + 3600);
    for (i, amount) in [1000i64, 2000, 500].into_iter().enumerate() {
        let mut fields = HashMap::new();
        fields.insert("owner_id".to_string(), string_field("maria-santos"));
        fields.insert("amount_cents".to_string(), integer_field(amount));
        create_document(&ctx, "trip_expenses", &format!("agg-multi-{i}"), fields, Some(&token))
            .await
            .expect("seed trip_expenses document");
    }
    (ctx, token)
}

const OWN: &[(&str, &str)] = &[("owner_id", "maria-santos")];

/// count + sum + avg in one request: each keyed by its client alias; the
/// empty alias is server-assigned `field_1`.
#[tokio::test]
async fn count_sum_and_avg_in_one_request_return_values_keyed_by_alias() {
    let (ctx, token) = seeded().await;

    let got = run_aggregations(
        &ctx,
        "trip_expenses",
        OWN,
        vec![
            ("n", Op::Count(Count { up_to: None })),
            ("total", Op::Sum(Sum { field: field("amount_cents") })),
            ("", Op::Avg(Avg { field: field("amount_cents") })),
        ],
        Some(&token),
    )
    .await
    .expect("multi-aggregation request must be admitted");

    assert_eq!(got.len(), 3, "got: {got:?}");
    assert_eq!(got["n"].value_type, Some(ValueType::IntegerValue(3)));
    assert_eq!(got["total"].value_type, Some(ValueType::DoubleValue(3500.0)));
    assert_eq!(
        got["field_1"].value_type,
        Some(ValueType::DoubleValue(3500.0 / 3.0))
    );
}

/// `Count.up_to` caps the count at min(count, up_to); two counts with
/// different caps coexist; `up_to <= 0` is invalid_argument.
#[tokio::test]
async fn count_up_to_caps_the_count_and_non_positive_up_to_is_invalid() {
    let (ctx, token) = seeded().await;

    let got = run_aggregations(
        &ctx,
        "trip_expenses",
        OWN,
        vec![
            ("capped", Op::Count(Count { up_to: Some(2) })),
            ("roomy", Op::Count(Count { up_to: Some(10) })),
        ],
        Some(&token),
    )
    .await
    .expect("count up_to must be admitted");
    assert_eq!(got["capped"].value_type, Some(ValueType::IntegerValue(2)));
    assert_eq!(got["roomy"].value_type, Some(ValueType::IntegerValue(3)));

    let err = run_aggregations(
        &ctx,
        "trip_expenses",
        OWN,
        vec![("n", Op::Count(Count { up_to: Some(0) }))],
        Some(&token),
    )
    .await
    .expect_err("up_to = 0 must be rejected");
    assert_eq!(err.code(), tonic::Code::InvalidArgument, "got: {err:?}");
}

/// Duplicate explicit aliases and an empty aggregation list are
/// invalid_argument (Firestore semantics), never silently merged.
#[tokio::test]
async fn duplicate_aliases_and_empty_aggregation_list_are_invalid_argument() {
    let (ctx, token) = seeded().await;

    let dup = run_aggregations(
        &ctx,
        "trip_expenses",
        OWN,
        vec![
            ("x", Op::Count(Count { up_to: None })),
            ("x", Op::Sum(Sum { field: field("amount_cents") })),
        ],
        Some(&token),
    )
    .await
    .expect_err("duplicate alias must be rejected");
    assert_eq!(dup.code(), tonic::Code::InvalidArgument, "got: {dup:?}");

    let none = run_aggregations(&ctx, "trip_expenses", OWN, vec![], Some(&token))
        .await
        .expect_err("zero aggregations must be rejected");
    assert_eq!(none.code(), tonic::Code::InvalidArgument, "got: {none:?}");
}
