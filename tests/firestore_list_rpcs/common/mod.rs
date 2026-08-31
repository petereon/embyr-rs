//! Common test infrastructure — firestore-list-rpcs acceptance tests
//! (Slice 01, US-01, ADR-050/051).
//!
//! Reuses `SecurityRulesFullContext`/`create_document`/
//! `mint_client_identity_token`/`string_field` via a path import (per
//! DESIGN's own Reuse Analysis — this feature is a new consumer of
//! already-shipped fixtures, not a new context class), mirroring
//! `firestore_batch_write`'s/`batch_get_documents`'s own identical
//! precedent for a brand-new driving-port RPC layered on the SAME
//! `SecurityRulesFullContext`.

#![allow(dead_code, unused_imports)]

#[path = "../../security_rules_write_path/common/mod.rs"]
mod security_rules_write_path_common;
pub use security_rules_write_path_common::{
    create_document, mint_client_identity_token, now_unix, string_field, SecurityRulesFullContext,
};

use embyr_proto::firestore::{
    firestore_client::FirestoreClient, ListCollectionIdsRequest, ListCollectionIdsResponse,
    ListDocumentsRequest, ListDocumentsResponse,
};

/// Real gRPC `ListDocuments` call — driving port entry (Pillar 3), mirroring
/// `create_document`'s own shape (real `FirestoreClient`, real
/// `authorization` + optional `x-embyr-client-identity` metadata).
pub async fn list_documents(
    ctx: &SecurityRulesFullContext,
    parent: &str,
    collection_id: &str,
    page_size: i32,
    page_token: &str,
    client_identity_token: Option<&str>,
) -> Result<ListDocumentsResponse, tonic::Status> {
    let channel = tonic::transport::Endpoint::new(format!("http://{}", ctx.server.grpc_addr))
        .expect("valid endpoint")
        .connect()
        .await
        .expect("connect to gRPC server");
    let mut client = FirestoreClient::new(channel);

    let mut request = tonic::Request::new(ListDocumentsRequest {
        parent: parent.to_string(),
        collection_id: collection_id.to_string(),
        page_size,
        page_token: page_token.to_string(),
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

    Ok(client.list_documents(request).await?.into_inner())
}

/// Real gRPC `ListCollectionIds` call — driving port entry (Pillar 3),
/// mirroring `list_documents`'s own shape (Slice 02, US-02, ADR-051).
pub async fn list_collection_ids(
    ctx: &SecurityRulesFullContext,
    parent: &str,
    page_size: i32,
    page_token: &str,
) -> Result<ListCollectionIdsResponse, tonic::Status> {
    let channel = tonic::transport::Endpoint::new(format!("http://{}", ctx.server.grpc_addr))
        .expect("valid endpoint")
        .connect()
        .await
        .expect("connect to gRPC server");
    let mut client = FirestoreClient::new(channel);

    let mut request = tonic::Request::new(ListCollectionIdsRequest {
        parent: parent.to_string(),
        page_size,
        page_token: page_token.to_string(),
    });
    request.metadata_mut().insert(
        "authorization",
        format!("Bearer {}", ctx.api_key).parse().unwrap(),
    );

    Ok(client.list_collection_ids(request).await?.into_inner())
}

/// The database-root `parent` resource name (no trailing document path).
pub fn root_parent(ctx: &SecurityRulesFullContext) -> String {
    format!("projects/{}/databases/(default)/documents", ctx.project_id)
}

/// A nested `parent` resource name — `doc_path` is a document path relative
/// to the database root (e.g. `"users/maria-santos-a1b2"`).
pub fn nested_parent(ctx: &SecurityRulesFullContext, doc_path: &str) -> String {
    format!(
        "projects/{}/databases/(default)/documents/{doc_path}",
        ctx.project_id
    )
}
