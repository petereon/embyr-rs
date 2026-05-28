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

use embyr_proto::agent::{
    value::ValueType, CreateDocumentRequest, DeleteDocumentRequest, Document, DocChangeKind,
    SubscribeRequest, Value,
};

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
///   And   the event reflects the committed document
#[tokio::test]
async fn change_notification_arrives_within_two_seconds_of_committed_write() {
    let (_handle, mut client) = start_test_agent("finops-prod").await;

    // Open subscribe stream on "orders" collection.
    let mut sub_stream = client
        .subscribe(tonic::Request::new(SubscribeRequest {
            collection_path: "orders".to_string(),
        }))
        .await
        .expect("subscribe call failed")
        .into_inner();

    // Write a document to the subscribed collection.
    client
        .create_document(tonic::Request::new(CreateDocumentRequest {
            parent: "projects/finops-prod/databases/(default)/documents".to_string(),
            collection_id: "orders".to_string(),
            document_id: "ord-2026-001".to_string(),
            document: Some(Document {
                fields: {
                    let mut m = std::collections::HashMap::new();
                    m.insert(
                        "status".to_string(),
                        Value {
                            value_type: Some(ValueType::StringValue("shipped".to_string())),
                        },
                    );
                    m
                },
                ..Default::default()
            }),
            ..Default::default()
        }))
        .await
        .expect("create_document failed");

    // Assert a DocChange arrives within 2 seconds.
    let change = tokio::time::timeout(
        std::time::Duration::from_secs(CHANGE_DELIVERY_DEADLINE_SECS),
        sub_stream.message(),
    )
    .await
    .expect("timeout: no change event arrived within 2 seconds")
    .expect("stream error")
    .expect("stream ended without a message");

    assert!(
        change.document_name.contains("ord-2026-001"),
        "expected document_name to contain 'ord-2026-001', got: {}",
        change.document_name
    );
}

/// @driving_port @us_a05 @real_io
///
/// Feature: Change event for an upsert carries a generation of at least one
///   Given a document is created in the project
///   When  the resulting change event is received by an active subscriber
///   Then  the event kind is "upsert" and the generation is at least 1
#[tokio::test]
async fn change_event_for_upsert_carries_generation_of_at_least_one() {
    let (_handle, mut client) = start_test_agent("finops-gen").await;

    let mut sub_stream = client
        .subscribe(tonic::Request::new(SubscribeRequest {
            collection_path: "invoices".to_string(),
        }))
        .await
        .expect("subscribe failed")
        .into_inner();

    client
        .create_document(tonic::Request::new(CreateDocumentRequest {
            parent: "projects/finops-gen/databases/(default)/documents".to_string(),
            collection_id: "invoices".to_string(),
            document_id: "inv-001".to_string(),
            document: Some(Document {
                fields: {
                    let mut m = std::collections::HashMap::new();
                    m.insert(
                        "amount".to_string(),
                        Value {
                            value_type: Some(ValueType::IntegerValue(100)),
                        },
                    );
                    m
                },
                ..Default::default()
            }),
            ..Default::default()
        }))
        .await
        .expect("create_document failed");

    let change = tokio::time::timeout(
        std::time::Duration::from_secs(CHANGE_DELIVERY_DEADLINE_SECS),
        sub_stream.message(),
    )
    .await
    .expect("timeout: no change event arrived within 2 seconds")
    .expect("stream error")
    .expect("stream ended without a message");

    assert_eq!(
        change.kind,
        DocChangeKind::Upsert as i32,
        "expected kind=UPSERT, got: {}",
        change.kind
    );
    assert!(
        change.generation >= 1,
        "expected generation >= 1, got: {}",
        change.generation
    );
}

/// @driving_port @us_a05 @real_io
///
/// Feature: Change event for a deleted document carries the delete kind
///   Given "orders/ord-2026-001" exists and a subscriber is active
///   When  the document is removed
///   Then  the subscriber receives a change event with kind="delete"
#[tokio::test]
async fn change_event_for_deleted_document_carries_delete_kind() {
    let (_handle, mut client) = start_test_agent("finops-del").await;

    // First create the document.
    client
        .create_document(tonic::Request::new(CreateDocumentRequest {
            parent: "projects/finops-del/databases/(default)/documents".to_string(),
            collection_id: "orders".to_string(),
            document_id: "ord-to-delete".to_string(),
            document: Some(Document {
                fields: {
                    let mut m = std::collections::HashMap::new();
                    m.insert(
                        "status".to_string(),
                        Value {
                            value_type: Some(ValueType::StringValue("active".to_string())),
                        },
                    );
                    m
                },
                ..Default::default()
            }),
            ..Default::default()
        }))
        .await
        .expect("create_document failed");

    // Now subscribe and then delete.
    let mut sub_stream = client
        .subscribe(tonic::Request::new(SubscribeRequest {
            collection_path: "orders".to_string(),
        }))
        .await
        .expect("subscribe failed")
        .into_inner();

    client
        .delete_document(tonic::Request::new(DeleteDocumentRequest {
            name: "projects/finops-del/databases/(default)/documents/orders/ord-to-delete"
                .to_string(),
            ..Default::default()
        }))
        .await
        .expect("delete_document failed");

    // Look for the DELETE event — drain up to a few events (the delete NOTIFY
    // arrives after subscription open, which may also deliver the prior UPSERT).
    let deadline = std::time::Duration::from_secs(CHANGE_DELIVERY_DEADLINE_SECS);
    let start = std::time::Instant::now();
    let mut found_delete = false;
    while start.elapsed() < deadline {
        match tokio::time::timeout(deadline - start.elapsed(), sub_stream.message()).await {
            Ok(Ok(Some(change))) => {
                if change.kind == DocChangeKind::Delete as i32
                    && change.document_name.contains("ord-to-delete")
                {
                    found_delete = true;
                    break;
                }
            }
            _ => break,
        }
    }

    assert!(
        found_delete,
        "expected a DELETE DocChange for 'ord-to-delete' within {} seconds",
        CHANGE_DELIVERY_DEADLINE_SECS
    );
}

// ---------------------------------------------------------------------------
// Property scenario
// ---------------------------------------------------------------------------

/// @driving_port @us_a05 @real_io @property
///
/// Feature: Change event carries complete document contents (no truncation)
///   Given documents with multiple fields
///   When  those documents are written and change events are pushed
///   Then  each event carries the complete document fields; none are missing or truncated
#[tokio::test]
async fn change_event_carries_complete_document_contents() {
    let (_handle, mut client) = start_test_agent("finops-fields").await;

    let mut sub_stream = client
        .subscribe(tonic::Request::new(SubscribeRequest {
            collection_path: "products".to_string(),
        }))
        .await
        .expect("subscribe failed")
        .into_inner();

    // Create document with multiple fields.
    let mut fields = std::collections::HashMap::new();
    fields.insert(
        "name".to_string(),
        Value {
            value_type: Some(ValueType::StringValue("Widget A".to_string())),
        },
    );
    fields.insert(
        "price".to_string(),
        Value {
            value_type: Some(ValueType::IntegerValue(999)),
        },
    );
    fields.insert(
        "in_stock".to_string(),
        Value {
            value_type: Some(ValueType::BooleanValue(true)),
        },
    );

    client
        .create_document(tonic::Request::new(CreateDocumentRequest {
            parent: "projects/finops-fields/databases/(default)/documents".to_string(),
            collection_id: "products".to_string(),
            document_id: "widget-a".to_string(),
            document: Some(Document {
                fields: fields.clone(),
                ..Default::default()
            }),
            ..Default::default()
        }))
        .await
        .expect("create_document failed");

    let change = tokio::time::timeout(
        std::time::Duration::from_secs(CHANGE_DELIVERY_DEADLINE_SECS),
        sub_stream.message(),
    )
    .await
    .expect("timeout: no change event within 2 seconds")
    .expect("stream error")
    .expect("stream ended without message");

    assert_eq!(
        change.kind,
        DocChangeKind::Upsert as i32,
        "expected UPSERT kind"
    );
    assert!(
        change.fields.contains_key("name"),
        "expected 'name' field in DocChange, got fields: {:?}",
        change.fields.keys().collect::<Vec<_>>()
    );
    assert!(
        change.fields.contains_key("price"),
        "expected 'price' field in DocChange"
    );
    assert!(
        change.fields.contains_key("in_stock"),
        "expected 'in_stock' field in DocChange"
    );
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
///   When  the subscription stream is interrupted and the caller reconnects
///   Then  after reconnection new writes are delivered within 2 seconds
#[tokio::test]
async fn subscription_stream_restores_delivery_after_interruption() {
    let (_handle, mut client) = start_test_agent("finops-reconnect").await;

    // First subscription — open and then drop it (simulating interruption).
    {
        let _stream = client
            .subscribe(tonic::Request::new(SubscribeRequest {
                collection_path: "events".to_string(),
            }))
            .await
            .expect("first subscribe failed")
            .into_inner();
        // Drop stream here — simulates client disconnect.
    }

    // Reconnect: open a new subscription on the same collection.
    let mut sub_stream = client
        .subscribe(tonic::Request::new(SubscribeRequest {
            collection_path: "events".to_string(),
        }))
        .await
        .expect("second subscribe (reconnect) failed")
        .into_inner();

    // Write a document after reconnection.
    client
        .create_document(tonic::Request::new(CreateDocumentRequest {
            parent: "projects/finops-reconnect/databases/(default)/documents".to_string(),
            collection_id: "events".to_string(),
            document_id: "evt-post-reconnect".to_string(),
            document: Some(Document {
                fields: {
                    let mut m = std::collections::HashMap::new();
                    m.insert(
                        "type".to_string(),
                        Value {
                            value_type: Some(ValueType::StringValue("login".to_string())),
                        },
                    );
                    m
                },
                ..Default::default()
            }),
            ..Default::default()
        }))
        .await
        .expect("create_document failed");

    // Assert DocChange arrives within 2 seconds on the new stream.
    let change = tokio::time::timeout(
        std::time::Duration::from_secs(CHANGE_DELIVERY_DEADLINE_SECS),
        sub_stream.message(),
    )
    .await
    .expect("timeout: no change event after reconnection within 2 seconds")
    .expect("stream error")
    .expect("stream ended without message");

    assert!(
        change.document_name.contains("evt-post-reconnect"),
        "expected change for 'evt-post-reconnect' after reconnection, got: {}",
        change.document_name
    );
}
