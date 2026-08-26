/// Handle an AddTarget Listen session: initial snapshot + keep-alive + NOTIFY fan-out.
use std::{collections::BTreeMap, sync::Arc, time::Duration};

use chrono::{Duration as ChronoDuration, Utc};
use embyr_core::{
    access_control::{
        check_query_compliance, evaluate, parse_condition, AuthContext, Condition,
        EvaluationOutcome, QueryComplianceOutcome,
    },
    client_identity::VerifiedEndUserIdentity,
    domain::{
        document::CollectionPath, field_value::FieldValue, project::ProjectId,
        query::StructuredQuery as DomainQuery,
    },
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
    adapters::{credential_cache::SharedBackendAdapter, system_db::SystemDb},
    encoding::firestore_proto::document_to_proto,
    grpc::handler::query_compliance_rejection,
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
///
/// security-rules-realtime (ADR-033 § Decision — Per-Event Composition,
/// US-04): every live `Changed` event is re-checked against `condition` (the
/// loop-lifetime local built once below, at subscribe time) via `evaluate()`
/// — reused completely unmodified from `embyr_core::access_control`.
pub async fn handle_add_target(
    first_msg: &ListenRequest,
    adapter: &SharedBackendAdapter,
    system_db: &Arc<SystemDb>,
    verified_identity: Option<VerifiedEndUserIdentity>,
    tx: &mpsc::Sender<Result<ListenResponse, Status>>,
    keepalive: Duration,
    registry: Arc<ListenRegistry>,
    channel: &str,
    resume_token: Option<Vec<u8>>,
) -> Result<(), Status> {
    // Extract AddTarget from the first message.
    let add_target = match &first_msg.target_change {
        Some(listen_request::TargetChange::AddTarget(t)) => t,
        _ => return Err(Status::invalid_argument("first message must be AddTarget")),
    };

    let target_id = add_target.target_id;

    // Extract collection_id, project_id, and the caller's own query filter
    // from the QueryTarget. security-rules-realtime (ADR-033 § Decision —
    // Subscribe-Time Composition, US-02): the filter is extracted and
    // translated via the SAME `translate_filter()` `RunQuery` already uses
    // — previously hardcoded to `None` (Finding 2), silently returning the
    // entire unfiltered collection regardless of what the client's own
    // StructuredQuery specified.
    let (project_id, collection_id, filter) = match &add_target.target_type {
        Some(TargetType::Query(qt)) => {
            let collection_id = collection_id_from_query_target(qt).map_err(Status::internal)?;
            let project_id = project_id_from_parent(&qt.parent).map_err(Status::internal)?;
            let filter = filter_from_query_target(qt).map_err(Status::internal)?;
            (project_id, collection_id, filter)
        }
        _ => return Err(Status::invalid_argument("target must have Query target type")),
    };

    let pid = ProjectId::new(&project_id).map_err(|e| Status::internal(e.to_string()))?;
    let collection = CollectionPath {
        project_id: pid,
        collection_path: collection_id,
    };

    // security-rules-realtime (ADR-033 § Decision — Subscribe-Time
    // Composition, US-03): the ONE-TIME subscribe-time compliance gate —
    // the IDENTICAL composition shape `handle_run_query`'s own non-group arm
    // already uses (ADR-031), applied at a new call site inside an async
    // streaming handler rather than a request/response one. Runs strictly
    // BEFORE `registry.register()`/`adapter.run_query()` below — a
    // non-compliant subscription is rejected outright, before any row is
    // read for the initial snapshot (AC-17-114).
    let auth_ctx = verified_identity
        .as_ref()
        .map(|v| AuthContext { uid: v.end_user_id.clone() });

    let rule_row = system_db
        .get_access_rule(&project_id, &collection.collection_path)
        .await
        .map_err(|e| Status::internal(e.to_string()))?;

    // `condition` is retained for the REST OF THIS FUNCTION's lifetime — see
    // ADR-033 § Decision — Per-Event Composition (Slice 04's own extension
    // point, not consumed here).
    let condition: Option<Condition> = match rule_row {
        None => None, // US-06: no rule -> unrestricted, both subscribe-time and per-event.
        Some(row) => {
            let condition = parse_condition(&row.condition_source).map_err(|e| {
                Status::internal(format!("stored access rule failed to re-parse: {e:?}"))
            })?;
            match check_query_compliance(&condition, filter.as_ref(), auth_ctx.as_ref()) {
                QueryComplianceOutcome::Admitted => {}
                // AC-17-116: SAME query_compliance_rejection() RunQuery
                // already uses -> Status::permission_denied with the SAME
                // [REASON_CODE] convention -> distinguishable from
                // authenticate()'s own Status::unauthenticated and from
                // Status::internal (genuine server error).
                outcome => return Err(query_compliance_rejection(&outcome)),
            }
            Some(condition)
        }
    };

    // Decode resume token: fresh (<=24h) tokens enable delta delivery.
    let since_update_time = resume_token
        .as_deref()
        .filter(|t| !t.is_empty() && !rt::is_stale(t))
        .and_then(rt::decode_ts);

    let domain_query = DomainQuery {
        collection_id: collection.collection_path.clone(),
        all_descendants: false,
        filter,
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
        .map_err(|e| Status::internal(e.to_string()))?;

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
        tx.send(Ok(response))
            .await
            .map_err(|_| Status::internal("channel closed"))?;
    }

    // Send CURRENT marker.
    let current = ListenResponse {
        response_type: Some(listen_response::ResponseType::TargetChange(TargetChange {
            target_change_type: TargetChangeType::Current as i32,
            target_ids: vec![target_id],
            ..Default::default()
        })),
    };
    tx.send(Ok(current))
        .await
        .map_err(|_| Status::internal("channel closed"))?;

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
    tx.send(Ok(no_change_with_token))
        .await
        .map_err(|_| Status::internal("channel closed"))?;

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
                        // US-01 (Finding 5 fix) — FIRST, unconditional, rule-independent.
                        if doc.path.collection_path != collection.collection_path {
                            continue;
                        }
                        // security-rules-realtime (ADR-033 § Decision —
                        // Per-Event Composition, US-04): re-check the
                        // ALREADY-in-memory `doc.fields` against the
                        // subscription's own rule, reusing `evaluate()`
                        // (ADR-027/030) completely unmodified — zero
                        // additional I/O (AC-17-121). `condition`/`auth_ctx`
                        // are the SAME loop-lifetime locals built once at
                        // subscribe time (Slice 03), never re-parsed or
                        // re-fetched per event.
                        if let Some(condition) = &condition {
                            let empty_fields: BTreeMap<String, FieldValue> = BTreeMap::new();
                            // ADR-030's own empty-map convention: Listen has
                            // no "proposed new document" concept, mirrors
                            // handle_get_document exactly (AC-17-120).
                            if evaluate(condition, auth_ctx.as_ref(), &doc.fields, &empty_fields)
                                == EvaluationOutcome::Deny
                            {
                                continue; // US-04: withheld, never sent, never a crash (AC-17-118/119).
                            }
                        }
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
                    Some(ListenEvent::Removed { path, fields }) => {
                        // US-01 (Finding 5 fix) — FIRST, unconditional, rule-independent.
                        if path.collection_path != collection.collection_path {
                            continue;
                        }
                        // security-rules-realtime (ADR-033 § Decision —
                        // Delete Non-Leakage, US-05): re-check the
                        // pre-deletion `fields` snapshot against the
                        // subscription's own rule, identically to the
                        // `Changed` arm's own US-04 gate above —
                        // `fields` is NEVER serialized into the
                        // `DocumentDelete` response below, it exists only
                        // as `evaluate()`'s own decision input.
                        if let Some(condition) = &condition {
                            let empty_fields: BTreeMap<String, FieldValue> = BTreeMap::new();
                            if evaluate(condition, auth_ctx.as_ref(), &fields, &empty_fields)
                                == EvaluationOutcome::Deny
                            {
                                continue; // US-05: withheld, never sent.
                            }
                        }
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

/// Extract and translate the caller's own `where_` filter from a
/// `QueryTarget`'s `StructuredQuery`, reusing `translate_filter()` — the
/// SAME function `handle_run_query` already uses — unchanged. security-
/// rules-realtime (ADR-033 § Decision — Subscribe-Time Composition, US-02).
fn filter_from_query_target(
    qt: &embyr_proto::firestore::target::QueryTarget,
) -> Result<Option<embyr_core::domain::query::QueryFilter>, String> {
    use embyr_proto::firestore::target::query_target::QueryType;
    match &qt.query_type {
        Some(QueryType::StructuredQuery(sq)) => sq
            .r#where
            .as_ref()
            .and_then(crate::grpc::handler::translate_filter)
            .transpose(),
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
