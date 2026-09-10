// SCAFFOLD: true
//! US-A08 (sanitize-backend-error-messages, AC-SBM-02 + AC-SBM-08) — the
//! agent's own `core_error_to_status`'s explicit `BackendUnavailable` arm is
//! sanitized identically to embyr-server's, and `get_document` is fixed to
//! route through it exactly like its 6 sibling handlers already do (a pure
//! consistency fix, feature-delta.md § DESIGN Decision 3 — zero new logic).
//!
//! Driving port: StorageAgent gRPC service (mTLS :9191) — `GetDocument` RPC.
//!
//! Red classification (pre-fix): `server.rs:310`'s own bespoke
//! `Status::internal(format!("{e}"))` returns the raw sqlx pool-closed text
//! verbatim, not the fixed generic message `core_error_to_status`'s sibling
//! handlers (`BeginTransaction`/`Commit`/`Rollback`) already return for the
//! identical underlying failure class.

use embyr_proto::agent::GetDocumentRequest;

use super::agent_common::start_test_agent;

/// @driving_port @real-io @AC-SBM-02 @AC-SBM-08
///
/// Feature: get_document never discloses driver text when the local backend becomes unreachable
///   Given the embyr agent for project "sbm-agent-unreachable" has lost its local Postgres connection
///   When embyr-server calls GetDocument on the agent's own StorageAgent service
///   Then the RPC returns INTERNAL with the fixed, generic message
///   And the message matches the same convention Commit/Rollback/BeginTransaction already use
#[tokio::test]
async fn get_document_never_discloses_driver_text_when_local_backend_unreachable() {
    let (handle, mut client) = start_test_agent("sbm-agent-unreachable").await;

    // And: the local Postgres connection is lost (closing the pool the
    // storage adapter was constructed from — `new_from_pool` shares the
    // same underlying pool, so this closes it for the adapter too).
    handle.pool.close().await;

    let status = client
        .get_document(tonic::Request::new(GetDocumentRequest {
            name: "projects/sbm-agent-unreachable/databases/(default)/documents/widgets/doc-1"
                .to_string(),
            ..Default::default()
        }))
        .await
        .expect_err("get_document against a closed local pool must fail, not succeed");

    assert_eq!(
        status.code(),
        tonic::Code::Internal,
        "must fail closed with Status::internal, got: {status:?}"
    );
    assert_eq!(
        status.message(),
        "internal server error",
        "get_document (server.rs:310) must route through core_error_to_status exactly like \
         its 6 sibling handlers, returning the same fixed generic message — got: {:?}",
        status.message()
    );
}
