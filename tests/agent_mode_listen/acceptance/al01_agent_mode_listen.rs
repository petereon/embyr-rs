//! agent-mode-listen — `Listen` against a `backend_mode=agent` project.
//!
//! Real `embyr-agent` (own Postgres, mTLS) + real `embyr-server`: a write made
//! through the server travels agent -> Postgres NOTIFY -> agent `Subscribe` ->
//! server `ChangeFeedListener` -> `ListenRegistry` -> the client's Listen
//! stream. Before this feature the credential cache held an empty DSN for
//! agent projects and Listen failed outright.
//!
//! DOCKER-BACKED: do not run locally (CI only). Compile with `--no-run`.

use std::{collections::HashMap, time::Duration};

use embyr_proto::firestore::{
    firestore_client::FirestoreClient,
    listen_request, listen_response,
    structured_query::CollectionSelector,
    target::{self, query_target},
    target_change::TargetChangeType,
    value::ValueType,
    CreateDocumentRequest, Document, ListenRequest, ListenResponse, StructuredQuery, Target, Value,
};
use tokio_stream::StreamExt;

#[path = "../../agent_mode_write_streaming/common/mod.rs"]
mod common;
use common::AgentModeWriteStreamingContext;

fn client(ctx: &AgentModeWriteStreamingContext) -> FirestoreClient<tonic::transport::Channel> {
    let channel = tonic::transport::Endpoint::new(format!("http://{}", ctx.server.grpc_addr))
        .expect("valid endpoint")
        .connect_lazy();
    FirestoreClient::new(channel)
}

fn authed<T>(ctx: &AgentModeWriteStreamingContext, message: T) -> tonic::Request<T> {
    let mut request = tonic::Request::new(message);
    request
        .metadata_mut()
        .insert("authorization", format!("bearer {}", ctx.api_key).parse().unwrap());
    request
}

/// Next message that is not a NO_CHANGE keep-alive, generous timeout.
async fn next(stream: &mut tonic::Streaming<ListenResponse>) -> listen_response::ResponseType {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(30), stream.next())
            .await
            .expect("timed out waiting for the next Listen message")
            .expect("stream ended")
            .expect("stream error");
        match message.response_type.expect("response_type set") {
            listen_response::ResponseType::TargetChange(tc)
                if tc.target_change_type == TargetChangeType::NoChange as i32 => {}
            other => return other,
        }
    }
}

/// A document created through the server after a Listen target is open is
/// delivered on that stream — Listen works for agent-mode projects.
#[tokio::test]
async fn listen_delivers_live_changes_for_an_agent_mode_project() {
    let ctx = AgentModeWriteStreamingContext::new("al01-agent-listen").await;
    let docs_root = format!("projects/{}/databases/(default)/documents", ctx.project_id);

    let first = ListenRequest {
        database: format!("projects/{}/databases/(default)", ctx.project_id),
        target_change: Some(listen_request::TargetChange::AddTarget(Target {
            target_id: 1,
            target_type: Some(target::TargetType::Query(target::QueryTarget {
                parent: docs_root.clone(),
                query_type: Some(query_target::QueryType::StructuredQuery(StructuredQuery {
                    from: vec![CollectionSelector {
                        collection_id: "patients".into(),
                        all_descendants: false,
                    }],
                    ..Default::default()
                })),
            })),
            ..Default::default()
        })),
        ..Default::default()
    };
    let (request_tx, request_rx) = tokio::sync::mpsc::channel(4);
    request_tx.send(first).await.unwrap();
    let mut stream = client(&ctx)
        .listen(authed(&ctx, tokio_stream::wrappers::ReceiverStream::new(request_rx)))
        .await
        .expect("Listen must open for an agent-mode project")
        .into_inner();

    // Empty collection: ADD then CURRENT.
    assert!(matches!(
        next(&mut stream).await,
        listen_response::ResponseType::TargetChange(tc) if tc.target_change_type == TargetChangeType::Add as i32
    ));
    assert!(matches!(
        next(&mut stream).await,
        listen_response::ResponseType::TargetChange(tc) if tc.target_change_type == TargetChangeType::Current as i32
    ));

    client(&ctx)
        .create_document(authed(
            &ctx,
            CreateDocumentRequest {
                parent: docs_root,
                collection_id: "patients".into(),
                document_id: "p-1".into(),
                document: Some(Document {
                    fields: HashMap::from([(
                        "status".to_string(),
                        Value { value_type: Some(ValueType::StringValue("admitted".into())) },
                    )]),
                    ..Default::default()
                }),
                ..Default::default()
            },
        ))
        .await
        .expect("create_document");

    match next(&mut stream).await {
        listen_response::ResponseType::DocumentChange(dc) => {
            let doc = dc.document.expect("document");
            assert!(doc.name.ends_with("/patients/p-1"), "got {}", doc.name);
            assert_eq!(dc.target_ids, vec![1]);
        }
        other => panic!("expected DocumentChange, got {other:?}"),
    }
}
