//! Common test infrastructure — agent-mode-list-collection-ids acceptance
//! tests (ADR-059: new unary `ListCollectionIds` RPC on `storage_agent.proto`,
//! zero SQL duplication, `AgentBackendAdapter`'s own page-flattening loop).
//!
//! WHY-NEW-FILE: tests/agent_mode_list_collection_ids/common/mod.rs
//!   CLOSEST-EXISTING: tests/agent_mode_write_streaming/common/mod.rs
//!   EXTENSION-COST: that file's own `AgentModeWriteStreamingContext` is a
//!     concrete, feature-named struct — reusable via path import (as done
//!     below), but its own file header already documents that a NEW driving
//!     port under real backend_mode=agent composition warrants a sibling
//!     file, not an edit to a previously-shipped, already-committed feature's
//!     own test module.
//!   PARALLEL-RATIONALE: this feature calls a DIFFERENT driving port
//!     (`ListCollectionIds`, unary) than write-streaming's own bidi `Write`
//!     stream — the context class (real embyr-agent + real Postgres + real
//!     embyr-server) is identical and fully reused unchanged via path import;
//!     only the seeding/query helpers below are new, and they belong beside
//!     THIS feature's own acceptance tests, not inside write-streaming's.

#![allow(dead_code, unused_imports)]

use std::collections::HashMap;

use tokio_stream::StreamExt;

#[path = "../../agent_mode_write_streaming/common/mod.rs"]
mod agent_mode_write_streaming_common;
pub use agent_mode_write_streaming_common::{
    handshake_request, open_write_stream, single_write_request, string_field, update_write,
    AgentModeWriteStreamingContext,
};

use embyr_proto::firestore::{
    firestore_client::FirestoreClient, ListCollectionIdsRequest, ListCollectionIdsResponse,
};

/// Seed one document per `(subcollection, document_id)` pair under
/// `parent_doc_path`, all over ONE `Write` stream session (token refreshed
/// per write, mirroring `aw02`'s own multi-write-in-one-session shape) —
/// avoids paying a fresh mTLS handshake per document when a test seeds many
/// (e.g. the pagination-boundary scenario, 101 distinct subcollections).
pub async fn seed_documents(
    ctx: &AgentModeWriteStreamingContext,
    parent_doc_path: &str,
    subcollections_and_doc_ids: &[(String, &str)],
) {
    let (req_tx, result) = open_write_stream(ctx, handshake_request(&ctx.project_id)).await;
    let mut resp_stream = result.expect("Write stream should open against an agent-mode project");
    let mut handshake_resp = resp_stream
        .next()
        .await
        .expect("stream should yield handshake response")
        .expect("handshake response should be Ok");

    for (subcollection, doc_id) in subcollections_and_doc_ids {
        let resource_name = format!(
            "projects/{}/databases/(default)/documents/{parent_doc_path}/{subcollection}/{doc_id}",
            ctx.project_id
        );
        let mut fields = HashMap::new();
        fields.insert("seeded".to_string(), string_field("true"));

        req_tx
            .send(single_write_request(
                &ctx.project_id,
                &handshake_resp.stream_id,
                handshake_resp.stream_token.clone(),
                update_write(&resource_name, fields),
            ))
            .await
            .expect("send seed write");

        let write_resp = resp_stream
            .next()
            .await
            .expect("stream should yield a write response")
            .expect("write response should be Ok");
        assert_eq!(write_resp.write_results.len(), 1, "seed write must be acknowledged");

        handshake_resp.stream_id = write_resp.stream_id;
        handshake_resp.stream_token = write_resp.stream_token;
    }
}

/// Real gRPC `ListCollectionIds` call against `embyr-server` — driving port
/// entry, mirroring `firestore_list_rpcs::common::list_collection_ids`'s own
/// shape, adapted to `AgentModeWriteStreamingContext`.
pub async fn list_collection_ids(
    ctx: &AgentModeWriteStreamingContext,
    parent: &str,
    page_size: i32,
    page_token: &str,
) -> Result<ListCollectionIdsResponse, tonic::Status> {
    let channel = tonic::transport::Endpoint::new(format!("http://{}", ctx.server.grpc_addr))
        .expect("valid endpoint")
        .connect()
        .await
        .expect("connect to gRPC server");
    let mut client = FirestoreClient::new(channel);

    let mut request = tonic::Request::new(ListCollectionIdsRequest {
        parent: parent.to_string(),
        page_size,
        page_token: page_token.to_string(),
    });
    request.metadata_mut().insert(
        "authorization",
        format!("Bearer {}", ctx.api_key).parse().unwrap(),
    );

    Ok(client.list_collection_ids(request).await?.into_inner())
}

/// A nested `parent` resource name — `doc_path` is a document path relative
/// to the database root (e.g. `"patients/p-204"`).
pub fn nested_parent(ctx: &AgentModeWriteStreamingContext, doc_path: &str) -> String {
    format!(
        "projects/{}/databases/(default)/documents/{doc_path}",
        ctx.project_id
    )
}
