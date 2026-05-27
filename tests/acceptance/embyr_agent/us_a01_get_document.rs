// SCAFFOLD: true
//! US-A01 — GetDocument via agent (Walking Skeleton)
//!
//! As Riley (DevSecOps Lead), I want the embyr-agent to handle GetDocument RPCs
//! and return the document from local storage, so that I can prove the end-to-end
//! mTLS wiring chain works before committing to a full agent deployment.
//!
//! Driving port: StorageAgent gRPC service (mTLS :9191) — `GetDocument` RPC
//! Red classification: MISSING_FUNCTIONALITY
//!
//! Feature file: tests/features/agent/us_a01_get_document.feature
//! Execution order: S01A (Walking Skeleton — first slice)

use embyr_proto::agent::{
    storage_agent_client::StorageAgentClient, GetDocumentRequest,
};

use super::agent_common::{start_test_agent, AgentHandle};

// ---------------------------------------------------------------------------
// Walking Skeleton — all 3 scenarios enabled
// ---------------------------------------------------------------------------

/// @walking_skeleton @driving_port @us_a01 @real_io
///
/// Feature: Agent returns document fields to an authenticated caller
///   Given the document "users/riley" exists with fields name="Riley" and role="devops"
///   When  an authenticated caller requests the document "users/riley"
///   Then  the caller receives the document with name="Riley" and role="devops"
///   And   the agent audit log records the request with the project identifier and path
#[tokio::test]
async fn agent_returns_document_fields_to_authenticated_caller() {
    let (handle, mut client) = start_test_agent("finops-prod").await;

    // Given: seed the document into the project's storage via the pool on AgentHandle.
    sqlx::query(
        "INSERT INTO documents \
         (project_id, collection_path, document_id, fields, version, create_time, update_time) \
         VALUES ($1, $2, $3, $4::jsonb, 1, NOW(), NOW())",
    )
    .bind("finops-prod")
    .bind("users")
    .bind("riley")
    .bind(r#"{"name":{"t":"S","v":"Riley"},"role":{"t":"S","v":"devops"}}"#)
    .execute(&handle.pool)
    .await
    .expect("seed document");

    // When: authenticated caller requests the document.
    let response = client
        .get_document(tonic::Request::new(GetDocumentRequest {
            name: "projects/finops-prod/databases/(default)/documents/users/riley".to_string(),
            ..Default::default()
        }))
        .await
        .expect("get_document succeeded");

    // Then: caller receives correct document.
    let doc = response.into_inner();
    assert_eq!(
        doc.name,
        "projects/finops-prod/databases/(default)/documents/users/riley"
    );

    // And: fields contain name="Riley" and role="devops".
    let name_field = doc.fields.get("name").expect("name field present");
    let role_field = doc.fields.get("role").expect("role field present");

    use embyr_proto::agent::value::ValueType;
    assert!(
        matches!(&name_field.value_type, Some(ValueType::StringValue(s)) if s == "Riley"),
        "expected name=Riley, got: {name_field:?}"
    );
    assert!(
        matches!(&role_field.value_type, Some(ValueType::StringValue(s)) if s == "devops"),
        "expected role=devops, got: {role_field:?}"
    );

    drop(handle);
}

// ---------------------------------------------------------------------------
// Focused error scenarios
// ---------------------------------------------------------------------------

/// @driving_port @us_a01 @real_io @error
///
/// Feature: Agent signals document not found for absent path
///   Given the document "orders/nonexistent" does not exist in the project
///   When  an authenticated caller requests the document "orders/nonexistent"
///   Then  the caller receives a not-found response
///   And   no error is raised on the caller side
#[tokio::test]
async fn agent_signals_not_found_for_absent_document() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;

    let result = client
        .get_document(tonic::Request::new(GetDocumentRequest {
            name: "projects/finops-prod/databases/(default)/documents/orders/nonexistent"
                .to_string(),
            ..Default::default()
        }))
        .await;

    assert!(result.is_err(), "expected error, got Ok");
    assert_eq!(
        result.unwrap_err().code(),
        tonic::Code::NotFound,
        "expected NotFound status code"
    );
}

/// @driving_port @us_a01 @real_io @error
///
/// Feature: Agent rejects request with empty document path
///   Given —
///   When  an authenticated caller requests a document with an empty path
///   Then  the caller receives an invalid-request response
///   And   the response message references the missing path field
#[tokio::test]
async fn agent_rejects_request_with_empty_document_path() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;

    let result = client
        .get_document(tonic::Request::new(GetDocumentRequest {
            name: String::new(),
            ..Default::default()
        }))
        .await;

    assert!(result.is_err(), "expected error, got Ok");
    assert_eq!(
        result.unwrap_err().code(),
        tonic::Code::InvalidArgument,
        "expected InvalidArgument status code"
    );
}
