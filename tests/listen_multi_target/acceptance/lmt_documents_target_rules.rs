//! listen-multi-target — Documents targets obey the collection's read rule,
//! evaluated per document (the same decision GetDocument makes), and a
//! denied target on a multiplexed stream fails alone.
//!
//! Driving port: gRPC :8080 `Listen` with an `x-embyr-client-identity` token.

#![allow(unused_imports)]

#[path = "../../security_rules_realtime/common/mod.rs"]
mod common;
use common::{
    create_document, mint_client_identity_token, now_unix, string_field, update_document,
    SecurityRulesFullContext,
};

use std::{collections::HashMap, time::Duration};

use ed25519_dalek::SigningKey;
use embyr_proto::firestore::{
    firestore_client::FirestoreClient, listen_request, listen_response, target,
    target_change::TargetChangeType, ListenRequest, ListenResponse, Target,
};
use rand_core::OsRng;
use tokio_stream::StreamExt;

fn documents_request(ctx: &SecurityRulesFullContext, target_id: i32, doc_ids: &[String]) -> ListenRequest {
    ListenRequest {
        database: format!("projects/{}/databases/(default)", ctx.project_id),
        target_change: Some(listen_request::TargetChange::AddTarget(Target {
            target_id,
            target_type: Some(target::TargetType::Documents(target::DocumentsTarget {
                documents: doc_ids
                    .iter()
                    .map(|id| {
                        format!(
                            "projects/{}/databases/(default)/documents/journal_entries/{id}",
                            ctx.project_id
                        )
                    })
                    .collect(),
            })),
            ..Default::default()
        })),
        ..Default::default()
    }
}

async fn listen_as(
    ctx: &SecurityRulesFullContext,
    token: &str,
    first: ListenRequest,
) -> (tokio::sync::mpsc::Sender<ListenRequest>, tonic::Streaming<ListenResponse>) {
    let channel = tonic::transport::Endpoint::new(format!("http://{}", ctx.server.grpc_addr))
        .expect("valid endpoint")
        .connect()
        .await
        .expect("connect to gRPC server");
    let (request_tx, request_rx) = tokio::sync::mpsc::channel(8);
    request_tx.send(first).await.unwrap();
    let mut request = tonic::Request::new(tokio_stream::wrappers::ReceiverStream::new(request_rx));
    request
        .metadata_mut()
        .insert("authorization", format!("Bearer {}", ctx.api_key).parse().unwrap());
    request
        .metadata_mut()
        .insert("x-embyr-client-identity", format!("Bearer {token}").parse().unwrap());
    let stream = FirestoreClient::new(channel)
        .listen(request)
        .await
        .expect("listen should succeed")
        .into_inner();
    (request_tx, stream)
}

async fn next_message(stream: &mut tonic::Streaming<ListenResponse>) -> Result<listen_response::ResponseType, tonic::Status> {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(20), stream.next())
            .await
            .expect("timed out waiting for the next Listen message")
            .expect("stream ended while waiting for the next Listen message")?;
        match message.response_type.expect("response_type set") {
            listen_response::ResponseType::TargetChange(tc)
                if tc.target_change_type == TargetChangeType::NoChange as i32 =>
            {
                continue
            }
            other => return Ok(other),
        }
    }
}

/// A Documents target on a document the rule denies ends a first-message
/// stream with permission_denied — nothing about the document is sent.
///
/// @driving_port @real-io
#[tokio::test]
async fn documents_target_on_a_denied_document_is_rejected() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-lmt-rules-deny").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes()).await;
    ctx.seed_access_rule("journal_entries", "request.auth.uid == resource.data.owner_id").await;
    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    let maria_doc = format!("{}-maria-doc", ctx.project_id);
    let (_request_tx, mut stream) =
        listen_as(&ctx, &danas_token, documents_request(&ctx, 1, &[maria_doc])).await;

    let status = next_message(&mut stream)
        .await
        .expect_err("a denied document must end the stream with an error, not deliver it");
    assert_eq!(status.code(), tonic::Code::PermissionDenied, "got: {status:?}");
}

/// On a multiplexed stream a denied Documents target fails alone
/// (TargetChange REMOVE with a permission_denied cause); the admitted target
/// keeps delivering.
///
/// @driving_port @real-io
#[tokio::test]
async fn denied_documents_target_fails_alone_and_admitted_target_keeps_delivering() {
    let ctx = SecurityRulesFullContext::new("trailmark-prod-lmt-rules-mixed").await;
    let signing_key = SigningKey::generate(&mut OsRng);
    ctx.seed_client_identity_credential(&signing_key.verifying_key().to_bytes()).await;
    ctx.seed_access_rule("journal_entries", "request.auth.uid == resource.data.owner_id").await;
    let danas_token =
        mint_client_identity_token(&signing_key, "dana-kim", &ctx.project_id, now_unix() + 3600);

    create_document(
        &ctx,
        "journal_entries",
        "lmt-dana-doc",
        HashMap::from([("owner_id".to_string(), string_field("dana-kim"))]),
        None,
    )
    .await
    .expect("seed dana's document");

    let (request_tx, mut stream) = listen_as(
        &ctx,
        &danas_token,
        documents_request(&ctx, 1, &["lmt-dana-doc".to_string()]),
    )
    .await;
    assert!(matches!(
        next_message(&mut stream).await.unwrap(),
        listen_response::ResponseType::TargetChange(tc) if tc.target_change_type == TargetChangeType::Add as i32
    ));
    assert!(matches!(
        next_message(&mut stream).await.unwrap(),
        listen_response::ResponseType::DocumentChange(dc) if dc.target_ids == vec![1]
    ));
    assert!(matches!(
        next_message(&mut stream).await.unwrap(),
        listen_response::ResponseType::TargetChange(tc) if tc.target_change_type == TargetChangeType::Current as i32
    ));

    // Second target: Maria's document, which Dana may not read.
    let maria_doc = format!("{}-maria-doc", ctx.project_id);
    request_tx.send(documents_request(&ctx, 2, &[maria_doc])).await.unwrap();
    match next_message(&mut stream).await.unwrap() {
        listen_response::ResponseType::TargetChange(tc) => {
            assert_eq!(tc.target_change_type, TargetChangeType::Remove as i32);
            assert_eq!(tc.target_ids, vec![2]);
            assert_eq!(
                tc.cause.map(|c| c.code),
                Some(tonic::Code::PermissionDenied as i32)
            );
        }
        other => panic!("expected TargetChange(REMOVE) for target 2, got {other:?}"),
    }

    // Target 1 is unaffected.
    update_document(
        &ctx,
        &format!(
            "projects/{}/databases/(default)/documents/journal_entries/lmt-dana-doc",
            ctx.project_id
        ),
        HashMap::from([
            ("owner_id".to_string(), string_field("dana-kim")),
            ("note".to_string(), string_field("updated")),
        ]),
        None,
    )
    .await
    .expect("update dana's document");
    assert!(matches!(
        next_message(&mut stream).await.unwrap(),
        listen_response::ResponseType::DocumentChange(dc) if dc.target_ids == vec![1]
    ));
}
