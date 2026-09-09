//! CIX03 (Slice 03, US-03, Release 1, LAST slice) — Alex Deletes a
//! Composite Index He No Longer Needs.
//!
//! Acceptance criteria verified here (feature-delta.md US-03, ADR-068):
//!   AC-CIX-07: `DELETE .../indexes/:index_id` removes the row; a
//!              subsequent `ListIndexes` no longer includes it.
//!   AC-CIX-08: deleting an index does NOT check whether a live query
//!              still depends on it — the next such query simply fails
//!              `FAILED_PRECONDITION` again.
//!   AC-CIX-09: only Owner/Admin roles may call `DeleteIndex`.
//!
//! Driving port: admin HTTP :9090 + gRPC :8080 `RunQuery`, via
//! `SecurityRulesFullContext`.

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

fn field_ref(path: &str) -> FieldReference {
    FieldReference { field_path: path.to_string() }
}

/// composite-index-real-creation (ADR-072) superseded synchronous
/// `status='ready'` with a genuine async build — `create_index` waits for
/// the row to reach 'ready' before returning, so every caller here keeps
/// its own original "the index is usable" precondition true, unchanged.
async fn create_index(ctx: &SecurityRulesFullContext, cookie: &str) -> String {
    let resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!("/admin/v1/projects/{}/indexes", ctx.project_id)))
        .header("Cookie", cookie)
        .json(&serde_json::json!({
            "collection_path": "products",
            "fields": [{"field": "category", "order": "ASC"}, {"field": "score", "order": "DESC"}],
        }))
        .send()
        .await
        .expect("create_composite_index request failed");
    assert_eq!(resp.status().as_u16(), 200);
    let body: serde_json::Value = resp.json().await.expect("JSON body");
    let index_id = body["id"].as_str().unwrap().to_string();

    for _ in 0..100 {
        let list_resp = reqwest::Client::new()
            .get(ctx.admin_url(&format!("/admin/v1/projects/{}/indexes", ctx.project_id)))
            .header("Cookie", cookie)
            .send()
            .await
            .expect("list_composite_indexes request failed");
        let rows: Vec<serde_json::Value> = list_resp.json().await.expect("JSON array");
        if let Some(row) = rows.iter().find(|r| r["id"] == index_id) {
            if row["status"] == "ready" {
                return index_id;
            }
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("index {index_id} did not become ready in time");
}

/// AC-CIX-07
///
/// @driving_port @real-io @US-03 @AC-CIX-07
#[tokio::test]
async fn deleting_an_index_removes_it_from_a_subsequent_list() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cix03-delete").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let index_id = create_index(&ctx, &cookie).await;

    let delete_resp = reqwest::Client::new()
        .delete(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/indexes/{}",
            ctx.project_id, index_id
        )))
        .header("Cookie", &cookie)
        .send()
        .await
        .expect("delete_composite_index request failed");
    assert_eq!(delete_resp.status().as_u16(), 204, "AC-CIX-07: delete must succeed");

    let list_resp = reqwest::Client::new()
        .get(ctx.admin_url(&format!("/admin/v1/projects/{}/indexes", ctx.project_id)))
        .header("Cookie", &cookie)
        .send()
        .await
        .expect("list_composite_indexes request failed");
    let body: Vec<serde_json::Value> = list_resp.json().await.expect("JSON array");
    assert!(
        body.is_empty(),
        "AC-CIX-07: the deleted index must no longer appear in ListIndexes, got: {body:?}"
    );
}

/// AC-CIX-08
///
/// @driving_port @real-io @US-03 @AC-CIX-08
#[tokio::test]
async fn deleting_the_only_ready_index_makes_a_dependent_query_fail_again() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cix03-dependency").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));

    let mut fields = std::collections::HashMap::new();
    fields.insert("category".to_string(), string_value("B"));
    let parent = format!("projects/{}/databases/(default)/documents", ctx.project_id);
    client
        .create_document(make_authed_request(
            CreateDocumentRequest {
                parent: parent.clone(),
                collection_id: "products".to_string(),
                document_id: "prod-1".to_string(),
                document: Some(Document { name: String::new(), fields, ..Default::default() }),
                ..Default::default()
            },
            &ctx.api_key,
        ))
        .await
        .expect("seed document");

    let index_id = create_index(&ctx, &cookie).await;

    let sq = StructuredQuery {
        from: vec![CollectionSelector { collection_id: "products".to_string(), all_descendants: false }],
        r#where: Some(Filter {
            filter_type: Some(FilterType::FieldFilter(FieldFilter {
                field: Some(field_ref("category")),
                op: FieldOp::Equal as i32,
                value: Some(string_value("B")),
            })),
        }),
        order_by: vec![Order { field: Some(field_ref("score")), direction: Direction::Descending as i32 }],
        ..Default::default()
    };
    let before = client
        .run_query(make_authed_request(
            RunQueryRequest {
                parent: parent.clone(),
                query_type: Some(QueryType::StructuredQuery(sq.clone())),
                ..Default::default()
            },
            &ctx.api_key,
        ))
        .await;
    assert!(before.is_ok(), "AC-CIX-08: the query must succeed while the index is ready");

    reqwest::Client::new()
        .delete(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/indexes/{}",
            ctx.project_id, index_id
        )))
        .header("Cookie", &cookie)
        .send()
        .await
        .expect("delete_composite_index request failed");

    let after = client
        .run_query(make_authed_request(
            RunQueryRequest { parent, query_type: Some(QueryType::StructuredQuery(sq)), ..Default::default() },
            &ctx.api_key,
        ))
        .await;
    let err = after.expect_err(
        "AC-CIX-08: once the only ready index for this collection is deleted, the SAME query must \
         fail FAILED_PRECONDITION again — no dependency-safety check blocks the delete",
    );
    assert_eq!(err.code(), tonic::Code::FailedPrecondition, "got {:?}: {}", err.code(), err.message());
}

/// AC-CIX-09
///
/// @error @driving_port @real-io @US-03 @AC-CIX-09
#[tokio::test]
async fn a_non_admin_role_cannot_delete_an_index() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cix03-role-gate").await;
    let owner_cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let index_id = create_index(&ctx, &owner_cookie).await;

    let viewer_cookie = ctx.seed_session("dana@trailmark.example", "Viewer").await;
    let resp = reqwest::Client::new()
        .delete(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/indexes/{}",
            ctx.project_id, index_id
        )))
        .header("Cookie", &viewer_cookie)
        .send()
        .await
        .expect("delete_composite_index request failed");
    assert_eq!(resp.status().as_u16(), 403, "AC-CIX-09: a Viewer role must be rejected");
}

/// AC-CIX-09's own boundary: `Admin` (not `Owner`) is also allowed — mirrors
/// CIX01's own identical `an_admin_role_exactly_can_create_an_index` proof.
///
/// @driving_port @real-io @US-03 @AC-CIX-09
#[tokio::test]
async fn an_admin_role_exactly_can_delete_an_index() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cix03-admin-role").await;
    let owner_cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let index_id = create_index(&ctx, &owner_cookie).await;

    let admin_cookie = ctx.seed_session("priya@trailmark.example", "Admin").await;
    let resp = reqwest::Client::new()
        .delete(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/indexes/{}",
            ctx.project_id, index_id
        )))
        .header("Cookie", &admin_cookie)
        .send()
        .await
        .expect("delete request failed");
    assert_eq!(resp.status().as_u16(), 204, "an exact Admin role must be allowed to delete");
}

/// Deleting a never-created (but well-formed) index id is a clean 404 —
/// `result.rows_affected() == 0` must be observably distinguishable from
/// the successful-delete path, never silently treated as success.
///
/// @error @driving_port @real-io @US-03
#[tokio::test]
async fn deleting_a_nonexistent_index_id_returns_404() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cix03-nonexistent").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;

    let never_created_id = uuid::Uuid::new_v4();
    let resp = reqwest::Client::new()
        .delete(ctx.admin_url(&format!(
            "/admin/v1/projects/{}/indexes/{}",
            ctx.project_id, never_created_id
        )))
        .header("Cookie", &cookie)
        .send()
        .await
        .expect("delete request failed");
    assert_eq!(resp.status().as_u16(), 404, "deleting a never-created index id must be a clean 404");
}
