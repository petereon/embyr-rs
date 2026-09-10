// SCAFFOLD: true
// @driving_port @real-io @AC-SBM-07
//! SBM-04 (AC-SBM-07) — Listen (onSnapshot) against a customer Postgres that
//! becomes unreachable AFTER authentication has already been cached never
//! discloses driver text. Covers BOTH `handle_listen` sites in one real
//! RPC: the bare `sqlx::Error` notify-listener-pool provisioning
//! (`handler.rs:3556` — the ONE site in this feature's entire inventory
//! that never goes through `CoreError` at all, ADR-075 Decision 4) and its
//! `CoreError`-wrapped sibling (`handler.rs:3564`).
//!
//! Mechanism: the credential cache is warmed with a healthy backend (a
//! prior GetDocument), then the customer Postgres CONTAINER is stopped
//! (`testcontainers`'s own `ContainerAsync::stop()`) — not just its pool
//! closed — so every subsequent connection attempt fails the same way a
//! real "customer's own Postgres instance becomes unreachable" incident
//! would (feature-delta.md § DISCUSS Domain Examples), regardless of which
//! of the two sites inside `handle_listen` observes the failure first; both
//! share the identical fix, so the acceptance-observable behavior is the
//! same either way.
//!
//! Red classification (pre-fix): both sites wrap the raw connect failure
//! directly into `Status::internal(...)`, no sanitization.

#[path = "../common/mod.rs"]
mod common;

use common::{authed_request, make_channel, ServerTestContext};
use embyr_proto::firestore::{
    firestore_client::FirestoreClient,
    listen_request,
    target::{self, query_target},
    structured_query::CollectionSelector,
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

/// @driving_port @real-io @AC-SBM-07
///
/// Feature: Starting a real-time listener against an unreachable backend never discloses driver text
///   Given project "sbm-listen-unreachable" is configured with backend_mode "direct_pg"
///   And a first request has already succeeded, warming the authentication cache
///   And the customer's own Postgres instance then becomes unreachable
///   When a client calls Listen (onSnapshot) for a collection in "sbm-listen-unreachable"
///   Then the RPC returns INTERNAL with a fixed, generic message
#[tokio::test]
async fn listen_against_backend_that_becomes_unreachable_never_discloses_driver_text() {
    let ctx = ServerTestContext::new().await;
    let (api_key, cust_container, _cust_pool) = ctx
        .insert_project_with_working_backend("sbm-listen-unreachable")
        .await;

    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));

    // Given: a first request succeeds, warming the authentication cache
    // while the customer backend is still healthy.
    let warm_up = authed_request(
        GetDocumentRequest {
            name: "projects/sbm-listen-unreachable/databases/(default)/documents/widgets/doc-warm"
                .to_string(),
            ..Default::default()
        },
        &api_key,
    );
    client
        .get_document(warm_up)
        .await
        .expect_err("doc-warm does not exist yet — expect NOT_FOUND, not success");

    // And: the customer's own Postgres instance becomes unreachable.
    cust_container
        .stop()
        .await
        .expect("stop customer Postgres container");

    // When: a client calls Listen for a collection in this project.
    let mut request = tonic::Request::new(tokio_stream::once(add_target_request(
        "sbm-listen-unreachable",
        "widgets",
    )));
    request.metadata_mut().insert(
        "authorization",
        format!("bearer {api_key}").parse().expect("valid metadata value"),
    );

    let status = client
        .listen(request)
        .await
        .expect_err("Listen against an unreachable customer backend must fail, not succeed");

    assert_eq!(status.code(), tonic::Code::Internal, "got: {status:?}");
    assert_eq!(
        status.message(),
        "internal server error",
        "handle_listen's own notify-listener-pool provisioning (handler.rs:3556, the one bare \
         sqlx::Error site in this feature's inventory) and its CoreError-wrapped sibling \
         (handler.rs:3564) must both be sanitized — got: {:?}",
        status.message()
    );
}
