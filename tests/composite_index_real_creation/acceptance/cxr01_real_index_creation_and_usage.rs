//! CXR01 (US-01, Release 1, Walking Skeleton) — Alex's Composite Index Is a
//! Real, Working Postgres Index, Not Just a Metadata Row.
//!
//! Acceptance criteria verified here (feature-delta.md US-01, ADR-072):
//!   AC-CXR-01: `CreateIndex` against a real customer database results in a
//!              real Postgres index object existing on `documents` —
//!              verified directly against the customer database's own
//!              index catalog, never merely the `composite_indexes`
//!              metadata row.
//!   AC-CXR-02: the row's `status` reaches a genuinely-ready value only
//!              AFTER the real index has actually finished building —
//!              never synchronously on INSERT.
//!   AC-CXR-03: once ready, an `EXPLAIN` of the exact `RunQuery` shape the
//!              index was created for shows Postgres's planner choosing an
//!              index-based plan, not a full sequential scan.
//!   AC-CXR-04: a `field_path` outside the safe charset is rejected before
//!              any DDL is constructed or executed.
//!   AC-CXR-05 (regression guard): `firestore-composite-indexes-admin-api`'s
//!              own AC-CIX-01..04 and `composite-index-requirement-rules`'
//!              own AC-CIR-01..07 remain unchanged — proven by those
//!              features' own already-registered test suites, not
//!              duplicated here.
//!
//! Plus two regression/boundary guards this DISTILL session adds per DESIGN's
//! own peer-review condition and the task's own explicit gate-regression ask:
//!   - a query against a still-`building` (not yet `ready`) index must be
//!     rejected FAILED_PRECONDITION exactly as before (the existing,
//!     unmodified Firestore-parity gate).
//!   - ADR-072 Decision D's own named, NOT-claimed-correct boundary (a
//!     2-orderBy/no-filter shape) is encoded as an executable, explicitly
//!     "planner usage not asserted" scenario, not left as ADR prose alone.
//!
//! Driving port: admin HTTP :9090 (`POST/GET .../indexes`) + gRPC :8080
//! `RunQuery`/`CreateDocument`, via `SecurityRulesFullContext` — the SAME
//! production composition root `firestore-composite-indexes-admin-api`'s
//! own CIX01 uses (Pillar 3).

#![allow(unused_imports)]

#[path = "../common/mod.rs"]
mod common;
use common::{
    cix_prefixed_index_names, category_equal_score_desc_structured_query,
    explain_category_equal_electronics_order_by_score_desc, field_ref, index_is_valid,
    integer_value, make_channel, run_grpc_query, seed_document_via_grpc, string_value,
    wait_until_status_is_one_of, SecurityRulesFullContext,
};

use embyr_proto::firestore::{
    firestore_client::FirestoreClient,
    structured_query::{CollectionSelector, Direction, FieldReference, Order},
    StructuredQuery,
};

/// AC-CXR-01, AC-CXR-02, AC-CXR-03
///
/// @walking_skeleton @driving_port @real-io @US-01 @AC-CXR-01 @AC-CXR-02 @AC-CXR-03
#[tokio::test]
async fn creating_an_index_builds_a_real_postgres_index_that_the_planner_uses() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cxr01-ws").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));

    // Given: Trailmark's "products" collection has real documents.
    for (doc_id, category, score) in
        [("prod-1", "electronics", 200i64), ("prod-2", "electronics", 50), ("prod-3", "garden", 900)]
    {
        let mut fields = std::collections::HashMap::new();
        fields.insert("category".to_string(), string_value(category));
        fields.insert("score".to_string(), integer_value(score));
        seed_document_via_grpc(&mut client, &ctx.project_id, &ctx.api_key, "products", doc_id, fields).await;
    }

    // When: Alex calls CreateIndex for category ASC / score DESC.
    let create_body: serde_json::Value = reqwest::Client::new()
        .post(ctx.admin_url(&format!("/admin/v1/projects/{}/indexes", ctx.project_id)))
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
        .expect("create_composite_index request failed")
        .json()
        .await
        .expect("response body must be JSON");
    let index_id = create_body["id"].as_str().expect("response must carry an id").to_string();

    // Then (AC-CXR-02): status must NOT claim ready synchronously on INSERT
    // — it starts life as "building" and only becomes "ready" once the real
    // index build genuinely finishes.
    assert_eq!(
        create_body["status"], "building",
        "AC-CXR-02: status must never be 'ready' synchronously on INSERT, before any build step has run"
    );

    // And: the row eventually, genuinely transitions to "ready".
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
    assert_eq!(
        final_status.as_deref(),
        Some("ready"),
        "AC-CXR-02: the build must eventually complete and become genuinely ready"
    );

    // Then (AC-CXR-01): a REAL Postgres index now exists on trailmark-prod's
    // own documents table — verified directly against the index catalog,
    // never merely the composite_indexes metadata row.
    let real_index_names = cix_prefixed_index_names(&ctx.cust_pool).await;
    assert!(
        !real_index_names.is_empty(),
        "AC-CXR-01: a real Postgres index matching the created composite index's fields must exist \
         on trailmark-prod's own index catalog, not merely the composite_indexes metadata row"
    );
    for name in &real_index_names {
        assert!(
            index_is_valid(&ctx.cust_pool, name).await,
            "AC-CXR-01: the real index '{name}' must be genuinely valid (indisvalid = true), never a \
             leftover INVALID index from a dropped connection"
        );
    }

    // Then (AC-CXR-03): re-running the EXACT query the index was created
    // for now shows Postgres's own planner choosing an index-based plan —
    // proven via EXPLAIN (cheaper and more reliable than a wall-clock
    // timing assertion, per the task's own explicit guidance), not a
    // wall-clock measurement.
    let plan = explain_category_equal_electronics_order_by_score_desc(
        &ctx.cust_pool,
        &ctx.project_id,
        "products",
    )
    .await;
    let plan_text = plan.join("\n");
    assert!(
        plan_text.contains("Index Scan") || plan_text.contains("Index Only Scan"),
        "AC-CXR-03: expected an index-based plan for category==ASC/score DESC, got:\n{plan_text}"
    );
    assert!(
        real_index_names.iter().any(|n| plan_text.contains(n)),
        "AC-CXR-03: the plan must reference the NEW composite index by name, got:\n{plan_text}"
    );
    assert!(
        !plan_text.contains("Seq Scan"),
        "AC-CXR-03: must not fall back to a sequential scan once the index is ready, got:\n{plan_text}"
    );
}

/// AC-CXR-04
///
/// @error @driving_port @real-io @US-01 @AC-CXR-04
#[tokio::test]
async fn a_field_path_outside_the_safe_charset_is_rejected_before_any_ddl_runs() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cxr01-injection").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;

    let resp = reqwest::Client::new()
        .post(ctx.admin_url(&format!("/admin/v1/projects/{}/indexes", ctx.project_id)))
        .header("Cookie", &cookie)
        .json(&serde_json::json!({
            "collection_path": "products",
            "fields": [{"field": "category\"); DROP TABLE documents; --", "order": "ASC"}],
        }))
        .send()
        .await
        .expect("create_composite_index request failed");

    assert!(
        resp.status().is_client_error(),
        "AC-CXR-04: a field path outside validate_field_path's safe charset must be rejected before \
         any DDL statement is constructed or sent to the customer database, got {}",
        resp.status()
    );

    let real_index_names = cix_prefixed_index_names(&ctx.cust_pool).await;
    assert!(
        real_index_names.is_empty(),
        "AC-CXR-04: no DDL of any kind must ever be constructed or executed for a rejected field path, \
         found: {real_index_names:?}"
    );
}

/// Firestore-parity gate regression guard (`composite-index-requirement-
/// rules`'s own unmodified `is_index_ready`/`requires_composite_index` call
/// sites): a query against an index that exists but is still "building"
/// (not yet "ready") must be rejected FAILED_PRECONDITION exactly as if no
/// index existed at all — `is_index_ready` only ever counts `status =
/// 'ready'` rows, unchanged by this feature.
///
/// @regression @driving_port @real-io @US-01
#[tokio::test]
async fn a_query_against_a_still_building_index_is_rejected_exactly_as_before() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cxr01-building-gate").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));

    let mut fields = std::collections::HashMap::new();
    fields.insert("category".to_string(), string_value("electronics"));
    fields.insert("score".to_string(), integer_value(5));
    seed_document_via_grpc(&mut client, &ctx.project_id, &ctx.api_key, "products", "prod-1", fields).await;

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
    assert_eq!(
        create_body["status"], "building",
        "precondition (AC-CXR-02): the row must start life as 'building', not immediately 'ready'"
    );

    let sq = category_equal_score_desc_structured_query("products");
    let result = run_grpc_query(&mut client, &ctx.project_id, &ctx.api_key, sq).await;
    let err = result.expect_err(
        "a query against a still-building (not yet ready) index must be rejected FAILED_PRECONDITION, \
         exactly as if no index existed at all",
    );
    assert_eq!(err.code(), tonic::Code::FailedPrecondition, "got {:?}: {}", err.code(), err.message());
}

/// Peer-review-mandated regression guard (ADR-072 Decision D's own named,
/// explicitly-NOT-claimed-correct boundary; feature-delta.md § Peer-review
/// condition, binding on DISTILL): the equality-filter + trailing-sort-field
/// convention this feature's DDL builder uses must not be silently implied
/// correct for a DIFFERENT trigger shape it was never proven to serve — a
/// query ordered by 2 fields with NO filter at all. This proves such a
/// query still EXECUTES successfully once a (any) ready index exists for
/// the collection (the coarse, pre-existing `is_index_ready` granularity,
/// unchanged by this feature) — it deliberately does NOT assert that
/// Postgres's planner actually USES the new index for this specific shape.
/// Both remain real Postgres indexes that pass CreateIndex/DeleteIndex's own
/// ACs; planner selection for this shape is a named, deferred follow-up
/// (ADR-072 Decision D), not a regression this feature introduces — neither
/// shape has any real index today either.
///
/// @regression @not-asserted-boundary @US-01 @AC-CXR-01
#[tokio::test]
async fn a_two_orderby_no_filter_query_still_executes_once_an_index_exists_planner_usage_not_asserted()
{
    let ctx = SecurityRulesFullContext::new("trailmark-prod-cxr01-two-orderby-boundary").await;
    let cookie = ctx.seed_session("alex@trailmark.example", "Owner").await;
    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));

    let mut fields = std::collections::HashMap::new();
    fields.insert("category".to_string(), string_value("electronics"));
    fields.insert("score".to_string(), integer_value(5));
    seed_document_via_grpc(&mut client, &ctx.project_id, &ctx.api_key, "products", "prod-1", fields).await;

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

    let status = wait_until_status_is_one_of(
        &ctx,
        &cookie,
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
        "precondition (AC-CXR-02): the index must genuinely finish building before this boundary check runs"
    );

    let sq = StructuredQuery {
        from: vec![CollectionSelector { collection_id: "products".to_string(), all_descendants: false }],
        order_by: vec![
            Order { field: Some(field_ref("category")), direction: Direction::Ascending as i32 },
            Order { field: Some(field_ref("score")), direction: Direction::Descending as i32 },
        ],
        ..Default::default()
    };
    let result = run_grpc_query(&mut client, &ctx.project_id, &ctx.api_key, sq).await;
    assert!(
        result.is_ok(),
        "the 2-orderBy/no-filter query must still execute successfully once a (any) ready index exists \
         for this collection — planner USAGE for this specific shape is deliberately not asserted here \
         (ADR-072 Decision D's own named limitation), got: {:?}",
        result.err()
    );
}
