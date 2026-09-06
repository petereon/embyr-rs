//! RNG02 (Slice 02, US-02, LAST slice) — Array/Map Range Queries Get a
//! Clean Rejection Instead of a Crash.
//!
//! Acceptance criteria verified here (feature-delta.md US-02):
//!   AC-RNG-05: a real `RunQuery` with a range-operator filter on an
//!              `Array`-valued field returns a clean `INVALID_ARGUMENT` —
//!              no panic, no transport reset.
//!   AC-RNG-06: same for `Map`.
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
    ArrayValue, MapValue, RunQueryRequest, StructuredQuery, Value,
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

fn map_value(fields: std::collections::HashMap<String, Value>) -> Value {
    Value { value_type: Some(ValueType::MapValue(MapValue { fields })) }
}

fn field_ref(path: &str) -> FieldReference {
    FieldReference { field_path: path.to_string() }
}

async fn run_query_expect_err(
    client: &mut FirestoreClient<tonic::transport::Channel>,
    project_id: &str,
    api_key: &str,
    sq: StructuredQuery,
) -> tonic::Status {
    let parent = format!("projects/{project_id}/databases/(default)/documents");
    let req = make_authed_request(
        RunQueryRequest { parent, query_type: Some(QueryType::StructuredQuery(sq)), ..Default::default() },
        api_key,
    );
    client
        .run_query(req)
        .await
        .expect_err("RunQuery must return a clean gRPC error, not succeed and not panic")
}

/// AC-RNG-05
///
/// @driving_port @real-io @US-02 @AC-RNG-05
#[tokio::test]
async fn greater_than_on_array_returns_a_clean_invalid_argument() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-rng02-array").await;
    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));

    let sq = StructuredQuery {
        from: vec![CollectionSelector { collection_id: "posts".to_string(), all_descendants: false }],
        r#where: Some(Filter {
            filter_type: Some(FilterType::FieldFilter(FieldFilter {
                field: Some(field_ref("tags")),
                op: FieldOp::GreaterThan as i32,
                value: Some(array_value(vec![string_value("a")])),
            })),
        }),
        ..Default::default()
    };
    let err = run_query_expect_err(&mut client, &ctx.project_id, &ctx.api_key, sq).await;
    assert_eq!(err.code(), tonic::Code::InvalidArgument, "got {:?}: {}", err.code(), err.message());
}

/// AC-RNG-06
///
/// @driving_port @real-io @US-02 @AC-RNG-06
#[tokio::test]
async fn less_than_on_map_returns_a_clean_invalid_argument() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-rng02-map").await;
    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));

    let mut fields = std::collections::HashMap::new();
    fields.insert("a".to_string(), string_value("1"));

    let sq = StructuredQuery {
        from: vec![CollectionSelector { collection_id: "posts".to_string(), all_descendants: false }],
        r#where: Some(Filter {
            filter_type: Some(FilterType::FieldFilter(FieldFilter {
                field: Some(field_ref("metadata")),
                op: FieldOp::LessThan as i32,
                value: Some(map_value(fields)),
            })),
        }),
        ..Default::default()
    };
    let err = run_query_expect_err(&mut client, &ctx.project_id, &ctx.api_key, sq).await;
    assert_eq!(err.code(), tonic::Code::InvalidArgument, "got {:?}: {}", err.code(), err.message());
}
