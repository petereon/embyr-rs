//! Slice 02 (US-02, LAST slice) — Multi-Write Session and Disconnect
//! Recovery for `backend_mode=agent` (ADR-060).
//!
//! Per DESIGN's own finding, no new mechanism exists to build here: the
//! reused, unchanged `write_stream.rs` (ADR-046) already handles multi-write
//! sessions and disconnect recovery uniformly across every backend mode.
//! This slice adds coverage proving that composition holds for agent-mode
//! too — zero new production code.

use std::collections::HashMap;
use std::time::Duration;

use tokio_stream::StreamExt;

#[path = "../common/mod.rs"]
mod common;
use common::{
    handshake_request, open_write_stream, single_write_request, string_field, update_write,
    AgentModeWriteStreamingContext,
};

async fn document_version(ctx: &AgentModeWriteStreamingContext, doc_id: &str) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT version FROM documents WHERE project_id = $1 \
         AND collection_path = 'patients' AND document_id = $2",
    )
    .bind(&ctx.project_id)
    .bind(doc_id)
    .fetch_one(&ctx.agent_pool)
    .await
    .expect("read document version from agent's own Postgres")
}

/// AC-01: 3 sequential writes to 3 different documents, in one session, are
/// each independently acknowledged with a populated `updateTime`, and each
/// document is readable afterward.
#[tokio::test]
async fn three_sequential_writes_in_one_session_are_each_acknowledged_independently() {
    let ctx = AgentModeWriteStreamingContext::new("aw02-three-writes").await;

    let (req_tx, result) = open_write_stream(&ctx, handshake_request(&ctx.project_id)).await;
    let mut resp_stream = result.expect("Write stream should open");
    let mut handshake_resp = resp_stream
        .next()
        .await
        .expect("handshake response")
        .expect("handshake response should be Ok");

    let doc_ids = ["p-301", "p-302", "p-303"];
    for doc_id in doc_ids {
        let resource_name = format!(
            "projects/{}/databases/(default)/documents/patients/{doc_id}",
            ctx.project_id
        );
        let mut fields = HashMap::new();
        fields.insert("status".to_string(), string_field("admitted"));

        req_tx
            .send(single_write_request(
                &ctx.project_id,
                &handshake_resp.stream_id,
                handshake_resp.stream_token.clone(),
                update_write(&resource_name, fields),
            ))
            .await
            .expect("send write");

        let write_resp = resp_stream
            .next()
            .await
            .expect("write response")
            .expect("write response should be Ok");
        assert_eq!(write_resp.write_results.len(), 1);
        assert!(write_resp.write_results[0].update_time.is_some());

        let doc = ctx
            .get_document(&resource_name)
            .await
            .unwrap_or_else(|e| panic!("{resource_name} must be readable: {e}"));
        assert_eq!(doc.into_inner().name, resource_name);

        handshake_resp.stream_id = write_resp.stream_id;
        handshake_resp.stream_token = write_resp.stream_token;
    }
}

/// AC-02/AC-03: a stream disconnect after 2 acknowledged writes does not
/// lose them, and a NEW stream opened afterward accepts further writes
/// normally, without re-applying the already-acknowledged ones.
#[tokio::test]
async fn disconnect_preserves_acknowledged_writes_and_new_stream_continues_normally() {
    let ctx = AgentModeWriteStreamingContext::new("aw02-disconnect-reconnect").await;

    let resource_a = format!(
        "projects/{}/databases/(default)/documents/patients/p-401",
        ctx.project_id
    );
    let resource_b = format!(
        "projects/{}/databases/(default)/documents/patients/p-402",
        ctx.project_id
    );

    {
        let (req_tx, result) = open_write_stream(&ctx, handshake_request(&ctx.project_id)).await;
        let mut resp_stream = result.expect("Write stream should open");
        let mut handshake_resp = resp_stream
            .next()
            .await
            .expect("handshake response")
            .expect("handshake response should be Ok");

        for resource_name in [&resource_a, &resource_b] {
            let mut fields = HashMap::new();
            fields.insert("status".to_string(), string_field("admitted"));
            req_tx
                .send(single_write_request(
                    &ctx.project_id,
                    &handshake_resp.stream_id,
                    handshake_resp.stream_token.clone(),
                    update_write(resource_name, fields),
                ))
                .await
                .expect("send write");
            let write_resp = resp_stream
                .next()
                .await
                .expect("write response")
                .expect("write response should be Ok");
            handshake_resp.stream_id = write_resp.stream_id;
            handshake_resp.stream_token = write_resp.stream_token;
        }

        // Disconnect: drop both halves of the call without a clean close.
        drop(req_tx);
        drop(resp_stream);
    }

    // AC-02: both already-acknowledged writes remain durably persisted.
    for resource_name in [&resource_a, &resource_b] {
        let doc = ctx
            .get_document(resource_name)
            .await
            .unwrap_or_else(|e| panic!("{resource_name} must remain readable post-disconnect: {e}"));
        assert_eq!(doc.into_inner().name, *resource_name);
    }
    assert_eq!(document_version(&ctx, "p-401").await, 1);
    assert_eq!(document_version(&ctx, "p-402").await, 1);

    // AC-03: a brand-new stream accepts a further write normally.
    let resource_c = format!(
        "projects/{}/databases/(default)/documents/patients/p-403",
        ctx.project_id
    );
    let (req_tx2, result2) = open_write_stream(&ctx, handshake_request(&ctx.project_id)).await;
    let mut resp_stream2 = result2.expect("a new session must open after the disconnect");
    let handshake2 = resp_stream2
        .next()
        .await
        .expect("handshake response")
        .expect("handshake response should be Ok");

    let mut fields = HashMap::new();
    fields.insert("status".to_string(), string_field("admitted"));
    req_tx2
        .send(single_write_request(
            &ctx.project_id,
            &handshake2.stream_id,
            handshake2.stream_token.clone(),
            update_write(&resource_c, fields),
        ))
        .await
        .expect("send write on new session");
    let write_resp2 = resp_stream2
        .next()
        .await
        .expect("write response on new session")
        .expect("write response should be Ok");
    assert_eq!(write_resp2.write_results.len(), 1);

    // No duplicate application: the two pre-disconnect documents are still
    // at version 1 (not re-applied by the new session).
    assert_eq!(document_version(&ctx, "p-401").await, 1);
    assert_eq!(document_version(&ctx, "p-402").await, 1);
    assert_eq!(document_version(&ctx, "p-403").await, 1);
}

/// AC-04: two sequential writes to the SAME document, in one session, apply
/// in order — final state reflects the second write, and the document's
/// `version` (this codebase's own OCC generation counter) increments once
/// per write.
#[tokio::test]
async fn two_sequential_writes_to_same_document_apply_in_order() {
    let ctx = AgentModeWriteStreamingContext::new("aw02-same-doc-order").await;

    let resource_name = format!(
        "projects/{}/databases/(default)/documents/patients/p-501",
        ctx.project_id
    );

    let (req_tx, result) = open_write_stream(&ctx, handshake_request(&ctx.project_id)).await;
    let mut resp_stream = result.expect("Write stream should open");
    let mut handshake_resp = resp_stream
        .next()
        .await
        .expect("handshake response")
        .expect("handshake response should be Ok");

    for status in ["in_review", "discharged"] {
        let mut fields = HashMap::new();
        fields.insert("status".to_string(), string_field(status));
        req_tx
            .send(single_write_request(
                &ctx.project_id,
                &handshake_resp.stream_id,
                handshake_resp.stream_token.clone(),
                update_write(&resource_name, fields),
            ))
            .await
            .expect("send write");
        let write_resp = resp_stream
            .next()
            .await
            .expect("write response")
            .expect("write response should be Ok");
        handshake_resp.stream_id = write_resp.stream_id;
        handshake_resp.stream_token = write_resp.stream_token;
    }

    let doc = ctx
        .get_document(&resource_name)
        .await
        .expect("document must be readable")
        .into_inner();
    let status = doc.fields["status"].value_type.as_ref().expect("status field present");
    assert!(
        matches!(status, embyr_proto::firestore::value::ValueType::StringValue(s) if s == "discharged"),
        "final state must reflect the SECOND write"
    );
    assert_eq!(
        document_version(&ctx, "p-501").await,
        2,
        "version must increment once per write"
    );
}

/// AC-05: a stream sitting idle between writes (no traffic for a few
/// seconds) is NOT killed or errored purely from elapsed idle time — the
/// AC is satisfied by this absence (ADR-060 § Decision 7: no idle-reaping
/// mechanism exists anywhere in the reused loop, for any backend).
#[tokio::test]
async fn idle_stream_raises_no_error_purely_from_elapsed_idle_time() {
    let ctx = AgentModeWriteStreamingContext::new("aw02-idle-stream").await;

    let (req_tx, result) = open_write_stream(&ctx, handshake_request(&ctx.project_id)).await;
    let mut resp_stream = result.expect("Write stream should open");
    let handshake_resp = resp_stream
        .next()
        .await
        .expect("handshake response")
        .expect("handshake response should be Ok");

    // Idle period: no WriteRequest sent for a few seconds.
    tokio::time::sleep(Duration::from_secs(2)).await;

    let resource_name = format!(
        "projects/{}/databases/(default)/documents/patients/p-601",
        ctx.project_id
    );
    let mut fields = HashMap::new();
    fields.insert("status".to_string(), string_field("admitted"));
    req_tx
        .send(single_write_request(
            &ctx.project_id,
            &handshake_resp.stream_id,
            handshake_resp.stream_token.clone(),
            update_write(&resource_name, fields),
        ))
        .await
        .expect("send write after idle period");

    let write_resp = resp_stream
        .next()
        .await
        .expect("stream must still be alive and responsive after the idle period")
        .expect("write after an idle period must succeed, not error");
    assert_eq!(write_resp.write_results.len(), 1);
}
