//! Slice 01 (US-01, Walking Skeleton) — Single-Write `Write` Stream Against
//! an Agent-Mode Project (ADR-060).
//!
//! `write_stream.rs` (ADR-046) is reused completely unchanged for
//! `backend_mode=agent` — the only production change this feature makes is
//! `embyr-agent`'s own `commit()` handler populating `CommitResponse.write_results`
//! from its own real `commit_transaction` result instead of discarding it
//! (previously always empty, blocking AC-01 below).
//!
//! Driving port: gRPC data port (:8080) — `Write` bidirectional streaming
//! RPC, exercised against a REAL `embyr-agent` binary + real Postgres (no
//! mocked transport, DESIGN's own WS Strategy B).
//!
//! Precondition-failure and malformed-write behavior are WHOLE-STREAM
//! TERMINATION, matching ADR-046's own already-shipped mechanism exactly —
//! NOT the "scoped rejection, stream stays open" shape DISCUSS's own AC text
//! described (corrected by ADR-060 § Decision 6; `Write` is structurally
//! incapable of differing from the non-agent path since it reuses
//! `write_stream.rs` verbatim).

use std::collections::HashMap;

use tokio_stream::StreamExt;

#[path = "../common/mod.rs"]
mod common;
use common::{
    handshake_request, open_write_stream, single_write_request, string_field, update_write,
    write_requiring_stale_update_time, AgentModeWriteStreamingContext,
};

/// AC-01 (WS core): a write sent over a newly-opened stream against a
/// `backend_mode=agent` project is persisted and acknowledged with a
/// populated `updateTime` — the assertion the pre-fix `commit()` bug (always
/// empty `write_results`) fails: before the fix, `write_resp.write_results`
/// is empty (len 0), not one entry.
#[tokio::test]
async fn single_write_is_applied_and_readable_via_get_document() {
    let ctx = AgentModeWriteStreamingContext::new("aw01-single-write").await;

    let (req_tx, result) = open_write_stream(&ctx, handshake_request(&ctx.project_id)).await;
    let mut resp_stream = result.expect("Write stream should open against an agent-mode project");

    let handshake_resp = resp_stream
        .next()
        .await
        .expect("stream should yield handshake response")
        .expect("handshake response should be Ok");
    assert!(!handshake_resp.stream_id.is_empty());
    assert!(!handshake_resp.stream_token.is_empty());

    let resource_name = format!(
        "projects/{}/databases/(default)/documents/patients/p-204",
        ctx.project_id
    );
    let mut fields = HashMap::new();
    fields.insert("status".to_string(), string_field("admitted"));
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

    assert_eq!(
        write_resp.write_results.len(),
        1,
        "exactly one write_result expected — empty means the agent-side \
         commit() write_results bug is still present"
    );
    assert!(
        write_resp.write_results[0].update_time.is_some(),
        "write_result must carry a populated updateTime"
    );
    assert!(write_resp.commit_time.is_some());

    let doc = ctx
        .get_document(&resource_name)
        .await
        .expect("document must be readable via GetDocument immediately after");
    assert_eq!(doc.into_inner().name, resource_name);
}

/// AC-02: a precondition failure on a write terminates the WHOLE stream
/// (ADR-046 § Decision 4, inherited unchanged — ADR-060 § Decision 6
/// corrects DISCUSS's own "scoped, stream stays open" framing for this
/// sibling feature).
///
/// Ground-truth note (found empirically writing this test, out of scope to
/// fix per this feature's own explicit boundary): the non-agent path
/// surfaces this as `Status::Aborted` (`ws04`'s own
/// `server_side_apply_error_propagates_and_terminates_the_stream`), but for
/// `backend_mode=agent` the error crosses `AgentBackendAdapter::grpc_err`
/// (`crates/embyr-server/src/adapters/agent_backend.rs`, pre-existing,
/// explicitly out of scope for this feature) first, which collapses EVERY
/// tonic `Status` the agent returns into `CoreError::BackendUnavailable` ->
/// `Status::Internal` — losing the Aborted-vs-genuinely-unreachable
/// distinction. The malformed-write scenario below does not hit this,
/// because that error is raised at the translation layer, before the
/// adapter is ever called. Named here, not hidden, mirroring this feature's
/// own ADRs' "negative, named explicitly" convention — whole-stream
/// termination (this AC's actual substance) still holds.
#[tokio::test]
async fn precondition_failure_terminates_the_whole_stream() {
    let ctx = AgentModeWriteStreamingContext::new("aw01-precondition-failure").await;

    let (req_tx, result) = open_write_stream(&ctx, handshake_request(&ctx.project_id)).await;
    let mut resp_stream = result.expect("Write stream should open");
    let handshake_resp = resp_stream
        .next()
        .await
        .expect("stream should yield handshake response")
        .expect("handshake response should be Ok");

    let resource_name = format!(
        "projects/{}/databases/(default)/documents/patients/p-205",
        ctx.project_id
    );
    let mut fields = HashMap::new();
    fields.insert("status".to_string(), string_field("admitted"));

    req_tx
        .send(single_write_request(
            &ctx.project_id,
            &handshake_resp.stream_id,
            handshake_resp.stream_token.clone(),
            write_requiring_stale_update_time(&resource_name, fields),
        ))
        .await
        .expect("send OCC-violating write");

    let outcome = resp_stream
        .next()
        .await
        .expect("stream must yield a terminal message, not hang");
    let status = outcome.expect_err("a precondition violation must terminate the stream");
    assert_eq!(
        status.code(),
        tonic::Code::Internal,
        "backend_mode=agent surfaces this via grpc_err's pre-existing \
         BackendUnavailable collapse, not Aborted — see doc comment above"
    );

    let terminal = resp_stream.next().await;
    assert!(
        terminal.is_none(),
        "the whole stream must be closed after the error, got: {terminal:?}"
    );
}

/// AC-03: a malformed write (invalid document path — missing `/documents/`)
/// terminates the WHOLE stream with `InvalidArgument`, mirroring AC-02's own
/// whole-stream-termination shape (ADR-046 § Decision 4).
#[tokio::test]
async fn malformed_write_terminates_the_whole_stream_with_invalid_argument() {
    let ctx = AgentModeWriteStreamingContext::new("aw01-malformed-write").await;

    let (req_tx, result) = open_write_stream(&ctx, handshake_request(&ctx.project_id)).await;
    let mut resp_stream = result.expect("Write stream should open");
    let handshake_resp = resp_stream
        .next()
        .await
        .expect("stream should yield handshake response")
        .expect("handshake response should be Ok");

    let mut fields = HashMap::new();
    fields.insert("status".to_string(), string_field("admitted"));
    let malformed_write = update_write("not-a-valid-resource-name", fields);

    req_tx
        .send(single_write_request(
            &ctx.project_id,
            &handshake_resp.stream_id,
            handshake_resp.stream_token.clone(),
            malformed_write,
        ))
        .await
        .expect("send malformed write");

    let outcome = resp_stream
        .next()
        .await
        .expect("stream must yield a terminal message, not hang");
    let status = outcome.expect_err("a malformed write must terminate the stream");
    assert_eq!(status.code(), tonic::Code::InvalidArgument);

    let terminal = resp_stream.next().await;
    assert!(
        terminal.is_none(),
        "the whole stream must be closed after the error, got: {terminal:?}"
    );
}

/// AC-04: the client can close the stream cleanly after zero writes — the
/// server terminates with no further message.
#[tokio::test]
async fn client_can_close_the_stream_cleanly() {
    let ctx = AgentModeWriteStreamingContext::new("aw01-clean-close").await;

    let (req_tx, result) = open_write_stream(&ctx, handshake_request(&ctx.project_id)).await;
    let mut resp_stream = result.expect("Write stream should open");
    resp_stream
        .next()
        .await
        .expect("stream should yield handshake response")
        .expect("handshake response should be Ok");

    drop(req_tx);

    let next = resp_stream.next().await;
    assert!(
        next.is_none(),
        "server must close cleanly with no further message, got: {next:?}"
    );
}
