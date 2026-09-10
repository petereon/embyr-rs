// SCAFFOLD: true
// @driving_port @real-io @AC-SBM-10
//! SBM-02 — AC-SBM-10 completeness-gate spot check. The identical
//! sanitize-and-log fix pattern applied at `authenticate()` (SBM-01) must
//! also close DESIGN's own 13 newly-classified access-rule-lookup sites
//! (feature-delta.md § DESIGN Decision 3) — sites DISCUSS's narrower
//! `authenticate()`/`handle_listen` reading never enumerated. Two genuinely
//! different call sites are proven here, not just re-runs of the same one:
//!   - `resolve_access_rule_pattern`'s READ-rule routing lookup
//!     (`handler.rs:768`, reached via GetDocument)
//!   - `evaluate_write_rule_for_commit`'s sibling WRITE-rule lookup
//!     (`handler.rs:1624`, reached via CreateDocument)
//!
//! Both scenarios use the SAME "cache-warm, then break the system DB"
//! technique: a first request succeeds while the system database is
//! healthy (warming `authenticate()`'s own credential cache, per
//! `crates/embyr-server/src/grpc/handler.rs`'s `CredentialCache` — a cache
//! hit skips `system_db` entirely), then the system pool is closed. The
//! SECOND request's own `authenticate()` call is served from cache (no
//! system_db use), but the access-rule lookup that follows it always
//! queries `system_db` fresh — isolating exactly the site under test.
//!
//! Red classification (pre-fix): both sites do
//! `.map_err(|e| Status::internal(e.to_string()))?` with no sanitization —
//! today's response echoes the raw `sqlx::Error` (e.g. "attempted to
//! acquire a connection on a closed pool"), not the fixed generic message.

#[path = "../common/mod.rs"]
mod common;

use common::{authed_request, make_channel, ServerTestContext};
use embyr_proto::firestore::firestore_client::FirestoreClient;
use embyr_proto::firestore::{CreateDocumentRequest, Document, GetDocumentRequest};

/// @driving_port @real-io @AC-SBM-10
///
/// Feature: The access-rule routing lookup never discloses driver text when the system database becomes unreachable mid-session
///   Given project "sbm-cg-read" has a reachable direct_pg backend and a defined access rule on "widgets"
///   And a first request has already succeeded, warming the authentication cache
///   And the system database then becomes unreachable
///   When a client requests a document in "widgets"
///   Then the RPC returns INTERNAL with the fixed, generic message
#[tokio::test]
async fn get_document_access_rule_routing_lookup_sanitized_when_system_db_unreachable() {
    let ctx = ServerTestContext::new().await;
    let (api_key, _cust_container, _cust_pool) = ctx
        .insert_project_with_working_backend("sbm-cg-read")
        .await;
    ctx.seed_access_rule("sbm-cg-read", "widgets", "true").await;

    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));

    // Given: a first request succeeds, warming the credential cache while
    // the system database is still healthy.
    let warm_up = authed_request(
        GetDocumentRequest {
            name: "projects/sbm-cg-read/databases/(default)/documents/widgets/doc-warm"
                .to_string(),
            ..Default::default()
        },
        &api_key,
    );
    let warm_status = client
        .get_document(warm_up)
        .await
        .expect_err("doc-warm does not exist yet — expect NOT_FOUND, not success");
    assert_eq!(
        warm_status.code(),
        tonic::Code::NotFound,
        "cache-warming call must succeed past authentication, got: {warm_status:?}"
    );

    // And: the system database becomes unreachable (closing any pool clone
    // closes the shared pool for every clone, including the one embyr_server
    // itself holds via its own `Arc<SystemDb>`).
    ctx.sys_pool.close().await;

    // When: a second request for the SAME project is served from the
    // now-warm credential cache (skips system_db for authenticate()) but
    // still must resolve the access-rule ROUTING lookup via a fresh
    // system_db query.
    let req = authed_request(
        GetDocumentRequest {
            name: "projects/sbm-cg-read/databases/(default)/documents/widgets/doc-2".to_string(),
            ..Default::default()
        },
        &api_key,
    );
    let status = client
        .get_document(req)
        .await
        .expect_err("the access-rule routing lookup must fail once the system DB is unreachable");

    assert_eq!(status.code(), tonic::Code::Internal, "got: {status:?}");
    assert_eq!(
        status.message(),
        "internal server error",
        "the access-rule routing lookup (handler.rs:768, one of DESIGN's 13 newly-classified \
         sites) must be sanitized by the identical fix pattern used at authenticate() — got: {:?}",
        status.message()
    );
}

/// @driving_port @real-io @AC-SBM-10
///
/// Feature: The write-rule lookup never discloses driver text when the system database becomes unreachable mid-session
///   (chained — reuses the SAME cache-warming Given+When as the read-rule scenario above,
///   Pillar 2, swapping only the follow-up request from GetDocument to CreateDocument)
#[tokio::test]
async fn create_document_write_rule_lookup_sanitized_when_system_db_unreachable() {
    let ctx = ServerTestContext::new().await;
    let (api_key, _cust_container, _cust_pool) = ctx
        .insert_project_with_working_backend("sbm-cg-write")
        .await;

    let mut client = FirestoreClient::new(make_channel(ctx.server.grpc_addr));

    let warm_up = authed_request(
        GetDocumentRequest {
            name: "projects/sbm-cg-write/databases/(default)/documents/widgets/doc-warm"
                .to_string(),
            ..Default::default()
        },
        &api_key,
    );
    client
        .get_document(warm_up)
        .await
        .expect_err("doc-warm does not exist yet — expect NOT_FOUND, not success");

    ctx.sys_pool.close().await;

    let create_req = authed_request(
        CreateDocumentRequest {
            parent: "projects/sbm-cg-write/databases/(default)/documents".to_string(),
            collection_id: "widgets".to_string(),
            document_id: "doc-new".to_string(),
            document: Some(Document {
                name: String::new(),
                fields: Default::default(),
                ..Default::default()
            }),
            ..Default::default()
        },
        &api_key,
    );
    let status = client
        .create_document(create_req)
        .await
        .expect_err("the write-rule lookup must fail once the system DB is unreachable");

    assert_eq!(status.code(), tonic::Code::Internal, "got: {status:?}");
    assert_eq!(
        status.message(),
        "internal server error",
        "the write-rule lookup (handler.rs:1624, one of DESIGN's 13 newly-classified sites) \
         must be sanitized by the identical fix pattern — got: {:?}",
        status.message()
    );
}
