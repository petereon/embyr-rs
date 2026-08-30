//! Slice 04 (US-04) — Correct Termination Under Every Documented Close
//! Condition (ADR-046 § Decision 4).
//!
//! Driving port: gRPC data port (:8080) — `Write` bidirectional streaming RPC.
//! Covers AC-04-01 through AC-04-04 (slice-04-termination-conditions.md).
//!
//! Black-box by design (Mandate 1): each scenario asserts OBSERVABLE outcomes
//! only — document state via a fresh `GetDocument` call, and whether the
//! response stream yields `None`/`Err`/hangs — never which internal `match`
//! arm of `run_write_session`'s loop fired. This mirrors ws01's own house
//! style (`stream_closed_after_handshake_only_terminates_cleanly`).

use std::{collections::HashMap, time::Duration};

use tokio_stream::StreamExt;

#[path = "../common/mod.rs"]
mod common;
use common::{
    handshake_request, open_write_stream, single_write_request, string_field, update_write,
    write_requiring_stale_update_time, SecurityRulesFullContext,
};

const TERMINATION_TIMEOUT: Duration = Duration::from_secs(5);

/// AC-04-01: a client-initiated `io.EOF` close — occurring mid-loop, after at
/// least one write has already round-tripped successfully, not merely at the
/// handshake (ws01's own `stream_closed_after_handshake_only_terminates_cleanly`
/// already covers the handshake-only case) — terminates the server side
/// cleanly, with no error, and does not hang (AC-04-04).
#[tokio::test]
async fn client_eof_after_a_successful_write_terminates_cleanly_with_no_error() {
    let ctx = SecurityRulesFullContext::new("ws04-eof-mid-loop").await;

    let (req_tx, result) = open_write_stream(&ctx, handshake_request(&ctx.project_id)).await;
    let mut resp_stream = result.expect("Write stream should open for a valid handshake");
    let handshake_resp = resp_stream
        .next()
        .await
        .expect("stream should yield handshake response")
        .expect("handshake response should be Ok");

    let resource_name = format!(
        "projects/{}/databases/(default)/documents/trip_entries/eof-mid-loop-doc",
        ctx.project_id
    );
    let mut fields = HashMap::new();
    fields.insert("owner_id".to_string(), string_field("maria-santos"));
    req_tx
        .send(single_write_request(
            &ctx.project_id,
            &handshake_resp.stream_id,
            handshake_resp.stream_token.clone(),
            update_write(&resource_name, fields),
        ))
        .await
        .expect("send write before closing");

    let write_resp = resp_stream
        .next()
        .await
        .expect("stream should yield a write response")
        .expect("write response should be Ok");
    assert_eq!(write_resp.write_results.len(), 1);

    // Client-initiated clean close (io.EOF), mid-loop: no further
    // WriteRequest is ever sent.
    drop(req_tx);

    let terminal = tokio::time::timeout(TERMINATION_TIMEOUT, resp_stream.next())
        .await
        .expect("server must terminate promptly on client EOF, not hang (AC-04-04)");
    assert!(
        terminal.is_none(),
        "server must close cleanly with no error on client EOF, got: {terminal:?}"
    );

    let doc = ctx
        .get_document(&resource_name, None)
        .await
        .expect("the write applied before EOF must remain durably readable");
    assert_eq!(doc.into_inner().name, resource_name);
}

/// AC-04-02: a client cancellation mid-write — the in-flight call is dropped
/// (simulating the OS killing the app) immediately after a `WriteRequest` is
/// enqueued, before its `WriteResponse` is ever read — terminates the server
/// side cleanly and never leaves a partially-applied write: the document
/// (with 2 fields) must read back either fully absent or fully present,
/// never with only some of its fields. AC-04-04 is proven by opening a
/// second, brand-new session afterward and completing a full round trip —
/// a leaked task or corrupted server state would manifest as a hang here.
#[tokio::test]
async fn client_cancellation_mid_write_never_leaves_a_partial_write() {
    let ctx = SecurityRulesFullContext::new("ws04-cancel-mid-write").await;

    let (req_tx, result) = open_write_stream(&ctx, handshake_request(&ctx.project_id)).await;
    let mut resp_stream = result.expect("Write stream should open");
    let handshake_resp = resp_stream
        .next()
        .await
        .expect("stream should yield handshake response")
        .expect("handshake response should be Ok");

    let resource_name = format!(
        "projects/{}/databases/(default)/documents/trip_entries/cancel-mid-write-doc",
        ctx.project_id
    );
    let mut fields = HashMap::new();
    fields.insert("owner_id".to_string(), string_field("maria-santos"));
    fields.insert("destination".to_string(), string_field("lisbon"));
    req_tx
        .send(single_write_request(
            &ctx.project_id,
            &handshake_resp.stream_id,
            handshake_resp.stream_token.clone(),
            update_write(&resource_name, fields),
        ))
        .await
        .expect("enqueue write before cancelling");

    // Client cancellation: drop the whole in-flight call (both directions of
    // the bidi stream) before ever reading the write's own response.
    drop(resp_stream);
    drop(req_tx);

    // Let the server observe the disconnect and settle any in-flight apply —
    // Postgres transaction atomicity means it is either fully committed or
    // not committed at all by the time this elapses.
    tokio::time::sleep(Duration::from_millis(300)).await;

    match ctx.get_document(&resource_name, None).await {
        Err(status) => assert_eq!(
            status.code(),
            tonic::Code::NotFound,
            "an unapplied write must read back as NotFound, never a partial document"
        ),
        Ok(doc) => assert_eq!(
            doc.into_inner().fields.len(),
            2,
            "an applied write must be COMPLETE (all fields), never partial"
        ),
    }

    // AC-04-04: the server must still be healthy/responsive — a leaked task
    // or corrupted session state from the cancellation would hang or fail a
    // brand-new session.
    let (_req_tx2, result2) = open_write_stream(&ctx, handshake_request(&ctx.project_id)).await;
    let mut resp_stream2 =
        result2.expect("server must still accept new Write sessions after a cancellation");
    let handshake2 = tokio::time::timeout(TERMINATION_TIMEOUT, resp_stream2.next())
        .await
        .expect("server must still respond promptly, not hung by the earlier cancellation")
        .expect("stream should yield a handshake response")
        .expect("handshake response should be Ok");
    assert!(!handshake2.stream_id.is_empty());
}

/// AC-04-03: a genuine server-side apply error (an OCC precondition
/// violation — `commit_transaction`'s own established `UpdateTime` check,
/// `backend_adapter.rs:895-932`, reused unchanged from `Commit`) terminates
/// the stream WITH the error propagated to the client — distinguishable, on
/// the wire, from both clean-close conditions above (`Err`, not `None`) —
/// and the stream is fully closed afterward, not left open for further
/// messages (ADR-046 § Decision 4: one error terminates the whole stream, no
/// per-message recoverable-rejection shape for `Write`).
#[tokio::test]
async fn server_side_apply_error_propagates_and_terminates_the_stream() {
    let ctx = SecurityRulesFullContext::new("ws04-apply-error").await;

    let (req_tx, result) = open_write_stream(&ctx, handshake_request(&ctx.project_id)).await;
    let mut resp_stream = result.expect("Write stream should open");
    let handshake_resp = resp_stream
        .next()
        .await
        .expect("stream should yield handshake response")
        .expect("handshake response should be Ok");

    let resource_name = format!(
        "projects/{}/databases/(default)/documents/trip_entries/apply-error-doc",
        ctx.project_id
    );
    let mut fields = HashMap::new();
    fields.insert("owner_id".to_string(), string_field("maria-santos"));
    req_tx
        .send(single_write_request(
            &ctx.project_id,
            &handshake_resp.stream_id,
            handshake_resp.stream_token.clone(),
            update_write(&resource_name, fields.clone()),
        ))
        .await
        .expect("send first write");
    let first_resp = resp_stream
        .next()
        .await
        .expect("stream should yield first write response")
        .expect("first write response should be Ok");
    assert_eq!(first_resp.write_results.len(), 1);

    // A second write on the SAME now-existing document, presenting a stale
    // (deliberately wrong) update_time OCC precondition — a genuine apply
    // error `commit_transaction` itself detects and rejects.
    req_tx
        .send(single_write_request(
            &ctx.project_id,
            &first_resp.stream_id,
            first_resp.stream_token.clone(),
            write_requiring_stale_update_time(&resource_name, fields),
        ))
        .await
        .expect("send OCC-violating write");

    let error_resp = resp_stream
        .next()
        .await
        .expect("stream must yield a terminal message, not hang");
    let status = error_resp.expect_err(
        "a genuine apply error must surface as an Err, distinguishable from a clean close",
    );
    assert_eq!(status.code(), tonic::Code::Aborted);

    let terminal = tokio::time::timeout(TERMINATION_TIMEOUT, resp_stream.next())
        .await
        .expect("server must terminate promptly after the apply error, not hang");
    assert!(
        terminal.is_none(),
        "the stream must be fully closed after the error, got: {terminal:?}"
    );

    let doc = ctx
        .get_document(&resource_name, None)
        .await
        .expect("original document must remain readable, unmodified by the rejected write");
    assert_eq!(
        doc.into_inner().fields.len(),
        1,
        "the failed write must not have modified the document"
    );
}
