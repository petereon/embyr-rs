//! ENV01 (Slice 01, Walking Skeleton, US-01, the entire feature) —
//! `Equal`/`NotEqual` Queries Stop Crashing on Every Field Type.
//!
//! Acceptance criteria verified here (feature-delta.md US-01):
//!   AC-ENV-01: a real `RunQuery` with an `Equal` filter on a `Timestamp`
//!              -valued field returns documents whose field matches
//!              EXACTLY that timestamp.
//!   AC-ENV-02: a real `RunQuery` with a `NotEqual` filter on a
//!              `Reference`-valued field correctly excludes the matching
//!              reference and includes non-matching ones.
//!   AC-ENV-03: a real `RunQuery` with an `Equal` filter on an `Array`
//!              -valued field matches only an identically-ordered,
//!              identically-valued array.
//!   AC-ENV-05: no panic, no transport reset, for any of the above.
//!   AC-ENV-06: (regression guard) `Equal`'s own pre-existing behavior for
//!              type-matched `String` targets is unchanged.
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
use prost_types::Timestamp;

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

fn timestamp_value(seconds: i64, nanos: i32) -> Value {
    Value { value_type: Some(ValueType::TimestampValue(Timestamp { seconds, nanos })) }
}

fn reference_value(r: &str) -> Value {
    Value { value_type: Some(ValueType::ReferenceValue(r.to_string())) }
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

fn doc_names(docs: &[Document]) -> std::collections::BTreeSet<String> {
    docs.iter().map(|d| d.name.rsplit('/').next().unwrap().to_string()).collect()
}

/// AC-ENV-01, AC-ENV-05
///
/// @walking_skeleton @driving_port @real-io @US-01 @AC-ENV-01 @AC-ENV-05
#[tokio::test]
async fn equal_on_timestamp_returns_only_the_exact_match_no_panic() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-env01-timestamp").await;
    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));

    let mut matching = std::collections::HashMap::new();
    matching.insert("createdAt".to_string(), timestamp_value(1_700_000_000, 0));
    seed_document(&mut client, &ctx.project_id, &ctx.api_key, "events", "e-1", matching).await;

    let mut non_matching = std::collections::HashMap::new();
    non_matching.insert("createdAt".to_string(), timestamp_value(1_700_000_001, 0));
    seed_document(&mut client, &ctx.project_id, &ctx.api_key, "events", "e-2", non_matching).await;

    let sq = StructuredQuery {
        from: vec![CollectionSelector { collection_id: "events".to_string(), all_descendants: false }],
        r#where: Some(Filter {
            filter_type: Some(FilterType::FieldFilter(FieldFilter {
                field: Some(field_ref("createdAt")),
                op: FieldOp::Equal as i32,
                value: Some(timestamp_value(1_700_000_000, 0)),
            })),
        }),
        ..Default::default()
    };
    let docs = run_query(&mut client, &ctx.project_id, &ctx.api_key, sq).await;
    assert_eq!(doc_names(&docs), std::collections::BTreeSet::from(["e-1".to_string()]));
}

/// AC-ENV-02, AC-ENV-05
///
/// @driving_port @real-io @US-01 @AC-ENV-02 @AC-ENV-05
#[tokio::test]
async fn not_equal_on_reference_excludes_the_matching_reference() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-env01-reference").await;
    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));

    let excluded_ref = reference_value(&format!(
        "projects/{}/databases/(default)/documents/users/alice",
        ctx.project_id
    ));
    let other_ref = reference_value(&format!(
        "projects/{}/databases/(default)/documents/users/bob",
        ctx.project_id
    ));

    let mut doc1 = std::collections::HashMap::new();
    doc1.insert("owner".to_string(), excluded_ref.clone());
    seed_document(&mut client, &ctx.project_id, &ctx.api_key, "posts", "p-1", doc1).await;

    let mut doc2 = std::collections::HashMap::new();
    doc2.insert("owner".to_string(), other_ref);
    seed_document(&mut client, &ctx.project_id, &ctx.api_key, "posts", "p-2", doc2).await;

    let sq = StructuredQuery {
        from: vec![CollectionSelector { collection_id: "posts".to_string(), all_descendants: false }],
        r#where: Some(Filter {
            filter_type: Some(FilterType::FieldFilter(FieldFilter {
                field: Some(field_ref("owner")),
                op: FieldOp::NotEqual as i32,
                value: Some(excluded_ref),
            })),
        }),
        ..Default::default()
    };
    let docs = run_query(&mut client, &ctx.project_id, &ctx.api_key, sq).await;
    assert_eq!(doc_names(&docs), std::collections::BTreeSet::from(["p-2".to_string()]));
}

/// AC-ENV-03, AC-ENV-05
///
/// @driving_port @real-io @US-01 @AC-ENV-03 @AC-ENV-05
#[tokio::test]
async fn equal_on_array_matches_only_identical_order_and_values() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-env01-array").await;
    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));

    let mut matching = std::collections::HashMap::new();
    matching.insert("tags".to_string(), array_value(vec![string_value("a"), string_value("b")]));
    seed_document(&mut client, &ctx.project_id, &ctx.api_key, "posts", "p-1", matching).await;

    // Same elements, different order — must NOT match whole-array equality.
    let mut reordered = std::collections::HashMap::new();
    reordered.insert("tags".to_string(), array_value(vec![string_value("b"), string_value("a")]));
    seed_document(&mut client, &ctx.project_id, &ctx.api_key, "posts", "p-2", reordered).await;

    let sq = StructuredQuery {
        from: vec![CollectionSelector { collection_id: "posts".to_string(), all_descendants: false }],
        r#where: Some(Filter {
            filter_type: Some(FilterType::FieldFilter(FieldFilter {
                field: Some(field_ref("tags")),
                op: FieldOp::Equal as i32,
                value: Some(array_value(vec![string_value("a"), string_value("b")])),
            })),
        }),
        ..Default::default()
    };
    let docs = run_query(&mut client, &ctx.project_id, &ctx.api_key, sq).await;
    assert_eq!(doc_names(&docs), std::collections::BTreeSet::from(["p-1".to_string()]));
}

/// AC-ENV-06 (regression guard)
///
/// @driving_port @real-io @US-01 @AC-ENV-06
#[tokio::test]
async fn equal_on_string_still_works_correctly() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-env01-string-regression").await;
    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));

    let mut fields = std::collections::HashMap::new();
    fields.insert("category".to_string(), string_value("B"));
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
        ..Default::default()
    };
    let docs = run_query(&mut client, &ctx.project_id, &ctx.api_key, sq).await;
    assert_eq!(docs.len(), 1, "AC-ENV-06: Equal on String must still work correctly, got: {docs:?}");
}
