// @driving_port @real-io @US-01
#![allow(unused_imports)]
//! US-01 (preauth-db-amplification, finding #14) — REST/gRPC-Web surface.
//!
//! `rest_rate_limit_middleware` (rate_limit.rs:377-393) is a THIRD, independent
//! choke point (does not go through `handler.rs::extract_project_id` at all —
//! axum's own `Path` extractor supplies `project_id` directly). Its guard must
//! be ACTION-AWARE: `signInWithCustomToken` has its own distinct rejection
//! shape (400 MALFORMED_TOKEN, via `rest::sign_in::malformed_response`) that
//! does NOT match every other action's shape (401 INVALID_API_KEY, via
//! `rest::sign_up::invalid_api_key`) — DESIGN's own peer review iteration 1
//! caught that an unconditional guard would silently break
//! `signInWithCustomToken`'s contract (AC-PDA-03).
//!
//! Acceptance criteria verified here:
//!   AC-PDA-01/AC-PDA-03: charset-invalid project_id -> zero rate_buckets
//!     rows created, per action, with the SAME response shape that action
//!     already produces today (only earlier).
//!   AC-PDA-02: known, provisioned project_id -> unaffected (regression).
//!   AC-PDA-05: well-formed-but-unprovisioned project_id -> residual NOT closed.
//!
//! Driving port: REST :8081 — `POST /v1/projects/{project_id}/accounts:<action>`.
//! Assertion level: real Postgres container (testcontainers-rs) + in-process
//! REST server, same Postgres-backed RateLimiter wiring as the gRPC suite
//! (`start_distributed_grpc_server` starts both `grpc_addr` and `rest_addr`).
//!
//! DB-round-trip observability: the `embyr_rate_limit_requests_total{
//! project_id="unconfirmed",outcome="allowed"}` counter (see b17's doc
//! comment and common/mod.rs::metric_value for the full rationale, including
//! why row-existence in `rate_buckets` and a `pg_stat_user_tables` snapshot
//! were both tried and rejected). Row existence IS still valid, and used
//! below, for the KNOWN-project test — a real `projects` row exists there,
//! so the `rate_buckets` FK is satisfied and the row genuinely persists.
//!
//! Not #[ignore]: single atomic guard-clause diff (see b17). Contract/shape
//! assertions here are already true against unfixed code (both response
//! builders are reused verbatim by DESIGN) — the counter-delta assertions
//! are the RED-required part.

#[path = "../common/mod.rs"]
mod common;
use common::{
    garbage_project_ids, get_metrics, metric_value, start_distributed_grpc_server, DrlTestContext,
};

fn accounts_url(rest_addr: std::net::SocketAddr, project_id: &str, action: &str) -> String {
    format!("http://{rest_addr}/v1/projects/{project_id}/accounts:{action}")
}

/// Current value of the "a never-before-seen project_id was checked at all"
/// counter — see b17's module doc comment.
async fn unconfirmed_allowed_total(admin_client: &reqwest::Client, admin_addr: std::net::SocketAddr) -> f64 {
    let body = get_metrics(admin_client, admin_addr).await;
    metric_value(
        &body,
        "embyr_rate_limit_requests_total",
        &[("project_id", "unconfirmed"), ("outcome", "allowed")],
    )
}

// ─── AC-PDA-01/03: signInWithCustomToken -> 400 MALFORMED_TOKEN ────────────

/// A charset-invalid project_id on `signInWithCustomToken` returns 400
/// MALFORMED_TOKEN (its own distinct shape — NOT the other actions' 401
/// INVALID_API_KEY) and creates no rate_buckets row for it.
///
/// A non-empty `token` is supplied so that, against UNFIXED code, the request
/// reaches `sign_in_with_custom_token`'s own credential lookup (which finds
/// no row for a garbage project_id and returns this SAME `malformed_response()`)
/// rather than short-circuiting on `MISSING_TOKEN` — this is what makes the
/// status/body assertions below valid regression guards (true before AND
/// after the fix), isolating the RED signal to the row-existence assertion.
///
/// AC-PDA-01, AC-PDA-03
///
/// RED today: unfixed `rest_rate_limit_middleware` has no charset guard, so
/// `rate_limiter.check()` runs check_pg's 3-round-trip path (creating a
/// persisted row) before the request ever reaches the handler.
///
/// @driving_port @real-io @US-01 @AC-PDA-01 @AC-PDA-03
#[tokio::test]
async fn garbage_project_id_sign_in_with_custom_token_returns_malformed_token_with_no_db_row() {
    let ctx = DrlTestContext::new(10.0).await;
    let (_addr, server) = start_distributed_grpc_server(&ctx).await;
    let client = reqwest::Client::new();
    let admin_client = reqwest::Client::new();

    let before = unconfirmed_allowed_total(&admin_client, server.admin_addr).await;

    let response = client
        .post(accounts_url(
            server.rest_addr,
            "DROP_TABLE_123",
            "signInWithCustomToken",
        ))
        .json(&serde_json::json!({ "token": "some-opaque-custom-token" }))
        .send()
        .await
        .expect("POST accounts:signInWithCustomToken");

    assert_eq!(response.status().as_u16(), 400, "expected 400 Bad Request");
    let body: serde_json::Value = response.json().await.expect("valid JSON body");
    assert_eq!(
        body["reason"], "MALFORMED_TOKEN",
        "AC-PDA-03: signInWithCustomToken's OWN rejection shape must be preserved \
         verbatim — got: {body}"
    );

    let after = unconfirmed_allowed_total(&admin_client, server.admin_addr).await;
    assert_eq!(
        after, before,
        "AC-PDA-01: a charset-invalid project_id on signInWithCustomToken must never \
         invoke RateLimiter::check() (before={before}, after={after})"
    );
    let _ = &ctx; // held for container lifetime only
}

// ─── AC-PDA-01/03: every other action -> 401 INVALID_API_KEY ───────────────

/// A charset-invalid project_id on `signUp` (representative of every action
/// OTHER than `signInWithCustomToken`) returns 401 INVALID_API_KEY and
/// creates no rate_buckets row — a DIFFERENT shape than
/// signInWithCustomToken's, proving the guard is action-aware, not a single
/// unconditional response.
///
/// No `?key=` is supplied, which ALSO produces 401 INVALID_API_KEY on unfixed
/// code (sign_up's own first check, before any project/customer-DB lookup) —
/// making the status/body assertions valid regression guards independent of
/// the fix.
///
/// AC-PDA-01, AC-PDA-03
///
/// RED today: same reason as the signInWithCustomToken test above.
///
/// @driving_port @real-io @US-01 @AC-PDA-01 @AC-PDA-03
#[tokio::test]
async fn garbage_project_id_sign_up_returns_invalid_api_key_with_no_db_row() {
    let ctx = DrlTestContext::new(10.0).await;
    let (_addr, server) = start_distributed_grpc_server(&ctx).await;
    let client = reqwest::Client::new();
    let admin_client = reqwest::Client::new();

    let before = unconfirmed_allowed_total(&admin_client, server.admin_addr).await;

    let response = client
        .post(accounts_url(server.rest_addr, "' OR 1=1--", "signUp"))
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("POST accounts:signUp");

    assert_eq!(response.status().as_u16(), 401, "expected 401 Unauthorized");
    let body: serde_json::Value = response.json().await.expect("valid JSON body");
    assert_eq!(
        body["reason"], "INVALID_API_KEY",
        "AC-PDA-03: every action other than signInWithCustomToken must keep its \
         existing INVALID_API_KEY shape — got: {body}"
    );

    let after = unconfirmed_allowed_total(&admin_client, server.admin_addr).await;
    assert_eq!(
        after, before,
        "AC-PDA-01: a charset-invalid project_id on signUp must never invoke \
         RateLimiter::check() (before={before}, after={after})"
    );
    let _ = &ctx; // held for container lifetime only
}

// ─── AC-PDA-02: known, provisioned project_id is unaffected ────────────────

/// A syntactically valid, ALREADY-provisioned project_id's REST request is
/// unaffected by this fix — its existing single-round-trip UPDATE path runs
/// exactly as before (token count decrements, no new row is created).
///
/// AC-PDA-02
///
/// Already true both before AND after this fix (the guard is a pass-through
/// no-op for any well-formed project_id) — included as a regression guard.
///
/// @driving_port @real-io @US-01 @AC-PDA-02
#[tokio::test]
async fn known_provisioned_project_id_rest_request_is_unaffected_by_the_guard() {
    let ctx = DrlTestContext::new(10.0).await;
    ctx.insert_project_with_bucket("known-rest-project", "known-rest-key", 10.0)
        .await;
    let (_addr, server) = start_distributed_grpc_server(&ctx).await;
    let client = reqwest::Client::new();

    let (tokens_before, _) = ctx
        .query_rate_bucket("known-rest-project")
        .await
        .expect("known-rest-project bucket must already exist");

    let response = client
        .post(accounts_url(server.rest_addr, "known-rest-project", "signUp"))
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("POST accounts:signUp");

    assert_ne!(
        response.status().as_u16(),
        429,
        "a known project with available tokens must not be rate-limited"
    );

    let (tokens_after, _) = ctx
        .query_rate_bucket("known-rest-project")
        .await
        .expect("known-rest-project bucket must still exist");
    assert!(
        tokens_after < tokens_before,
        "AC-PDA-02: a known project's REST request must still decrement its token \
         count exactly as before this fix — before={tokens_before}, after={tokens_after}"
    );
    assert_eq!(
        ctx.rate_bucket_row_count("known-rest-project").await,
        1,
        "AC-PDA-02: a known project's REST request must never create a second row"
    );
}

// ─── AC-PDA-05: well-formed-but-unprovisioned residual, REST side ──────────

/// A syntactically valid but never-provisioned project_id still reaches the
/// full 3-round-trip `check_pg` path via REST — this residual (OQ-PDA-02) is
/// deliberately NOT closed by this feature. Guards against DELIVER
/// accidentally over-implementing a closure of this residual on the REST side.
///
/// AC-PDA-05
///
/// Already true both before AND after this fix.
///
/// @driving_port @real-io @US-01 @AC-PDA-05
#[tokio::test]
async fn well_formed_but_unprovisioned_project_id_still_reaches_shared_database_via_rest() {
    let ctx = DrlTestContext::new(10.0).await;
    let (_addr, server) = start_distributed_grpc_server(&ctx).await;
    let client = reqwest::Client::new();
    let admin_client = reqwest::Client::new();

    let before = unconfirmed_allowed_total(&admin_client, server.admin_addr).await;

    let response = client
        .post(accounts_url(server.rest_addr, "acme-corp-2026-rest", "signUp"))
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("POST accounts:signUp");

    // No ?key= supplied -> sign_up's own first check fires regardless of
    // project provisioning state -> 401 INVALID_API_KEY, unaffected by
    // whether the guard exists or not (same shape either way).
    assert_eq!(response.status().as_u16(), 401);

    let after = unconfirmed_allowed_total(&admin_client, server.admin_addr).await;
    assert_eq!(
        after,
        before + 1.0,
        "AC-PDA-05: the residual path must still invoke RateLimiter::check() via \
         REST (before={before}, after={after})"
    );
    let _ = &ctx; // held for container lifetime only
}
