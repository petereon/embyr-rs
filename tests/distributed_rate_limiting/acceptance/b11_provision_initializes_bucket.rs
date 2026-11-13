// @driving_port @real-io @adapter-integration @US-11
//! US-11 — Provisioning creates rate_buckets row atomically.
//!
//! Acceptance criteria verified here:
//!   AC-11-01: POST /admin/v1/projects creates a rate_buckets row atomically with the projects row.
//!   AC-11-02: If rate_buckets INSERT fails, the projects INSERT is rolled back (no orphaned row).
//!
//! Driving port: admin HTTP :9090 (reqwest → POST /admin/v1/projects).
//! Assertion level: real Postgres container (testcontainers-rs).
//!
//! All tests run in CI (implemented 2026-11-13; formerly `todo!()` scaffolds).
//! AC-11-02 forces the `rate_buckets` INSERT to fail with a NOT VALID
//! `CHECK (false)` constraint (enforced for new rows only) rather than a PK
//! conflict, because the duplicate-id pre-check would short-circuit before the
//! transaction ever starts.

#[path = "../common/mod.rs"]
mod common;
use common::{start_distributed_grpc_server, DrlTestContext, ADMIN_KEY};
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt},
};

/// Start a separate, empty customer Postgres and return (container, DSN).
async fn customer_db() -> (ContainerAsync<Postgres>, String) {
    let c = Postgres::default()
        .with_tag("15-alpine")
        .start()
        .await
        .expect("start customer Postgres container");
    let port = c.get_host_port_ipv4(5432).await.expect("customer pg port");
    (c, format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres"))
}

/// POST /admin/v1/projects (direct_pg) and return the HTTP status.
async fn provision(ctx: &DrlTestContext, project_id: &str, dsn: &str) -> reqwest::StatusCode {
    ctx.admin_client
        .post(ctx.admin_url("/admin/v1/projects"))
        .header("Authorization", format!("Bearer {ADMIN_KEY}"))
        .json(&serde_json::json!({
            "project_id": project_id,
            "dsn": dsn,
            "backend_mode": "direct_pg",
        }))
        .send()
        .await
        .expect("POST /admin/v1/projects failed")
        .status()
}

// ─── AC-11-01: provisioning creates rate_bucket row ──────────────────────────

/// Provisioning a project creates a rate_buckets row in the same transaction.
///
/// AC-11-01
///
/// @driving_port @real-io @US-11 @AC-11-01
#[tokio::test]
async fn provision_creates_rate_bucket_row() {
    let ctx = DrlTestContext::new(10.0).await;
    let (_addr, server) = start_distributed_grpc_server(&ctx).await;
    let ctx = ctx.with_admin_url(format!("http://{}", server.admin_addr));
    let (_customer, dsn) = customer_db().await;

    let status = provision(&ctx, "proj-provisioned", &dsn).await;

    assert!(status.is_success(), "provision must succeed; got {status}");
    assert_eq!(ctx.rate_bucket_row_count("proj-provisioned").await, 1);
    let (tokens, _) = ctx
        .query_rate_bucket("proj-provisioned")
        .await
        .expect("rate_buckets row must exist");
    assert!(tokens > 0.0, "initial bucket must start with capacity, got {tokens}");
}

// ─── AC-11-02: provision atomicity — rate_buckets failure rolls back project ─

/// A failed rate_buckets INSERT during provisioning rolls back the projects row.
///
/// AC-11-02
///
/// @driving_port @real-io @US-11 @AC-11-02
#[tokio::test]
async fn provision_rate_bucket_is_transactional() {
    let ctx = DrlTestContext::new(10.0).await;
    sqlx::query("ALTER TABLE rate_buckets ADD CONSTRAINT force_insert_fail CHECK (false) NOT VALID")
        .execute(&ctx.pool)
        .await
        .expect("install failing constraint on rate_buckets");
    let (_addr, server) = start_distributed_grpc_server(&ctx).await;
    let ctx = ctx.with_admin_url(format!("http://{}", server.admin_addr));
    let (_customer, dsn) = customer_db().await;

    let status = provision(&ctx, "proj-orphan-check", &dsn).await;

    assert!(
        status.is_server_error(),
        "provision must fail when the rate_buckets INSERT fails; got {status}"
    );
    assert_eq!(ctx.project_row_count("proj-orphan-check").await, 0, "no orphaned projects row");
    assert_eq!(ctx.rate_bucket_row_count("proj-orphan-check").await, 0);
}
