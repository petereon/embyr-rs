//! Slice 01 (US-01, Walking Skeleton) — A Working Write Stream, Single Write,
//! Happy Path (ADR-046).
//!
//! Driving port: gRPC data port (:8080) — `Write` bidirectional streaming RPC.
//! Covers AC-01-01 through AC-01-06 (feature-delta.md, slice-01 brief).

use std::collections::HashMap;

use embyr_proto::firestore::WriteRequest;
use tokio_stream::StreamExt;

#[path = "../common/mod.rs"]
mod common;
use common::{
    handshake_request, open_write_stream, single_write_request, string_field, update_write,
    SecurityRulesFullContext,
};

/// AC-01-01: opening a `Write` stream with a valid, empty handshake
/// `WriteRequest` returns a `WriteResponse` carrying `stream_id`,
/// `stream_token`, `commit_time`, with no `write_results`.
#[tokio::test]
async fn handshake_returns_stream_id_token_and_commit_time_with_no_write_results() {
    let ctx = SecurityRulesFullContext::new("ws01-handshake").await;

    let (_req_tx, result) =
        open_write_stream(&ctx, handshake_request(&ctx.project_id)).await;
    let mut resp_stream = result.expect("Write stream should open for a valid handshake");

    let handshake_resp = resp_stream
        .next()
        .await
        .expect("stream should yield a handshake response")
        .expect("handshake response should be Ok");

    assert!(!handshake_resp.stream_id.is_empty(), "stream_id must be issued");
    assert!(!handshake_resp.stream_token.is_empty(), "stream_token must be issued");
    assert!(handshake_resp.commit_time.is_some(), "commit_time must be set");
    assert!(
        handshake_resp.write_results.is_empty(),
        "handshake response must carry no write_results"
    );
}

/// AC-01-02: a subsequent `WriteRequest` containing exactly one write,
/// presenting the issued `stream_id`/`stream_token`, is applied atomically
/// and the server replies with a `WriteResponse` containing exactly one
/// `write_result` and a rotated `stream_token`. The document is readable via
/// `GetDocument` immediately after (UAT scenario, feature-delta.md).
#[tokio::test]
async fn single_write_is_applied_and_readable_via_get_document() {
    let ctx = SecurityRulesFullContext::new("ws01-single-write").await;

    let (req_tx, result) = open_write_stream(&ctx, handshake_request(&ctx.project_id)).await;
    let mut resp_stream = result.expect("Write stream should open");

    let handshake_resp = resp_stream
        .next()
        .await
        .expect("stream should yield handshake response")
        .expect("handshake response should be Ok");

    let resource_name = format!(
        "projects/{}/databases/(default)/documents/trip_entries/maria-trip-1",
        ctx.project_id
    );
    let mut fields = HashMap::new();
    fields.insert("owner_id".to_string(), string_field("maria-santos"));
    let write = update_write(&resource_name, fields);

    req_tx
        .send(single_write_request(
            &ctx.project_id,
            &handshake_resp.stream_id,
            handshake_resp.stream_token.clone(),
            write,
        ))
        .await
        .expect("send single-write WriteRequest");

    let write_resp = resp_stream
        .next()
        .await
        .expect("stream should yield a write response")
        .expect("write response should be Ok");

    assert_eq!(write_resp.write_results.len(), 1, "exactly one write_result expected");
    assert_ne!(
        write_resp.stream_token, handshake_resp.stream_token,
        "stream_token must rotate on every response"
    );
    assert!(write_resp.commit_time.is_some());

    let doc = ctx
        .get_document(&resource_name, None)
        .await
        .expect("document must be readable via GetDocument immediately after");
    assert_eq!(doc.into_inner().name, resource_name);
}

/// AC-01-03: a stream that receives no `WriteRequest` beyond the handshake,
/// then closes, terminates cleanly with zero writes applied.
#[tokio::test]
async fn stream_closed_after_handshake_only_terminates_cleanly() {
    let ctx = SecurityRulesFullContext::new("ws01-handshake-only-close").await;

    let (req_tx, result) = open_write_stream(&ctx, handshake_request(&ctx.project_id)).await;
    let mut resp_stream = result.expect("Write stream should open");

    resp_stream
        .next()
        .await
        .expect("stream should yield handshake response")
        .expect("handshake response should be Ok");

    // Client closes the stream without ever sending a subsequent WriteRequest.
    drop(req_tx);

    let next = resp_stream.next().await;
    assert!(
        next.is_none(),
        "server must close cleanly with no further message, got: {next:?}"
    );
}

/// AC-01-04: a first (handshake) `WriteRequest` that is not empty (carries
/// writes) is rejected as invalid before any write is applied.
#[tokio::test]
async fn nonempty_handshake_is_rejected_before_any_write() {
    let ctx = SecurityRulesFullContext::new("ws01-invalid-handshake").await;

    let resource_name = format!(
        "projects/{}/databases/(default)/documents/trip_entries/should-never-be-written",
        ctx.project_id
    );
    let mut fields = HashMap::new();
    fields.insert("owner_id".to_string(), string_field("maria-santos"));
    let bogus_handshake = WriteRequest {
        database: format!("projects/{}/databases/(default)", ctx.project_id),
        stream_id: String::new(),
        writes: vec![update_write(&resource_name, fields)],
        stream_token: vec![],
    };

    let (_req_tx, result) = open_write_stream(&ctx, bogus_handshake).await;

    let status = result.expect_err("a non-empty handshake must be rejected");
    assert_eq!(status.code(), tonic::Code::InvalidArgument);

    let doc = ctx.get_document(&resource_name, None).await;
    assert!(doc.is_err(), "no document must have been written");
}

/// AC-01-05: rate-limiting, authentication, suspension-check, and
/// client-identity resolution each run exactly once per stream, at handshake
/// time — proven observably by two successive write round trips succeeding
/// on the SAME session (single `authorization` header, no per-message
/// re-authentication), each issuing its own newly rotated `stream_token`.
#[tokio::test]
async fn successive_writes_on_same_session_each_get_a_freshly_rotated_token() {
    let ctx = SecurityRulesFullContext::new("ws01-successive-writes").await;

    let (req_tx, result) = open_write_stream(&ctx, handshake_request(&ctx.project_id)).await;
    let mut resp_stream = result.expect("Write stream should open");

    let handshake_resp = resp_stream
        .next()
        .await
        .expect("stream should yield handshake response")
        .expect("handshake response should be Ok");

    let resource_a = format!(
        "projects/{}/databases/(default)/documents/trip_entries/maria-trip-a",
        ctx.project_id
    );
    let mut fields_a = HashMap::new();
    fields_a.insert("owner_id".to_string(), string_field("maria-santos"));
    req_tx
        .send(single_write_request(
            &ctx.project_id,
            &handshake_resp.stream_id,
            handshake_resp.stream_token.clone(),
            update_write(&resource_a, fields_a),
        ))
        .await
        .expect("send first write");

    let resp_a = resp_stream
        .next()
        .await
        .expect("stream should yield first write response")
        .expect("first write response should be Ok");
    assert_eq!(resp_a.write_results.len(), 1);

    let resource_b = format!(
        "projects/{}/databases/(default)/documents/trip_entries/maria-trip-b",
        ctx.project_id
    );
    let mut fields_b = HashMap::new();
    fields_b.insert("owner_id".to_string(), string_field("maria-santos"));
    req_tx
        .send(single_write_request(
            &ctx.project_id,
            &resp_a.stream_id,
            resp_a.stream_token.clone(),
            update_write(&resource_b, fields_b),
        ))
        .await
        .expect("send second write on the SAME session, no re-authentication");

    let resp_b = resp_stream
        .next()
        .await
        .expect("stream should yield second write response")
        .expect("second write response should be Ok");
    assert_eq!(resp_b.write_results.len(), 1);

    assert_ne!(handshake_resp.stream_token, resp_a.stream_token);
    assert_ne!(resp_a.stream_token, resp_b.stream_token);
}

/// AC-01-06: a suspended project's `Write` stream is rejected with
/// `permission_denied` at handshake, before any write is accepted.
#[tokio::test]
async fn suspended_project_write_stream_rejected_at_handshake() {
    let ctx = SecurityRulesFullContext::new("ws01-suspended").await;

    sqlx::query("UPDATE projects SET status = 'suspended' WHERE id = $1")
        .bind(&ctx.project_id)
        .execute(&ctx.sys_pool)
        .await
        .expect("suspend project");

    let (_req_tx, result) = open_write_stream(&ctx, handshake_request(&ctx.project_id)).await;

    let status = result.expect_err("a suspended project's Write stream must be rejected");
    assert_eq!(status.code(), tonic::Code::PermissionDenied);
}
