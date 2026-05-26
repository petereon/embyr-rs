// SCAFFOLD: true
//! US-05 — Listen for real-time changes (onSnapshot)
//!
//! As Alex, I want to call onSnapshot and receive live updates within 2 seconds
//! of a write, so that I can build collaborative features without a separate
//! pub-sub service.
//!
//! Driving port: gRPC data port (:8080) — Listen bidirectional streaming RPC
//! Highest-risk story: 6 ACs, NOTIFY fan-out, resume tokens, delta delivery
//! Red classification: MISSING_FUNCTIONALITY

use embyr_core::auth::{argon2, ecies};
use embyr_proto::firestore::{
    firestore_client::FirestoreClient,
    listen_request, listen_response,
    target::{self, query_target},
    structured_query::CollectionSelector,
    CreateDocumentRequest, Document, ListenRequest, StructuredQuery, Target,
    target_change::TargetChangeType,
};
use embyr_server::{adapters::system_db::SystemDb, start_test_server_with_keepalive};
use std::{collections::HashMap, sync::Arc, time::Duration};
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt},
};
use tokio_stream::StreamExt;

async fn start_postgres() -> (ContainerAsync<Postgres>, String) {
    let container = Postgres::default()
        .with_tag("15-alpine")
        .start()
        .await
        .expect("Failed to start Postgres container");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("Failed to get host port");
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    (container, url)
}

struct TestEnv {
    _sys_container: ContainerAsync<Postgres>,
    _cust_container: ContainerAsync<Postgres>,
    project_id: String,
    api_key: String,
    server: embyr_server::TestServer,
}

async fn setup(api_key: &str, project_id: &str, keepalive: Duration) -> TestEnv {
    let (_sys_container, sys_url) = start_postgres().await;
    let (_cust_container, cust_url) = start_postgres().await;

    let system_db = Arc::new(SystemDb::new(&sys_url).await.unwrap());
    system_db.migrate().await.unwrap();

    let cust_pool = sqlx::PgPool::connect(&cust_url).await.unwrap();
    sqlx::migrate!("../../migrations/customer")
        .run(&cust_pool)
        .await
        .unwrap();

    let api_key_hash = argon2::hash_api_key(api_key.as_bytes()).unwrap();
    let pub_key = ecies::derive_public_key(api_key.as_bytes());
    let encrypted_dsn = ecies::encrypt(&pub_key, cust_url.as_bytes()).unwrap();

    let sys_pool = sqlx::PgPool::connect(&sys_url).await.unwrap();
    sqlx::query(
        "INSERT INTO projects \
         (id, status, backend_mode, api_key_hash_current, ecies_encrypted_dsn) \
         VALUES ($1, 'active', 'direct_pg', $2, $3)",
    )
    .bind(project_id)
    .bind(&api_key_hash)
    .bind(&encrypted_dsn)
    .execute(&sys_pool)
    .await
    .unwrap();

    let server = start_test_server_with_keepalive(system_db, keepalive).await;

    TestEnv {
        _sys_container,
        _cust_container,
        project_id: project_id.to_string(),
        api_key: api_key.to_string(),
        server,
    }
}

fn make_channel(addr: std::net::SocketAddr) -> tonic::transport::Channel {
    tonic::transport::Channel::from_shared(format!("http://{addr}"))
        .unwrap()
        .connect_lazy()
}

async fn seed_document(
    client: &mut FirestoreClient<tonic::transport::Channel>,
    project_id: &str,
    api_key: &str,
    collection: &str,
    doc_id: &str,
) {
    let parent = format!("projects/{project_id}/databases/(default)/documents");
    let mut req = tonic::Request::new(CreateDocumentRequest {
        parent,
        collection_id: collection.to_string(),
        document_id: doc_id.to_string(),
        document: Some(Document { name: String::new(), fields: HashMap::new(), ..Default::default() }),
        ..Default::default()
    });
    req.metadata_mut().insert(
        "authorization",
        format!("bearer {api_key}").parse().unwrap(),
    );
    client.create_document(req).await.expect("seed document should succeed");
}

fn add_target_request(project_id: &str, collection: &str) -> ListenRequest {
    ListenRequest {
        database: format!("projects/{project_id}/databases/(default)"),
        target_change: Some(listen_request::TargetChange::AddTarget(Target {
            target_id: 1,
            target_type: Some(target::TargetType::Query(target::QueryTarget {
                parent: format!("projects/{project_id}/databases/(default)/documents"),
                query_type: Some(query_target::QueryType::StructuredQuery(StructuredQuery {
                    from: vec![CollectionSelector {
                        collection_id: collection.to_string(),
                        all_descendants: false,
                    }],
                    ..Default::default()
                })),
            })),
            ..Default::default()
        })),
        ..Default::default()
    }
}

/// AC-05a: onSnapshot delivers all current documents as the initial snapshot
///
/// Given:  a provisioned project with 3 documents in "messages" collection
/// When:   a Listen stream is opened with an AddTarget for "messages"
/// Then:   the client receives exactly 3 DocumentChange(Added) events
/// And:    the CURRENT marker TargetChange is delivered after the 3rd document
#[tokio::test]
async fn listen_delivers_full_initial_snapshot() {
    let env = setup(
        "test-sk-us05-listen-01",
        "us05-listen-project-01",
        Duration::from_millis(500),
    )
    .await;
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    // Seed 3 documents into "messages"
    for doc_id in ["msg-1", "msg-2", "msg-3"] {
        seed_document(&mut client, &env.project_id, &env.api_key, "messages", doc_id).await;
    }

    // Open Listen stream with AddTarget for "messages"
    let req_stream = tokio_stream::once(add_target_request(&env.project_id, "messages"));
    let mut request = tonic::Request::new(req_stream);
    request
        .metadata_mut()
        .insert("authorization", format!("bearer {}", env.api_key).parse().unwrap());

    let mut listen_stream = client.listen(request).await.expect("listen should succeed").into_inner();

    // Collect responses until we see CURRENT
    let mut doc_change_count = 0usize;
    let mut received_current = false;

    while let Some(msg) = listen_stream.next().await {
        let msg = msg.expect("listen stream message should be Ok");
        match msg.response_type {
            Some(listen_response::ResponseType::DocumentChange(_)) => {
                doc_change_count += 1;
            }
            Some(listen_response::ResponseType::TargetChange(tc)) => {
                let change_type = TargetChangeType::try_from(tc.target_change_type)
                    .unwrap_or(TargetChangeType::NoChange);
                if change_type == TargetChangeType::Current {
                    received_current = true;
                    break;
                }
            }
            _ => {}
        }
    }

    assert_eq!(doc_change_count, 3, "expected exactly 3 DocumentChange events before CURRENT");
    assert!(received_current, "CURRENT marker must be delivered");
}

/// AC-05b: CURRENT marker arrives after the last document in the initial snapshot
///
/// Given:  a provisioned project with documents in "items"
/// When:   a Listen stream sends AddTarget for "items"
/// Then:   the TargetChange(CURRENT) event arrives strictly after all DocumentChange events
/// And:    no NO_CHANGE follows until a new write occurs or keep-alive fires
#[tokio::test]
async fn current_marker_arrives_after_last_initial_document() {
    let env = setup(
        "test-sk-us05-listen-02",
        "us05-listen-project-02",
        Duration::from_millis(500),
    )
    .await;
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    // Seed 2 documents into "items"
    for doc_id in ["item-a", "item-b"] {
        seed_document(&mut client, &env.project_id, &env.api_key, "items", doc_id).await;
    }

    let req_stream = tokio_stream::once(add_target_request(&env.project_id, "items"));
    let mut request = tonic::Request::new(req_stream);
    request
        .metadata_mut()
        .insert("authorization", format!("bearer {}", env.api_key).parse().unwrap());

    let mut listen_stream = client.listen(request).await.expect("listen should succeed").into_inner();

    // Collect events in order until CURRENT
    let mut events: Vec<&'static str> = Vec::new();
    while let Some(msg) = listen_stream.next().await {
        let msg = msg.expect("listen stream message should be Ok");
        match msg.response_type {
            Some(listen_response::ResponseType::DocumentChange(_)) => {
                events.push("DOC");
            }
            Some(listen_response::ResponseType::TargetChange(tc)) => {
                let change_type = TargetChangeType::try_from(tc.target_change_type)
                    .unwrap_or(TargetChangeType::NoChange);
                if change_type == TargetChangeType::Current {
                    events.push("CURRENT");
                    break;
                }
            }
            _ => {}
        }
    }

    // CURRENT must be the last event — all DOCs must come before it
    assert_eq!(events.last(), Some(&"CURRENT"), "CURRENT must be the final event");
    let doc_count = events.iter().filter(|&&e| e == "DOC").count();
    assert_eq!(doc_count, 2, "expected exactly 2 DocumentChange events before CURRENT");
    // Verify ordering: no CURRENT appears before all DOCs
    let current_pos = events.iter().position(|&e| e == "CURRENT").unwrap();
    for i in 0..current_pos {
        assert_eq!(events[i], "DOC", "all events before CURRENT must be DOC");
    }
}

/// AC-05c: write from a second client triggers onSnapshot within 2 seconds (KPI)
///
/// Given:  a Listen stream is active for collection "counters"
/// When:   a second gRPC client writes a new document to "counters" within the project
/// Then:   the Listen stream delivers a DocumentChange(Modified) event
/// And:    the event arrives within 2000 milliseconds of the write completing
#[tokio::test]
#[ignore = "us-05 AC-05c — RED scaffold, not yet implemented"]
async fn write_triggers_listen_callback_within_two_seconds() {
    panic!("RED scaffold — not yet implemented");
}

/// AC-05d: delete triggers a REMOVED change event on the listener
///
/// Given:  a document exists and a Listen stream is watching its collection
/// When:   the document is deleted via a DeleteDocument call
/// Then:   the Listen stream delivers a DocumentChange(Removed) event for that document
/// And:    a subsequent GetDocument for the path returns exists=false
#[tokio::test]
#[ignore = "us-05 AC-05d — RED scaffold, not yet implemented"]
async fn delete_triggers_removed_change_event_on_listener() {
    panic!("RED scaffold — not yet implemented");
}

/// AC-05e: reconnecting with resume token delivers only the delta (not a full re-snapshot)
///
/// Given:  a Listen stream was active and received a NO_CHANGE with a resume token
/// And:    the connection was dropped for 30 seconds
/// And:    3 documents were written during the disconnect
/// When:   the client reconnects using the saved resume token
/// Then:   only the 3 new DocumentChange events are delivered (delta)
/// And:    the full initial snapshot is NOT re-sent
#[tokio::test]
#[ignore = "us-05 AC-05e — RED scaffold, not yet implemented"]
async fn reconnect_with_resume_token_delivers_only_delta() {
    panic!("RED scaffold — not yet implemented");
}

/// AC-05f (error path): resume token older than 24 hours triggers full re-snapshot, no error
///
/// Given:  a resume token encoding a timestamp 25 hours in the past
/// When:   the client opens a Listen stream presenting that stale resume token
/// Then:   the server delivers a full snapshot (all current documents)
/// And:    no error status is returned to the client
/// And:    the client eventually receives a new fresh resume token in a NO_CHANGE response
#[tokio::test]
#[ignore = "us-05 AC-05f — RED scaffold, not yet implemented"]
async fn stale_resume_token_triggers_full_resnapshot_not_error() {
    panic!("RED scaffold — not yet implemented");
}

/// Error path: slow consumer buffer overflow triggers RESET
///
/// Given:  a Listen stream with a consumer that does not drain its receive buffer
/// When:   more than 64 DocChange events accumulate in the subscriber channel
/// Then:   the server sends a TargetChange(RESET)
/// And:    subsequent AddTarget from the client causes a fresh full snapshot
#[tokio::test]
#[ignore = "us-05 error — RED scaffold, not yet implemented"]
async fn slow_consumer_overflow_triggers_reset() {
    panic!("RED scaffold — not yet implemented");
}

/// Property: listen latency p99 <= 2 seconds under 100 concurrent listeners
///
/// Given:  100 concurrent Listen streams are active for 100 separate collections
/// When:   10 writes per second are applied across those collections
/// Then:   at least 99 of every 100 write-to-callback durations are <= 2000ms
///
/// Note: this is an invariant test — the KPI from feature-delta.md
#[tokio::test]
#[ignore = "us-05 property kpi — RED scaffold, not yet implemented"]
async fn listen_latency_p99_under_2_seconds_with_100_concurrent_listeners() {
    panic!("RED scaffold — not yet implemented");
}

/// Keep-alive: idle stream receives NO_CHANGE every 30 seconds
///
/// Given:  a Listen stream is open with no pending writes
/// When:   30 seconds elapse (500ms in test with fast keepalive)
/// Then:   the stream receives a NO_CHANGE TargetChange with no resume token
#[tokio::test]
async fn idle_listen_stream_receives_no_change_keep_alive() {
    let env = setup(
        "test-sk-us05-listen-03",
        "us05-listen-project-03",
        Duration::from_millis(500),
    )
    .await;
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    // No documents seeded — empty snapshot
    let req_stream = tokio_stream::once(add_target_request(&env.project_id, "empty-col"));
    let mut request = tonic::Request::new(req_stream);
    request
        .metadata_mut()
        .insert("authorization", format!("bearer {}", env.api_key).parse().unwrap());

    let mut listen_stream = client.listen(request).await.expect("listen should succeed").into_inner();

    // Drain until CURRENT (empty snapshot)
    while let Some(msg) = listen_stream.next().await {
        let msg = msg.expect("listen stream message should be Ok");
        if let Some(listen_response::ResponseType::TargetChange(tc)) = msg.response_type {
            let change_type = TargetChangeType::try_from(tc.target_change_type)
                .unwrap_or(TargetChangeType::NoChange);
            if change_type == TargetChangeType::Current {
                break;
            }
        }
    }

    // Wait 700ms — fast keepalive of 500ms should fire at least once
    tokio::time::sleep(Duration::from_millis(700)).await;

    // The next message should be NO_CHANGE
    let msg = listen_stream
        .next()
        .await
        .expect("should have another message")
        .expect("message should be Ok");

    match msg.response_type {
        Some(listen_response::ResponseType::TargetChange(tc)) => {
            let change_type = TargetChangeType::try_from(tc.target_change_type)
                .unwrap_or(TargetChangeType::Add);
            assert_eq!(change_type, TargetChangeType::NoChange, "expected NO_CHANGE keep-alive");
        }
        other => panic!("expected TargetChange(NO_CHANGE), got: {other:?}"),
    }
}

#[tokio::test]
#[ignore = "us-04 us-05 error — RED scaffold, not yet implemented"]
async fn subscription_with_multi_field_filter_without_index_is_rejected() {
    // AC-04f applied to Listen: composite filter without a READY index → FAILED_PRECONDITION
    panic!("RED scaffold — not yet implemented");
}
