//! protocol-correctness (go-live review P6/P7/P8) — driving port: gRPC :8080.
//!
//! P6: a cursor / filter that cannot be converted is INVALID_ARGUMENT, never
//!     silently dropped (which returned unpaginated / unfiltered results).
//! P7: documents over 1 MiB or nested deeper than 20 levels are rejected with
//!     INVALID_ARGUMENT on CreateDocument, UpdateDocument and Commit.
//! P8: the composite-index gate is FAILED_PRECONDITION with Firestore's own
//!     "The query requires an index." wording.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::SecurityRulesFullContext;

use std::collections::HashMap;

use embyr_proto::firestore::{
    firestore_client::FirestoreClient,
    run_query_request::QueryType,
    structured_query::{
        field_filter::Operator as FieldOp, filter::FilterType, CollectionSelector, Direction,
        FieldFilter, FieldReference, Filter, Order,
    },
    value::ValueType,
    write::Operation,
    CommitRequest, CreateDocumentRequest, Cursor, Document, LatLng, MapValue, RunQueryRequest,
    StructuredQuery, UpdateDocumentRequest, Value, Write,
};

type Client = FirestoreClient<tonic::transport::Channel>;

fn client(ctx: &SecurityRulesFullContext) -> Client {
    FirestoreClient::new(
        tonic::transport::Channel::from_shared(format!("http://{}", ctx.server.grpc_addr))
            .unwrap()
            .connect_lazy(),
    )
}

fn authed<T>(payload: T, api_key: &str) -> tonic::Request<T> {
    let mut req = tonic::Request::new(payload);
    req.metadata_mut()
        .insert("authorization", format!("bearer {api_key}").parse().unwrap());
    req
}

fn value(v: ValueType) -> Value {
    Value { value_type: Some(v) }
}

fn string_value(s: &str) -> Value {
    value(ValueType::StringValue(s.to_string()))
}

fn geo_value() -> Value {
    value(ValueType::GeoPointValue(LatLng { latitude: 1.0, longitude: 2.0 }))
}

fn parent(ctx: &SecurityRulesFullContext) -> String {
    format!("projects/{}/databases/(default)/documents", ctx.project_id)
}

fn doc_name(ctx: &SecurityRulesFullContext, collection: &str, id: &str) -> String {
    format!("{}/{collection}/{id}", parent(ctx))
}

fn fields_of(entries: Vec<(&str, Value)>) -> HashMap<String, Value> {
    entries.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

/// `levels` maps nested inside one another; the leaf scalar sits at depth `levels + 1`.
fn nested_fields(levels: usize) -> HashMap<String, Value> {
    let mut v = value(ValueType::IntegerValue(1));
    for _ in 0..levels {
        v = value(ValueType::MapValue(MapValue { fields: fields_of(vec![("a", v)]) }));
    }
    fields_of(vec![("a", v)])
}

async fn create(
    ctx: &SecurityRulesFullContext,
    c: &mut Client,
    id: &str,
    fields: HashMap<String, Value>,
) -> Result<(), tonic::Status> {
    let req = CreateDocumentRequest {
        parent: parent(ctx),
        collection_id: "items".to_string(),
        document_id: id.to_string(),
        document: Some(Document { fields, ..Default::default() }),
        ..Default::default()
    };
    c.create_document(authed(req, &ctx.api_key)).await.map(|_| ())
}

async fn run_query(
    ctx: &SecurityRulesFullContext,
    c: &mut Client,
    sq: StructuredQuery,
) -> Result<usize, tonic::Status> {
    let req = RunQueryRequest {
        parent: parent(ctx),
        query_type: Some(QueryType::StructuredQuery(sq)),
        ..Default::default()
    };
    let mut stream = c.run_query(authed(req, &ctx.api_key)).await?.into_inner();
    let mut docs = 0;
    while let Some(r) = stream.message().await? {
        docs += r.document.is_some() as usize;
    }
    Ok(docs)
}

fn items_query() -> StructuredQuery {
    StructuredQuery {
        from: vec![CollectionSelector { collection_id: "items".to_string(), all_descendants: false }],
        order_by: vec![Order {
            field: Some(FieldReference { field_path: "n".to_string() }),
            direction: Direction::Ascending as i32,
        }],
        ..Default::default()
    }
}

fn equal_filter(field: &str, v: Value) -> Filter {
    Filter {
        filter_type: Some(FilterType::FieldFilter(FieldFilter {
            field: Some(FieldReference { field_path: field.to_string() }),
            op: FieldOp::Equal as i32,
            value: Some(v),
        })),
    }
}

/// P6
///
/// @error @driving_port @real-io @P6
#[tokio::test]
async fn a_cursor_that_cannot_be_converted_is_rejected_not_dropped() {
    let ctx = SecurityRulesFullContext::new("protocol-pc01-cursor").await;
    let mut c = client(&ctx);
    create(&ctx, &mut c, "d1", fields_of(vec![("n", value(ValueType::IntegerValue(1)))]))
        .await
        .expect("seed");

    for bad in [geo_value(), Value { value_type: None }] {
        let mut start = items_query();
        start.start_at = Some(Cursor { values: vec![bad.clone()], before: true });
        let err = run_query(&ctx, &mut c, start).await.expect_err("start_at must not be dropped");
        assert_eq!(err.code(), tonic::Code::InvalidArgument, "{}", err.message());
        assert!(err.message().contains("start_at"), "got: {}", err.message());

        let mut end = items_query();
        end.end_at = Some(Cursor { values: vec![bad], before: false });
        let err = run_query(&ctx, &mut c, end).await.expect_err("end_at must not be dropped");
        assert_eq!(err.code(), tonic::Code::InvalidArgument, "{}", err.message());
        assert!(err.message().contains("end_at"), "got: {}", err.message());
    }
}

/// P6 (sibling: a filter value that cannot be converted used to mean "no filter").
///
/// @error @driving_port @real-io @P6
#[tokio::test]
async fn a_filter_value_that_cannot_be_converted_is_rejected_not_dropped() {
    let ctx = SecurityRulesFullContext::new("protocol-pc01-filter").await;
    let mut c = client(&ctx);
    create(&ctx, &mut c, "d1", fields_of(vec![("n", value(ValueType::IntegerValue(1)))]))
        .await
        .expect("seed");

    let mut q = items_query();
    q.r#where = Some(equal_filter("loc", geo_value()));
    let err = run_query(&ctx, &mut c, q).await.expect_err("filter must not be dropped");
    assert_eq!(err.code(), tonic::Code::InvalidArgument, "{}", err.message());
}

/// P8
///
/// @error @driving_port @real-io @P8
#[tokio::test]
async fn composite_index_gate_is_failed_precondition_with_firestore_wording() {
    let ctx = SecurityRulesFullContext::new("protocol-pc01-index").await;
    let mut c = client(&ctx);
    let mut q = items_query();
    q.r#where = Some(equal_filter("category", string_value("B")));
    let err = run_query(&ctx, &mut c, q).await.expect_err("no ready index");
    assert_eq!(err.code(), tonic::Code::FailedPrecondition, "{}", err.message());
    assert!(
        err.message().contains("The query requires an index."),
        "got: {}",
        err.message()
    );
}

/// P7
///
/// @error @driving_port @real-io @P7
#[tokio::test]
async fn oversized_and_too_deeply_nested_documents_are_rejected_on_create_and_update() {
    let ctx = SecurityRulesFullContext::new("protocol-pc01-limits").await;
    let mut c = client(&ctx);

    let big = fields_of(vec![("s", string_value(&"x".repeat(1_048_576)))]);
    let err = create(&ctx, &mut c, "big", big.clone()).await.expect_err("over 1 MiB");
    assert_eq!(err.code(), tonic::Code::InvalidArgument, "{}", err.message());

    let err = create(&ctx, &mut c, "deep", nested_fields(20)).await.expect_err("depth 21");
    assert_eq!(err.code(), tonic::Code::InvalidArgument, "{}", err.message());

    // Boundary: 19 nested maps (depth 20) is accepted; Update then enforces the same limits.
    create(&ctx, &mut c, "ok", nested_fields(19)).await.expect("depth 20 is allowed");
    let req = UpdateDocumentRequest {
        document: Some(Document { name: doc_name(&ctx, "items", "ok"), fields: big, ..Default::default() }),
        ..Default::default()
    };
    let err = c.update_document(authed(req, &ctx.api_key)).await.expect_err("update over 1 MiB");
    assert_eq!(err.code(), tonic::Code::InvalidArgument, "{}", err.message());
}

/// P7 — Commit shares the write translation with BatchWrite and the Write stream.
///
/// @error @driving_port @real-io @P7
#[tokio::test]
async fn an_oversized_document_in_a_commit_is_rejected() {
    let ctx = SecurityRulesFullContext::new("protocol-pc01-commit").await;
    let mut c = client(&ctx);
    let big = fields_of(vec![("s", string_value(&"x".repeat(1_048_576)))]);
    let req = CommitRequest {
        database: format!("projects/{}/databases/(default)", ctx.project_id),
        writes: vec![Write {
            operation: Some(Operation::Update(Document {
                name: doc_name(&ctx, "items", "big"),
                fields: big,
                ..Default::default()
            })),
            ..Default::default()
        }],
        ..Default::default()
    };
    let err = c.commit(authed(req, &ctx.api_key)).await.expect_err("over 1 MiB");
    assert_eq!(err.code(), tonic::Code::InvalidArgument, "{}", err.message());
}
