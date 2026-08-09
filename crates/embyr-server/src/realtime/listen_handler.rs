/// Handle an AddTarget Listen session: initial snapshot + keep-alive + NOTIFY fan-out.
use std::{sync::Arc, time::Duration};

use chrono::{Duration as ChronoDuration, Utc};
use embyr_core::domain::{
    document::CollectionPath,
    project::ProjectId,
    query::StructuredQuery as DomainQuery,
};
use embyr_proto::firestore::{
    listen_request, listen_response,
    target::TargetType,
    target_change::TargetChangeType,
    DocumentChange, DocumentDelete, ListenRequest, ListenResponse, TargetChange,
};
use tonic::Status;
use tokio::sync::mpsc;

use crate::{
    adapters::credential_cache::SharedBackendAdapter,
    encoding::firestore_proto::document_to_proto,
    realtime::{
        listen_registry::{ListenEvent, ListenRegistry},
        resume_token as rt,
    },
};

/// Process an AddTarget message: register subscriber, run the initial snapshot query,
/// send `DocumentChange` events for each doc, send `TargetChange(CURRENT)`,
/// then loop on keepalive timer, registry receiver, and RESET notifier.
///
/// Slow consumer detection: if fan_out detects the subscriber channel is full,
/// it fires `reset_notify`. The handler responds by sending `TargetChange(RESET)`
/// and closing the stream.
pub async fn handle_add_target(
    first_msg: &ListenRequest,
    adapter: &SharedBackendAdapter,
    tx: &mpsc::Sender<Result<ListenResponse, Status>>,
    keepalive: Duration,
    registry: Arc<ListenRegistry>,
    channel: &str,
    resume_token: Option<Vec<u8>>,
) -> Result<(), String> {
    // Extract AddTarget from the first message.
    let add_target = match &first_msg.target_change {
        Some(listen_request::TargetChange::AddTarget(t)) => t,
        _ => return Err("first message must be AddTarget".into()),
    };

    let target_id = add_target.target_id;

    // Extract collection_id and project_id from the QueryTarget.
    let (project_id, collection_id) = match &add_target.target_type {
        Some(TargetType::Query(qt)) => {
            let collection_id = collection_id_from_query_target(qt)?;
            let project_id = project_id_from_parent(&qt.parent)?;
            (project_id, collection_id)
        }
        _ => return Err("target must have Query target type".into()),
    };

    let pid = ProjectId::new(&project_id).map_err(|e| e.to_string())?;
    let collection = CollectionPath {
        project_id: pid,
        collection_path: collection_id,
    };

    // Decode resume token: fresh (<=24h) tokens enable delta delivery.
    let since_update_time = resume_token
        .as_deref()
        .filter(|t| !t.is_empty() && !rt::is_stale(t))
        .and_then(rt::decode_ts);

    let domain_query = DomainQuery {
        collection_id: collection.collection_path.clone(),
        all_descendants: false,
        filter: None,
        order_by: vec![],
        limit: None,
        offset: None,
        start_at: None,
        end_at: None,
        since_update_time,
    };

    // Record snapshot start time BEFORE the query so the resume token is anchored
    // at a time that is <= all documents in the initial snapshot. We add 1 second to
    // avoid sub-second precision issues: docs written in the same second as snapshot_ts
    // have update_time <= (snapshot_ts + 1s), so the delta filter update_time > token_ts
    // correctly excludes them.
    let snapshot_ts = Utc::now() + ChronoDuration::seconds(1);

    // Register subscriber BEFORE running the initial snapshot to avoid missing
    // concurrent writes.
    let handle = registry.register(channel).await;
    let mut event_rx = handle.event_rx;
    let reset_notify = handle.reset_notify;

    // Run the initial snapshot query (or delta query if resume token is fresh).
    let docs = adapter
        .run_query(&collection, &domain_query, None)
        .await
        .map_err(|e| e.to_string())?;

    // Send one DocumentChange per doc.
    for doc in docs {
        let proto_doc = document_to_proto(doc);
        let response = ListenResponse {
            response_type: Some(listen_response::ResponseType::DocumentChange(DocumentChange {
                document: Some(proto_doc),
                target_ids: vec![target_id],
                removed_target_ids: vec![],
            })),
        };
        tx.send(Ok(response)).await.map_err(|_| "channel closed".to_string())?;
    }

    // Send CURRENT marker.
    let current = ListenResponse {
        response_type: Some(listen_response::ResponseType::TargetChange(TargetChange {
            target_change_type: TargetChangeType::Current as i32,
            target_ids: vec![target_id],
            ..Default::default()
        })),
    };
    tx.send(Ok(current)).await.map_err(|_| "channel closed".to_string())?;

    // Send NO_CHANGE with fresh resume token immediately after CURRENT.
    // The token encodes snapshot_ts (= now + 1s at snapshot time), ensuring that
    // on reconnect the delta filter update_time > token_ts excludes all initial-snapshot docs.
    let fresh_token = rt::encode(snapshot_ts, collection.project_id.as_str());
    let no_change_with_token = ListenResponse {
        response_type: Some(listen_response::ResponseType::TargetChange(TargetChange {
            target_change_type: TargetChangeType::NoChange as i32,
            resume_token: fresh_token,
            ..Default::default()
        })),
    };
    tx.send(Ok(no_change_with_token)).await.map_err(|_| "channel closed".to_string())?;

    // Main loop: keepalive + NOTIFY events + RESET signal.
    loop {
        tokio::select! {
            _ = tokio::time::sleep(keepalive) => {
                // Keepalive token is anchored 1s in the future for the same precision reason.
                let keepalive_token = rt::encode(
                    Utc::now() + ChronoDuration::seconds(1),
                    collection.project_id.as_str(),
                );
                let no_change = ListenResponse {
                    response_type: Some(listen_response::ResponseType::TargetChange(TargetChange {
                        target_change_type: TargetChangeType::NoChange as i32,
                        target_ids: vec![],
                        resume_token: keepalive_token,
                        ..Default::default()
                    })),
                };
                if tx.send(Ok(no_change)).await.is_err() {
                    // Client disconnected.
                    break;
                }
            }
            event = event_rx.recv() => {
                match event {
                    Some(ListenEvent::Changed(doc)) => {
                        let proto_doc = document_to_proto(doc);
                        let response = ListenResponse {
                            response_type: Some(listen_response::ResponseType::DocumentChange(DocumentChange {
                                document: Some(proto_doc),
                                target_ids: vec![target_id],
                                removed_target_ids: vec![],
                            })),
                        };
                        if tx.send(Ok(response)).await.is_err() {
                            break;
                        }
                    }
                    Some(ListenEvent::Removed(path)) => {
                        let doc_name = format!(
                            "projects/{}/databases/(default)/documents/{}/{}",
                            path.project_id.as_str(),
                            path.collection_path,
                            path.document_id,
                        );
                        let response = ListenResponse {
                            response_type: Some(listen_response::ResponseType::DocumentDelete(
                                DocumentDelete {
                                    document: doc_name,
                                    removed_target_ids: vec![target_id],
                                    read_time: None,
                                },
                            )),
                        };
                        if tx.send(Ok(response)).await.is_err() {
                            break;
                        }
                    }
                    None => {
                        // Registry channel closed unexpectedly.
                        break;
                    }
                }
            }
            _ = reset_notify.notified() => {
                // Slow consumer detected by fan_out — send RESET.
                let reset = ListenResponse {
                    response_type: Some(listen_response::ResponseType::TargetChange(
                        TargetChange {
                            target_change_type: TargetChangeType::Reset as i32,
                            target_ids: vec![target_id],
                            ..Default::default()
                        },
                    )),
                };
                // Use try_send; if the client's gRPC channel is also full, the
                // RESET may be lost, but the stream will close momentarily anyway.
                let _ = tx.send(Ok(reset)).await;
                break;
            }
        }
    }

    Ok(())
}

/// Extract `collection_id` from a `QueryTarget` by reading `from[0]` in the
/// `StructuredQuery`.
fn collection_id_from_query_target(qt: &embyr_proto::firestore::target::QueryTarget) -> Result<String, String> {
    use embyr_proto::firestore::target::query_target::QueryType;
    match &qt.query_type {
        Some(QueryType::StructuredQuery(sq)) => {
            sq.from
                .first()
                .map(|cs| cs.collection_id.clone())
                .ok_or_else(|| "structured query has no collection selector".into())
        }
        None => Err("QueryTarget has no query_type".into()),
    }
}

/// Extract `project_id` from a Firestore parent path.
///
/// Format: `projects/{pid}/databases/(default)/documents`
fn project_id_from_parent(parent: &str) -> Result<String, String> {
    let mut parts = parent.splitn(5, '/');
    match (parts.next(), parts.next()) {
        (Some("projects"), Some(pid)) if !pid.is_empty() => Ok(pid.to_string()),
        _ => Err(format!("invalid parent path: {parent}")),
    }
}
