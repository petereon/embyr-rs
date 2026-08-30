//! firestore-write-streaming (Slice 01, ADR-046) — the `Write` bidi-stream's
//! own spawned-task session loop.
//!
//! Sibling to `handler.rs`, not under `realtime/` (ADR-046 § Decision 3):
//! `Write` is BC-2's own mutation-apply mechanism; `realtime/` is BC-3's
//! Listen-specific module, reused by `handle_write` only for its bidi-stream
//! SCAFFOLD SHAPE, not as a shared domain module.
//!
//! All session state (`stream_id`, the current `stream_token`) is local to
//! `run_write_session`'s own stack — no `stream_id`-keyed registry anywhere
//! in this file (ADR-046 § Decision Driver 4: `stream_id` is an opaque echo
//! value, never a lookup key).

use chrono::SecondsFormat;
use prost_types::Timestamp;
use tokio::sync::mpsc;
use tokio_stream::StreamExt as _;
use tonic::Status;

use embyr_core::{
    client_identity::VerifiedEndUserIdentity,
    domain::{project::ProjectId, transaction::TransactionOptions},
};
use embyr_proto::firestore::{WriteRequest, WriteResponse};

use crate::adapters::{credential_cache::SharedBackendAdapter, system_db::SystemDb};

use super::handler::{core_error_to_status, FirestoreService};

/// `stream_id` format (ADR-046, informative from `docs/SPEC.md`): 16-hex-
/// encoded unix nanoseconds. Held constant for the session's lifetime, never
/// used as a lookup key — a same-nanosecond collision across two concurrent
/// handshakes has zero behavioral consequence.
pub(crate) fn generate_stream_id() -> String {
    let nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
    format!("{nanos:016x}")
}

/// `stream_token` format (ADR-046, informative from `docs/SPEC.md`):
/// RFC3339Nano timestamp, UTF-8 bytes into the `bytes stream_token` field —
/// opaque to any real client. Regenerated on every response, including the
/// handshake response.
pub(crate) fn generate_stream_token() -> String {
    chrono::Utc::now().to_rfc3339_opts(SecondsFormat::Nanos, true)
}

fn now_timestamp() -> Timestamp {
    let now = chrono::Utc::now();
    Timestamp {
        seconds: now.timestamp(),
        nanos: now.timestamp_subsec_nanos() as i32,
    }
}

/// The spawned task's own body (ADR-046 § Decision 2-3): owns the
/// `Streaming<WriteRequest>` and the response `mpsc::Sender` exclusively.
/// Sends the handshake `WriteResponse` first, then loops processing every
/// subsequent `WriteRequest` as its own apply-and-reply cycle — unlike
/// `handle_listen`'s spawned task, which only drains further messages after
/// its one-time setup.
pub(crate) async fn run_write_session(
    mut in_stream: tonic::Streaming<WriteRequest>,
    tx: mpsc::Sender<Result<WriteResponse, Status>>,
    adapter: SharedBackendAdapter,
    system_db: std::sync::Arc<SystemDb>,
    project_id_str: String,
    verified_identity: Option<VerifiedEndUserIdentity>,
) {
    let stream_id = generate_stream_id();
    let mut current_token = generate_stream_token();

    let handshake = WriteResponse {
        stream_id: stream_id.clone(),
        stream_token: current_token.clone().into_bytes(),
        write_results: vec![],
        commit_time: Some(now_timestamp()),
    };
    if tx.send(Ok(handshake)).await.is_err() {
        return;
    }

    loop {
        let msg = match in_stream.next().await {
            // Client `io.EOF` — clean close, zero further writes (AC-01-03).
            None => return,
            // Client cancellation — clean close, no error surfaced.
            Some(Err(status)) if status.code() == tonic::Code::Cancelled => return,
            // Any other error — propagate as-is, terminating the stream.
            Some(Err(status)) => {
                let _ = tx.send(Err(status)).await;
                return;
            }
            Some(Ok(msg)) => msg,
        };

        let project_id = match ProjectId::new(&project_id_str) {
            Ok(p) => p,
            Err(e) => {
                let _ = tx.send(Err(Status::invalid_argument(e.to_string()))).await;
                return;
            }
        };

        let domain_writes = match FirestoreService::translate_writes_for_commit(
            &system_db,
            &adapter,
            &project_id_str,
            verified_identity.as_ref(),
            &msg.writes,
        )
        .await
        {
            Ok(writes) => writes,
            Err(status) => {
                let _ = tx.send(Err(status)).await;
                return;
            }
        };

        // Ad-hoc, client-invisible transaction (ADR-046 § Context finding
        // 1): `commit_transaction` requires a pre-existing `active`-status
        // transaction row; `Write`'s own wire contract has no transaction
        // field, so one is synthesized immediately before each apply.
        let txn_id = match adapter
            .begin_transaction(&project_id, TransactionOptions::ReadWrite)
            .await
        {
            Ok(id) => id,
            Err(e) => {
                let _ = tx.send(Err(core_error_to_status(e))).await;
                return;
            }
        };

        let write_results = match adapter
            .commit_transaction(&project_id, &txn_id, domain_writes)
            .await
        {
            Ok(results) => results,
            Err(e) => {
                let _ = tx.send(Err(core_error_to_status(e))).await;
                return;
            }
        };

        current_token = generate_stream_token();
        let proto_results: Vec<embyr_proto::firestore::WriteResult> = write_results
            .into_iter()
            .map(|wr| embyr_proto::firestore::WriteResult {
                update_time: Some(Timestamp {
                    seconds: wr.update_time.0,
                    nanos: wr.update_time.1,
                }),
                transform_results: vec![],
            })
            .collect();

        let response = WriteResponse {
            stream_id: stream_id.clone(),
            stream_token: current_token.clone().into_bytes(),
            write_results: proto_results,
            commit_time: Some(now_timestamp()),
        };
        if tx.send(Ok(response)).await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_stream_id_is_16_lowercase_hex_chars() {
        let id = generate_stream_id();
        assert_eq!(id.len(), 16, "stream_id must be 16 hex chars: {id}");
        assert!(
            id.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "stream_id must be lowercase hex: {id}"
        );
    }

    #[test]
    fn generate_stream_token_is_rfc3339_nanos() {
        let token = generate_stream_token();
        chrono::DateTime::parse_from_rfc3339(&token)
            .unwrap_or_else(|e| panic!("stream_token {token} is not RFC3339: {e}"));
    }
}
