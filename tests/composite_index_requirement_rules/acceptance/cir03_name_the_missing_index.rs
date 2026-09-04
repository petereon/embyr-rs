//! CIR03 (Slice 03, US-03, Release 2, LAST slice) — The Rejection Names
//! the Specific Missing Index.
//!
//! Acceptance criteria verified here (feature-delta.md US-03):
//!   AC-CIR-07: a `FAILED_PRECONDITION` rejection from any detected trigger
//!              shape includes the specific `collection_path` and `fields`
//!              the missing index needs.
//!
//! Driving port: gRPC :8080 `RunQuery`, via `SecurityRulesFullContext`.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::SecurityRulesFullContext;

use embyr_proto::firestore::{
    firestore_client::FirestoreClient,
    run_query_request::QueryType,
    structured_query::{
        field_filter::Operator as FieldOp, filter::FilterType, CollectionSelector, Direction,
        FieldFilter, FieldReference, Filter, Order,
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

/// AC-CIR-07
///
/// @driving_port @real-io @US-03 @AC-CIR-07
#[tokio::test]
async fn the_rejection_names_the_specific_collection_and_fields_needed() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cir03-name-index").await;
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
    let err = run_query(&mut client, &ctx.project_id, &ctx.api_key, sq)
        .await
        .expect_err("must be rejected without a ready index");
    assert_eq!(err.code(), tonic::Code::FailedPrecondition);
    assert!(
        err.message().contains("products"),
        "AC-CIR-07: rejection must name the collection, got: {}",
        err.message()
    );
    assert!(
        err.message().contains("category") && err.message().contains("score"),
        "AC-CIR-07: rejection must name the specific missing fields, got: {}",
        err.message()
    );
}
