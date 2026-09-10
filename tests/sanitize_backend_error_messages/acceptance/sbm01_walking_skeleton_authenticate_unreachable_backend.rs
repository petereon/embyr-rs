// SCAFFOLD: true
// @driving_port @walking_skeleton @real-io @US-01 @US-02 @AC-SBM-01 @AC-SBM-03 @AC-SBM-06 @AC-SBM-11
//! SBM-01 — Walking Skeleton: `authenticate()`'s direct_pg branch, hit by
//! EVERY authenticated RPC, never discloses driver text when the backend is
//! unreachable — and the real error stays server-observable via tracing
//! even though it was discarded from the client-facing `Status`
//! (feature-delta.md § DISCUSS Walking Skeleton Strategy; ADR-075).
//!
//! Driving port: gRPC :8080 (tonic `FirestoreClient` → `GetDocument`).
//!
//! Red classification (pre-fix): `authenticate()`'s direct_pg branch
//! (`crates/embyr-server/src/grpc/handler.rs:347`) does
//! `Status::internal(e.to_string())` with NO `tracing::error!` call at all
//! — so BOTH assertions below fail today: the client-facing message is the
//! raw connect-failure text (not the fixed generic string), and the
//! captured server-side tracing output gains nothing new during the call.

#[path = "../common/mod.rs"]
mod common;

use common::{authed_request, make_channel, tracing_capture, ServerTestContext};
use embyr_proto::firestore::firestore_client::FirestoreClient;
use embyr_proto::firestore::GetDocumentRequest;

/// @walking_skeleton @driving_port @real-io @AC-SBM-01 @AC-SBM-03 @AC-SBM-06
///
/// Feature: A backend failure during authentication never discloses driver text
///   Given project "sbm-ws-unreachable" is configured with backend_mode "direct_pg"
///     and a stored connection string pointing at an address nothing is listening on
///   When any client authenticates against "sbm-ws-unreachable" via GetDocument
///   Then the RPC returns INTERNAL with a fixed, generic message
///   And the server's own tracing output gains a new ERROR-level entry during the call
#[tokio::test]
async fn authenticate_against_unreachable_direct_pg_backend_never_discloses_driver_text() {
    let logs = tracing_capture::init();
    let ctx = ServerTestContext::new().await;
    let api_key = ctx
        .insert_project_with_unreachable_backend("sbm-ws-unreachable")
        .await;

    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));
    let req = authed_request(
        GetDocumentRequest {
            name: "projects/sbm-ws-unreachable/databases/(default)/documents/widgets/doc-1"
                .to_string(),
            ..Default::default()
        },
        &api_key,
    );

    let before = tracing_capture::len(&logs);

    let status = client
        .get_document(req)
        .await
        .expect_err("a direct_pg project with an unreachable backend must fail, not succeed");

    assert_eq!(
        status.code(),
        tonic::Code::Internal,
        "must fail closed with Status::internal, got: {status:?}"
    );
    assert_eq!(
        status.message(),
        "internal server error",
        "client-facing message must be the fixed, generic string (ADR-075 Decision 2), never \
         the raw driver/connection text authenticate()'s direct_pg branch wraps today — got: {:?}",
        status.message()
    );

    let new_output = tracing_capture::new_output(&logs, before);
    assert!(
        new_output.to_uppercase().contains("ERROR"),
        "AC-SBM-03: the real connection error must remain server-observable via \
         tracing::error! even though it was discarded from the client-facing Status — no new \
         ERROR-level output was captured while handling this request. New output since request \
         start: {new_output:?}"
    );
}

/// Regression guard (AC-SBM-11): a healthy direct_pg backend is completely
/// unaffected by this fix. Reuses the SAME context-construction step as the
/// failure scenario above (Pillar 2 — chained narrative), swapping only the
/// backend's own reachability.
///
/// Feature: A well-formed authentication attempt is unaffected by this fix
///   Given project "sbm-ws-healthy" has a well-formed system-DB row and a reachable backend
///   When a client authenticates with a valid API key via GetDocument
///   Then authentication succeeds — the RPC surfaces NOT_FOUND for the probed document,
///     never INTERNAL
#[tokio::test]
async fn authenticate_against_healthy_direct_pg_backend_still_succeeds() {
    let ctx = ServerTestContext::new().await;
    let (api_key, _cust_container, _cust_pool) = ctx
        .insert_project_with_working_backend("sbm-ws-healthy")
        .await;

    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));
    let req = authed_request(
        GetDocumentRequest {
            name: "projects/sbm-ws-healthy/databases/(default)/documents/widgets/doc-absent"
                .to_string(),
            ..Default::default()
        },
        &api_key,
    );

    let status = client
        .get_document(req)
        .await
        .expect_err("a nonexistent document must still surface as NOT_FOUND");

    assert_eq!(
        status.code(),
        tonic::Code::NotFound,
        "authentication must succeed for a healthy backend — any INTERNAL here would mean \
         this feature regressed the happy path; got: {status:?}"
    );
}
