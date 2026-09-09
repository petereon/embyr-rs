//! CXR03 (US-03, Release 2, LAST slice) — Deleting a Composite Index
//! Removes the Real Postgres Index, Not Just the Metadata Row.
//!
//! Acceptance criteria verified here (feature-delta.md US-03, ADR-072):
//!   AC-CXR-10: deleting a composite index whose `status` is ready (a real
//!              index exists) removes BOTH the `composite_indexes`
//!              metadata row AND the real Postgres index object from the
//!              customer database.
//!   AC-CXR-11: deleting a composite index that is still building or has
//!              failed does not error, regardless of whether a real
//!              (possibly partial) index object exists yet.
//!   AC-CXR-12 (regression guard): a subsequent query that newly requires
//!              the deleted index fails FAILED_PRECONDITION again,
//!              unchanged from `firestore-composite-indexes-admin-api`'s
//!              own AC-CIX-08 guarantee — now also true because the real
//!              underlying index storage is genuinely gone, not merely the
//!              metadata row.
//!
//! Driving port: admin HTTP :9090 (`POST/GET/DELETE .../indexes`) + gRPC
//! :8080 `RunQuery`/`CreateDocument`, via `SecurityRulesFullContext`.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    category_equal_score_desc_structured_query, cix_prefixed_index_names, insert_agent_mode_project,
    integer_value, make_channel, run_grpc_query, seed_document_via_grpc, string_value,
    wait_until_status_is_one_of, SecurityRulesFullContext,
};

use embyr_proto::firestore::firestore_client::FirestoreClient;

async fn create_ready_index(ctx: &SecurityRulesFullContext, cookie: &str) -> String {
    let create_body: serde_json::Value = reqwest::Client::new()
        .post(ctx.admin_url(&format!("/admin/v1/projects/{}/indexes", ctx.project_id)))
        .header("Cookie", cookie)
        .json(&serde_json::json!({
            "collection_path": "products",
            "fields": [{"field": "category", "order": "ASC"}, {"field": "score", "order": "DESC"}],
        }))
        .send()
        .await
        .expect("create request failed")
        .json()
        .await
        .expect("JSON body");
    let index_id = create_body["id"].as_str().expect("response must carry an id").to_string();

    let status = wait_until_status_is_one_of(
        ctx,
        cookie,
        &ctx.project_id,
        &index_id,
        &["ready", "failed"],
        100,
        std::time::Duration::from_millis(100),
    )
    .await;
    assert_eq!(
        status.as_deref(),
        Some("ready"),
        "precondition (AC-CXR-02): the index must genuinely finish building before this scenario runs, \
         got: {status:?}"
    );
    index_id
}

/// AC-CXR-10
///
/// @driving_port @real-io @US-03 @AC-CXR-10
#[tokio::test]
async fn deleting_a_ready_index_removes_the_real_postgres_index_too() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cxr03-delete-real").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let index_id = create_ready_index(&ctx, &cookie).await;

    let before_names = cix_prefixed_index_names(&ctx.cust_pool).await;
    assert!(
        !before_names.is_empty(),
        "precondition (AC-CXR-01): a real Postgres index must exist before this delete proof runs, \
         got: {before_names:?}"
    );

    let delete_resp = reqwest::Client::new()
        .delete(ctx.admin_url(&format!("/admin/v1/projects/{}/indexes/{}", ctx.project_id, index_id)))
        .header("Cookie", &cookie)
        .send()
        .await
        .expect("delete request failed");
    assert_eq!(delete_resp.status().as_u16(), 204, "delete must succeed");

    let after_names = cix_prefixed_index_names(&ctx.cust_pool).await;
    assert!(
        after_names.is_empty(),
        "AC-CXR-10: the real Postgres index must be gone from trailmark-prod's own index catalog after \
         delete, not merely the composite_indexes metadata row, got: {after_names:?}"
    );
}

/// AC-CXR-11 (still-building sub-case)
///
/// @driving_port @real-io @US-03 @AC-CXR-11
#[tokio::test]
async fn deleting_an_index_that_is_still_building_does_not_error() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cxr03-delete-building").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;

    let create_body: serde_json::Value = reqwest::Client::new()
        .post(ctx.admin_url(&format!("/admin/v1/projects/{}/indexes", ctx.project_id)))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "products",
            "fields": [{"field": "category", "order": "ASC"}, {"field": "score", "order": "DESC"}],
        }))
        .send()
        .await
        .expect("create request failed")
        .json()
        .await
        .expect("JSON body");
    let index_id = create_body["id"].as_str().expect("response must carry an id").to_string();

    // Deliberately delete without waiting for the build to finish — this
    // is the crash-recovery path ADR-072 Decision C names ("a stuck
    // 'building' row recovers via the already-designed US-03 delete-then
    // -recreate path").
    let delete_resp = reqwest::Client::new()
        .delete(ctx.admin_url(&format!("/admin/v1/projects/{}/indexes/{}", ctx.project_id, index_id)))
        .header("Cookie", &cookie)
        .send()
        .await
        .expect("delete request failed");
    assert_eq!(
        delete_resp.status().as_u16(),
        204,
        "AC-CXR-11: deleting a still-building index must succeed without error, regardless of whether a \
         real (possibly partial or INVALID) index object exists yet"
    );
}

/// AC-CXR-11 (failed-build sub-case)
///
/// @error @driving_port @real-io @US-03 @AC-CXR-11
#[tokio::test]
async fn deleting_an_index_whose_build_previously_failed_does_not_error() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cxr03-delete-failed").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let agent_project = "trailmark-agent-cxr03-delete-failed";
    insert_agent_mode_project(&ctx, agent_project).await;

    let create_body: serde_json::Value = reqwest::Client::new()
        .post(ctx.admin_url(&format!("/admin/v1/projects/{agent_project}/indexes")))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "products",
            "fields": [{"field": "category", "order": "ASC"}],
        }))
        .send()
        .await
        .expect("create request failed")
        .json()
        .await
        .expect("JSON body");
    let index_id = create_body["id"].as_str().expect("response must carry an id").to_string();

    let status = wait_until_status_is_one_of(
        &ctx,
        &cookie,
        agent_project,
        &index_id,
        &["ready", "failed"],
        50,
        std::time::Duration::from_millis(100),
    )
    .await;
    assert_eq!(
        status.as_deref(),
        Some("failed"),
        "precondition (AC-CXR-09): the build must genuinely fail for an unreachable backend, got: {status:?}"
    );

    let delete_resp = reqwest::Client::new()
        .delete(ctx.admin_url(&format!("/admin/v1/projects/{agent_project}/indexes/{index_id}")))
        .header("Cookie", &cookie)
        .send()
        .await
        .expect("delete request failed");
    assert_eq!(
        delete_resp.status().as_u16(),
        204,
        "AC-CXR-11: deleting a failed-build index must succeed cleanly — there is no real index to \
         drop, and its absence is not treated as an error"
    );
}

/// AC-CXR-12 (regression guard — mirrors `firestore-composite-indexes-
/// admin-api`'s own `deleting_the_only_ready_index_makes_a_dependent_query_
/// fail_again`; this feature does not change the gate itself, only whether
/// a REAL index also backed the deleted row).
///
/// @regression @driving_port @real-io @US-03 @AC-CXR-12
#[tokio::test]
async fn a_query_newly_requiring_the_deleted_index_fails_again_unchanged() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cxr03-regression").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));

    let mut fields = std::collections::HashMap::new();
    fields.insert("category".to_string(), string_value("electronics"));
    fields.insert("score".to_string(), integer_value(5));
    seed_document_via_grpc(&mut client, &ctx.project_id, &ctx.api_key, "products", "prod-1", fields).await;

    let index_id = create_ready_index(&ctx, &cookie).await;

    let sq = category_equal_score_desc_structured_query("products");
    let before = run_grpc_query(&mut client, &ctx.project_id, &ctx.api_key, sq.clone()).await;
    assert!(
        before.is_ok(),
        "precondition (AC-CXR-12): the query must succeed while the ready index exists, got: {:?}",
        before.err()
    );

    let delete_resp = reqwest::Client::new()
        .delete(ctx.admin_url(&format!("/admin/v1/projects/{}/indexes/{}", ctx.project_id, index_id)))
        .header("Cookie", &cookie)
        .send()
        .await
        .expect("delete request failed");
    assert_eq!(delete_resp.status().as_u16(), 204);

    let after = run_grpc_query(&mut client, &ctx.project_id, &ctx.api_key, sq).await;
    let err = after.expect_err(
        "AC-CXR-12: once the only index for this collection is deleted, the SAME query must fail \
         FAILED_PRECONDITION again — no dependency-safety check blocks the delete",
    );
    assert_eq!(err.code(), tonic::Code::FailedPrecondition, "got {:?}: {}", err.code(), err.message());
}
