//! Common test infrastructure — firestore-write-streaming acceptance tests
//! (Slice 01, US-01, ADR-046).
//!
//! Reuses `SecurityRulesFullContext` via a path import (per DESIGN's own
//! Reuse Analysis: this feature adds zero new fixture context class — same
//! precedent `batch_get_documents`/`security_rules_write_path` already
//! established for a brand-new driving-port RPC layered on the SAME
//! `SecurityRulesFullContext`).

#![allow(dead_code, unused_imports)]

#[path = "../../security_rules_write_path/common/mod.rs"]
mod security_rules_write_path_common;
pub use security_rules_write_path_common::{string_field, update_write, SecurityRulesFullContext};

use embyr_proto::firestore::{firestore_client::FirestoreClient, WriteRequest, WriteResponse};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

/// Open a real gRPC `Write` bidi-stream — driving port entry (Pillar 3).
/// Sends `first_request` (typically the handshake) before issuing the call,
/// since `handle_write` blocks on reading the first outbound message before
/// returning the initial `Response` — mirrors `handle_listen`'s own
/// peek-first-message mechanism from the client side.
///
/// Returns the outbound sender (for further `WriteRequest`s) and the raw
/// `Result` of the call itself, so callers can assert on BOTH the happy path
/// (`Ok(Streaming<WriteResponse>)`) and the reject-before-any-write path
/// (`Err(Status)`, AC-01-04/AC-01-06) without a panic-on-unwrap helper.
pub async fn open_write_stream(
    ctx: &SecurityRulesFullContext,
    first_request: WriteRequest,
) -> (
    mpsc::Sender<WriteRequest>,
    Result<tonic::Streaming<WriteResponse>, tonic::Status>,
) {
    let channel = tonic::transport::Endpoint::new(format!("http://{}", ctx.server.grpc_addr))
        .expect("valid endpoint")
        .connect()
        .await
        .expect("connect to gRPC server");
    let mut client = FirestoreClient::new(channel);

    let (req_tx, req_rx) = mpsc::channel::<WriteRequest>(4);
    req_tx
        .send(first_request)
        .await
        .expect("send first WriteRequest into outbound channel");

    let outbound = ReceiverStream::new(req_rx);
    let mut request = tonic::Request::new(outbound);
    request.metadata_mut().insert(
        "authorization",
        format!("Bearer {}", ctx.api_key).parse().unwrap(),
    );

    let result = client.write(request).await.map(|r| r.into_inner());
    (req_tx, result)
}

/// The empty handshake `WriteRequest` (ADR-046 § Decision 1): `writes` and
/// `stream_id` both empty — the ONLY valid shape for a stream's first message.
pub fn handshake_request(project_id: &str) -> WriteRequest {
    WriteRequest {
        database: format!("projects/{project_id}/databases/(default)"),
        stream_id: String::new(),
        writes: vec![],
        stream_token: vec![],
    }
}

/// A single-write `WriteRequest` presenting the given `stream_id`/`stream_token`
/// (Slice 01's own shape: exactly one write per batch).
pub fn single_write_request(
    project_id: &str,
    stream_id: &str,
    stream_token: Vec<u8>,
    write: embyr_proto::firestore::Write,
) -> WriteRequest {
    WriteRequest {
        database: format!("projects/{project_id}/databases/(default)"),
        stream_id: stream_id.to_string(),
        writes: vec![write],
        stream_token,
    }
}
