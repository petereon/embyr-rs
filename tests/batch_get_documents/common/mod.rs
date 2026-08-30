//! Common test infrastructure — batch-get-documents acceptance tests
//! (Slice 01, US-01, ADR-042).
//!
//! Reuses `SecurityRulesFullContext`/`seed_access_rule`/
//! `seed_client_identity_credential`/`create_document`/`delete_document`/
//! `mint_client_identity_token` via a path import (per DESIGN's own Reuse
//! Analysis: 9 REUSE-UNCHANGED, 2 EXTEND, 0 CREATE NEW — this feature is a
//! pure new consumer of already-shipped mechanisms). No new fixture context
//! class, mirroring `aggregation-queries`' own precedent for a brand-new
//! driving-port RPC layered on the SAME `SecurityRulesFullContext`.

#![allow(dead_code, unused_imports)]

#[path = "../../security_rules_write_path/common/mod.rs"]
mod security_rules_write_path_common;
pub use security_rules_write_path_common::{
    create_document, delete_document, mint_client_identity_token, now_unix, string_field,
    SecurityRulesFullContext,
};

use embyr_proto::firestore::{
    batch_get_documents_response::Result as BatchResult, firestore_client::FirestoreClient,
    BatchGetDocumentsRequest,
};

/// One resolved item from a real `BatchGetDocuments` stream. `Found` carries
/// the document's own resource name (sufficient for these scenarios —
/// `create_document`'s own return value already proves field content
/// round-trips correctly, this feature adds no new field-encoding logic).
/// `Missing` carries the requested-but-unresolved name verbatim, exactly as
/// the wire shape delivers it (`oneof result { Document found; string
/// missing; }` — no third "denied" arm, per ADR-042).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchItem {
    Found(String),
    Missing(String),
}

/// Real gRPC `BatchGetDocuments` call — driving port entry (Pillar 3),
/// mirroring `SecurityRulesFullContext::get_document`'s own shape (real
/// `FirestoreClient`, real `authorization` + optional
/// `x-embyr-client-identity` metadata), collecting the full response stream
/// eagerly (mirrors DDD-BGD-8's own eager-`Vec` server-side construction).
pub async fn batch_get_documents(
    ctx: &SecurityRulesFullContext,
    document_names: &[String],
    client_identity_token: Option<&str>,
) -> Result<Vec<BatchItem>, tonic::Status> {
    let channel = tonic::transport::Endpoint::new(format!("http://{}", ctx.server.grpc_addr))
        .expect("valid endpoint")
        .connect()
        .await
        .expect("connect to gRPC server");
    let mut client = FirestoreClient::new(channel);

    let mut request = tonic::Request::new(BatchGetDocumentsRequest {
        database: format!("projects/{}/databases/(default)", ctx.project_id),
        documents: document_names.to_vec(),
        ..Default::default()
    });
    request.metadata_mut().insert(
        "authorization",
        format!("Bearer {}", ctx.api_key).parse().unwrap(),
    );
    if let Some(token) = client_identity_token {
        request.metadata_mut().insert(
            "x-embyr-client-identity",
            format!("Bearer {token}").parse().unwrap(),
        );
    }

    let mut stream = client.batch_get_documents(request).await?.into_inner();
    use tokio_stream::StreamExt;
    let mut items = Vec::new();
    while let Some(item) = stream.next().await {
        let response = item?;
        match response.result {
            Some(BatchResult::Found(doc)) => items.push(BatchItem::Found(doc.name)),
            Some(BatchResult::Missing(name)) => items.push(BatchItem::Missing(name)),
            None => panic!("BatchGetDocumentsResponse carried neither found nor missing"),
        }
    }
    Ok(items)
}
