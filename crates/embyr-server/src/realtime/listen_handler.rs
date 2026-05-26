/// Handle an AddTarget Listen session: initial snapshot + keep-alive loop.
use std::time::Duration;

use embyr_core::domain::{
    document::CollectionPath,
    project::ProjectId,
    query::StructuredQuery as DomainQuery,
};
use embyr_proto::firestore::{
    listen_request, listen_response,
    target::TargetType,
    target_change::TargetChangeType,
    DocumentChange, ListenRequest, ListenResponse, TargetChange,
};
use tonic::{Status};
use tokio::sync::mpsc;

use crate::{
    adapters::credential_cache::SharedBackendAdapter,
    encoding::firestore_proto::document_to_proto,
};

/// Process an AddTarget message: run the initial snapshot query, send
/// `DocumentChange` events for each doc, send `TargetChange(CURRENT)`,
/// then loop sending `TargetChange(NO_CHANGE)` every `keepalive` interval.
pub async fn handle_add_target(
    first_msg: &ListenRequest,
    adapter: &SharedBackendAdapter,
    tx: &mpsc::Sender<Result<ListenResponse, Status>>,
    keepalive: Duration,
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

    let domain_query = DomainQuery {
        collection_id: collection.collection_path.clone(),
        all_descendants: false,
        filter: None,
        order_by: vec![],
        limit: None,
        offset: None,
        start_at: None,
        end_at: None,
    };

    // Run the initial snapshot query.
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

    // Keep-alive loop: send NO_CHANGE on each tick.
    loop {
        tokio::time::sleep(keepalive).await;
        let no_change = ListenResponse {
            response_type: Some(listen_response::ResponseType::TargetChange(TargetChange {
                target_change_type: TargetChangeType::NoChange as i32,
                target_ids: vec![],
                ..Default::default()
            })),
        };
        if tx.send(Ok(no_change)).await.is_err() {
            // Client disconnected — end the loop gracefully.
            break;
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
