//! QFO04 (Slice 04, US-04, Release 1, LAST slice) — `not-in` Queries Stop
//! Crashing, Matching Real Firestore's Field-Must-Exist Rule.
//!
//! Acceptance criteria verified here (feature-delta.md US-04):
//!   AC-QFO-08: a real `RunQuery` with a `not-in` filter returns documents
//!              whose field EXISTS and is not among the listed values.
//!   AC-QFO-09: a document where the filtered field is ENTIRELY ABSENT is
//!              correctly EXCLUDED (live-verified field-must-exist rule).
//!   AC-QFO-10: a document where the filtered field is explicitly `null`
//!              (not absent) is correctly INCLUDED when `null` is not
//!              among the excluded values — proves the distinction between
//!              "absent" and "present but null."
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

/// AC-QFO-08, AC-QFO-09, AC-QFO-10
///
/// @driving_port @real-io @US-04 @AC-QFO-08 @AC-QFO-09 @AC-QFO-10
#[tokio::test]
async fn not_in_excludes_listed_values_and_absent_fields_but_includes_present_null_fields() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-qfo04-not-in").await;
    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));

    // AC-QFO-08: status exists, not in the excluded list — must be included.
    let mut open_ticket = std::collections::HashMap::new();
    open_ticket.insert("status".to_string(), string_value("open"));
    seed_document(&mut client, &ctx.project_id, &ctx.api_key, "tickets", "t-open", open_ticket).await;

    // status exists, IS in the excluded list — must be excluded.
    let mut closed_ticket = std::collections::HashMap::new();
    closed_ticket.insert("status".to_string(), string_value("closed"));
    seed_document(&mut client, &ctx.project_id, &ctx.api_key, "tickets", "t-closed", closed_ticket).await;

    // AC-QFO-09: status field entirely absent — must be excluded.
    let absent_status = std::collections::HashMap::new();
    seed_document(&mut client, &ctx.project_id, &ctx.api_key, "tickets", "t-no-status", absent_status).await;

    // AC-QFO-10: status field explicitly present but null (not absent),
    // and null is not in the excluded list — must be included.
    let mut null_status = std::collections::HashMap::new();
    null_status.insert("status".to_string(), null_value());
    seed_document(&mut client, &ctx.project_id, &ctx.api_key, "tickets", "t-null-status", null_status).await;

    let sq = StructuredQuery {
        from: vec![CollectionSelector { collection_id: "tickets".to_string(), all_descendants: false }],
        r#where: Some(Filter {
            filter_type: Some(FilterType::FieldFilter(FieldFilter {
                field: Some(field_ref("status")),
                op: FieldOp::NotIn as i32,
                value: Some(array_value(vec![string_value("closed")])),
            })),
        }),
        ..Default::default()
    };
    let docs = run_query(&mut client, &ctx.project_id, &ctx.api_key, sq).await;
    let names: std::collections::BTreeSet<String> =
        docs.iter().map(|d| d.name.rsplit('/').next().unwrap().to_string()).collect();
    assert_eq!(
        names,
        std::collections::BTreeSet::from(["t-open".to_string(), "t-null-status".to_string()]),
        "AC-QFO-08/09/10: expected t-open (not excluded) and t-null-status (present-but-null, not \
         excluded) — NOT t-closed (excluded value) or t-no-status (field entirely absent), got: \
         {names:?}"
    );
}
