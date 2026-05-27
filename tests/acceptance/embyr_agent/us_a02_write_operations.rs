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
    storage_agent_client::StorageAgentClient,
    CreateDocumentRequest, DeleteDocumentRequest, UpdateDocumentRequest,
};

use super::agent_common::{start_test_agent, AgentHandle};

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
    panic!("Not yet implemented — RED scaffold");
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
    panic!("Not yet implemented — RED scaffold");
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
    panic!("Not yet implemented — RED scaffold");
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
    panic!("Not yet implemented — RED scaffold");
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
    panic!("Not yet implemented — RED scaffold");
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
    panic!("Not yet implemented — RED scaffold");
}

// ---------------------------------------------------------------------------
// Error scenarios
// ---------------------------------------------------------------------------

/// @driving_port @us_a02 @real_io @error
///
/// Feature: Concurrent write on same generation is rejected
///   Given "orders/ord-2026-001" exists at generation two
///   When  two callers simultaneously overwrite it both asserting generation two
///   Then  exactly one succeeds; the other receives a precondition-failed response
#[tokio::test]
#[ignore = "requires Docker — unskip in S02A delivery"]
async fn concurrent_write_on_same_generation_is_rejected() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;
    panic!("Not yet implemented — RED scaffold");
}

/// @driving_port @us_a02 @real_io @error
///
/// Feature: Updating a document that does not exist returns not-found
///   Given "orders/ord-2026-999" does not exist
///   When  a caller updates it
///   Then  the caller receives a not-found response; no document is created
#[tokio::test]
#[ignore = "requires Docker — unskip in S02A delivery"]
async fn updating_absent_document_returns_not_found() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;
    panic!("Not yet implemented — RED scaffold");
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
    panic!("Not yet implemented — RED scaffold");
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
    panic!("Not yet implemented — RED scaffold");
}
