//! Slice 02 (US-02) — Multi-Write Batches in One `WriteRequest` (ADR-046).
//!
//! Pure composition on Slice 01's own receive-and-reply loop
//! (`run_write_session`): confirms `handle_commit`'s own existing
//! multi-`Write`-per-call translation/atomic-apply logic (already proven for
//! N writes in one `CommitRequest.writes`) is reused unchanged for N writes
//! arriving in one `WriteRequest.writes` batch. Covers AC-02-01 through
//! AC-02-04 (feature-delta.md, slice-02 brief).

use std::collections::HashMap;

use embyr_proto::firestore::{
    precondition::ConditionType, write::Operation, Document, Precondition, Write, WriteRequest,
};
use prost_types::Timestamp;
use tokio_stream::StreamExt;

#[path = "../common/mod.rs"]
mod common;
use common::{
    handshake_request, open_write_stream, string_field, update_write, SecurityRulesFullContext,
};

fn multi_write_request(
    project_id: &str,
    stream_id: &str,
    stream_token: Vec<u8>,
    writes: Vec<Write>,
) -> WriteRequest {
    WriteRequest {
        database: format!("projects/{project_id}/databases/(default)"),
        stream_id: stream_id.to_string(),
        writes,
        stream_token,
    }
}

/// A write whose `current_document` precondition (`update_time`) can never
/// match real document state — deterministically triggers `commit_transaction`'s
/// own OCC check (`CoreError::TransactionAborted`) regardless of whether the
/// target document exists yet.
fn stale_precondition_write(
    resource_name: &str,
    fields: HashMap<String, embyr_proto::firestore::Value>,
) -> Write {
    Write {
        update_mask: None,
        update_transforms: vec![],
        current_document: Some(Precondition {
            condition_type: Some(ConditionType::UpdateTime(Timestamp {
                seconds: 1,
                nanos: 0,
            })),
        }),
        operation: Some(Operation::Update(Document {
            name: resource_name.to_string(),
            fields,
            ..Default::default()
        })),
    }
}

fn owner_fields() -> HashMap<String, embyr_proto::firestore::Value> {
    let mut fields = HashMap::new();
    fields.insert("owner_id".to_string(), string_field("maria-santos"));
    fields
}

/// AC-02-01/AC-02-02: a `WriteRequest` batching N (>1) writes spanning two
/// collections is applied atomically, and `write_results` has exactly N
/// entries in request order.
#[tokio::test]
async fn batch_of_three_writes_spanning_two_collections_applied_atomically_in_order() {
    let ctx = SecurityRulesFullContext::new("ws02-multi-write").await;

    let (req_tx, result) = open_write_stream(&ctx, handshake_request(&ctx.project_id)).await;
    let mut resp_stream = result.expect("Write stream should open");

    let handshake_resp = resp_stream
        .next()
        .await
        .expect("stream should yield handshake response")
        .expect("handshake response should be Ok");

    let entry_a = format!(
        "projects/{}/databases/(default)/documents/trip_entries/maria-trip-a",
        ctx.project_id
    );
    let entry_b = format!(
        "projects/{}/databases/(default)/documents/trip_entries/maria-trip-b",
        ctx.project_id
    );
    let expense_c = format!(
        "projects/{}/databases/(default)/documents/trip_entries/maria-trip-a/expenses/exp-1",
        ctx.project_id
    );

    let writes = vec![
        update_write(&entry_a, owner_fields()),
        update_write(&entry_b, owner_fields()),
        update_write(&expense_c, owner_fields()),
    ];

    req_tx
        .send(multi_write_request(
            &ctx.project_id,
            &handshake_resp.stream_id,
            handshake_resp.stream_token.clone(),
            writes,
        ))
        .await
        .expect("send 3-write batch WriteRequest");

    let write_resp = resp_stream
        .next()
        .await
        .expect("stream should yield a write response")
        .expect("write response should be Ok");

    assert_eq!(
        write_resp.write_results.len(),
        3,
        "write_results must contain exactly one entry per write, in request order"
    );
    assert_ne!(write_resp.stream_token, handshake_resp.stream_token);

    for resource_name in [&entry_a, &entry_b, &expense_c] {
        let doc = ctx
            .get_document(resource_name, None)
            .await
            .unwrap_or_else(|e| panic!("{resource_name} must be readable via GetDocument: {e}"));
        assert_eq!(doc.into_inner().name, *resource_name);
    }
}

/// AC-02-03: a batch containing one precondition-violating write applies
/// zero writes from that batch — mirrors `Commit`'s own all-or-nothing
/// atomicity, extended to a batch of N > 1.
#[tokio::test]
async fn batch_with_one_precondition_violation_applies_none_of_its_writes() {
    let ctx = SecurityRulesFullContext::new("ws02-precondition-violation").await;

    let (req_tx, result) = open_write_stream(&ctx, handshake_request(&ctx.project_id)).await;
    let mut resp_stream = result.expect("Write stream should open");

    let handshake_resp = resp_stream
        .next()
        .await
        .expect("stream should yield handshake response")
        .expect("handshake response should be Ok");

    let entry_a = format!(
        "projects/{}/databases/(default)/documents/trip_entries/maria-trip-ok-a",
        ctx.project_id
    );
    let entry_b = format!(
        "projects/{}/databases/(default)/documents/trip_entries/maria-trip-ok-b",
        ctx.project_id
    );
    let entry_bad = format!(
        "projects/{}/databases/(default)/documents/trip_entries/maria-trip-bad",
        ctx.project_id
    );

    let writes = vec![
        update_write(&entry_a, owner_fields()),
        stale_precondition_write(&entry_bad, owner_fields()),
        update_write(&entry_b, owner_fields()),
    ];

    req_tx
        .send(multi_write_request(
            &ctx.project_id,
            &handshake_resp.stream_id,
            handshake_resp.stream_token.clone(),
            writes,
        ))
        .await
        .expect("send batch with one precondition-violating write");

    let outcome = resp_stream
        .next()
        .await
        .expect("stream should yield a response for the failed batch");
    let status = outcome.expect_err("a precondition violation must be reported as an error");
    assert_eq!(status.code(), tonic::Code::Aborted);

    for resource_name in [&entry_a, &entry_b, &entry_bad] {
        let doc = ctx.get_document(resource_name, None).await;
        assert!(
            doc.is_err(),
            "{resource_name} must NOT have been written — batch is all-or-nothing"
        );
    }
}

/// AC-02-04: successive batches (N > 1 writes each) on the same session each
/// get their own newly rotated `stream_token`, distinct from the one issued
/// before it — rotation holds batch size notwithstanding.
#[tokio::test]
async fn successive_multi_write_batches_each_get_a_freshly_rotated_token() {
    let ctx = SecurityRulesFullContext::new("ws02-successive-batches").await;

    let (req_tx, result) = open_write_stream(&ctx, handshake_request(&ctx.project_id)).await;
    let mut resp_stream = result.expect("Write stream should open");

    let handshake_resp = resp_stream
        .next()
        .await
        .expect("stream should yield handshake response")
        .expect("handshake response should be Ok");

    let batch_1 = vec![
        update_write(
            &format!(
                "projects/{}/databases/(default)/documents/trip_entries/batch1-a",
                ctx.project_id
            ),
            owner_fields(),
        ),
        update_write(
            &format!(
                "projects/{}/databases/(default)/documents/trip_entries/batch1-b",
                ctx.project_id
            ),
            owner_fields(),
        ),
    ];
    req_tx
        .send(multi_write_request(
            &ctx.project_id,
            &handshake_resp.stream_id,
            handshake_resp.stream_token.clone(),
            batch_1,
        ))
        .await
        .expect("send first batch");

    let resp_a = resp_stream
        .next()
        .await
        .expect("stream should yield first batch response")
        .expect("first batch response should be Ok");
    assert_eq!(resp_a.write_results.len(), 2);

    let batch_2 = vec![
        update_write(
            &format!(
                "projects/{}/databases/(default)/documents/trip_entries/batch2-a",
                ctx.project_id
            ),
            owner_fields(),
        ),
        update_write(
            &format!(
                "projects/{}/databases/(default)/documents/trip_entries/batch2-b",
                ctx.project_id
            ),
            owner_fields(),
        ),
    ];
    req_tx
        .send(multi_write_request(
            &ctx.project_id,
            &resp_a.stream_id,
            resp_a.stream_token.clone(),
            batch_2,
        ))
        .await
        .expect("send second batch on the SAME session");

    let resp_b = resp_stream
        .next()
        .await
        .expect("stream should yield second batch response")
        .expect("second batch response should be Ok");
    assert_eq!(resp_b.write_results.len(), 2);

    assert_ne!(handshake_resp.stream_token, resp_a.stream_token);
    assert_ne!(resp_a.stream_token, resp_b.stream_token);
}
