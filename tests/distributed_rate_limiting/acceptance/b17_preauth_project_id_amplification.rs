// @driving_port @real-io @US-01 @property
#![allow(unused_imports)]
//! US-01 (preauth-db-amplification, finding #14, High/Reliability) — a
//! charset-invalid `project_id` must be rejected BEFORE `RateLimiter::check()`'s
//! 3-round-trip `check_pg` path (UPDATE + SELECT EXISTS + INSERT) ever runs,
//! for every gRPC call site including `Listen` (the one site that does not
//! funnel through `FirestoreService::extract_project_id`).
//!
//! Acceptance criteria verified here:
//!   AC-PDA-01: charset-invalid project_id -> zero DB round trips.
//!   AC-PDA-02: known, provisioned project_id -> unaffected (regression).
//!   AC-PDA-03: eventual rejection contract (Status/message) unchanged.
//!   AC-PDA-05: well-formed-but-unprovisioned project_id -> residual NOT closed.
//!   AC-PDA-06: sustained garbage flood -> zero measurable DB activity.
//!   (Example 4, pre-existing): empty project_id -> already zero round trips.
//!
//! Driving port: gRPC :8080 (tonic FirestoreClient -> GetDocument, Listen).
//! Assertion level: real Postgres container (testcontainers-rs) + in-process
//! gRPC server, using the SAME Postgres-backed RateLimiter production wiring
//! (`start_distributed_grpc_server` -> `RateLimiter::with_pg`).
//!
//! DB-round-trip observability mechanism (see common/mod.rs::metric_value doc
//! comment for the full rationale): the ALREADY-EXISTING
//! `embyr_rate_limit_requests_total{project_id="unconfirmed",outcome="allowed"}`
//! counter (OBS-04/ADR-069), scraped from the real admin :9090 `/metrics`
//! endpoint. It increments exactly once per `RateLimiter::check()`
//! INVOCATION for any never-before-seen project_id — a delta of 0 across a
//! request is direct proof `check()`/`check_pg`'s 3-round-trip path never
//! ran; a delta of 1 is direct proof it did.
//!
//! Two alternatives were tried first and rejected:
//!   - `pg_stat_user_tables` scan/insert counters — empirically measured
//!     (this session) to lag commits by Postgres's own internal
//!     `PGSTAT_MIN_INTERVAL` (~1s) stats flush, producing false-zero deltas.
//!   - `rate_buckets` row EXISTENCE — invalid for garbage/never-provisioned
//!     ids specifically: `rate_buckets.project_id` has a FOREIGN KEY to
//!     `projects(id)` (migration 0018), so `check_pg`'s own
//!     `INSERT ... ON CONFLICT DO NOTHING` silently fails (swallowed by
//!     `let _ =`) whenever no `projects` row exists — meaning row-count
//!     stays 0 whether or not the 3-round-trip path ran, for exactly the
//!     inputs this feature cares about most. (Row existence remains valid
//!     and is still used below for the KNOWN-project case, where a real
//!     `projects` row exists and the FK is satisfied.)
//!
//! Not #[ignore]: this feature is one atomic guard-clause diff across 3
//! functions (DESIGN § Decision — no independently-shippable slices), so all
//! scenarios here are enabled together, matching DELIVER's single TDD cycle.
//! Expected RED today against unfixed code (see doc comment on each test for
//! why); AC-PDA-02/03/05/Example-4 tests are regression guards already true
//! today (pre-fix) and must stay true after the fix — included here so
//! DELIVER's diff cannot silently break them.

#[path = "../common/mod.rs"]
mod common;
use common::{
    classify_grpc_status, garbage_project_ids, get_document_request, get_metrics, metric_value,
    start_distributed_grpc_server, DrlTestContext,
};
use embyr_proto::firestore::{
    firestore_client::FirestoreClient,
    listen_request,
    structured_query::CollectionSelector,
    target::{self, query_target},
    GetDocumentRequest, ListenRequest, StructuredQuery, Target,
};

fn add_target_request(project_id: &str, collection: &str) -> ListenRequest {
    ListenRequest {
        database: format!("projects/{project_id}/databases/(default)"),
        target_change: Some(listen_request::TargetChange::AddTarget(Target {
            target_id: 1,
            target_type: Some(target::TargetType::Query(target::QueryTarget {
                parent: format!("projects/{project_id}/databases/(default)/documents"),
                query_type: Some(query_target::QueryType::StructuredQuery(StructuredQuery {
                    from: vec![CollectionSelector {
                        collection_id: collection.to_string(),
                        all_descendants: false,
                    }],
                    ..Default::default()
                })),
            })),
            ..Default::default()
        })),
        ..Default::default()
    }
}

/// Current value of the "a never-before-seen project_id was checked at all"
/// counter — see module doc comment.
async fn unconfirmed_allowed_total(admin_client: &reqwest::Client, admin_addr: std::net::SocketAddr) -> f64 {
    let body = get_metrics(admin_client, admin_addr).await;
    metric_value(
        &body,
        "embyr_rate_limit_requests_total",
        &[("project_id", "unconfirmed"), ("outcome", "allowed")],
    )
}

// ─── Walking skeleton ──────────────────────────────────────────────────────

/// Walking skeleton (DISCUSS § Walking Skeleton Strategy): a real flood of
/// charset-invalid `project_id` GetDocument requests never invokes
/// `RateLimiter::check()` at all, while a known provisioned project's
/// request in the same run is unaffected (tokens still decrement normally).
///
/// AC-PDA-01, AC-PDA-02, AC-PDA-06
///
/// Journey:
///   Given: an unauthenticated caller sends 5 GetDocument requests using
///          project_id values that can never match any real project's naming
///          rules ("DROP_TABLE_123", "' OR 1=1--", a 500-char string, ...)
///   When:  all 5 requests are sent
///   Then:  the shared database's rate-limit-check counter shows zero new checks
///   Given: Fernbank Analytics, a provisioned project with an existing bucket
///   When:  Fernbank Analytics sends a GetDocument request using its own project_id
///   Then:  its token count still decrements normally (unaffected by this fix)
///
/// RED today: unfixed `extract_project_id` (handler.rs:99-107) only checks
/// non-emptiness, so each garbage id reaches `rate_limiter.check()` and
/// increments the "unconfirmed" counter once per garbage id sent.
///
/// @walking_skeleton @driving_port @real-io @property @US-01 @AC-PDA-01 @AC-PDA-02 @AC-PDA-06
#[tokio::test]
async fn garbage_project_id_flood_never_invokes_rate_limiter_check_while_known_project_unaffected()
{
    let ctx = DrlTestContext::new(10.0).await;
    ctx.insert_project_with_bucket("fernbank-analytics", "fernbank-key", 10.0)
        .await;
    let (addr, server) = start_distributed_grpc_server(&ctx).await;
    let mut client = FirestoreClient::new(
        tonic::transport::Channel::from_shared(format!("http://{addr}"))
            .expect("valid gRPC channel URI")
            .connect_lazy(),
    );
    let admin_client = reqwest::Client::new();

    let before = unconfirmed_allowed_total(&admin_client, server.admin_addr).await;

    for garbage in garbage_project_ids() {
        let name = format!("projects/{garbage}/databases/(default)/documents/widgets/doc1");
        let mut req = tonic::Request::new(GetDocumentRequest {
            name,
            ..Default::default()
        });
        req.metadata_mut().insert(
            "authorization",
            "bearer unauthenticated-flood-key"
                .parse()
                .expect("valid metadata value"),
        );
        let _ = client.get_document(req).await;
    }

    let after_flood = unconfirmed_allowed_total(&admin_client, server.admin_addr).await;
    assert_eq!(
        after_flood, before,
        "AC-PDA-01/06: a flood of {} charset-invalid project_id requests must never \
         invoke RateLimiter::check() — unconfirmed/allowed counter must not move \
         (before={before}, after={after_flood})",
        garbage_project_ids().len(),
    );

    // Known, provisioned project's request in the SAME run remains unaffected:
    // tokens still decrement via check_pg's existing Some(remaining) branch.
    let (tokens_before, _) = ctx
        .query_rate_bucket("fernbank-analytics")
        .await
        .expect("fernbank-analytics bucket must already exist");
    let result = client
        .get_document(get_document_request("fernbank-analytics", "fernbank-key"))
        .await;
    assert_ne!(
        classify_grpc_status(&result),
        "resource_exhausted",
        "AC-PDA-02: known project with 10.0 tokens must not be rejected"
    );
    let (tokens_after, _) = ctx
        .query_rate_bucket("fernbank-analytics")
        .await
        .expect("fernbank-analytics bucket must still exist");
    assert!(
        tokens_after < tokens_before,
        "AC-PDA-02: a known project's request must still decrement its token count \
         exactly as before this fix — before={tokens_before}, after={tokens_after}"
    );
}

// ─── AC-PDA-01: single-request isolation (easier to debug than the flood) ───

/// A single charset-invalid project_id GetDocument request never invokes
/// `RateLimiter::check()`.
///
/// AC-PDA-01
///
/// RED today: same reason as the walking skeleton, isolated to one request.
///
/// @driving_port @real-io @US-01 @AC-PDA-01
#[tokio::test]
async fn single_garbage_project_id_get_document_never_invokes_rate_limiter_check() {
    let ctx = DrlTestContext::new(10.0).await;
    let (addr, server) = start_distributed_grpc_server(&ctx).await;
    let mut client = FirestoreClient::new(
        tonic::transport::Channel::from_shared(format!("http://{addr}"))
            .expect("valid gRPC channel URI")
            .connect_lazy(),
    );
    let admin_client = reqwest::Client::new();

    let before = unconfirmed_allowed_total(&admin_client, server.admin_addr).await;

    let result = client
        .get_document(get_document_request("' OR 1=1--", "attacker-key"))
        .await;
    let _ = classify_grpc_status(&result);

    let after = unconfirmed_allowed_total(&admin_client, server.admin_addr).await;
    assert_eq!(
        after, before,
        "a single charset-invalid project_id must never invoke RateLimiter::check() \
         (before={before}, after={after})"
    );
}

// ─── AC-PDA-03: rejection contract is byte-identical, only earlier ─────────

/// A charset-invalid project_id's GetDocument call returns the exact same
/// `Status::invalid_argument` with the exact same message text as today —
/// this assertion is ALREADY true against unfixed code (both paths reuse
/// `ProjectId::new(...).map_err(...)` verbatim; DESIGN's fix only moves the
/// existing `authenticate()` check earlier). Included as a regression guard,
/// not a RED-required test — it must stay true after DELIVER's diff lands.
///
/// AC-PDA-03
///
/// @driving_port @real-io @US-01 @AC-PDA-03
#[tokio::test]
async fn garbage_project_id_get_document_returns_same_invalid_argument_status_as_before_the_fix() {
    let ctx = DrlTestContext::new(10.0).await;
    let (addr, _server) = start_distributed_grpc_server(&ctx).await;
    let mut client = FirestoreClient::new(
        tonic::transport::Channel::from_shared(format!("http://{addr}"))
            .expect("valid gRPC channel URI")
            .connect_lazy(),
    );

    let result = client
        .get_document(get_document_request("Not_Lowercase", "attacker-key"))
        .await;
    let status = result.expect_err("charset-invalid project_id must be rejected");
    assert_eq!(status.code(), tonic::Code::InvalidArgument);
    assert_eq!(
        status.message(),
        "invalid argument: project id must match ^[a-z][a-z0-9-]{0,62}$, got: Not_Lowercase",
        "AC-PDA-03: rejection message must be byte-identical to authenticate()'s own \
         pre-existing ProjectId::new check — got: {:?}",
        status.message()
    );
}

// ─── AC-PDA-05: well-formed-but-unprovisioned residual is NOT closed ───────

/// A syntactically valid but never-provisioned project_id still invokes
/// `RateLimiter::check()`'s full 3-round-trip `check_pg` path — this is
/// Investigation 3's accepted residual (OQ-PDA-02), NOT built by this
/// feature. This test protects against DELIVER accidentally
/// over-implementing a closure of this residual.
///
/// AC-PDA-05
///
/// Already true both before AND after this fix — the guard added by this
/// feature is a pure charset check and cannot distinguish "well-formed and
/// real" from "well-formed and never provisioned".
///
/// @driving_port @real-io @US-01 @AC-PDA-05
#[tokio::test]
async fn well_formed_but_unprovisioned_project_id_still_invokes_rate_limiter_check() {
    let ctx = DrlTestContext::new(10.0).await;
    let (addr, server) = start_distributed_grpc_server(&ctx).await;
    let mut client = FirestoreClient::new(
        tonic::transport::Channel::from_shared(format!("http://{addr}"))
            .expect("valid gRPC channel URI")
            .connect_lazy(),
    );
    let admin_client = reqwest::Client::new();

    let before = unconfirmed_allowed_total(&admin_client, server.admin_addr).await;

    let result = client
        .get_document(get_document_request("acme-corp-2026", "guessing-key"))
        .await;
    let _ = classify_grpc_status(&result);

    let after = unconfirmed_allowed_total(&admin_client, server.admin_addr).await;
    assert_eq!(
        after,
        before + 1.0,
        "AC-PDA-05: a well-formed-but-unprovisioned project_id must still invoke \
         RateLimiter::check() exactly once — this residual is deliberately NOT \
         closed by this feature (before={before}, after={after})"
    );
}

// ─── Pre-existing boundary (Example 4): empty project_id, unaffected ───────

/// An empty project_id (`projects//databases/...`) is already rejected by
/// `extract_project_id` before ever reaching the rate limiter TODAY —
/// unaffected by, and not claimed as part of, this fix.
///
/// Already true both before AND after this fix.
///
/// @driving_port @real-io @US-01 @regression
#[tokio::test]
async fn empty_project_id_is_rejected_before_reaching_rate_limiter_exactly_as_today() {
    let ctx = DrlTestContext::new(10.0).await;
    let (addr, server) = start_distributed_grpc_server(&ctx).await;
    let mut client = FirestoreClient::new(
        tonic::transport::Channel::from_shared(format!("http://{addr}"))
            .expect("valid gRPC channel URI")
            .connect_lazy(),
    );
    let admin_client = reqwest::Client::new();

    let before = unconfirmed_allowed_total(&admin_client, server.admin_addr).await;

    let mut req = tonic::Request::new(GetDocumentRequest {
        name: "projects//databases/(default)/documents/widgets/doc1".to_string(),
        ..Default::default()
    });
    req.metadata_mut().insert(
        "authorization",
        "bearer whatever-key".parse().expect("valid metadata value"),
    );
    let status = client
        .get_document(req)
        .await
        .expect_err("empty project_id segment must be rejected");
    assert_eq!(status.code(), tonic::Code::InvalidArgument);

    let after = unconfirmed_allowed_total(&admin_client, server.admin_addr).await;
    assert_eq!(
        after, before,
        "empty project_id must never reach RateLimiter::check() — pre-existing behavior"
    );
}

// ─── Listen RPC: the one call site outside extract_project_id ─────────────

/// The `Listen` RPC resolves its project_id via
/// `extract_project_id_from_listen_request` (handler.rs:3900), NOT
/// `FirestoreService::extract_project_id` — it needs its own guard.
///
/// AC-PDA-01 (extended to the Listen call site)
///
/// RED today: unfixed `extract_project_id_from_listen_request` only checks
/// non-emptiness, so a garbage project_id reaches `rate_limiter.check()`
/// exactly like the 14 other handlers did before their own fix.
///
/// @driving_port @real-io @US-01 @AC-PDA-01
#[tokio::test]
async fn listen_rpc_with_garbage_project_id_never_invokes_rate_limiter_check() {
    let ctx = DrlTestContext::new(10.0).await;
    let (addr, server) = start_distributed_grpc_server(&ctx).await;
    let mut client = FirestoreClient::new(
        tonic::transport::Channel::from_shared(format!("http://{addr}"))
            .expect("valid gRPC channel URI")
            .connect_lazy(),
    );
    let admin_client = reqwest::Client::new();

    let before = unconfirmed_allowed_total(&admin_client, server.admin_addr).await;

    let mut request = tonic::Request::new(tokio_stream::once(add_target_request(
        "DROP_TABLE_123",
        "widgets",
    )));
    request.metadata_mut().insert(
        "authorization",
        "bearer flood-key".parse().expect("valid metadata value"),
    );
    let result = client.listen(request).await;
    // Any gRPC status is acceptable here — what matters is zero DB activity,
    // not the exact status (Listen's own error surface is out of this
    // feature's scope; AC-PDA-01 only requires the round-trip elimination).
    let _ = result;

    let after = unconfirmed_allowed_total(&admin_client, server.admin_addr).await;
    assert_eq!(
        after, before,
        "AC-PDA-01: Listen with a charset-invalid project_id must never invoke \
         RateLimiter::check(), same as every other gRPC call site"
    );
}
