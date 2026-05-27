// SCAFFOLD: true
//! US-A05 — Real-time change subscription via Subscribe stream
//!
//! As Alex (SDK Developer), I want onSnapshot to fire for every committed write
//! to an agent-backed project within 2 seconds, so that collaborative features
//! work identically to direct-mode Firestore.
//!
//! Driving port: StorageAgent gRPC service (mTLS :9191)
//!   RPC: Subscribe (server-streaming) — proto extension declared in ADR-A01
//! Red classification: MISSING_FUNCTIONALITY
//!
//! Feature file: tests/features/agent/us_a05_subscribe.feature
//! Execution order: S05A (last — depends on all prior slices)
//!
//! Note: Subscribe is a new server-streaming RPC. It must be added to
//! storage_agent.proto before this test file can compile. The import below
//! is intentionally written for the post-extension proto shape.

// SubscribeRequest and DocChange will be available after the proto extension:
// use embyr_proto::agent::{storage_agent_client::StorageAgentClient, SubscribeRequest, DocChange};

use super::agent_common::{start_test_agent, AgentHandle};

// ---------------------------------------------------------------------------
// Timing constants
// ---------------------------------------------------------------------------

/// Maximum time from write-commit to change-event arrival (Mandate: 2 seconds).
const CHANGE_DELIVERY_DEADLINE_SECS: u64 = 2;

// ---------------------------------------------------------------------------
// Happy path scenarios
// ---------------------------------------------------------------------------

/// @driving_port @us_a05 @real_io
///
/// Feature: Change notification arrives within 2 seconds of a committed write
///   Given a caller has opened a change subscription on the "orders" collection
///   When  a write commits setting "orders/ord-2026-001" status="shipped"
///   Then  the subscription caller receives a change event within 2 seconds
///   And   the event reflects status="shipped"
#[tokio::test]
#[ignore = "requires Docker + proto extension (Subscribe RPC) — unskip in S05A delivery"]
async fn change_notification_arrives_within_two_seconds_of_committed_write() {
    let (_handle, _client) = start_test_agent("finops-prod").await;
    // timing assertion: event must arrive within CHANGE_DELIVERY_DEADLINE_SECS
    panic!("Not yet implemented — RED scaffold");
}

/// @driving_port @us_a05 @real_io
///
/// Feature: Change event for an upsert carries a generation of at least one
///   Given a document is created in the project
///   When  the resulting change event is received by an active subscriber
///   Then  the event kind is "upsert" and the generation is at least 1
#[tokio::test]
#[ignore = "requires Docker + proto extension — unskip in S05A delivery"]
async fn change_event_for_upsert_carries_generation_of_at_least_one() {
    let (_handle, _client) = start_test_agent("finops-prod").await;
    panic!("Not yet implemented — RED scaffold");
}

/// @driving_port @us_a05 @real_io
///
/// Feature: Change event for a deleted document carries the delete kind
///   Given "orders/ord-2026-001" exists and a subscriber is active
///   When  the document is removed
///   Then  the subscriber receives a change event with kind="delete"
#[tokio::test]
#[ignore = "requires Docker + proto extension — unskip in S05A delivery"]
async fn change_event_for_deleted_document_carries_delete_kind() {
    let (_handle, _client) = start_test_agent("finops-prod").await;
    panic!("Not yet implemented — RED scaffold");
}

// ---------------------------------------------------------------------------
// Property scenario
// ---------------------------------------------------------------------------

/// @driving_port @us_a05 @real_io @property
///
/// Feature: Change event carries complete document contents (no truncation)
///   Given documents with fields totalling up to 100 kilobytes
///   When  those documents are written and change events are pushed
///   Then  each event carries the complete document fields; none are missing or truncated
#[tokio::test]
#[ignore = "requires Docker + proto extension — unskip in S05A delivery"]
async fn change_event_carries_complete_document_contents() {
    let (_handle, _client) = start_test_agent("finops-prod").await;
    panic!("Not yet implemented — RED scaffold");
}

// ---------------------------------------------------------------------------
// Error / edge scenarios
// ---------------------------------------------------------------------------

/// @driving_port @us_a05 @real_io @error
///
/// Feature: Overflow of pending change events triggers a reset notification
///   Given 64 pending change events are buffered in the subscription channel
///   When  one additional document is written
///   Then  the overflow indicator is set
///   And   all active subscribers receive a reset-and-resync notification
#[tokio::test]
#[ignore = "requires Docker + proto extension — unskip in S05A delivery"]
async fn overflow_of_pending_events_triggers_reset_notification() {
    let (_handle, _client) = start_test_agent("finops-prod").await;
    panic!("Not yet implemented — RED scaffold");
}

/// @driving_port @us_a05 @real_io @error
///
/// Feature: Subscription stream restores delivery after a connection interruption
///   Given a caller has an active change subscription
///   When  the subscription stream is interrupted
///   Then  the caller receives a reset-and-resync notification
///   And   after reconnection new writes are delivered within 2 seconds
#[tokio::test]
#[ignore = "requires Docker + proto extension — unskip in S05A delivery"]
async fn subscription_stream_restores_delivery_after_interruption() {
    let (_handle, _client) = start_test_agent("finops-prod").await;
    panic!("Not yet implemented — RED scaffold");
}
