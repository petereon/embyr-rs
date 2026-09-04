//! CIX02 (Slice 02, US-02, Release 1) — Alex Lists His Project's Own
//! Composite Indexes.
//!
//! Acceptance criteria verified here (feature-delta.md US-02, ADR-068):
//!   AC-CIX-05: `GET /admin/v1/projects/:project_id/indexes` returns every
//!              `composite_indexes` row for that project (any role,
//!              read-only).
//!   AC-CIX-06: a project with zero indexes returns an empty list, never
//!              an error.
//!
//! Driving port: admin HTTP :9090, via `SecurityRulesFullContext`.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::SecurityRulesFullContext;

/// AC-CIX-06
///
/// @driving_port @real-io @US-02 @AC-CIX-06
#[tokio::test]
async fn a_project_with_zero_indexes_returns_an_empty_list() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cix02-empty").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;

    let resp = reqwest::Client::new()
        .get(ctx.admin_url(&format!("/admin/v1/projects/{}/indexes", ctx.project_id)))
        .header("Cookie", &cookie)
        .send()
        .await
        .expect("list_composite_indexes request failed");
    assert_eq!(resp.status().as_u16(), 200);
    let body: Vec<serde_json::Value> = resp.json().await.expect("response body must be a JSON array");
    assert!(body.is_empty(), "AC-CIX-06: expected an empty list, got: {body:?}");
}

/// AC-CIX-05
///
/// @driving_port @real-io @US-02 @AC-CIX-05
#[tokio::test]
async fn listing_returns_every_index_for_the_project_any_role() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cix02-populated").await;
    let owner_cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;

    for (collection, field) in [("products", "category"), ("orders", "status")] {
        let resp = reqwest::Client::new()
            .post(ctx.admin_url(&format!("/admin/v1/projects/{}/indexes", ctx.project_id)))
            .header("Cookie", &owner_cookie)
            .json(&serde_json::json!({
                "collection_path": collection,
                "fields": [{"field": field, "order": "ASC"}],
            }))
            .send()
            .await
            .expect("create_composite_index request failed");
        assert_eq!(resp.status().as_u16(), 200);
    }

    // A Viewer (any role, read-only per AC-CIX-05) can list.
    let viewer_cookie = ctx.seed_session("dana@trailmark.example", "Viewer").await;
    let list_resp = reqwest::Client::new()
        .get(ctx.admin_url(&format!("/admin/v1/projects/{}/indexes", ctx.project_id)))
        .header("Cookie", &viewer_cookie)
        .send()
        .await
        .expect("list_composite_indexes request failed");
    assert_eq!(list_resp.status().as_u16(), 200, "AC-CIX-05: a Viewer must be able to list");
    let body: Vec<serde_json::Value> = list_resp.json().await.expect("response body must be a JSON array");
    assert_eq!(body.len(), 2, "expected both created indexes to be listed, got: {body:?}");
    let collections: std::collections::BTreeSet<&str> =
        body.iter().map(|i| i["collection_path"].as_str().unwrap()).collect();
    assert_eq!(
        collections,
        std::collections::BTreeSet::from(["products", "orders"])
    );
}
