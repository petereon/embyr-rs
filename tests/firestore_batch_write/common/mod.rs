//! Common test infrastructure — firestore-batch-write acceptance tests
//! (Slice 01, US-01, ADR-048).
//!
//! Reuses `SecurityRulesFullContext`/`create_document`/`delete_document`/
//! `mint_client_identity_token`/`update_write` via a path import (per
//! DESIGN's own Reuse Analysis: 8 REUSE, 2 CREATE NEW, 1 EXTEND — this
//! feature is a new consumer of already-shipped fixtures, not a new context
//! class), mirroring `batch_get_documents`' own identical precedent for a
//! brand-new driving-port RPC layered on the SAME `SecurityRulesFullContext`.

#![allow(dead_code, unused_imports)]

#[path = "../../security_rules_write_path/common/mod.rs"]
mod security_rules_write_path_common;
pub use security_rules_write_path_common::{
    create_document, delete_document, mint_client_identity_token, now_unix, string_field,
    update_write, SecurityRulesFullContext,
};

use embyr_proto::firestore::{
    firestore_client::FirestoreClient, BatchWriteRequest, BatchWriteResponse, Write,
};

/// Real gRPC `BatchWrite` call — driving port entry (Pillar 3), mirroring
/// `commit_writes`'s own shape (real `FirestoreClient`, real
/// `authorization` + optional `x-embyr-client-identity` metadata). Unlike
/// `commit_writes`, there is no `transaction` field — `BatchWrite`'s own
/// wire contract applies each write independently, never atomically
/// (docs/SPEC.md §BatchWrite).
pub async fn batch_write(
    ctx: &SecurityRulesFullContext,
    writes: Vec<Write>,
    client_identity_token: Option<&str>,
) -> Result<BatchWriteResponse, tonic::Status> {
    let channel = tonic::transport::Endpoint::new(format!("http://{}", ctx.server.grpc_addr))
        .expect("valid endpoint")
        .connect()
        .await
        .expect("connect to gRPC server");
    let mut client = FirestoreClient::new(channel);

    let mut request = tonic::Request::new(BatchWriteRequest {
        database: format!("projects/{}/databases/(default)", ctx.project_id),
        writes,
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

    Ok(client.batch_write(request).await?.into_inner())
}

/// Build a `Write { Operation::Update }` carrying a `current_document.exists
/// = false` precondition (Slice 02, US-02) — the exact failure mode US-02's
/// own UAT scenarios exercise: a duplicate of an already-imported document.
/// Mirrors `update_write`'s own shape, differing only in `current_document`.
pub fn update_write_requiring_not_exists(
    resource_name: &str,
    fields: std::collections::HashMap<String, embyr_proto::firestore::Value>,
) -> Write {
    use embyr_proto::firestore::{precondition::ConditionType, Precondition};
    Write {
        update_mask: None,
        update_transforms: vec![],
        current_document: Some(Precondition {
            condition_type: Some(ConditionType::Exists(false)),
        }),
        operation: Some(embyr_proto::firestore::write::Operation::Update(
            embyr_proto::firestore::Document {
                name: resource_name.to_string(),
                fields,
                ..Default::default()
            },
        )),
    }
}
