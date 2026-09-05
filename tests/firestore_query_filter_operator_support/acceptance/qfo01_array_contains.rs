//! QFO01 (Slice 01, Walking Skeleton, US-01, Release 1) — `array-contains`
//! Queries Stop Crashing.
//!
//! Acceptance criteria verified here (feature-delta.md US-01):
//!   AC-QFO-01: a real `RunQuery` with an `array-contains` filter on a
//!              field whose array DOES contain the target value returns
//!              that document.
//!   AC-QFO-02: a document whose array does NOT contain the target value
//!              is not returned (no false positive).
//!   AC-QFO-03: no panic, no transport reset — the request completes with
//!              a normal gRPC response either way.
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

async fn run_query(
    client: &mut FirestoreClient<tonic::transport::Channel>,
    project_id: &str,
    api_key: &str,
    sq: StructuredQuery,
) -> Vec<Document> {
    let parent = format!("projects/{project_id}/databases/(default)/documents");
    let req = make_authed_request(
        RunQueryRequest { parent, query_type: Some(QueryType::StructuredQuery(sq)), ..Default::default() },
        api_key,
    );
    let stream = client.run_query(req).await.expect("RunQuery must not error").into_inner();
    use tokio_stream::StreamExt;
    let responses: Vec<_> = stream.collect().await;
    responses
        .into_iter()
        .filter_map(|r| r.expect("must not panic mid-stream").document)
        .collect()
}

/// AC-QFO-01, AC-QFO-02, AC-QFO-03
///
/// @walking_skeleton @driving_port @real-io @US-01 @AC-QFO-01 @AC-QFO-02 @AC-QFO-03
#[tokio::test]
async fn array_contains_returns_matching_documents_and_excludes_non_matching_ones() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-qfo01-array-contains").await;
    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));

    let mut matching = std::collections::HashMap::new();
    matching.insert("tags".to_string(), array_value(vec![string_value("urgent"), string_value("beach")]));
    seed_document(&mut client, &ctx.project_id, &ctx.api_key, "posts", "post-1", matching).await;

    let mut non_matching = std::collections::HashMap::new();
    non_matching.insert("tags".to_string(), array_value(vec![string_value("beach")]));
    seed_document(&mut client, &ctx.project_id, &ctx.api_key, "posts", "post-2", non_matching).await;

    let sq = StructuredQuery {
        from: vec![CollectionSelector { collection_id: "posts".to_string(), all_descendants: false }],
        r#where: Some(Filter {
            filter_type: Some(FilterType::FieldFilter(FieldFilter {
                field: Some(field_ref("tags")),
                op: FieldOp::ArrayContains as i32,
                value: Some(string_value("urgent")),
            })),
        }),
        ..Default::default()
    };
    let docs = run_query(&mut client, &ctx.project_id, &ctx.api_key, sq).await;
    assert_eq!(docs.len(), 1, "AC-QFO-01/02: expected exactly the matching document, got: {docs:?}");
    assert!(docs[0].name.ends_with("post-1"), "got: {}", docs[0].name);
}
