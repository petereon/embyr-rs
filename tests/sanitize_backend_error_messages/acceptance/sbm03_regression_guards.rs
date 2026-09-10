// SCAFFOLD: true
// @error @AC-SBM-04 @AC-SBM-05 @AC-SBM-11
//! SBM-03 — Regression guards. Every `CoreError` variant OTHER than
//! `BackendUnavailable` is byte-for-byte unchanged by this feature
//! (AC-SBM-05), and the gRPC status CODE is unaffected everywhere
//! (AC-SBM-04) — only `BackendUnavailable`'s message CONTENT changes. Per
//! feature-delta.md § DISCUSS Investigation Finding 4, no existing test in
//! this workspace asserts on `Status::internal`'s own message content for
//! an Internal-code response, so these regression guards target the OTHER
//! status codes this feature must never touch.

#[path = "../common/mod.rs"]
mod common;

use common::{authed_request, make_channel, ServerTestContext};
use embyr_proto::firestore::firestore_client::FirestoreClient;
use embyr_proto::firestore::GetDocumentRequest;

/// @error @AC-SBM-05 @AC-SBM-11
///
/// Feature: A DocumentNotFound failure is completely unaffected by this fix
///   Given no document exists at "widgets/absent" in project "sbm-reg-notfound"
///   When a client calls GetDocument for that path
///   Then the RPC returns NOT_FOUND, echoing the document id exactly as before this feature
#[tokio::test]
async fn document_not_found_message_unchanged_by_this_feature() {
    let ctx = ServerTestContext::new().await;
    let (api_key, _cust_container, _cust_pool) = ctx
        .insert_project_with_working_backend("sbm-reg-notfound")
        .await;
    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));

    let req = authed_request(
        GetDocumentRequest {
            name: "projects/sbm-reg-notfound/databases/(default)/documents/widgets/absent"
                .to_string(),
            ..Default::default()
        },
        &api_key,
    );
    let status = client
        .get_document(req)
        .await
        .expect_err("an absent document must surface as NOT_FOUND");

    assert_eq!(status.code(), tonic::Code::NotFound, "got: {status:?}");
    assert!(
        status.message().contains("absent"),
        "DocumentNotFound's own message shape (echoing the document id) must be untouched by \
         this feature — got: {:?}",
        status.message()
    );
}

/// @error @AC-SBM-05 @AC-SBM-11
///
/// Feature: An invalid API key is rejected exactly as before this feature
///   Given project "sbm-reg-badkey" has a reachable backend
///   When a client sends a GetDocument request with a completely wrong API key
///   Then the RPC returns UNAUTHENTICATED, untouched by this feature
#[tokio::test]
async fn wrong_api_key_rejection_unchanged_by_this_feature() {
    let ctx = ServerTestContext::new().await;
    let (_api_key, _cust_container, _cust_pool) = ctx
        .insert_project_with_working_backend("sbm-reg-badkey")
        .await;
    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));

    let req = authed_request(
        GetDocumentRequest {
            name: "projects/sbm-reg-badkey/databases/(default)/documents/widgets/doc-1"
                .to_string(),
            ..Default::default()
        },
        "wrong-api-key-entirely",
    );
    let status = client
        .get_document(req)
        .await
        .expect_err("a wrong API key must be rejected");

    assert_eq!(
        status.code(),
        tonic::Code::Unauthenticated,
        "invalid-api-key rejection must be untouched by this feature — got: {status:?}"
    );
}
