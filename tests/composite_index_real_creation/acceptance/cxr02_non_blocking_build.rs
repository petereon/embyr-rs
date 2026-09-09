//! CXR02 (US-02, Release 1) — Building a Composite Index Never Blocks
//! Writes to a Large, Live Collection.
//!
//! Acceptance criteria verified here (feature-delta.md US-02, ADR-072):
//!   AC-CXR-06: creating a composite index against a collection under
//!              active, concurrent write traffic does not block, fail, or
//!              measurably stall those writes while the build is in
//!              progress.
//!   AC-CXR-07: while a composite index is building, its `status` is a
//!              value distinguishable from both `'ready'` and a terminal
//!              failure state.
//!   AC-CXR-08: once the build genuinely completes, `status` transitions to
//!              a value distinguishable as ready, and the `RunQuery` gate
//!              begins admitting matching queries that were previously
//!              rejected — proving the transition is truthful, not merely
//!              time-delayed.
//!   AC-CXR-09: if the build genuinely fails, the row's `status` reflects a
//!              state distinguishable from both "building" and "ready".
//!
//! Driving port: admin HTTP :9090 (`POST/GET .../indexes`) + gRPC :8080
//! `RunQuery`/`CreateDocument`, via `SecurityRulesFullContext`.

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    category_equal_score_desc_structured_query, current_index_status, insert_agent_mode_project,
    integer_value, make_channel, run_grpc_query, seed_bulk_products, seed_document_via_grpc,
    string_value, wait_until_status_is_one_of, SecurityRulesFullContext,
};

use embyr_proto::firestore::firestore_client::FirestoreClient;

/// AC-CXR-06, AC-CXR-07
///
/// @driving_port @real-io @US-02 @AC-CXR-06 @AC-CXR-07
#[tokio::test]
async fn creating_an_index_against_a_large_live_collection_does_not_block_concurrent_writes() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cxr02-non-blocking").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));

    // Given: "products" already holds a few thousand documents — large
    // enough that a write-blocking plain `CREATE INDEX` would leave an
    // observable lock window, unlike `CREATE INDEX CONCURRENTLY`'s
    // non-blocking behavior (ADR-072 Decision B).
    seed_bulk_products(&ctx.cust_pool, &ctx.project_id, 3000).await;

    // When: an operator creates a new composite index for that collection.
    let create_body: serde_json::Value = reqwest::Client::new()
        .post(ctx.admin_url(&format!("/admin/v1/projects/{}/indexes", ctx.project_id)))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "products",
            "fields": [{"field": "category", "order": "ASC"}, {"field": "score", "order": "DESC"}],
        }))
        .send()
        .await
        .expect("create_composite_index request failed")
        .json()
        .await
        .expect("response body must be JSON");
    let index_id = create_body["id"].as_str().expect("response must carry an id").to_string();

    // While the build is still genuinely in progress...
    let observed = wait_until_status_is_one_of(
        &ctx,
        &cookie,
        &ctx.project_id,
        &index_id,
        &["building"],
        20,
        std::time::Duration::from_millis(50),
    )
    .await;
    assert_eq!(
        observed.as_deref(),
        Some("building"),
        "AC-CXR-07: the build must be observably 'building' — distinguishable from 'ready' — at some \
         point before it completes"
    );

    // Then: a normal write to the SAME collection, issued via the real
    // gRPC driving port, completes without waiting for the build.
    let mut fields = std::collections::HashMap::new();
    fields.insert("category".to_string(), string_value("electronics"));
    fields.insert("score".to_string(), integer_value(1));
    seed_document_via_grpc(&mut client, &ctx.project_id, &ctx.api_key, "products", "prod-during-build", fields)
        .await;
    // seed_document_via_grpc already asserts success internally via .expect
    // — AC-CXR-06's own proof is that this call above returned at all
    // while the build was still "building", not merely that it eventually
    // succeeded.

    // And: the build itself is unaffected by the concurrent write — it
    // still eventually reaches a genuine "ready".
    let final_status = wait_until_status_is_one_of(
        &ctx,
        &cookie,
        &ctx.project_id,
        &index_id,
        &["ready", "failed"],
        200,
        std::time::Duration::from_millis(100),
    )
    .await;
    assert_eq!(
        final_status.as_deref(),
        Some("ready"),
        "the build must still succeed after the concurrent write, got {final_status:?}"
    );
}

/// AC-CXR-07 (standalone, minimal proof — distinguishability itself, not
/// tangled with the large-collection/concurrent-write scenario above).
///
/// @driving_port @real-io @US-02 @AC-CXR-07
#[tokio::test]
async fn listing_indexes_immediately_after_create_shows_a_status_distinct_from_ready() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cxr02-distinguish").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;

    let create_body: serde_json::Value = reqwest::Client::new()
        .post(ctx.admin_url(&format!("/admin/v1/projects/{}/indexes", ctx.project_id)))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "products",
            "fields": [{"field": "category", "order": "ASC"}],
        }))
        .send()
        .await
        .expect("create_composite_index request failed")
        .json()
        .await
        .expect("response body must be JSON");
    let index_id = create_body["id"].as_str().expect("response must carry an id").to_string();

    let status = current_index_status(&ctx, &cookie, &ctx.project_id, &index_id).await;
    assert_ne!(
        status.as_deref(),
        Some("ready"),
        "AC-CXR-07: status immediately after create must be distinguishable from 'ready' — an operator \
         polling ListIndexes cannot mistake 'still building' for 'safe to rely on now', got: {status:?}"
    );
}

/// AC-CXR-08
///
/// @driving_port @real-io @US-02 @AC-CXR-08
#[tokio::test]
async fn status_genuinely_transitions_to_ready_and_unblocks_the_matching_query() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cxr02-transition").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));

    let mut fields = std::collections::HashMap::new();
    fields.insert("category".to_string(), string_value("electronics"));
    fields.insert("score".to_string(), integer_value(5));
    seed_document_via_grpc(&mut client, &ctx.project_id, &ctx.api_key, "products", "prod-1", fields).await;

    let sq = category_equal_score_desc_structured_query("products");
    let before = run_grpc_query(&mut client, &ctx.project_id, &ctx.api_key, sq.clone()).await;
    let err = before.expect_err("the query must fail FAILED_PRECONDITION before any matching index exists");
    assert_eq!(err.code(), tonic::Code::FailedPrecondition, "got {:?}: {}", err.code(), err.message());

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

    let final_status = wait_until_status_is_one_of(
        &ctx,
        &cookie,
        &ctx.project_id,
        &index_id,
        &["ready", "failed"],
        100,
        std::time::Duration::from_millis(100),
    )
    .await;
    assert_eq!(final_status.as_deref(), Some("ready"), "AC-CXR-08: the build must genuinely complete");

    let after = run_grpc_query(&mut client, &ctx.project_id, &ctx.api_key, sq).await;
    assert!(
        after.is_ok(),
        "AC-CXR-08: once status is genuinely 'ready', the SAME query must now succeed, got: {:?}",
        after.err()
    );
}

/// AC-CXR-09
///
/// @error @driving_port @real-io @US-02 @AC-CXR-09
#[tokio::test]
async fn a_build_against_an_unreachable_backend_mode_ends_in_a_distinct_failed_status() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cxr02-failed-build").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let agent_project = "trailmark-agent-cxr02-failed-build";
    insert_agent_mode_project(&ctx, agent_project).await;

    let create_resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!("/admin/v1/projects/{agent_project}/indexes")))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "products",
            "fields": [{"field": "category", "order": "ASC"}],
        }))
        .send()
        .await
        .expect("create_composite_index request failed");
    assert_eq!(
        create_resp.status().as_u16(),
        200,
        "the row must still be created (200) even for a backend_mode this feature cannot reach — it \
         starts 'building' and later, honestly, transitions to 'failed', never a silent immediate \
         rejection"
    );
    let create_body: serde_json::Value = create_resp.json().await.expect("JSON body");
    let index_id = create_body["id"].as_str().expect("response must carry an id").to_string();

    let final_status = wait_until_status_is_one_of(
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
        final_status.as_deref(),
        Some("failed"),
        "AC-CXR-09: a build with no reachable customer database must end in 'failed', never a silent \
         'ready', and never left stuck ambiguously in 'building' forever, got: {final_status:?}"
    );
}
