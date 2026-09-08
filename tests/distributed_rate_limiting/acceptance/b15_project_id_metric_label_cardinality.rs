// @driving_port @real-io @adapter-integration @US-RLV-01
#![allow(unused_imports)]
//! US-RLV-01 — Unauthenticated, attacker-chosen project_id strings must not grow
//! embyr_rate_limit_requests_total's own Prometheus label cardinality unboundedly.
//!
//! Acceptance criteria verified here:
//!   AC-RLV-01: sending N requests with N distinct arbitrary/never-provisioned project_id
//!              strings before authentication must not cause N new, unique project_id label
//!              values to be recorded for embyr_rate_limit_requests_total.
//!
//! Driving port: gRPC :8080 (tonic FirestoreClient -> GetDocument, unauthenticated — the
//!               rate limiter runs before authenticate()) then Admin :9090 (GET /metrics).
//! Assertion level: real Postgres container (testcontainers-rs) + in-process gRPC/admin servers —
//!               exercises RateLimiter::check() -> check_inner() -> check_pg() exactly as
//!               production does (ADR-069's own mechanism reuses check_pg's existence check).
//!
//! Mechanism under test (docs/product/architecture/adr-069-rate-limit-metric-label-cardinality-bounding.md):
//!   today, RateLimiter::check() (crates/embyr-server/src/middleware/rate_limit.rs:140-150)
//!   labels the metric with the RAW project_id unconditionally. This test proves that gap is
//!   still open (RED) — DELIVER wires `known_existing` and the "unconfirmed" sentinel label.
//!
//! This test is intentionally NOT #[ignore] — it is expected to be RED today (the
//! vulnerability is real and unfixed) and GREEN once DELIVER lands ADR-069's fix.

#[path = "../common/mod.rs"]
mod common;
use common::{
    get_metrics, rate_limit_metric_project_id_labels, start_distributed_grpc_server, DrlTestContext,
};
use embyr_proto::firestore::firestore_client::FirestoreClient;
use embyr_proto::firestore::GetDocumentRequest;

/// A flood of never-provisioned, distinct garbage project_ids collapses onto a
/// small, bounded set of metric labels instead of growing one-per-request.
///
/// AC-RLV-01
///
/// Journey:
///   Given no authenticated session exists and none of the following project_ids
///     were ever provisioned via POST /admin/v1/projects
///   When an attacker sends 25 GetDocument requests, each carrying a distinct,
///     freshly-generated random string as the project_id in the resource path
///   Then embyr_rate_limit_requests_total gains at most 1 new distinct project_id
///     label value attributable to these 25 requests, not 25
///
/// @driving_port @real-io @US-RLV-01 @AC-RLV-01
#[tokio::test]
async fn flood_of_garbage_project_ids_does_not_grow_metric_label_cardinality() {
    let ctx = DrlTestContext::new(10.0).await;
    let (_addr, server) = start_distributed_grpc_server(&ctx).await;

    let admin_client = reqwest::Client::new();
    let before_body = get_metrics(&admin_client, server.admin_addr).await;
    let before_labels = rate_limit_metric_project_id_labels(&before_body);

    let channel = tonic::transport::Channel::from_shared(format!("http://{}", server.grpc_addr))
        .expect("valid gRPC channel URI")
        .connect_lazy();
    let mut client = FirestoreClient::new(channel);

    const N: usize = 25;
    let mut sent_ids = Vec::with_capacity(N);
    for _ in 0..N {
        let garbage_project_id = format!("attacker-{}", uuid::Uuid::new_v4());
        sent_ids.push(garbage_project_id.clone());
        let name = format!("projects/{garbage_project_id}/databases/(default)/documents/x/doc1");
        let mut req = tonic::Request::new(GetDocumentRequest {
            name,
            ..Default::default()
        });
        // A syntactically-valid but never-registered bearer key — just enough to pass
        // extract_api_key() (handler.rs:160-173, which runs before rate_limiter.check())
        // without ever succeeding authenticate(). This is the pre-auth, unauthenticated
        // path the rate limiter itself runs on (handler.rs: check() at :1267, before
        // authenticate() at :1272), matching obs04_rate_limit_metrics.rs's own
        // make_grpc_call() convention ("counter fires regardless").
        req.metadata_mut().insert(
            "authorization",
            "bearer attacker-supplied-key"
                .parse()
                .expect("valid metadata value"),
        );
        let _ = client.get_document(req).await;
    }

    let after_body = get_metrics(&admin_client, server.admin_addr).await;
    let after_labels = rate_limit_metric_project_id_labels(&after_body);

    let new_labels: std::collections::HashSet<_> =
        after_labels.difference(&before_labels).cloned().collect();

    // Vulnerable today: every one of the 25 distinct garbage project_ids becomes its
    // own permanent label -> new_labels.len() == 25. Fixed: they all collapse onto
    // the "unconfirmed" sentinel -> new_labels.len() <= 1.
    assert!(
        new_labels.len() <= 1,
        "expected at most 1 new project_id label (the bounded sentinel) for {N} distinct \
         never-provisioned project_ids, got {} new label(s): {:?} — Prometheus label \
         cardinality is growing unboundedly with attacker-chosen input (finding #2, ADR-069 unfixed)",
        new_labels.len(),
        new_labels
    );

    // None of the attacker's own literal, distinct strings should ever become a
    // permanent label value — that is precisely the unbounded-growth vector.
    let leaked_raw_ids: Vec<_> = sent_ids
        .iter()
        .filter(|id| new_labels.contains(*id))
        .collect();
    assert!(
        leaked_raw_ids.is_empty(),
        "attacker-chosen project_id strings leaked directly into Prometheus labels: {leaked_raw_ids:?}"
    );
}
