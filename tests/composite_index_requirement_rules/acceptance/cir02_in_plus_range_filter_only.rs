//! CIR02 (Slice 02, US-02, Release 1) — A Filter-Only `IN` + Range Query
//! Correctly Requires a Composite Index.
//!
//! Acceptance criteria verified here (feature-delta.md US-02):
//!   AC-CIR-04: a real, filter-only (no `orderBy`) `RunQuery` combining
//!              `IN` on one field with a range comparison on a DIFFERENT
//!              field is rejected `FAILED_PRECONDITION` when no matching
//!              composite index exists. Proven end-to-end here — the
//!              `requires_composite_index` gate runs BEFORE `adapter.
//!              run_query()`, so this REJECTION path never reaches the
//!              query executor at all.
//!
//! AC-CIR-05 (false-positive guard: `IN` + a separate EQUALITY filter does
//! NOT require composite) and AC-CIR-06 (lone `array-contains-any`/`not-in`
//! do NOT require composite) are proven at the UNIT level ONLY
//! (`crates/embyr-server/src/grpc/handler.rs::composite_index_requirement_
//! tests`), not end-to-end here — a genuine, PRE-EXISTING, unrelated gap
//! discovered during this slice's own DELIVER: `crates/embyr-pg-storage/
//! src/encoding/query.rs::append_field_filter` only implements `<`/`<=`/
//! `>`/`>=`/`==`/`!=` — it `panic!()`s on `In`/`NotIn`/`ArrayContains`/
//! `ArrayContainsAny` (confirmed by direct trace, this feature's own
//! initial DISCUSS Reading Confirmation incorrectly claimed these were
//! already supported — corrected here, not silently left wrong). Since a
//! "succeeds with no index required" proof requires the query to actually
//! EXECUTE, and the executor itself cannot run these operators yet
//! (independent of anything this feature changes), an end-to-end success
//! -path proof for AC-CIR-05/06 is not currently possible in this codebase.
//! Named explicitly as a discovered, deferred, higher-severity follow-up
//! (this pre-existing gap panics — not cleanly errors — inside a live
//! request handler for any real query using these operators today,
//! regardless of this feature) — see feature-delta.md § Discovered Gap.
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
        composite_filter::Operator as CompositeOp, field_filter::Operator as FieldOp,
        filter::FilterType, CollectionSelector, CompositeFilter, FieldFilter, FieldReference,
        Filter,
    },
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

/// AC-CIR-04
///
/// @driving_port @real-io @US-02 @AC-CIR-04
#[tokio::test]
async fn in_filter_plus_range_on_a_different_field_with_no_order_by_is_rejected() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cir02-in-range").await;
    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));

    let mut fields = std::collections::HashMap::new();
    fields.insert("category".to_string(), string_value("B"));
    fields.insert("population".to_string(), integer_value(700_000));
    seed_document(&mut client, &ctx.project_id, &ctx.api_key, "cities", "city-1", fields).await;

    let sq = StructuredQuery {
        from: vec![CollectionSelector { collection_id: "cities".to_string(), all_descendants: false }],
        r#where: Some(Filter {
            filter_type: Some(FilterType::CompositeFilter(CompositeFilter {
                op: CompositeOp::And as i32,
                filters: vec![
                    Filter {
                        filter_type: Some(FilterType::FieldFilter(FieldFilter {
                            field: Some(field_ref("category")),
                            op: FieldOp::In as i32,
                            value: Some(array_value(vec![string_value("A"), string_value("B")])),
                        })),
                    },
                    Filter {
                        filter_type: Some(FilterType::FieldFilter(FieldFilter {
                            field: Some(field_ref("population")),
                            op: FieldOp::GreaterThan as i32,
                            value: Some(integer_value(690_000)),
                        })),
                    },
                ],
            })),
        }),
        ..Default::default()
    };
    let result = run_query(&mut client, &ctx.project_id, &ctx.api_key, sq).await;
    let err = result.expect_err(
        "AC-CIR-04: IN + a range filter on a different field, with no orderBy, must require a \
         composite index",
    );
    assert_eq!(err.code(), tonic::Code::FailedPrecondition, "got {:?}: {}", err.code(), err.message());
}

// AC-CIR-05/AC-CIR-06's own end-to-end success-path proof is not currently
// possible against a real RunQuery — see this file's own module doc for
// why (a pre-existing, unrelated query-executor gap: `append_field_filter`
// panics on In/NotIn/ArrayContains/ArrayContainsAny). Both ACs are proven
// at the unit level instead:
// `crates/embyr-server/src/grpc/handler.rs::composite_index_requirement_
// tests::{in_filter_plus_a_separate_equality_filter_does_not_require_
// composite_index, lone_array_contains_any_does_not_require_composite_
// index, lone_not_in_does_not_require_composite_index}`.
