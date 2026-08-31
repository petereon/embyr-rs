// SCAFFOLD: true
//! US-A07 (agent-mode-field-transforms, Slice 01) — field transforms wired
//! through the agent's own Commit RPC.
//!
//! As Alex (SDK Developer), I want FieldValue.serverTimestamp()/increment()
//! writes to actually compute against a backend_mode=agent project, so that
//! transform-based writes never silently no-op.
//!
//! Driving port: StorageAgent gRPC service (mTLS test harness)
//!   RPCs: BeginTransaction, Commit, GetDocument
//! Feature: docs/feature/agent-mode-field-transforms/slices/slice-01-wire-transforms-through.md
//!
//! Covers both wire shapes named load-bearing by DISCUSS/DESIGN:
//! (1) a standalone `Write.transform` (no accompanying field update)
//! (2) a `Write.update_transforms` attached alongside a regular `update`

use std::collections::HashMap;

use embyr_proto::agent::{
    field_transform::TransformType, storage_agent_client::StorageAgentClient,
    value::ValueType, write::Operation, BeginTransactionRequest, CommitRequest,
    CreateDocumentRequest, Document, FieldTransform, GetDocumentRequest, ServerValue, Transform,
    Value, Write,
};
use tonic::transport::Channel;

use super::agent_common::start_test_agent;

fn str_val(s: &str) -> Value {
    Value { value_type: Some(ValueType::StringValue(s.to_string())) }
}

async fn create_doc(
    client: &mut StorageAgentClient<Channel>,
    project_id: &str,
    collection: &str,
    doc_id: &str,
    fields: HashMap<String, Value>,
) -> Document {
    let req = CreateDocumentRequest {
        parent: format!("projects/{project_id}/databases/(default)/documents"),
        collection_id: collection.to_string(),
        document_id: doc_id.to_string(),
        document: Some(Document { name: String::new(), fields, ..Default::default() }),
        ..Default::default()
    };
    client.create_document(req).await.expect("create_document").into_inner()
}

async fn get_doc(client: &mut StorageAgentClient<Channel>, name: &str) -> Document {
    let req = GetDocumentRequest { name: name.to_string(), ..Default::default() };
    client.get_document(req).await.expect("get_document").into_inner()
}

/// @driving_port @us_a07 @real_io
///
/// Feature: A standalone transform-only write computes serverTimestamp() and persists it
///   Given "patients/p_204" exists with no lastModified field
///   When  a caller commits a standalone Write.transform setting lastModified to serverTimestamp()
///   Then  reading the document afterward shows lastModified as a real timestamp
#[tokio::test]
async fn standalone_transform_computes_server_timestamp() {
    let (_handle, mut client) = start_test_agent("meridian-health-prod").await;
    let project_id = "meridian-health-prod";
    let doc_name =
        "projects/meridian-health-prod/databases/(default)/documents/patients/p_204";

    let mut fields = HashMap::new();
    fields.insert("status".to_string(), str_val("admitted"));
    create_doc(&mut client, project_id, "patients", "p_204", fields).await;

    let txn = client
        .begin_transaction(BeginTransactionRequest { ..Default::default() })
        .await
        .expect("begin_transaction")
        .into_inner()
        .transaction;

    let before = chrono::Utc::now();

    let write = Write {
        operation: Some(Operation::Transform(Transform {
            document: doc_name.to_string(),
            field_transforms: vec![FieldTransform {
                field_path: "lastModified".to_string(),
                transform_type: Some(TransformType::SetToServerValue(
                    ServerValue::RequestTime as i32,
                )),
            }],
        })),
        ..Default::default()
    };
    client
        .commit(CommitRequest { transaction: txn, writes: vec![write], ..Default::default() })
        .await
        .expect("commit must succeed");

    let after = chrono::Utc::now();

    let doc = get_doc(&mut client, doc_name).await;
    let last_modified = doc
        .fields
        .get("lastModified")
        .and_then(|v| v.value_type.as_ref())
        .expect("lastModified must be present, not silently dropped");
    match last_modified {
        ValueType::TimestampValue(ts) => {
            let computed = chrono::DateTime::from_timestamp(ts.seconds, ts.nanos as u32)
                .expect("valid timestamp");
            assert!(
                computed >= before && computed <= after,
                "lastModified must fall within the commit window, got {computed}"
            );
        }
        other => panic!("lastModified must be a real timestamp, got {other:?}"),
    }
    let status = doc.fields.get("status").and_then(|v| v.value_type.as_ref());
    assert_eq!(
        status,
        Some(&ValueType::StringValue("admitted".to_string())),
        "unrelated fields must survive a standalone transform-only write"
    );
}

/// @driving_port @us_a07 @real_io
///
/// Feature: A transform attached to a regular update (update_transforms) computes and persists
///   Given "patients/p_204" has viewCount = 5
///   When  a caller commits a Write.update setting status="reviewed" with update_transforms incrementing viewCount by 3
///   Then  reading the document afterward shows status="reviewed" and viewCount=8
#[tokio::test]
async fn update_transforms_attached_to_regular_update_computes_and_persists() {
    let (_handle, mut client) = start_test_agent("meridian-health-prod").await;
    let project_id = "meridian-health-prod";
    let doc_name =
        "projects/meridian-health-prod/databases/(default)/documents/patients/p_205";

    let mut fields = HashMap::new();
    fields.insert("status".to_string(), str_val("admitted"));
    fields.insert(
        "viewCount".to_string(),
        Value { value_type: Some(ValueType::IntegerValue(5)) },
    );
    create_doc(&mut client, project_id, "patients", "p_205", fields).await;

    let txn = client
        .begin_transaction(BeginTransactionRequest { ..Default::default() })
        .await
        .expect("begin_transaction")
        .into_inner()
        .transaction;

    let mut update_fields = HashMap::new();
    update_fields.insert("status".to_string(), str_val("reviewed"));
    let write = Write {
        operation: Some(Operation::Update(Document {
            name: doc_name.to_string(),
            fields: update_fields,
            ..Default::default()
        })),
        update_transforms: vec![FieldTransform {
            field_path: "viewCount".to_string(),
            transform_type: Some(TransformType::Increment(Value {
                value_type: Some(ValueType::IntegerValue(3)),
            })),
        }],
        ..Default::default()
    };
    client
        .commit(CommitRequest { transaction: txn, writes: vec![write], ..Default::default() })
        .await
        .expect("commit must succeed");

    let doc = get_doc(&mut client, doc_name).await;
    assert_eq!(
        doc.fields.get("status").and_then(|v| v.value_type.as_ref()),
        Some(&ValueType::StringValue("reviewed".to_string())),
        "regular field write must apply alongside the transform"
    );
    assert_eq!(
        doc.fields.get("viewCount").and_then(|v| v.value_type.as_ref()),
        Some(&ValueType::IntegerValue(8)),
        "update_transforms increment must compute against the pre-existing persisted value"
    );
}
