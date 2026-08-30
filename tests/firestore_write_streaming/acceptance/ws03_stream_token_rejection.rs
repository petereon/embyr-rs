//! Slice 03 (US-03) — A Stale or Mismatched `stream_token` Is Safely Rejected
//! (ADR-046 § Decision 3 / Escalation 1: `Status::aborted`, whole-stream
//! termination — reuses Slice 04's own error-exit path).
//!
//! Driving port: gRPC data port (:8080) — `Write` bidirectional streaming RPC.
//! Covers AC-03-01 through AC-03-03 (slice-03-stream-token-rejection.md).
//!
//! Black-box by design (Mandate 1): each scenario asserts OBSERVABLE outcomes
//! only — the response stream's `Err`/`Ok` shape and document state via a
//! fresh `GetDocument` call, never which internal `match` arm of
//! `run_write_session`'s loop fired.

use std::collections::HashMap;

use tokio_stream::StreamExt;

#[path = "../common/mod.rs"]
mod common;
use common::{
    handshake_request, open_write_stream, single_write_request, string_field, update_write,
    SecurityRulesFullContext,
};

/// AC-03-01 / AC-03-03: a `WriteRequest` presenting a `stream_token`
/// completely unrelated to the one just issued by the handshake is rejected
/// before any write is attempted, and no document is modified.
#[tokio::test]
async fn wrong_stream_token_is_rejected_before_any_write_is_applied() {
    let ctx = SecurityRulesFullContext::new("ws03-wrong-token").await;

    let (req_tx, result) = open_write_stream(&ctx, handshake_request(&ctx.project_id)).await;
    let mut resp_stream = result.expect("Write stream should open");
    let handshake_resp = resp_stream
        .next()
        .await
        .expect("stream should yield handshake response")
        .expect("handshake response should be Ok");

    let resource_name = format!(
        "projects/{}/databases/(default)/documents/trip_entries/wrong-token-doc",
        ctx.project_id
    );
    let mut fields = HashMap::new();
    fields.insert("owner_id".to_string(), string_field("maria-santos"));

    req_tx
        .send(single_write_request(
            &ctx.project_id,
            &handshake_resp.stream_id,
            b"completely-unrelated-wrong-token".to_vec(),
            update_write(&resource_name, fields),
        ))
        .await
        .expect("send WriteRequest presenting a wrong stream_token");

    let rejection = resp_stream
        .next()
        .await
        .expect("stream must yield a terminal message, not hang");
    let status = rejection.expect_err("a mismatched stream_token must surface as Err");
    assert_eq!(status.code(), tonic::Code::Aborted);

    let doc = ctx.get_document(&resource_name, None).await;
    assert!(doc.is_err(), "no document must have been written");
}

/// AC-03-01: an out-of-order resend presenting a previously-valid-but-now-
/// superseded token (issued two round trips ago) is rejected identically to a
/// wholly unrelated token — only the single most-recently-issued token is
/// ever valid.
#[tokio::test]
async fn superseded_stream_token_from_two_round_trips_ago_is_rejected() {
    let ctx = SecurityRulesFullContext::new("ws03-superseded-token").await;

    let (req_tx, result) = open_write_stream(&ctx, handshake_request(&ctx.project_id)).await;
    let mut resp_stream = result.expect("Write stream should open");
    let handshake_resp = resp_stream
        .next()
        .await
        .expect("stream should yield handshake response")
        .expect("handshake response should be Ok");

    let resource_a = format!(
        "projects/{}/databases/(default)/documents/trip_entries/superseded-token-doc-a",
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
        .expect("send first write, presenting the handshake token");
    let resp_a = resp_stream
        .next()
        .await
        .expect("stream should yield first write response")
        .expect("first write response should be Ok");
    assert_eq!(resp_a.write_results.len(), 1);

    let resource_b = format!(
        "projects/{}/databases/(default)/documents/trip_entries/superseded-token-doc-b",
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
        .expect("send second write, presenting the first response's own token");
    let resp_b = resp_stream
        .next()
        .await
        .expect("stream should yield second write response")
        .expect("second write response should be Ok");
    assert_eq!(resp_b.write_results.len(), 1);

    // A third WriteRequest resends the handshake's own token — now superseded
    // twice over (by resp_a's token, then by resp_b's token).
    let resource_c = format!(
        "projects/{}/databases/(default)/documents/trip_entries/superseded-token-doc-c",
        ctx.project_id
    );
    let mut fields_c = HashMap::new();
    fields_c.insert("owner_id".to_string(), string_field("maria-santos"));
    req_tx
        .send(single_write_request(
            &ctx.project_id,
            &resp_b.stream_id,
            handshake_resp.stream_token.clone(),
            update_write(&resource_c, fields_c),
        ))
        .await
        .expect("resend the now-superseded handshake token out of order");

    let rejection = resp_stream
        .next()
        .await
        .expect("stream must yield a terminal message, not hang");
    let status = rejection.expect_err("a superseded stream_token must surface as Err");
    assert_eq!(status.code(), tonic::Code::Aborted);

    let doc = ctx.get_document(&resource_c, None).await;
    assert!(
        doc.is_err(),
        "no document must have been written for the rejected request"
    );
}

/// AC-03-02: a `WriteRequest` presenting the correct, freshly-issued token is
/// accepted and applied normally — confirms the rejection rule does not also
/// reject valid tokens (regression guard against over-rejecting).
#[tokio::test]
async fn correct_freshly_issued_stream_token_is_accepted_normally() {
    let ctx = SecurityRulesFullContext::new("ws03-correct-token").await;

    let (req_tx, result) = open_write_stream(&ctx, handshake_request(&ctx.project_id)).await;
    let mut resp_stream = result.expect("Write stream should open");
    let handshake_resp = resp_stream
        .next()
        .await
        .expect("stream should yield handshake response")
        .expect("handshake response should be Ok");

    let resource_name = format!(
        "projects/{}/databases/(default)/documents/trip_entries/correct-token-doc",
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
        .expect("send WriteRequest presenting the correct, freshly-issued token");

    let write_resp = resp_stream
        .next()
        .await
        .expect("stream should yield a write response")
        .expect("write response should be Ok — a valid token must not be rejected");
    assert_eq!(write_resp.write_results.len(), 1);

    let doc = ctx
        .get_document(&resource_name, None)
        .await
        .expect("document must be readable via GetDocument immediately after");
    assert_eq!(doc.into_inner().name, resource_name);
}
