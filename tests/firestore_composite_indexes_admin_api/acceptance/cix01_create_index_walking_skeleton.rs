//! CIX01 (Slice 01, Walking Skeleton, US-01, Release 1) — Alex Creates a
//! Composite Index and His Query Finally Succeeds.
//!
//! Acceptance criteria verified here (feature-delta.md US-01, ADR-068):
//!   AC-CIX-01: `POST /admin/v1/projects/:project_id/indexes` with a valid
//!              `{collection_path, fields}` body inserts a `composite_
//!              indexes` row with `status = 'ready'` and returns 200.
//!   AC-CIX-02: a real `RunQuery` that previously failed
//!              `FAILED_PRECONDITION` for the SAME collection succeeds
//!              immediately after `CreateIndex`, with zero change to any
//!              query-path code.
//!   AC-CIX-03: `CreateIndex` called twice with the IDENTICAL spec is
//!              idempotent — the second call returns 200 with the SAME
//!              existing row, never a 409 or raw DB error.
//!   AC-CIX-04: only Owner/Admin roles may call `CreateIndex`.
//!
//! Driving port: admin HTTP :9090 (`POST .../indexes`) + gRPC :8080
//! `RunQuery`, via `SecurityRulesFullContext` (an unrestricted collection —
//! no access rule defined — so a plain API-key Bearer request is
//! sufficient, mirroring `us_04_query_collection.rs`'s own simpler
//! auth shape).

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

fn filter_and_order_by_query(collection: &str) -> StructuredQuery {
    StructuredQuery {
        from: vec![CollectionSelector { collection_id: collection.to_string(), all_descendants: false }],
        r#where: Some(Filter {
            filter_type: Some(FilterType::FieldFilter(FieldFilter {
                field: Some(field_ref("category")),
                op: FieldOp::Equal as i32,
                value: Some(string_value("B")),
            })),
        }),
        order_by: vec![Order { field: Some(field_ref("score")), direction: Direction::Descending as i32 }],
        ..Default::default()
    }
}

/// composite-index-real-creation (ADR-072) superseded synchronous
/// `status='ready'` with a genuine async build — poll `ListIndexes` until
/// the row reaches a terminal state, mirroring
/// `composite_index_real_creation`'s own `wait_until_status_is_one_of`.
async fn wait_until_ready(ctx: &SecurityRulesFullContext, cookie: &str, project_id: &str, index_id: &str) {
    for _ in 0..100 {
        let resp = reqwest::Client::new()
            .get(ctx.admin_url(&format!("/admin/v1/projects/{project_id}/indexes")))
            .header("Cookie", cookie)
            .send()
            .await
            .expect("list_composite_indexes request failed");
        let body: Vec<serde_json::Value> = resp.json().await.expect("JSON array");
        if let Some(row) = body.iter().find(|r| r["id"] == index_id) {
            if row["status"] == "ready" {
                return;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("index {index_id} did not become ready in time");
}

async fn run_query(
    client: &mut FirestoreClient<tonic::transport::Channel>,
    project_id: &str,
    api_key: &str,
    sq: StructuredQuery,
) -> Result<Vec<Document>, tonic::Status> {
    let parent = format!("projects/{project_id}/databases/(default)/documents");
    let req = make_authed_request(
        RunQueryRequest { parent, query_type: Some(QueryType::StructuredQuery(sq)), ..Default::default() },
        api_key,
    );
    let stream = client.run_query(req).await?.into_inner();
    use tokio_stream::StreamExt;
    let responses: Vec<_> = stream.collect().await;
    // A gRPC-level error surfaces mid-stream as an Err item — collect_query_docs'
    // own precedent (us_04) silently drops errors, but this test needs to
    // observe FAILED_PRECONDITION directly, so the first error short-circuits.
    let mut docs = Vec::new();
    for r in responses {
        let r = r?;
        if let Some(doc) = r.document {
            docs.push(doc);
        }
    }
    Ok(docs)
}

/// AC-CIX-01, AC-CIX-02
///
/// @walking_skeleton @driving_port @real-io @US-01 @AC-CIX-01 @AC-CIX-02
#[tokio::test]
async fn creating_an_index_unblocks_a_previously_stuck_query() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cix01-walking-skeleton").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));

    let mut fields = std::collections::HashMap::new();
    fields.insert("category".to_string(), string_value("B"));
    fields.insert("score".to_string(), integer_value(200));
    seed_document(&mut client, &ctx.project_id, &ctx.api_key, "products", "prod-1", fields).await;

    let sq = filter_and_order_by_query("products");
    let before = run_query(&mut client, &ctx.project_id, &ctx.api_key, sq.clone()).await;
    let err = before.expect_err(
        "AC-CIX-02: a filter+orderBy-on-a-different-field query must fail FAILED_PRECONDITION \
         before any composite index exists",
    );
    assert_eq!(err.code(), tonic::Code::FailedPrecondition, "got {:?}: {}", err.code(), err.message());

    let create_resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/indexes",
            ctx.project_id
        )))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "products",
            "fields": [
                {"field": "category", "order": "ASC"},
                {"field": "score", "order": "DESC"},
            ],
        }))
        .send()
        .await
        .expect("create_composite_index request failed");
    assert_eq!(create_resp.status().as_u16(), 200, "AC-CIX-01: create must succeed");
    let create_body: serde_json::Value = create_resp.json().await.expect("response body must be JSON");
    // composite-index-real-creation (ADR-072) reverses this feature's own
    // original metadata-only stub, by design (feature-delta.md [D2]):
    // status now starts 'building', never synchronously 'ready' — the
    // underlying AC-CIX-01/02 intent (create succeeds, query then works)
    // is unchanged, proven below once the build genuinely completes.
    assert_eq!(create_body["status"], "building", "AC-CIX-01: status starts 'building', not synchronously 'ready'");
    let index_id = create_body["id"].as_str().expect("response must carry an id");
    wait_until_ready(&ctx, &cookie, &ctx.project_id, index_id).await;

    let after = run_query(&mut client, &ctx.project_id, &ctx.api_key, sq).await;
    let docs = after.expect("AC-CIX-02: the identical query must succeed once a matching index exists");
    assert!(!docs.is_empty(), "expected at least one document returned by the newly-indexed query");
}

/// AC-CIX-03
///
/// @driving_port @real-io @US-01 @AC-CIX-03
#[tokio::test]
async fn creating_the_identical_index_spec_twice_is_idempotent() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cix01-idempotent").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;

    let body = serde_json::json!({
        "collection_path": "products",
        "fields": [{"field": "category", "order": "ASC"}, {"field": "score", "order": "DESC"}],
    });

    let first = reqwest::Client::new()
        .post(ctx.admin_url(&format!("/admin/v1/projects/{}/indexes", ctx.project_id)))
        .header("Cookie", &cookie)
        .json(&body)
        .send()
        .await
        .expect("first create request failed");
    assert_eq!(first.status().as_u16(), 200);
    let first_body: serde_json::Value = first.json().await.expect("JSON body");
    let first_id = first_body["id"].clone();

    let second = reqwest::Client::new()
        .post(ctx.admin_url(&format!("/admin/v1/projects/{}/indexes", ctx.project_id)))
        .header("Cookie", &cookie)
        .json(&body)
        .send()
        .await
        .expect("second create request failed");
    assert_eq!(
        second.status().as_u16(),
        200,
        "AC-CIX-03: a duplicate spec must be idempotent, never a 409/error"
    );
    let second_body: serde_json::Value = second.json().await.expect("JSON body");
    assert_eq!(
        second_body["id"], first_id,
        "AC-CIX-03: the duplicate call must return the SAME existing row's own id"
    );
}

/// AC-CIX-04
///
/// @error @driving_port @real-io @US-01 @AC-CIX-04
#[tokio::test]
async fn a_non_admin_role_cannot_create_an_index() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cix01-role-gate").await;
    let cookie = ctx.seed_session("dana@trailmark.example", "Viewer").await;

    let resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!("/admin/v1/projects/{}/indexes", ctx.project_id)))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "products",
            "fields": [{"field": "category", "order": "ASC"}],
        }))
        .send()
        .await
        .expect("create request failed");
    assert_eq!(resp.status().as_u16(), 403, "AC-CIX-04: a Viewer role must be rejected");
}

/// AC-CIX-04's own boundary: `Admin` (not `Owner`) is also allowed — the
/// role check is `role < Role::Admin`, never previously exercised with the
/// EXACT `Admin` role by any other test here (only `Owner`/`Viewer`), the
/// one input shape that distinguishes `<` from `<=`.
///
/// @driving_port @real-io @US-01 @AC-CIX-04
#[tokio::test]
async fn an_admin_role_exactly_can_create_an_index() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cix01-admin-role").await;
    let cookie = ctx.seed_session("priya@trailmark.example", "Admin").await;

    let resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!("/admin/v1/projects/{}/indexes", ctx.project_id)))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "products",
            "fields": [{"field": "category", "order": "ASC"}],
        }))
        .send()
        .await
        .expect("create request failed");
    assert_eq!(resp.status().as_u16(), 200, "an exact Admin role must be allowed to create");
}
