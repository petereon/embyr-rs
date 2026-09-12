// SCAFFOLD: true
//! US-A09 (agent-field-path-validation) — embyr-agent's own query-filter
//! field-path validator enforces the identical spec-mandated charset
//! embyr-server already does, closing the SQL-injection-reachable
//! asymmetry named by production-readiness-audit-2026-09-08 finding #11.
//!
//! Driving port: StorageAgent gRPC service (mTLS :9191) — `RunQuery` and
//! `RunAggregationQuery` (Count), the two confirmed callers of the one
//! call site (`proto_filter_to_domain` -> `validate_field_path`,
//! `crates/embyr-agent/src/server.rs:161`).
//!
//! Locked DESIGN (feature-delta.md § Wave: DESIGN): delete the local weak
//! `validate_field_path` (only rejected `".."`); call
//! `embyr_core::domain::query::validate_field_path` (charset guard
//! `^[a-zA-Z_][a-zA-Z0-9_.]*$`) via the already-existing
//! `core_error_to_status`.
//!
//! Red classification (pre-fix, confirmed empirically against the current
//! unfixed binary): every malformed-field-path test below currently FAILS
//! because the request that should be rejected with `INVALID_ARGUMENT`
//! instead reaches the raw-interpolating SQL builder in
//! `embyr-pg-storage/src/encoding/query.rs`, producing either a
//! `Status::internal` (Postgres syntax error surfacing through
//! `sanitize_backend_error`) or, in one case, a normal empty result —
//! never a clean `INVALID_ARGUMENT`. See the doc comment on each test for
//! the exact observed behavior.

use std::collections::HashMap;

use embyr_proto::agent::{
    filter::FilterType, run_aggregation_query_request::QueryType as AggQueryType,
    run_query_request::QueryType, structured_query::CollectionSelector, value::ValueType,
    CompositeFilterOp, CompositeFilterProto, CreateDocumentRequest, Document, FieldFilterOp,
    FieldFilterProto, Filter, GetDocumentRequest, RunAggregationQueryRequest, RunQueryRequest,
    StructuredQuery as ProtoStructuredQuery, Value,
};

use super::agent_common::start_test_agent;

const PROJECT: &str = "meridian-health";

fn parent() -> String {
    format!("projects/{PROJECT}/databases/(default)/documents")
}

fn str_val(s: &str) -> Value {
    Value { value_type: Some(ValueType::StringValue(s.to_string())) }
}

fn int_val(i: i64) -> Value {
    Value { value_type: Some(ValueType::IntegerValue(i)) }
}

fn field_filter(field_path: &str, op: FieldFilterOp, value: Value) -> Filter {
    Filter {
        filter_type: Some(FilterType::FieldFilter(FieldFilterProto {
            field_path: field_path.to_string(),
            op: op as i32,
            value: Some(value),
        })),
    }
}

fn run_query_with_filter(collection: &str, filter: Filter) -> RunQueryRequest {
    RunQueryRequest {
        parent: parent(),
        query_type: Some(QueryType::StructuredQuery(ProtoStructuredQuery {
            from: vec![CollectionSelector { collection_id: collection.to_string(), all_descendants: false }],
            filter: Some(filter),
        })),
        ..Default::default()
    }
}

fn run_aggregation_query_with_filter(collection: &str, filter: Filter) -> RunAggregationQueryRequest {
    RunAggregationQueryRequest {
        parent: parent(),
        query_type: Some(AggQueryType::StructuredQuery(ProtoStructuredQuery {
            from: vec![CollectionSelector { collection_id: collection.to_string(), all_descendants: false }],
            filter: Some(filter),
        })),
    }
}

/// Assert a `RunQuery` call is rejected with `INVALID_ARGUMENT` whose
/// message contains the shared, spec-mandated charset-violation substring
/// — either on the initial call (unary error) or on the first message of
/// the response stream (server-streaming error), matching the pattern
/// already established by `us_a03_query_operations.rs`.
async fn assert_run_query_rejected_with_field_path_message(
    client: &mut embyr_proto::agent::storage_agent_client::StorageAgentClient<tonic::transport::Channel>,
    req: RunQueryRequest,
) {
    let status = match client.run_query(tonic::Request::new(req)).await {
        Err(status) => status,
        Ok(response) => {
            let mut stream = response.into_inner();
            stream.message().await.expect_err("expected an error from the query stream, request should never have reached the SQL builder")
        }
    };
    assert_eq!(
        status.code(),
        tonic::Code::InvalidArgument,
        "expected INVALID_ARGUMENT before the request reaches the SQL builder, got {:?}: {}",
        status.code(),
        status.message()
    );
    assert!(
        status.message().contains("field path must match"),
        "expected the shared spec-mandated charset-violation message, got: {}",
        status.message()
    );
}

async fn assert_run_aggregation_query_rejected_with_field_path_message(
    client: &mut embyr_proto::agent::storage_agent_client::StorageAgentClient<tonic::transport::Channel>,
    req: RunAggregationQueryRequest,
) {
    let status = client
        .run_aggregation_query(tonic::Request::new(req))
        .await
        .expect_err("RunAggregationQuery with a malformed field_path must be rejected, not counted");
    assert_eq!(
        status.code(),
        tonic::Code::InvalidArgument,
        "expected INVALID_ARGUMENT, got {:?}: {}",
        status.code(),
        status.message()
    );
    assert!(
        status.message().contains("field path must match"),
        "expected the shared spec-mandated charset-violation message, got: {}",
        status.message()
    );
}

// ---------------------------------------------------------------------------
// AC-AFP-02 / AC-AFP-03 — SQL-injection-probe field paths are rejected
// ---------------------------------------------------------------------------

/// @walking_skeleton @driving_port @real_io @us_a09 @error @AC-AFP-02
///
/// Feature: A field path containing a SQL-injection probe character is rejected before reaching Postgres
///   Given Riley Nakamura's embyr agent for project "meridian-health" is running against a healthy Postgres
///   When a RunQuery call submits a filter whose field_path is "age' OR '1'='1"
///   Then the RPC returns INVALID_ARGUMENT naming the spec-mandated charset
///   And no SQL statement referencing that payload is ever sent to Postgres
///
/// Pre-fix RED classification (confirmed empirically against the unfixed
/// binary): the old validator only checks for `".."`, so the payload sails
/// through to `embyr-pg-storage`'s raw-interpolating `push_value_equality`,
/// producing `fields->'age' OR '1'='1' = $1::jsonb` — invalid SQL syntax.
/// The resulting `sqlx::Error` is routed through `sanitize_backend_error`,
/// so today's actual observed behavior is `Status::internal("internal
/// server error")`, NOT `Status::invalid_argument`. This assertion fails
/// today for that reason — a genuine RED, not a setup error.
#[tokio::test]
async fn run_query_rejects_sql_injection_probe_field_path_with_single_quote() {
    let (_handle, mut client) = start_test_agent(PROJECT).await;
    let filter = field_filter("age' OR '1'='1", FieldFilterOp::Equal, str_val("active"));
    let req = run_query_with_filter("patients", filter);
    assert_run_query_rejected_with_field_path_message(&mut client, req).await;
}

/// @driving_port @real_io @us_a09 @error @AC-AFP-03
///
/// Feature: RunAggregationQuery rejects the identical SQL-injection probe field path
///   Given Riley Nakamura's embyr agent for project "meridian-health" is running against a healthy Postgres
///   When a RunAggregationQuery count() call submits a filter whose field_path is "age' OR '1'='1"
///   Then the RPC returns INVALID_ARGUMENT naming the spec-mandated charset
///
/// Pre-fix RED classification: identical root cause to the RunQuery sibling
/// above — both share the one call site (`proto_filter_to_domain`).
#[tokio::test]
async fn run_aggregation_query_rejects_sql_injection_probe_field_path_with_single_quote() {
    let (_handle, mut client) = start_test_agent(PROJECT).await;
    let filter = field_filter("age' OR '1'='1", FieldFilterOp::Equal, str_val("active"));
    let req = run_aggregation_query_with_filter("patients", filter);
    assert_run_aggregation_query_rejected_with_field_path_message(&mut client, req).await;
}

/// @driving_port @real_io @us_a09 @error @AC-AFP-02
///
/// Feature: A named SQL-injection-shaped payload is rejected, mirroring embyr-core's own named test payload
///   Given Riley Nakamura's embyr agent for project "meridian-health" is running against a healthy Postgres
///   And a "patients" collection has one document
///   When a RunQuery call submits a filter whose field_path is "x'); DROP TABLE documents; --"
///   Then the RPC returns INVALID_ARGUMENT naming the spec-mandated charset
///   And the "documents" table survives — the previously-seeded document is still readable afterward
#[tokio::test]
async fn run_query_rejects_named_drop_table_injection_payload_and_documents_table_survives() {
    let (handle, mut client) = start_test_agent(PROJECT).await;
    sqlx::query(
        "INSERT INTO documents (project_id, collection_path, document_id, fields, version, create_time, update_time) \
         VALUES ($1,$2,$3,$4::jsonb,1,NOW(),NOW())",
    )
    .bind(PROJECT)
    .bind("patients")
    .bind("patient-1")
    .bind(r#"{"status":{"t":"S","v":"active"}}"#)
    .execute(&handle.pool)
    .await
    .unwrap();

    let filter = field_filter(
        "x'); DROP TABLE documents; --",
        FieldFilterOp::Equal,
        str_val("active"),
    );
    let req = run_query_with_filter("patients", filter);
    assert_run_query_rejected_with_field_path_message(&mut client, req).await;

    // The "documents" table (and this feature's seed row) must still exist —
    // proof no SQL statement referencing the payload ever reached Postgres.
    let survives: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM documents WHERE project_id = $1 AND document_id = $2",
    )
    .bind(PROJECT)
    .bind("patient-1")
    .fetch_one(&handle.pool)
    .await
    .expect("documents table must still exist and be queryable after the rejected request");
    assert_eq!(survives, 1, "the seeded document must still be present — the table was never dropped");
}

// ---------------------------------------------------------------------------
// AC-AFP-04 — regression guards: legitimate field paths keep working
// ---------------------------------------------------------------------------

/// @driving_port @real_io @us_a09 @AC-AFP-04
///
/// Feature: A simple field-path filter continues to work exactly as before this feature
///   Given a "patients" collection has a document with field "status" set to "active"
///   When Alex's app queries that collection with a filter on field "status" equal to "active"
///   Then the RunQuery call returns the matching document
#[tokio::test]
async fn simple_field_name_filter_continues_to_work_on_run_query() {
    let (handle, mut client) = start_test_agent(PROJECT).await;
    sqlx::query(
        "INSERT INTO documents (project_id, collection_path, document_id, fields, version, create_time, update_time) \
         VALUES ($1,$2,$3,$4::jsonb,1,NOW(),NOW())",
    )
    .bind(PROJECT)
    .bind("patients")
    .bind("patient-1")
    .bind(r#"{"status":{"t":"S","v":"active"}}"#)
    .execute(&handle.pool)
    .await
    .unwrap();

    let filter = field_filter("status", FieldFilterOp::Equal, str_val("active"));
    let req = run_query_with_filter("patients", filter);
    let mut stream = client.run_query(tonic::Request::new(req)).await.expect("run_query").into_inner();
    let mut docs = vec![];
    while let Some(msg) = stream.message().await.expect("next") {
        if let Some(doc) = msg.document {
            docs.push(doc);
        }
    }
    assert_eq!(docs.len(), 1, "expected the matching patient document, got: {docs:?}");
    assert!(docs[0].name.ends_with("patient-1"), "got: {}", docs[0].name);
}

/// @driving_port @real_io @us_a09 @AC-AFP-04
///
/// Feature: A simple field-path filter continues to work on RunAggregationQuery exactly as before this feature
///   Given a "patients" collection has 2 documents with field "status" set to "active"
///   When a caller runs a count aggregation filtered on field "status" equal to "active"
///   Then the caller receives a count of 2
#[tokio::test]
async fn simple_field_name_filter_continues_to_work_on_run_aggregation_query() {
    let (handle, mut client) = start_test_agent(PROJECT).await;
    for i in 0..2u8 {
        sqlx::query(
            "INSERT INTO documents (project_id, collection_path, document_id, fields, version, create_time, update_time) \
             VALUES ($1,$2,$3,$4::jsonb,1,NOW(),NOW())",
        )
        .bind(PROJECT)
        .bind("patients")
        .bind(format!("patient-{i}"))
        .bind(r#"{"status":{"t":"S","v":"active"}}"#)
        .execute(&handle.pool)
        .await
        .unwrap();
    }

    let filter = field_filter("status", FieldFilterOp::Equal, str_val("active"));
    let req = run_aggregation_query_with_filter("patients", filter);
    let response = client.run_aggregation_query(tonic::Request::new(req)).await.expect("run_aggregation_query");
    assert_eq!(response.into_inner().count, 2);
}

/// @driving_port @real_io @us_a09 @AC-AFP-04
///
/// Feature: A dotted nested-map field-path filter continues to work exactly as before this feature
///   Given a "patients" collection has a document with a nested field addressable as "address.city" set to "Boston"
///   When Alex's app queries that collection with a filter on field "address.city" equal to "Boston"
///   Then the RunQuery call returns the matching document
#[tokio::test]
async fn dotted_nested_field_path_filter_continues_to_work_on_run_query() {
    let (handle, mut client) = start_test_agent(PROJECT).await;
    sqlx::query(
        "INSERT INTO documents (project_id, collection_path, document_id, fields, version, create_time, update_time) \
         VALUES ($1,$2,$3,$4::jsonb,1,NOW(),NOW())",
    )
    .bind(PROJECT)
    .bind("patients")
    .bind("patient-1")
    .bind(r#"{"address.city":{"t":"S","v":"Boston"}}"#)
    .execute(&handle.pool)
    .await
    .unwrap();

    let filter = field_filter("address.city", FieldFilterOp::Equal, str_val("Boston"));
    let req = run_query_with_filter("patients", filter);
    let mut stream = client.run_query(tonic::Request::new(req)).await.expect("run_query").into_inner();
    let mut docs = vec![];
    while let Some(msg) = stream.message().await.expect("next") {
        if let Some(doc) = msg.document {
            docs.push(doc);
        }
    }
    assert_eq!(docs.len(), 1, "expected the matching patient document, got: {docs:?}");
}

// ---------------------------------------------------------------------------
// AC-AFP-05 — composite (AND) filters validate every leaf
// ---------------------------------------------------------------------------

/// @driving_port @real_io @us_a09 @error @AC-AFP-05
///
/// Feature: A composite (AND) filter rejects a malformed field path in any one of its leaves
///   Given a "patients" collection has documents
///   When a caller queries with an AND-composed filter whose first leaf is field "status" equal to "active"
///     and whose second leaf's field_path is "name; DROP TABLE documents; --"
///   Then the RPC returns INVALID_ARGUMENT naming the spec-mandated charset
///
/// Pre-fix RED classification (confirmed empirically): this payload's `;`
/// characters stay INSIDE the single-quoted SQL string literal
/// (`fields->'name; DROP TABLE documents; --' = $1::jsonb`) — a semicolon
/// inside a quoted literal is not a statement separator, so this specific
/// payload does not break SQL syntax and does not produce a
/// `Status::internal`. The actual observed pre-fix behavior is WORSE for
/// this exact payload than a crash: the request completes normally with
/// `RunQueryResponse { document: None, continuation_selector: Done(true) }`
/// (i.e. `INVALID_ARGUMENT` is never returned; zero results, no error) —
/// the confused-deputy validator silently accepts a field path outside the
/// spec-mandated charset instead of rejecting it, exactly the asymmetry
/// finding #11 named.
#[tokio::test]
async fn composite_filter_rejects_malformed_leaf_field_path_among_valid_leaves() {
    let (_handle, mut client) = start_test_agent(PROJECT).await;
    let valid_leaf = field_filter("status", FieldFilterOp::Equal, str_val("active"));
    let malicious_leaf =
        field_filter("name; DROP TABLE documents; --", FieldFilterOp::Equal, str_val("x"));
    let composite = Filter {
        filter_type: Some(FilterType::CompositeFilter(CompositeFilterProto {
            op: CompositeFilterOp::And as i32,
            filters: vec![valid_leaf, malicious_leaf],
        })),
    };
    let req = run_query_with_filter("patients", composite);
    assert_run_query_rejected_with_field_path_message(&mut client, req).await;
}

// ---------------------------------------------------------------------------
// AC-AFP-04 write-path confirmation — write paths never call validate_field_path
// ---------------------------------------------------------------------------

/// @driving_port @real_io @us_a09 @AC-AFP-04
///
/// Feature: Writing a document with an unusual field name is unaffected by the query-filter validator
///   Given a document does not exist
///   When a caller creates it with a field literally named "age' OR '1'='1"
///   Then the creation succeeds and the field is stored and readable exactly as named
///
/// This is a light confirmation, not a new RED test — DESIGN already proved
/// analytically that write paths never call `validate_field_path` at all
/// (field names on a write become ordinary JSON object keys inside a single
/// bound `$N::jsonb` parameter, never raw-SQL-interpolated). This assertion
/// passes identically BEFORE and AFTER this feature's fix.
#[tokio::test]
async fn create_document_accepts_a_field_name_with_special_characters_write_path_unaffected() {
    let (_handle, mut client) = start_test_agent(PROJECT).await;
    let mut fields: HashMap<String, Value> = HashMap::new();
    fields.insert("age' OR '1'='1".to_string(), str_val("irrelevant-value"));
    let req = CreateDocumentRequest {
        parent: parent(),
        collection_id: "patients".to_string(),
        document_id: "patient-write-path".to_string(),
        document: Some(Document { name: String::new(), fields, ..Default::default() }),
        ..Default::default()
    };
    let created = client
        .create_document(tonic::Request::new(req))
        .await
        .expect("create_document must succeed — write paths never validate field-path charset")
        .into_inner();

    let fetched = client
        .get_document(tonic::Request::new(GetDocumentRequest { name: created.name.clone(), ..Default::default() }))
        .await
        .expect("get_document")
        .into_inner();
    let value = fetched.fields.get("age' OR '1'='1").expect("field must be stored under its literal name");
    assert!(matches!(&value.value_type, Some(ValueType::StringValue(v)) if v == "irrelevant-value"));
}

// ---------------------------------------------------------------------------
// Deliberate behavioral delta (feature-delta.md § Finding 3) — documented,
// not an oversight
// ---------------------------------------------------------------------------

/// @driving_port @real_io @us_a09
///
/// Feature: A consecutive-dot field path is now ACCEPTED, matching zero documents, not rejected
///   Given a "patients" collection has a document with field "status" set to "active"
///   When a caller queries that collection with a filter on field_path "order..amount" (consecutive dots)
///   Then the RunQuery call succeeds (no INVALID_ARGUMENT) and returns zero documents
///
/// This is a DELIBERATE, already-reviewed behavior change, not a
/// regression: the old, weak `embyr-agent`-local validator rejected
/// consecutive dots as its ONLY check. The spec-mandated charset guard
/// (`^[a-zA-Z_][a-zA-Z0-9_.]*$`) this feature adopts does not independently
/// forbid consecutive dots — `"order..amount"` consists entirely of
/// characters in the allowed class and passes. This is
/// `embyr-server`'s own existing, already-live, spec-conformant behavior
/// today (confirmed by `embyr-core`'s own
/// `spec_compliant_paths_always_accepted` proptest, which samples a
/// generator that can and does produce consecutive dots and asserts
/// `is_ok()`) — adopting it in `embyr-agent` makes the two binaries'
/// behavior IDENTICAL, not newly permissive. `"order..amount"` carries no
/// SQL-injection risk (it stays entirely within the safe charset); it
/// addresses a JSONB key that structurally cannot match any real stored
/// field, so the query correctly returns zero rows rather than erroring.
/// See `docs/feature/agent-field-path-validation/feature-delta.md` §
/// Finding 3 for the full investigation. The sibling test in
/// `us_a03_query_operations.rs`
/// (`query_with_malformed_field_path_rejected_before_data_read`) was
/// updated to use a different malformed payload (a space) precisely
/// because of this delta.
#[tokio::test]
async fn consecutive_dot_field_path_is_now_accepted_and_matches_zero_documents() {
    let (handle, mut client) = start_test_agent(PROJECT).await;
    sqlx::query(
        "INSERT INTO documents (project_id, collection_path, document_id, fields, version, create_time, update_time) \
         VALUES ($1,$2,$3,$4::jsonb,1,NOW(),NOW())",
    )
    .bind(PROJECT)
    .bind("patients")
    .bind("patient-1")
    .bind(r#"{"status":{"t":"S","v":"active"}}"#)
    .execute(&handle.pool)
    .await
    .unwrap();

    let filter = field_filter("order..amount", FieldFilterOp::Equal, int_val(100));
    let req = run_query_with_filter("patients", filter);
    let mut stream = client
        .run_query(tonic::Request::new(req))
        .await
        .expect("run_query must succeed — consecutive dots are within the spec charset, not rejected")
        .into_inner();
    let mut docs = vec![];
    while let Some(msg) = stream.message().await.expect("stream must not error") {
        if let Some(doc) = msg.document {
            docs.push(doc);
        }
    }
    assert_eq!(
        docs.len(),
        0,
        "no real document has a literal 'order..amount' JSONB key, so zero rows match — got: {docs:?}"
    );
}
