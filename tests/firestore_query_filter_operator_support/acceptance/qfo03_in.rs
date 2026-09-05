//! QFO03 (Slice 03, US-03, Release 1) — `in` Queries Stop Crashing.
//!
//! Acceptance criteria verified here (feature-delta.md US-03):
//!   AC-QFO-06: a real `RunQuery` with an `in` filter returns documents
//!              matching ANY listed value.
//!   AC-QFO-07: (regression guard) `Equal`'s own pre-existing behavior is
//!              unchanged after the shared scalar-dispatch helper
//!              extraction.
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

/// AC-QFO-06
///
/// @driving_port @real-io @US-03 @AC-QFO-06
#[tokio::test]
async fn in_returns_documents_matching_any_listed_value() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-qfo03-in").await;
    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));

    for (id, status) in [("t-1", "open"), ("t-2", "pending"), ("t-3", "closed")] {
        let mut fields = std::collections::HashMap::new();
        fields.insert("status".to_string(), string_value(status));
        seed_document(&mut client, &ctx.project_id, &ctx.api_key, "tickets", id, fields).await;
    }

    let sq = StructuredQuery {
        from: vec![CollectionSelector { collection_id: "tickets".to_string(), all_descendants: false }],
        r#where: Some(Filter {
            filter_type: Some(FilterType::FieldFilter(FieldFilter {
                field: Some(field_ref("status")),
                op: FieldOp::In as i32,
                value: Some(array_value(vec![string_value("open"), string_value("pending")])),
            })),
        }),
        ..Default::default()
    };
    let docs = run_query(&mut client, &ctx.project_id, &ctx.api_key, sq).await;
    let names: std::collections::BTreeSet<String> =
        docs.iter().map(|d| d.name.rsplit('/').next().unwrap().to_string()).collect();
    assert_eq!(
        names,
        std::collections::BTreeSet::from(["t-1".to_string(), "t-2".to_string()]),
        "AC-QFO-06: expected exactly the 2 matching tickets, got: {names:?}"
    );
}

/// AC-QFO-07 (regression guard)
///
/// @driving_port @real-io @US-03 @AC-QFO-07
#[tokio::test]
async fn equal_still_behaves_correctly_after_the_shared_helper_refactor() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-qfo03-equal-regression").await;
    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));

    let mut fields = std::collections::HashMap::new();
    fields.insert("status".to_string(), string_value("open"));
    seed_document(&mut client, &ctx.project_id, &ctx.api_key, "tickets", "t-1", fields).await;

    let sq = StructuredQuery {
        from: vec![CollectionSelector { collection_id: "tickets".to_string(), all_descendants: false }],
        r#where: Some(Filter {
            filter_type: Some(FilterType::FieldFilter(FieldFilter {
                field: Some(field_ref("status")),
                op: FieldOp::Equal as i32,
                value: Some(string_value("open")),
            })),
        }),
        ..Default::default()
    };
    let docs = run_query(&mut client, &ctx.project_id, &ctx.api_key, sq).await;
    assert_eq!(docs.len(), 1, "AC-QFO-07: Equal must still work correctly, got: {docs:?}");
}
