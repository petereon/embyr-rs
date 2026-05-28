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

use embyr_proto::agent::DocChangeKind;

use super::agent_common::{start_test_agent, start_test_postgres};

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
///   And   the next DocChange delivered to active receivers carries kind=RESET
#[tokio::test]
async fn overflow_of_pending_events_triggers_reset_notification() {
    // -----------------------------------------------------------------------
    // GIVEN: A Postgres container with schema, and an AgentNotifyBridge.
    // The overflow test exercises AgentNotifyBridge directly (not via gRPC
    // Subscribe, which is implemented in step 06-02). Driving port:
    // AgentNotifyBridge::subscribe() — the port-level observable surface.
    // -----------------------------------------------------------------------
    let (_postgres, db_url) = start_test_postgres().await;

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&db_url)
        .await
        .expect("connect pool");

    embyr_pg_storage::backend_adapter::PostgresBackendAdapter::run_migrations(&pool)
        .await
        .expect("run migrations");

    let project_id = "test-overflow-project";
    let bridge = embyr_agent::notify_bridge::AgentNotifyBridge::new(pool.clone(), project_id.to_string());

    // Subscribe — returns a capacity-64 Receiver<DocChange>.
    let mut rx = bridge.subscribe().await.expect("subscribe");

    // -----------------------------------------------------------------------
    // WHEN: We fill the channel (64 slots) without reading, then drain, then
    // trigger one more notification.
    //
    // Protocol:
    //   1. Send 65 NOTIFY messages without reading from rx.
    //      Bridge processes: 64 try_send succeed (fills channel), 65th fails
    //      → overflow flag set to true.
    //   2. Drain all 64 buffered items (freeing channel space).
    //      Now overflow=true and channel is empty.
    //   3. Send one more NOTIFY. Bridge processes it:
    //      - Sees overflow=true → swap false → sends RESET to channel.
    //      - Then processes the real change payload.
    //   4. First item received after draining must be kind=RESET.
    // -----------------------------------------------------------------------
    let channel = embyr_pg_storage::notify_listener::notify_channel(project_id);

    // Insert a document so the bridge lookup returns UPSERT (exercises full path).
    sqlx::query(
        "INSERT INTO documents \
         (project_id, collection_path, document_id, fields, version, deleted) \
         VALUES ($1, 'overflow_col', 'doc-1', '{}'::jsonb, 1, false) \
         ON CONFLICT DO NOTHING",
    )
    .bind(project_id)
    .execute(&pool)
    .await
    .expect("insert test doc");

    // Step 1: send 65 NOTIFY messages. Channel capacity is 64; the 65th
    // try_send will fail → overflow flag set.
    for _ in 0..65_u32 {
        sqlx::query("SELECT pg_notify($1, $2)")
            .bind(&channel)
            .bind("overflow_col/doc-1")
            .execute(&pool)
            .await
            .expect("pg_notify");
    }

    // Wait for the bridge to process all 65 notifications.
    // 65 sequential PgListener recvs + DB lookups — allow up to 5 seconds.
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

    // Step 2: drain all buffered items without looking for RESET yet.
    // (These are the 64 items that fit; the 65th was dropped due to overflow.)
    let mut drained = 0usize;
    loop {
        match rx.try_recv() {
            Ok(_) => {
                drained += 1;
            }
            Err(_) => break,
        }
    }

    // Step 3: send one more NOTIFY to trigger the RESET delivery.
    // overflow=true + channel now has space → bridge sends RESET, then the change.
    sqlx::query("SELECT pg_notify($1, $2)")
        .bind(&channel)
        .bind("overflow_col/doc-1")
        .execute(&pool)
        .await
        .expect("pg_notify trigger");

    // Give the bridge time to process the trigger and send RESET.
    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

    // -----------------------------------------------------------------------
    // THEN: The first event received after the trigger must be kind=RESET.
    // -----------------------------------------------------------------------
    let mut saw_reset = false;
    let mut received = 0usize;

    loop {
        match rx.try_recv() {
            Ok(change) => {
                received += 1;
                let kind = DocChangeKind::try_from(change.kind).unwrap_or(DocChangeKind::Unspecified);
                if kind == DocChangeKind::Reset {
                    saw_reset = true;
                    break;
                }
            }
            Err(_) => break, // channel empty
        }
    }

    assert!(
        drained > 0,
        "expected to drain buffered events (channel should have been filled); drained {drained}"
    );
    assert!(
        saw_reset,
        "expected a RESET DocChange after channel overflow; drained {drained}, then received {received} events without RESET"
    );
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
