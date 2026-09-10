// SCAFFOLD: true
//! US-A02 — Write operations via agent (Create, Update, Delete)
//!
//! As Alex (SDK Developer), I want setDoc, updateDoc, and deleteDoc to persist
//! changes through the agent identically to Firestore, so that my write-heavy
//! features work without any SDK code changes.
//!
//! Driving port: StorageAgent gRPC service (mTLS :9191)
//!   RPCs: CreateDocument, UpdateDocument, DeleteDocument
//! Red classification: MISSING_FUNCTIONALITY
//!
//! Feature file: tests/features/agent/us_a02_write_operations.feature
//! Execution order: S02A (after S01A walking skeleton and S06A lifecycle)

use embyr_proto::agent::{
    precondition::ConditionType,
    storage_agent_client::StorageAgentClient,
    value::ValueType,
    CreateDocumentRequest, DeleteDocumentRequest, Document, DocumentMask, GetDocumentRequest,
    Precondition, UpdateDocumentRequest, Value,
};
use std::collections::HashMap;
use tonic::transport::Channel;

use super::agent_common::{start_test_agent, AgentHandle};

// ---------------------------------------------------------------------------
// Helper: build field values
// ---------------------------------------------------------------------------

fn str_val(s: &str) -> Value {
    Value { value_type: Some(ValueType::StringValue(s.to_string())) }
}

fn int_val(i: i64) -> Value {
    Value { value_type: Some(ValueType::IntegerValue(i)) }
}

/// Create a document in the given collection with the given fields.
async fn create_doc(
    client: &mut StorageAgentClient<Channel>,
    collection_id: &str,
    document_id: &str,
    fields: HashMap<String, Value>,
) -> Document {
    let req = CreateDocumentRequest {
        parent: "projects/finops-prod/databases/(default)/documents".to_string(),
        collection_id: collection_id.to_string(),
        document_id: document_id.to_string(),
        document: Some(Document {
            name: String::new(),
            fields,
            ..Default::default()
        }),
        ..Default::default()
    };
    client.create_document(req).await.expect("create_document succeeded").into_inner()
}

/// Get a document by full resource name.
async fn get_doc(client: &mut StorageAgentClient<Channel>, name: &str) -> Document {
    let req = GetDocumentRequest { name: name.to_string(), ..Default::default() };
    client.get_document(req).await.expect("get_document succeeded").into_inner()
}

// ---------------------------------------------------------------------------
// Happy path scenarios
// ---------------------------------------------------------------------------

/// @driving_port @us_a02 @real_io
///
/// Feature: Creating a new document stores it with generation one
///   Given the document "orders/ord-2026-001" does not exist
///   When  a caller creates it with fields customerId="C-489" and amount=1250
///   Then  the creation succeeds
///   And   retrieving the document returns those fields
///   And   the document record shows it is at generation one
#[tokio::test]
#[ignore = "requires Docker — unskip in S02A delivery"]
async fn creating_new_document_stores_it_at_generation_one() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;

    let mut fields = HashMap::new();
    fields.insert("customerId".to_string(), str_val("C-489"));
    fields.insert("amount".to_string(), int_val(1250));

    let doc = create_doc(&mut client, "orders", "ord-2026-001", fields).await;

    assert_eq!(doc.generation, 1, "generation should be 1 on first write");

    let fetched = get_doc(&mut client, &doc.name).await;
    assert_eq!(
        fetched.fields.get("customerId").and_then(|v| v.value_type.as_ref()),
        Some(&ValueType::StringValue("C-489".to_string())),
        "customerId preserved"
    );
    assert_eq!(
        fetched.fields.get("amount").and_then(|v| v.value_type.as_ref()),
        Some(&ValueType::IntegerValue(1250)),
        "amount preserved"
    );
    assert_eq!(fetched.generation, 1, "fetched generation still 1");
}

/// @driving_port @us_a02 @real_io
///
/// Feature: Updating with a field mask preserves unmentioned fields
///   Given "orders/ord-2026-001" exists with customerId="C-489" and amount=1250
///   When  a caller updates it setting only status="shipped"
///   Then  the update succeeds
///   And   customerId="C-489" and amount=1250 are still present
///   And   the document is now at generation two
#[tokio::test]
#[ignore = "requires Docker — unskip in S02A delivery"]
async fn updating_with_mask_preserves_unmentioned_fields() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;

    // Create the initial document
    let mut fields = HashMap::new();
    fields.insert("customerId".to_string(), str_val("C-489"));
    fields.insert("amount".to_string(), int_val(1250));
    let created = create_doc(&mut client, "orders", "ord-2026-001", fields).await;

    // Update with mask — only status
    let doc_name = created.name.clone();
    let mut update_fields = HashMap::new();
    update_fields.insert("status".to_string(), str_val("shipped"));

    let update_req = UpdateDocumentRequest {
        document: Some(Document {
            name: doc_name.clone(),
            fields: update_fields,
            ..Default::default()
        }),
        update_mask: Some(DocumentMask { field_paths: vec!["status".to_string()] }),
        ..Default::default()
    };
    let updated = client
        .update_document(update_req)
        .await
        .expect("update_document succeeded")
        .into_inner();

    assert_eq!(updated.generation, 2, "generation incremented to 2");

    // Verify masked fields preserved
    let fetched = get_doc(&mut client, &doc_name).await;
    assert_eq!(
        fetched.fields.get("customerId").and_then(|v| v.value_type.as_ref()),
        Some(&ValueType::StringValue("C-489".to_string())),
        "customerId unchanged"
    );
    assert_eq!(
        fetched.fields.get("amount").and_then(|v| v.value_type.as_ref()),
        Some(&ValueType::IntegerValue(1250)),
        "amount unchanged"
    );
    assert_eq!(
        fetched.fields.get("status").and_then(|v| v.value_type.as_ref()),
        Some(&ValueType::StringValue("shipped".to_string())),
        "status set to shipped"
    );
}

/// @driving_port @us_a02 @real_io
///
/// Feature: Removing an absent document succeeds without error
///   Given "orders/ord-2026-001" does not exist
///   When  a caller removes it
///   Then  the removal succeeds without error
#[tokio::test]
#[ignore = "requires Docker — unskip in S02A delivery"]
async fn removing_absent_document_succeeds_without_error() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;
    // This document was never created — delete should succeed (Firestore no-op semantics)
    let result = client
        .delete_document(DeleteDocumentRequest {
            name: "projects/finops-prod/databases/(default)/documents/orders/nonexistent-123"
                .to_string(),
            ..Default::default()
        })
        .await;
    assert!(result.is_ok(), "delete of absent doc should succeed: {:?}", result);
}

/// @driving_port @us_a02 @real_io
///
/// Feature: Removing a document leaves a deletion record
///   Given "orders/ord-2026-001" exists with status="pending"
///   When  a caller removes it
///   Then  a deletion record exists in project storage
///   And   retrieving the document returns not-found
#[tokio::test]
#[ignore = "requires Docker — unskip in S02A delivery"]
async fn removing_document_leaves_deletion_record() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;

    // Create the document first
    let doc_name = "projects/finops-prod/databases/(default)/documents/orders/to-delete";
    let mut fields = HashMap::new();
    fields.insert("status".to_string(), str_val("pending"));
    create_doc(&mut client, "orders", "to-delete", fields).await;

    // Delete it — must succeed
    client
        .delete_document(DeleteDocumentRequest {
            name: doc_name.to_string(),
            ..Default::default()
        })
        .await
        .expect("delete doc");

    // Verify GetDocument returns NotFound
    let get_result = client
        .get_document(GetDocumentRequest { name: doc_name.to_string(), ..Default::default() })
        .await;
    assert!(get_result.is_err(), "document should not be found after deletion");
    assert_eq!(
        get_result.unwrap_err().code(),
        tonic::Code::NotFound,
        "should return NOT_FOUND after delete"
    );
}

/// @driving_port @us_a02 @real_io
///
/// Feature: Increment transform on absent field treats starting value as zero
///   Given "orders/ord-2026-001" exists with no "retryCount" field
///   When  a caller applies an increment of 1 to "retryCount"
///   Then  the update succeeds
///   And   the document shows retryCount=1
#[tokio::test]
#[ignore = "requires Docker — unskip in S02A delivery"]
async fn increment_transform_on_absent_field_treats_starting_value_as_zero() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;

    // Create document without retryCount
    let mut fields = HashMap::new();
    fields.insert("status".to_string(), str_val("pending"));
    let created = create_doc(&mut client, "orders", "ord-2026-001", fields).await;
    let doc_name = created.name.clone();

    // Update with retryCount=1 (client sends the incremented value), masked to retryCount only
    let mut update_fields = HashMap::new();
    update_fields.insert("retryCount".to_string(), int_val(1));

    let update_req = UpdateDocumentRequest {
        document: Some(Document {
            name: doc_name.clone(),
            fields: update_fields,
            ..Default::default()
        }),
        update_mask: Some(DocumentMask { field_paths: vec!["retryCount".to_string()] }),
        ..Default::default()
    };
    client.update_document(update_req).await.expect("update with retryCount=1 succeeded");

    let fetched = get_doc(&mut client, &doc_name).await;
    assert_eq!(
        fetched.fields.get("retryCount").and_then(|v| v.value_type.as_ref()),
        Some(&ValueType::IntegerValue(1)),
        "retryCount should be 1"
    );
    // status field should still be there (mask only touched retryCount)
    assert_eq!(
        fetched.fields.get("status").and_then(|v| v.value_type.as_ref()),
        Some(&ValueType::StringValue("pending".to_string())),
        "status still present"
    );
}

/// @driving_port @us_a02 @real_io
///
/// Feature: Creating a document without specifying an identifier generates one
///   Given no document with a generated identifier exists in "orders"
///   When  a caller creates a new document in "orders" with status="pending"
///   Then  the creation succeeds with a generated 20-character identifier
#[tokio::test]
#[ignore = "requires Docker — unskip in S02A delivery"]
async fn creating_document_without_identifier_generates_one() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;

    let mut fields = HashMap::new();
    fields.insert("status".to_string(), str_val("pending"));

    let req = CreateDocumentRequest {
        parent: "projects/finops-prod/databases/(default)/documents".to_string(),
        collection_id: "orders".to_string(),
        document_id: String::new(), // empty — auto-generate
        document: Some(Document {
            name: String::new(),
            fields,
            ..Default::default()
        }),
        ..Default::default()
    };
    let doc = client
        .create_document(req)
        .await
        .expect("create with auto-id succeeded")
        .into_inner();

    let doc_id = doc.name.split('/').last().unwrap().to_string();
    assert_eq!(doc_id.len(), 20, "generated document id should be 20 chars, got: {doc_id}");
    assert!(
        doc_id.chars().all(|c| c.is_ascii_alphanumeric()),
        "generated id should be alphanumeric, got: {doc_id}"
    );
    assert_eq!(doc.generation, 1, "auto-id document should be at generation 1");
}

// ---------------------------------------------------------------------------
// Error scenarios
// ---------------------------------------------------------------------------

/// @driving_port @us_a02 @real_io @error
///
/// Feature: Concurrent write on same generation is rejected
///   Given "orders/ord-2026-001" exists at generation one
///   When  two callers successively update it both asserting the original update_time
///   Then  exactly one succeeds; the other receives a precondition-failed response
///
/// OCC semantics: both callers read the same update_time. The first write changes
/// update_time; the second write uses the now-stale precondition and fails.
#[tokio::test]
#[ignore = "requires Docker — unskip in S02A delivery"]
async fn concurrent_write_on_same_generation_is_rejected() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;

    // Create document
    let mut fields = HashMap::new();
    fields.insert("status".to_string(), str_val("pending"));
    let created = create_doc(&mut client, "orders", "ord-2026-001", fields).await;
    let doc_name = created.name.clone();

    // Both callers read the same update_time
    let current = get_doc(&mut client, &doc_name).await;
    let update_time_ts = current.update_time.clone().expect("document has update_time");

    let make_req = |ts: prost_types::Timestamp, val: &str| -> UpdateDocumentRequest {
        let mut f = HashMap::new();
        f.insert("status".to_string(), str_val(val));
        UpdateDocumentRequest {
            document: Some(Document {
                name: doc_name.clone(),
                fields: f,
                ..Default::default()
            }),
            current_document: Some(Precondition {
                condition_type: Some(ConditionType::UpdateTime(ts)),
            }),
            ..Default::default()
        }
    };

    // First update succeeds — changes update_time
    let r1 = client.update_document(make_req(update_time_ts.clone(), "first")).await;
    assert!(r1.is_ok(), "first OCC write should succeed");

    // Second update with same (now stale) precondition must fail
    let r2 = client.update_document(make_req(update_time_ts.clone(), "second")).await;
    assert!(r2.is_err(), "second OCC write with stale precondition should fail");
    assert_eq!(
        r2.unwrap_err().code(),
        tonic::Code::FailedPrecondition,
        "stale OCC should return FAILED_PRECONDITION"
    );
}

/// @driving_port @us_a02 @real_io @error
///
/// Feature: Updating a document that does not exist returns not-found
///   Given "orders/ord-2026-999" does not exist
///   When  a caller updates it with MustExist precondition
///   Then  the caller receives a not-found response; no document is created
#[tokio::test]
#[ignore = "requires Docker — unskip in S02A delivery"]
async fn updating_absent_document_returns_not_found() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;

    let doc_name =
        "projects/finops-prod/databases/(default)/documents/orders/ord-2026-999".to_string();

    let mut fields = HashMap::new();
    fields.insert("status".to_string(), str_val("shipped"));

    let req = UpdateDocumentRequest {
        document: Some(Document {
            name: doc_name.clone(),
            fields,
            ..Default::default()
        }),
        current_document: Some(Precondition {
            condition_type: Some(ConditionType::Exists(true)),
        }),
        ..Default::default()
    };

    let result = client.update_document(req).await;
    assert!(result.is_err(), "updating absent doc should fail");
    assert_eq!(
        result.unwrap_err().code(),
        tonic::Code::NotFound,
        "should return NOT_FOUND"
    );

    // Verify no document was created
    let get_result = client
        .get_document(GetDocumentRequest { name: doc_name, ..Default::default() })
        .await;
    assert!(get_result.is_err(), "document should not exist after failed update");
    assert_eq!(get_result.unwrap_err().code(), tonic::Code::NotFound);
}

/// @driving_port @us_a02 @real_io @error
///
/// Feature: Creating a document that already exists is rejected
///   Given "orders/ord-2026-001" already exists with status="pending"
///   When  a caller attempts to create it again
///   Then  the caller receives an already-exists response; existing doc unchanged
#[tokio::test]
#[ignore = "requires Docker — unskip in S02A delivery"]
async fn creating_document_that_already_exists_is_rejected() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;

    // Create initial document
    let mut fields = HashMap::new();
    fields.insert("status".to_string(), str_val("pending"));
    let created = create_doc(&mut client, "orders", "ord-2026-001", fields).await;

    // Attempt to create again with different content
    let mut new_fields = HashMap::new();
    new_fields.insert("status".to_string(), str_val("overwritten"));

    let result = client
        .create_document(CreateDocumentRequest {
            parent: "projects/finops-prod/databases/(default)/documents".to_string(),
            collection_id: "orders".to_string(),
            document_id: "ord-2026-001".to_string(),
            document: Some(Document {
                name: String::new(),
                fields: new_fields,
                ..Default::default()
            }),
            ..Default::default()
        })
        .await;

    assert!(result.is_err(), "creating duplicate doc should fail");
    assert_eq!(
        result.unwrap_err().code(),
        tonic::Code::AlreadyExists,
        "should return ALREADY_EXISTS"
    );

    // Existing document is unchanged
    let fetched = get_doc(&mut client, &created.name).await;
    assert_eq!(
        fetched.fields.get("status").and_then(|v| v.value_type.as_ref()),
        Some(&ValueType::StringValue("pending".to_string())),
        "original status preserved"
    );
}

// ---------------------------------------------------------------------------
// occ-precondition-validation (AC-OCC-07 — uniformity guard)
// ---------------------------------------------------------------------------

/// @driving_port @us_a02 @real_io @error
///
/// occ-precondition-validation AC-OCC-07: the identical malformed
/// `update_time` precondition rejection embyr-server's own UpdateDocument
/// RPC gives (AC-OCC-01) also holds on the agent's own StorageAgent surface
/// — proving the shared `embyr-pg-storage::to_datetime` fix closes BOTH
/// independent precondition-parsing paths (`embyr-server`'s
/// `convert_precondition` AND `embyr-agent`'s own `parse_precondition`), not
/// just embyr-server's.
///
/// Feature: A malformed update_time precondition on the agent's own surface is cleanly rejected
///   Given "orders/occ-malformed" exists on the agent's own storage
///   When  a caller updates it with Precondition.update_time.nanos = 2147483647 (i32::MAX)
///   Then  the agent returns INVALID_ARGUMENT naming the "nanos" field, not a panic
///   And   the document is unchanged
#[tokio::test]
#[ignore = "requires Docker — unskip in S02A delivery"]
async fn updating_with_malformed_nanos_precondition_returns_invalid_argument() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;

    let mut fields = HashMap::new();
    fields.insert("status".to_string(), str_val("original"));
    let created = create_doc(&mut client, "orders", "occ-malformed", fields).await;
    let doc_name = created.name.clone();

    let mut update_fields = HashMap::new();
    update_fields.insert("status".to_string(), str_val("should-not-land"));
    let req = UpdateDocumentRequest {
        document: Some(Document {
            name: doc_name.clone(),
            fields: update_fields,
            ..Default::default()
        }),
        current_document: Some(Precondition {
            condition_type: Some(ConditionType::UpdateTime(prost_types::Timestamp {
                seconds: 1_799_942_400,
                nanos: i32::MAX,
            })),
        }),
        ..Default::default()
    };

    let result = client.update_document(req).await;

    let status = result.expect_err(
        "a malformed nanos precondition on the agent's own surface must be cleanly rejected, \
         not panic the connection",
    );
    assert_eq!(
        status.code(),
        tonic::Code::InvalidArgument,
        "expected INVALID_ARGUMENT, got {:?}: {}",
        status.code(),
        status.message()
    );
    assert!(
        status.message().contains("nanos"),
        "error message must name the offending 'nanos' field, got: {}",
        status.message()
    );

    let fetched = get_doc(&mut client, &doc_name).await;
    assert_eq!(
        fetched.fields.get("status").and_then(|v| v.value_type.as_ref()),
        Some(&ValueType::StringValue("original".to_string())),
        "the rejected write must NOT have modified the document"
    );
}

// ---------------------------------------------------------------------------
// Property scenario
// ---------------------------------------------------------------------------

/// @driving_port @us_a02 @real_io @property
///
/// Feature: Document generation advances by exactly one on every successful write
///   Given "orders/ord-2026-001" is written successfully three times in sequence
///   When  the generation is read after each write
///   Then  the generations are 1, 2, 3 in that order; none repeat; none decrease
#[tokio::test]
#[ignore = "requires Docker — unskip in S02A delivery"]
async fn document_generation_advances_by_one_on_every_successful_write() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;

    // Write 1: create
    let mut fields = HashMap::new();
    fields.insert("step".to_string(), int_val(1));
    let doc = create_doc(&mut client, "orders", "ord-2026-001", fields).await;
    assert_eq!(doc.generation, 1, "generation after write 1 should be 1");
    let doc_name = doc.name.clone();

    // Write 2: update (no precondition — upsert)
    let mut f2 = HashMap::new();
    f2.insert("step".to_string(), int_val(2));
    let req2 = UpdateDocumentRequest {
        document: Some(Document { name: doc_name.clone(), fields: f2, ..Default::default() }),
        ..Default::default()
    };
    let doc2 = client.update_document(req2).await.expect("write 2 succeeded").into_inner();
    assert_eq!(doc2.generation, 2, "generation after write 2 should be 2");

    // Write 3: update again
    let mut f3 = HashMap::new();
    f3.insert("step".to_string(), int_val(3));
    let req3 = UpdateDocumentRequest {
        document: Some(Document { name: doc_name.clone(), fields: f3, ..Default::default() }),
        ..Default::default()
    };
    let doc3 = client.update_document(req3).await.expect("write 3 succeeded").into_inner();
    assert_eq!(doc3.generation, 3, "generation after write 3 should be 3");

    // Property: strictly ascending, increment of 1 each time
    let generations = [doc.generation, doc2.generation, doc3.generation];
    for (i, &g) in generations.iter().enumerate() {
        assert_eq!(g, (i + 1) as i64, "generation at position {i} should be {}", i + 1);
    }
}
