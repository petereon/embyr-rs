//! CIR01 (Slice 01, Walking Skeleton, US-01, Release 1) — A Multi-Field
//! Sort Correctly Requires a Composite Index.
//!
//! Acceptance criteria verified here (feature-delta.md US-01, § Resolution
//! 2/3):
//!   AC-CIR-01: a real `RunQuery` with 2+ `orderBy` fields and NO filter is
//!              rejected `FAILED_PRECONDITION` when no matching composite
//!              index exists.
//!   AC-CIR-02: a real `RunQuery` with 2+ `orderBy` fields where EVERY
//!              `orderBy` field happens to already be a filtered field is
//!              STILL rejected `FAILED_PRECONDITION`.
//!   AC-CIR-03: (regression guard) the EXISTING `category==`/`score`
//!              -orderBy single-`orderBy`-field shape is UNCHANGED —
//!              correctly rejected without a ready index, correctly
//!              succeeds once one exists.
//!
//! Driving port: gRPC :8080 `RunQuery`, via `SecurityRulesFullContext`
//! (no access rules involved — an unrestricted collection).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::SecurityRulesFullContext;

use embyr_proto::firestore::{
    firestore_client::FirestoreClient,
    run_query_request::QueryType,
    structured_query::{
        composite_filter::Operator as CompositeOp, field_filter::Operator as FieldOp,
        filter::FilterType, CollectionSelector, CompositeFilter, Direction, FieldFilter,
        FieldReference, Filter, Order,
    },
    value::ValueType,
    CreateDocumentRequest, Document, RunQueryRequest, StructuredQuery, Value,
};

fn make_channel(addr: std::net::SocketAddr) -> tonic::transport::Channel {
    tonic::transport::Channel::from_shared(format!("http://{addr}"))
        .unwrap()
        .connect_lazy()
}

fn make_authed_request<T>(payload: T, api_key: &str) -> tonic::Request<T> {
    let mut req = tonic::Request::new(payload);
    req.metadata_mut()
        .insert("authorization", format!("bearer {api_key}").parse().unwrap());
    req
}

fn string_value(s: &str) -> Value {
    Value { value_type: Some(ValueType::StringValue(s.to_string())) }
}

fn integer_value(i: i64) -> Value {
    Value { value_type: Some(ValueType::IntegerValue(i)) }
}

fn field_ref(path: &str) -> FieldReference {
    FieldReference { field_path: path.to_string() }
}

async fn seed_document(
    client: &mut FirestoreClient<tonic::transport::Channel>,
    project_id: &str,
    api_key: &str,
    collection: &str,
    doc_id: &str,
    fields: std::collections::HashMap<String, Value>,
) {
    let parent = format!("projects/{project_id}/databases/(default)/documents");
    let req = make_authed_request(
        CreateDocumentRequest {
            parent,
            collection_id: collection.to_string(),
            document_id: doc_id.to_string(),
            document: Some(Document { name: String::new(), fields, ..Default::default() }),
            ..Default::default()
        },
        api_key,
    );
    client.create_document(req).await.expect("seed document should succeed");
}

async fn run_query(
    client: &mut FirestoreClient<tonic::transport::Channel>,
    project_id: &str,
    api_key: &str,
    sq: StructuredQuery,
) -> Result<(), tonic::Status> {
    let parent = format!("projects/{project_id}/databases/(default)/documents");
    let req = make_authed_request(
        RunQueryRequest { parent, query_type: Some(QueryType::StructuredQuery(sq)), ..Default::default() },
        api_key,
    );
    let stream = client.run_query(req).await?.into_inner();
    use tokio_stream::StreamExt;
    let responses: Vec<_> = stream.collect().await;
    for r in responses {
        r?;
    }
    Ok(())
}

/// AC-CIR-01
///
/// @walking_skeleton @driving_port @real-io @US-01 @AC-CIR-01
#[tokio::test]
async fn two_order_by_fields_with_no_filter_is_rejected_without_a_ready_index() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cir01-no-filter").await;
    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));

    let mut fields = std::collections::HashMap::new();
    fields.insert("category".to_string(), string_value("B"));
    fields.insert("score".to_string(), integer_value(200));
    seed_document(&mut client, &ctx.project_id, &ctx.api_key, "products", "prod-1", fields).await;

    let sq = StructuredQuery {
        from: vec![CollectionSelector { collection_id: "products".to_string(), all_descendants: false }],
        order_by: vec![
            Order { field: Some(field_ref("category")), direction: Direction::Ascending as i32 },
            Order { field: Some(field_ref("score")), direction: Direction::Descending as i32 },
        ],
        ..Default::default()
    };
    let result = run_query(&mut client, &ctx.project_id, &ctx.api_key, sq).await;
    let err = result.expect_err(
        "AC-CIR-01: a 2-field orderBy with no filter must require a composite index",
    );
    assert_eq!(err.code(), tonic::Code::FailedPrecondition, "got {:?}: {}", err.code(), err.message());
}

/// AC-CIR-02
///
/// @driving_port @real-io @US-01 @AC-CIR-02
#[tokio::test]
async fn two_order_by_fields_that_are_both_already_filtered_is_still_rejected() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cir01-both-filtered").await;
    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));

    let mut fields = std::collections::HashMap::new();
    fields.insert("category".to_string(), string_value("B"));
    fields.insert("score".to_string(), integer_value(200));
    seed_document(&mut client, &ctx.project_id, &ctx.api_key, "products", "prod-1", fields).await;

    let sq = StructuredQuery {
        from: vec![CollectionSelector { collection_id: "products".to_string(), all_descendants: false }],
        r#where: Some(Filter {
            filter_type: Some(FilterType::CompositeFilter(CompositeFilter {
                op: CompositeOp::And as i32,
                filters: vec![
                    Filter {
                        filter_type: Some(FilterType::FieldFilter(FieldFilter {
                            field: Some(field_ref("category")),
                            op: FieldOp::Equal as i32,
                            value: Some(string_value("B")),
                        })),
                    },
                    Filter {
                        filter_type: Some(FilterType::FieldFilter(FieldFilter {
                            field: Some(field_ref("score")),
                            op: FieldOp::Equal as i32,
                            value: Some(integer_value(200)),
                        })),
                    },
                ],
            })),
        }),
        order_by: vec![
            Order { field: Some(field_ref("category")), direction: Direction::Ascending as i32 },
            Order { field: Some(field_ref("score")), direction: Direction::Descending as i32 },
        ],
        ..Default::default()
    };
    let result = run_query(&mut client, &ctx.project_id, &ctx.api_key, sq).await;
    let err = result.expect_err(
        "AC-CIR-02: a 2-field orderBy still requires a composite index even when every orderBy \
         field is already among the filtered fields",
    );
    assert_eq!(err.code(), tonic::Code::FailedPrecondition, "got {:?}: {}", err.code(), err.message());
}

/// AC-CIR-03 (regression guard): the pre-existing single-orderBy-field shape
/// is unchanged.
///
/// @driving_port @real-io @US-01 @AC-CIR-03
#[tokio::test]
async fn single_order_by_field_different_from_an_equality_filter_is_still_gated() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cir01-regression").await;
    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));

    let mut fields = std::collections::HashMap::new();
    fields.insert("category".to_string(), string_value("B"));
    fields.insert("score".to_string(), integer_value(200));
    seed_document(&mut client, &ctx.project_id, &ctx.api_key, "products", "prod-1", fields).await;

    let sq = StructuredQuery {
        from: vec![CollectionSelector { collection_id: "products".to_string(), all_descendants: false }],
        r#where: Some(Filter {
            filter_type: Some(FilterType::FieldFilter(FieldFilter {
                field: Some(field_ref("category")),
                op: FieldOp::Equal as i32,
                value: Some(string_value("B")),
            })),
        }),
        order_by: vec![Order { field: Some(field_ref("score")), direction: Direction::Descending as i32 }],
        ..Default::default()
    };
    let before = run_query(&mut client, &ctx.project_id, &ctx.api_key, sq.clone()).await;
    let err = before.expect_err("AC-CIR-03: unchanged — still gated without a ready index");
    assert_eq!(err.code(), tonic::Code::FailedPrecondition, "got {:?}: {}", err.code(), err.message());

    let create_resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!("/admin/v1/projects/{}/indexes", ctx.project_id)))
        .header(
            "Cookie",
            &ctx.seed_session("alex@trailmark.example", "Owner").await,
        )
        .json(&serde_json::json!({
            "collection_path": "products",
            "fields": [{"field": "category", "order": "ASC"}, {"field": "score", "order": "DESC"}],
        }))
        .send()
        .await
        .expect("create_composite_index request failed");
    assert_eq!(create_resp.status().as_u16(), 200);

    let after = run_query(&mut client, &ctx.project_id, &ctx.api_key, sq).await;
    assert!(
        after.is_ok(),
        "AC-CIR-03: unchanged — still succeeds once a matching index exists: {:?}",
        after.err()
    );
}
