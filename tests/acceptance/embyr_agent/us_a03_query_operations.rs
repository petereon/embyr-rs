// SCAFFOLD: true
//! US-A03 — Query operations via agent
//!
//! As Alex (SDK Developer), I want getDocs(query(...)), runAggregation(...),
//! and listDocuments to work through the agent, so that read-heavy features
//! (dashboards, lists, analytics) work without SDK changes.
//!
//! Driving port: StorageAgent gRPC service (mTLS :9191)
//!   RPCs: RunQuery, RunAggregationQuery, ListDocuments
//! Red classification: MISSING_FUNCTIONALITY
//!
//! Feature file: tests/features/agent/us_a03_query_operations.feature
//! Execution order: S03A (after S02A writes)
//!
//! Note: RunAggregationQuery and ListDocuments are proto extensions (ADR-A01).
//! These tests assume the proto has been extended with those RPCs before S03A.

use embyr_proto::agent::{
    storage_agent_client::StorageAgentClient,
    RunQueryRequest,
};
// RunAggregationQueryRequest, ListDocumentsRequest imported when proto is extended

use super::agent_common::{start_test_agent, AgentHandle};

// ---------------------------------------------------------------------------
// Happy path scenarios
// ---------------------------------------------------------------------------

/// @driving_port @us_a03 @real_io
///
/// Feature: Filtered query returns only matching documents
///   Given 5 orders exist: 3 with status="pending", 2 with status="shipped"
///   When  a caller queries for status equals "pending"
///   Then  exactly 3 documents are returned; none have status="shipped"
#[tokio::test]
#[ignore = "requires Docker — unskip in S03A delivery"]
async fn filtered_query_returns_only_matching_documents() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;
    panic!("Not yet implemented — RED scaffold");
}

/// @driving_port @us_a03 @real_io
///
/// Feature: Collection group query traverses nested collections
///   Given "orders/ord-001/line_items/item-1" and "orders/ord-002/line_items/item-2" exist
///   When  a caller runs a collection group query for all "line_items" documents
///   Then  both documents are returned regardless of parent path
#[tokio::test]
#[ignore = "requires Docker — unskip in S03A delivery"]
async fn collection_group_query_traverses_nested_collections() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;
    panic!("Not yet implemented — RED scaffold");
}

/// @driving_port @us_a03 @real_io
///
/// Feature: Count aggregation returns the correct total
///   Given 7 orders exist in the "orders" collection
///   When  a caller runs a count aggregation over "orders"
///   Then  the caller receives a count of 7
#[tokio::test]
#[ignore = "requires Docker — unskip in S03A delivery"]
async fn count_aggregation_returns_correct_total() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;
    panic!("Not yet implemented — RED scaffold");
}

/// @driving_port @us_a03 @real_io
///
/// Feature: Listing documents returns results in pages of up to one hundred
///   Given 150 documents exist in the "orders" collection
///   When  a caller lists documents in "orders"
///   Then  the first page contains 100 documents with a continuation token
///   And   the final page has 50 documents with no continuation token
#[tokio::test]
#[ignore = "requires Docker — unskip in S03A delivery"]
async fn listing_documents_returns_pages_of_at_most_one_hundred() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;
    panic!("Not yet implemented — RED scaffold");
}

/// @driving_port @us_a03 @real_io
///
/// Feature: Query excluding a field value omits documents missing that field
///   Given 3 orders have status="pending" and 2 orders have no status field
///   When  a caller queries for status not equal to "shipped"
///   Then  only the 3 orders with status="pending" are returned
#[tokio::test]
#[ignore = "requires Docker — unskip in S03A delivery"]
async fn query_excluding_value_omits_documents_missing_that_field() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;
    panic!("Not yet implemented — RED scaffold");
}

/// @driving_port @us_a03 @real_io
///
/// Feature: Streaming query response indicates completion at the end
///   Given 3 orders exist in the "orders" collection
///   When  a caller runs a streaming query
///   Then  the final message indicates the query is complete with no document payload
#[tokio::test]
#[ignore = "requires Docker — unskip in S03A delivery"]
async fn streaming_query_response_indicates_completion_at_end() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;
    panic!("Not yet implemented — RED scaffold");
}

// ---------------------------------------------------------------------------
// Error scenarios
// ---------------------------------------------------------------------------

/// @driving_port @us_a03 @real_io @error
///
/// Feature: Query over an empty collection returns no documents and a completion signal
///   Given no documents exist in the "invoices" collection
///   When  a caller queries the "invoices" collection without filters
///   Then  zero documents are returned and a completion signal is received
#[tokio::test]
#[ignore = "requires Docker — unskip in S03A delivery"]
async fn query_over_empty_collection_returns_no_documents_and_completion_signal() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;
    panic!("Not yet implemented — RED scaffold");
}

/// @driving_port @us_a03 @real_io @error
///
/// Feature: Query with a malformed field path is rejected before any data is read
///   Given the "orders" collection contains documents
///   When  a caller queries with field path "order..amount" (double dot)
///   Then  the caller receives an invalid-request response
///   And   no documents are scanned from storage
#[tokio::test]
#[ignore = "requires Docker — unskip in S03A delivery"]
async fn query_with_malformed_field_path_rejected_before_data_read() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;
    panic!("Not yet implemented — RED scaffold");
}
