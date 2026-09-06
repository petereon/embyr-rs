//! MFS01 (Slice 01, Walking Skeleton, US-01, the entire feature) —
//! Malformed Filter Shapes Get a Clean Rejection Instead of a Panic.
//!
//! Acceptance criteria verified here (feature-delta.md US-01):
//!   AC-MFS-01: a real `RunQuery` with a non-`Array` value paired with
//!              `In` returns a clean `INVALID_ARGUMENT` — no panic, no
//!              transport reset.
//!   AC-MFS-02: same for `NotIn` and `ArrayContainsAny`.
//!   AC-MFS-03: a real `RunQuery` with a `Null` value paired with a range
//!              operator returns a clean `INVALID_ARGUMENT`.
//!   AC-MFS-04: (regression guard) every well-formed filter shape fixed
//!              by the 3 prior features continues to work correctly.
//!
//! Driving port: gRPC :8080 `RunQuery`, via `SecurityRulesFullContext`.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::SecurityRulesFullContext;

use embyr_proto::firestore::{
    firestore_client::FirestoreClient,
    run_query_request::QueryType,
    structured_query::{field_filter::Operator as FieldOp, filter::FilterType, CollectionSelector, FieldFilter, FieldReference, Filter},
    value::ValueType,
    ArrayValue, CreateDocumentRequest, Document, RunQueryRequest, StructuredQuery, Value,
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

fn null_value() -> Value {
    Value { value_type: Some(ValueType::NullValue(0)) }
}

fn array_value(values: Vec<Value>) -> Value {
    Value { value_type: Some(ValueType::ArrayValue(ArrayValue { values })) }
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

async fn run_query_field_filter(
    client: &mut FirestoreClient<tonic::transport::Channel>,
    project_id: &str,
    api_key: &str,
    collection: &str,
    field: &str,
    op: FieldOp,
    value: Value,
) -> Result<Vec<Document>, tonic::Status> {
    let parent = format!("projects/{project_id}/databases/(default)/documents");
    let sq = StructuredQuery {
        from: vec![CollectionSelector { collection_id: collection.to_string(), all_descendants: false }],
        r#where: Some(Filter {
            filter_type: Some(FilterType::FieldFilter(FieldFilter {
                field: Some(field_ref(field)),
                op: op as i32,
                value: Some(value),
            })),
        }),
        ..Default::default()
    };
    let req = make_authed_request(
        RunQueryRequest { parent, query_type: Some(QueryType::StructuredQuery(sq)), ..Default::default() },
        api_key,
    );
    let stream = client.run_query(req).await?.into_inner();
    use tokio_stream::StreamExt;
    let responses: Vec<_> = stream.collect().await;
    let mut docs = Vec::new();
    for r in responses {
        if let Some(doc) = r?.document {
            docs.push(doc);
        }
    }
    Ok(docs)
}

/// AC-MFS-01
///
/// @walking_skeleton @driving_port @real-io @US-01 @AC-MFS-01
#[tokio::test]
async fn in_with_a_non_array_value_returns_a_clean_invalid_argument() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-mfs01-in").await;
    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));

    let err = run_query_field_filter(
        &mut client, &ctx.project_id, &ctx.api_key, "tickets", "status", FieldOp::In, string_value("open"),
    )
    .await
    .expect_err("a non-array value paired with In must be rejected cleanly");
    assert_eq!(err.code(), tonic::Code::InvalidArgument, "got {:?}: {}", err.code(), err.message());
}

/// AC-MFS-02 (NotIn, ArrayContainsAny)
///
/// @driving_port @real-io @US-01 @AC-MFS-02
#[tokio::test]
async fn not_in_and_array_contains_any_with_a_non_array_value_return_a_clean_invalid_argument() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-mfs01-notin-aca").await;
    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));

    let err1 = run_query_field_filter(
        &mut client, &ctx.project_id, &ctx.api_key, "tickets", "status", FieldOp::NotIn, string_value("closed"),
    )
    .await
    .expect_err("a non-array value paired with NotIn must be rejected cleanly");
    assert_eq!(err1.code(), tonic::Code::InvalidArgument, "got {:?}: {}", err1.code(), err1.message());

    let err2 = run_query_field_filter(
        &mut client, &ctx.project_id, &ctx.api_key, "posts", "tags", FieldOp::ArrayContainsAny, string_value("urgent"),
    )
    .await
    .expect_err("a non-array value paired with ArrayContainsAny must be rejected cleanly");
    assert_eq!(err2.code(), tonic::Code::InvalidArgument, "got {:?}: {}", err2.code(), err2.message());
}

/// AC-MFS-03
///
/// @driving_port @real-io @US-01 @AC-MFS-03
#[tokio::test]
async fn less_than_with_a_null_value_returns_a_clean_invalid_argument() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-mfs01-null-range").await;
    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));

    let err = run_query_field_filter(
        &mut client, &ctx.project_id, &ctx.api_key, "products", "score", FieldOp::LessThan, null_value(),
    )
    .await
    .expect_err("a Null value paired with LessThan must be rejected cleanly");
    assert_eq!(err.code(), tonic::Code::InvalidArgument, "got {:?}: {}", err.code(), err.message());
}

/// AC-MFS-04 (regression guard)
///
/// @driving_port @real-io @US-01 @AC-MFS-04
#[tokio::test]
async fn well_formed_in_and_range_queries_still_work_correctly() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-mfs01-regression").await;
    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));

    let mut fields = std::collections::HashMap::new();
    fields.insert("status".to_string(), string_value("open"));
    fields.insert("score".to_string(), integer_value(50));
    seed_document(&mut client, &ctx.project_id, &ctx.api_key, "tickets", "t-1", fields).await;

    let in_docs = run_query_field_filter(
        &mut client,
        &ctx.project_id,
        &ctx.api_key,
        "tickets",
        "status",
        FieldOp::In,
        array_value(vec![string_value("open"), string_value("pending")]),
    )
    .await
    .expect("AC-MFS-04: a well-formed In query must still succeed");
    assert_eq!(in_docs.len(), 1);

    let range_docs = run_query_field_filter(
        &mut client, &ctx.project_id, &ctx.api_key, "tickets", "score", FieldOp::GreaterThan, integer_value(10),
    )
    .await
    .expect("AC-MFS-04: a well-formed range query must still succeed");
    assert_eq!(range_docs.len(), 1);
}
