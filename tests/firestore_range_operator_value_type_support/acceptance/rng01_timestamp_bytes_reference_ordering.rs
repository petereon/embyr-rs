//! RNG01 (Slice 01, Walking Skeleton, US-01, the entire feature's own real
//! -capability half) — Range Queries Stop Crashing on Timestamp/Bytes/
//! Reference Fields.
//!
//! Acceptance criteria verified here (feature-delta.md US-01):
//!   AC-RNG-01: a real `RunQuery` with a `GreaterThan` filter on a
//!              `Timestamp`-valued field returns documents with a
//!              strictly later timestamp.
//!   AC-RNG-02: a real `RunQuery` with a `LessThan` filter on a `Bytes`
//!              -valued field correctly compares by raw BYTE value, not
//!              by base64 TEXT value.
//!   AC-RNG-03: a real `RunQuery` with a `GreaterThanOrEqual` filter on a
//!              `Reference`-valued field correctly compares by resource
//!              path.
//!   AC-RNG-04: (regression guard) the 4 pre-existing types are unchanged.
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
    CreateDocumentRequest, Document, RunQueryRequest, StructuredQuery, Value,
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

fn integer_value(i: i64) -> Value {
    Value { value_type: Some(ValueType::IntegerValue(i)) }
}

fn timestamp_value(seconds: i64, nanos: i32) -> Value {
    Value { value_type: Some(ValueType::TimestampValue(Timestamp { seconds, nanos })) }
}

fn bytes_value(b: Vec<u8>) -> Value {
    Value { value_type: Some(ValueType::BytesValue(b)) }
}

fn reference_value(r: &str) -> Value {
    Value { value_type: Some(ValueType::ReferenceValue(r.to_string())) }
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

/// AC-RNG-01
///
/// @walking_skeleton @driving_port @real-io @US-01 @AC-RNG-01
#[tokio::test]
async fn greater_than_on_timestamp_returns_only_strictly_later_documents() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-rng01-timestamp").await;
    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));

    let mut earlier = std::collections::HashMap::new();
    earlier.insert("createdAt".to_string(), timestamp_value(1_700_000_000, 0));
    seed_document(&mut client, &ctx.project_id, &ctx.api_key, "events", "e-earlier", earlier).await;

    let mut later = std::collections::HashMap::new();
    later.insert("createdAt".to_string(), timestamp_value(1_700_000_100, 0));
    seed_document(&mut client, &ctx.project_id, &ctx.api_key, "events", "e-later", later).await;

    let sq = StructuredQuery {
        from: vec![CollectionSelector { collection_id: "events".to_string(), all_descendants: false }],
        r#where: Some(Filter {
            filter_type: Some(FilterType::FieldFilter(FieldFilter {
                field: Some(field_ref("createdAt")),
                op: FieldOp::GreaterThan as i32,
                value: Some(timestamp_value(1_700_000_050, 0)),
            })),
        }),
        ..Default::default()
    };
    let docs = run_query(&mut client, &ctx.project_id, &ctx.api_key, sq).await;
    assert_eq!(doc_names(&docs), std::collections::BTreeSet::from(["e-later".to_string()]));
}

/// AC-RNG-02: `0xFF` (byte value 255) must be correctly evaluated as
/// GREATER than `0x00` (byte value 0) — but base64-encodes to `"/w=="` vs
/// `"AA=="`, where `'/'` (0x2F) sorts BEFORE `'A'` (0x41) as raw TEXT. A
/// bug comparing base64 TEXT directly would get this backwards.
///
/// @driving_port @real-io @US-01 @AC-RNG-02
#[tokio::test]
async fn less_than_on_bytes_compares_by_true_byte_value_not_base64_text() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-rng01-bytes").await;
    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));

    let mut low_byte = std::collections::HashMap::new();
    low_byte.insert("payload".to_string(), bytes_value(vec![0x00]));
    seed_document(&mut client, &ctx.project_id, &ctx.api_key, "blobs", "b-low", low_byte).await;

    let mut high_byte = std::collections::HashMap::new();
    high_byte.insert("payload".to_string(), bytes_value(vec![0xFF]));
    seed_document(&mut client, &ctx.project_id, &ctx.api_key, "blobs", "b-high", high_byte).await;

    // Filter: payload < 0x80 (128). By TRUE byte value, only 0x00 qualifies.
    // If the buggy base64-text comparison were used instead, the ordering
    // would be scrambled and this assertion would fail.
    let sq = StructuredQuery {
        from: vec![CollectionSelector { collection_id: "blobs".to_string(), all_descendants: false }],
        r#where: Some(Filter {
            filter_type: Some(FilterType::FieldFilter(FieldFilter {
                field: Some(field_ref("payload")),
                op: FieldOp::LessThan as i32,
                value: Some(bytes_value(vec![0x80])),
            })),
        }),
        ..Default::default()
    };
    let docs = run_query(&mut client, &ctx.project_id, &ctx.api_key, sq).await;
    assert_eq!(doc_names(&docs), std::collections::BTreeSet::from(["b-low".to_string()]));
}

/// AC-RNG-03
///
/// @driving_port @real-io @US-01 @AC-RNG-03
#[tokio::test]
async fn greater_than_or_equal_on_reference_compares_by_resource_path() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-rng01-reference").await;
    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));

    let ref_a = reference_value(&format!(
        "projects/{}/databases/(default)/documents/users/alice",
        ctx.project_id
    ));
    let ref_b = reference_value(&format!(
        "projects/{}/databases/(default)/documents/users/bob",
        ctx.project_id
    ));

    let mut doc_a = std::collections::HashMap::new();
    doc_a.insert("owner".to_string(), ref_a.clone());
    seed_document(&mut client, &ctx.project_id, &ctx.api_key, "posts", "p-a", doc_a).await;

    let mut doc_b = std::collections::HashMap::new();
    doc_b.insert("owner".to_string(), ref_b.clone());
    seed_document(&mut client, &ctx.project_id, &ctx.api_key, "posts", "p-b", doc_b).await;

    let sq = StructuredQuery {
        from: vec![CollectionSelector { collection_id: "posts".to_string(), all_descendants: false }],
        r#where: Some(Filter {
            filter_type: Some(FilterType::FieldFilter(FieldFilter {
                field: Some(field_ref("owner")),
                op: FieldOp::GreaterThanOrEqual as i32,
                value: Some(ref_b),
            })),
        }),
        ..Default::default()
    };
    let docs = run_query(&mut client, &ctx.project_id, &ctx.api_key, sq).await;
    assert_eq!(doc_names(&docs), std::collections::BTreeSet::from(["p-b".to_string()]));
}

/// AC-RNG-04 (regression guard)
///
/// @driving_port @real-io @US-01 @AC-RNG-04
#[tokio::test]
async fn greater_than_or_equal_on_integer_still_works_correctly() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-rng01-integer-regression").await;
    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));

    for (id, age) in [("u-1", 17), ("u-2", 18), ("u-3", 25)] {
        let mut fields = std::collections::HashMap::new();
        fields.insert("age".to_string(), integer_value(age));
        seed_document(&mut client, &ctx.project_id, &ctx.api_key, "users", id, fields).await;
    }

    let sq = StructuredQuery {
        from: vec![CollectionSelector { collection_id: "users".to_string(), all_descendants: false }],
        r#where: Some(Filter {
            filter_type: Some(FilterType::FieldFilter(FieldFilter {
                field: Some(field_ref("age")),
                op: FieldOp::GreaterThanOrEqual as i32,
                value: Some(integer_value(18)),
            })),
        }),
        ..Default::default()
    };
    let docs = run_query(&mut client, &ctx.project_id, &ctx.api_key, sq).await;
    assert_eq!(
        doc_names(&docs),
        std::collections::BTreeSet::from(["u-2".to_string(), "u-3".to_string()]),
        "AC-RNG-04: GreaterThanOrEqual on Integer must still work correctly"
    );
}
