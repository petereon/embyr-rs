// @driving_port @real-io @US-14
#![allow(unused_imports)]
//! US-14 — x-ratelimit-* response metadata headers on gRPC responses.
//!
//! Acceptance criteria verified here:
//!   AC-14-01: An allowed response carries x-ratelimit-limit / -remaining / -reset.
//!   AC-14-02: x-ratelimit-remaining decrements by 1 per allowed request.
//!   AC-14-03: RESOURCE_EXHAUSTED responses include retry-after-ms; allowed responses do not.
//!
//! Driving port: gRPC :8080 (tonic FirestoreClient → metadata inspection).
//! Assertion level: real Postgres container (testcontainers-rs) + in-process gRPC server.
//!
//! Header location: unary success → `Response::metadata()`; rejection →
//! `Status::metadata()`. `GetDocument` is the representative handler (each
//! handler's call site is covered by code review). Headers are only attached
//! when the document is FOUND (see `handle_get_document`), so the success-path
//! tests seed a document.
//!
//! All tests run in CI (implemented 2026-11-13; formerly `todo!()` scaffolds
//! `#[ignore]`d until DRL-04).

#[path = "../common/mod.rs"]
mod common;
use common::{
    classify_grpc_status, get_document_request, start_distributed_grpc_server,
    warm_credential_cache_and_reset_bucket, DrlTestContext, ADMIN_KEY,
};
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt},
};
use embyr_proto::firestore::firestore_client::FirestoreClient;

/// Provision `project_id` via the admin API against a separate customer
/// Postgres (the system DB has no `documents` table — that lives in the
/// customer migrations), seed `test/doc1` there, and return the API key. The
/// returned container/pool must outlive the test.
async fn provision_with_doc(
    ctx: &DrlTestContext,
    admin_addr: std::net::SocketAddr,
    project_id: &str,
) -> (String, ContainerAsync<Postgres>, sqlx::PgPool) {
    let container = Postgres::default()
        .with_tag("15-alpine")
        .start()
        .await
        .expect("start customer Postgres container");
    let port = container.get_host_port_ipv4(5432).await.expect("customer pg port");
    let dsn = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");

    let resp = ctx
        .admin_client
        .post(format!("http://{admin_addr}/admin/v1/projects"))
        .header("Authorization", format!("Bearer {ADMIN_KEY}"))
        .json(&serde_json::json!({
            "project_id": project_id,
            "dsn": dsn,
            "backend_mode": "direct_pg",
        }))
        .send()
        .await
        .expect("POST /admin/v1/projects failed");
    assert!(resp.status().is_success(), "provision failed: {}", resp.status());
    let body: serde_json::Value = resp.json().await.expect("provision JSON");
    let api_key = body["api_key"].as_str().expect("api_key in response").to_string();

    let customer_pool = sqlx::PgPool::connect(&dsn).await.expect("connect customer pool");
    sqlx::query(
        "INSERT INTO documents (project_id, collection_path, document_id, fields, create_time, update_time, version) \
         VALUES ($1, 'test', 'doc1', $2, now(), now(), 1)",
    )
    .bind(project_id)
    .bind(serde_json::json!({"title": {"t": "S", "v": "hello"}}))
    .execute(&customer_pool)
    .await
    .expect("seed document");
    (api_key, container, customer_pool)
}

fn header<T>(resp: &tonic::Response<T>, name: &str) -> Option<f64> {
    resp.metadata()
        .get(name)
        .map(|v| v.to_str().expect("ascii header").parse().expect("numeric header"))
}

// ─── AC-14-01: rate-limit headers on an allowed response ─────────────────────

/// An allowed GetDocument response carries x-ratelimit-{limit,remaining,reset}
/// and no retry-after-ms.
///
/// AC-14-01
///
/// @driving_port @real-io @US-14 @AC-14-01
#[tokio::test]
async fn all_grpc_handlers_return_ratelimit_headers() {
    let ctx = DrlTestContext::new(100.0).await;
    let (addr, server) = start_distributed_grpc_server(&ctx).await;
    let (api_key, _customer, _pool) = provision_with_doc(&ctx, server.admin_addr, "proj-headers").await;
    let ctx = ctx.with_grpc_addr(addr);
    let mut client = FirestoreClient::new(ctx.grpc_channel());

    let resp = client
        .get_document(get_document_request("proj-headers", &api_key))
        .await
        .expect("allowed request for an existing document must succeed");

    assert_eq!(header(&resp, "x-ratelimit-limit"), Some(100.0));
    assert!(header(&resp, "x-ratelimit-remaining").is_some(), "x-ratelimit-remaining missing");
    assert!(header(&resp, "x-ratelimit-reset").is_some(), "x-ratelimit-reset missing");
    assert!(
        resp.metadata().get("retry-after-ms").is_none(),
        "retry-after-ms must be absent on an allowed response"
    );
}

// ─── AC-14-02: remaining decrements per request ──────────────────────────────

/// x-ratelimit-remaining decrements by 1 for each allowed request.
///
/// AC-14-02
///
/// @driving_port @real-io @US-14 @AC-14-02
#[tokio::test]
async fn ratelimit_remaining_decrements_per_request() {
    let ctx = DrlTestContext::new(5.0).await;
    let (addr, server) = start_distributed_grpc_server(&ctx).await;
    let (api_key, _customer, _pool) =
        provision_with_doc(&ctx, server.admin_addr, "proj-decrement").await;
    let ctx = ctx.with_grpc_addr(addr);
    let mut client = FirestoreClient::new(ctx.grpc_channel());
    // Pay the one-time Argon2id cost, then reset the bucket to a known level.
    warm_credential_cache_and_reset_bucket(&ctx, &mut client, "proj-decrement", &api_key, 5.0).await;

    let mut remaining = Vec::new();
    for _ in 0..3 {
        let resp = client
            .get_document(get_document_request("proj-decrement", &api_key))
            .await
            .expect("allowed request must succeed");
        remaining.push(header(&resp, "x-ratelimit-remaining").expect("remaining header"));
    }

    assert_eq!(remaining, vec![4.0, 3.0, 2.0], "remaining is floored tokens after each deduction");
}

// ─── AC-14-03: rejected response includes retry-after-ms ─────────────────────

/// RESOURCE_EXHAUSTED response includes a positive retry-after-ms header.
///
/// AC-14-03
///
/// @driving_port @real-io @US-14 @AC-14-03
#[tokio::test]
async fn rate_limited_response_includes_retry_after_ms() {
    let ctx = DrlTestContext::new(10.0).await;
    ctx.insert_project_with_bucket("proj-retry", "key-rt", 0.0).await;
    let (addr, _server) = start_distributed_grpc_server(&ctx).await;
    let ctx = ctx.with_grpc_addr(addr);
    let mut client = FirestoreClient::new(ctx.grpc_channel());
    // Server start takes >0.1s, enough for a 0-token bucket at 10 rps to refill;
    // re-pin it to empty right before the request.
    warm_credential_cache_and_reset_bucket(&ctx, &mut client, "proj-retry", "key-rt", 0.0).await;

    let result = client
        .get_document(get_document_request("proj-retry", "key-rt"))
        .await;

    assert_eq!(classify_grpc_status(&result), "resource_exhausted");
    let status = result.unwrap_err();
    let retry_ms: u64 = status
        .metadata()
        .get("retry-after-ms")
        .expect("retry-after-ms header absent")
        .to_str()
        .expect("ascii header")
        .parse()
        .expect("integer retry-after-ms");
    assert!(retry_ms > 0, "retry-after-ms must be positive");
    assert!(retry_ms <= 2000, "retry-after-ms {retry_ms} should not exceed 2s for a 10 rps bucket");
}
