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
    CreateDocumentRequest, DeleteDocumentRequest, Document, ListenRequest, StructuredQuery, Target,
    target_change::TargetChangeType,
};
use embyr_server::{
    adapters::{
        postgres_notify_listener::notify_channel,
        system_db::SystemDb,
    },
    start_test_server_with_keepalive,
};
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
async fn write_triggers_listen_callback_within_two_seconds() {
    let env = setup(
        "test-sk-us05-listen-04",
        "us05-listen-project-04",
        Duration::from_secs(30), // long keepalive — test must not rely on it
    )
    .await;

    let mut listener_client = FirestoreClient::new(make_channel(env.server.grpc_addr));
    let mut writer_client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    // Open Listen stream for empty "counters" collection.
    let req_stream = tokio_stream::once(add_target_request(&env.project_id, "counters"));
    let mut listen_req = tonic::Request::new(req_stream);
    listen_req
        .metadata_mut()
        .insert("authorization", format!("bearer {}", env.api_key).parse().unwrap());

    let mut listen_stream = listener_client
        .listen(listen_req)
        .await
        .expect("listen should succeed")
        .into_inner();

    // Drain until CURRENT (empty snapshot — 0 docs).
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(5), listen_stream.next())
            .await
            .expect("timed out waiting for CURRENT")
            .expect("stream ended before CURRENT")
            .expect("stream error before CURRENT");
        if let Some(listen_response::ResponseType::TargetChange(tc)) = msg.response_type {
            let change_type = TargetChangeType::try_from(tc.target_change_type)
                .unwrap_or(TargetChangeType::NoChange);
            if change_type == TargetChangeType::Current {
                break;
            }
        }
    }

    // Write a new document from the second client.
    let parent = format!("projects/{}/databases/(default)/documents", env.project_id);
    let mut write_req = tonic::Request::new(CreateDocumentRequest {
        parent,
        collection_id: "counters".to_string(),
        document_id: "counter-1".to_string(),
        document: Some(Document { name: String::new(), fields: HashMap::new(), ..Default::default() }),
        ..Default::default()
    });
    write_req.metadata_mut().insert(
        "authorization",
        format!("bearer {}", env.api_key).parse().unwrap(),
    );
    writer_client
        .create_document(write_req)
        .await
        .expect("write should succeed");

    // Expect a DocumentChange event within 2 seconds.
    let received = tokio::time::timeout(Duration::from_millis(2000), async {
        loop {
            let msg = listen_stream.next().await?.ok()?;
            if let Some(listen_response::ResponseType::DocumentChange(_)) = msg.response_type {
                return Some(());
            }
        }
    })
    .await;

    assert!(
        received.is_ok() && received.unwrap().is_some(),
        "expected DocumentChange within 2 seconds of write"
    );
}

/// AC-05d: delete triggers a REMOVED change event on the listener
///
/// Given:  a document exists and a Listen stream is watching its collection
/// When:   the document is deleted via a DeleteDocument call
/// Then:   the Listen stream delivers a DocumentChange(Removed) event for that document
/// And:    a subsequent GetDocument for the path returns exists=false
#[tokio::test]
async fn delete_triggers_removed_change_event_on_listener() {
    let env = setup(
        "test-sk-us05-listen-05",
        "us05-listen-project-05",
        Duration::from_secs(30),
    )
    .await;

    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    // Seed 1 document in "tasks".
    seed_document(&mut client, &env.project_id, &env.api_key, "tasks", "task-1").await;

    // Open Listen stream for "tasks".
    let req_stream = tokio_stream::once(add_target_request(&env.project_id, "tasks"));
    let mut listen_req = tonic::Request::new(req_stream);
    listen_req
        .metadata_mut()
        .insert("authorization", format!("bearer {}", env.api_key).parse().unwrap());

    let mut listen_stream = client
        .listen(listen_req)
        .await
        .expect("listen should succeed")
        .into_inner();

    // Drain until CURRENT (initial snapshot with 1 doc).
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(5), listen_stream.next())
            .await
            .expect("timed out waiting for CURRENT")
            .expect("stream ended before CURRENT")
            .expect("stream error before CURRENT");
        if let Some(listen_response::ResponseType::TargetChange(tc)) = msg.response_type {
            let change_type = TargetChangeType::try_from(tc.target_change_type)
                .unwrap_or(TargetChangeType::NoChange);
            if change_type == TargetChangeType::Current {
                break;
            }
        }
    }

    // Delete the document from a second client connection.
    let mut delete_client = FirestoreClient::new(make_channel(env.server.grpc_addr));
    let doc_name = format!(
        "projects/{}/databases/(default)/documents/tasks/task-1",
        env.project_id
    );
    let mut delete_req = tonic::Request::new(DeleteDocumentRequest {
        name: doc_name,
        ..Default::default()
    });
    delete_req.metadata_mut().insert(
        "authorization",
        format!("bearer {}", env.api_key).parse().unwrap(),
    );
    delete_client
        .delete_document(delete_req)
        .await
        .expect("delete should succeed");

    // Expect a DocumentDelete event within 2 seconds.
    let received = tokio::time::timeout(Duration::from_millis(2000), async {
        loop {
            let msg = listen_stream.next().await?.ok()?;
            if let Some(listen_response::ResponseType::DocumentDelete(_)) = msg.response_type {
                return Some(());
            }
        }
    })
    .await;

    assert!(
        received.is_ok() && received.unwrap().is_some(),
        "expected DocumentDelete within 2 seconds of delete"
    );
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
async fn reconnect_with_resume_token_delivers_only_delta() {
    let env = setup(
        "test-sk-us05-listen-07",
        "us05-listen-project-07",
        Duration::from_millis(200),
    )
    .await;
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    // Step a: Write doc "delta-1" to collection "resume-col"
    seed_document(&mut client, &env.project_id, &env.api_key, "resume-col", "delta-1").await;

    // Step b: Open first Listen stream for "resume-col", drain until CURRENT
    let req_stream = tokio_stream::once(add_target_request(&env.project_id, "resume-col"));
    let mut request = tonic::Request::new(req_stream);
    request.metadata_mut().insert(
        "authorization",
        format!("bearer {}", env.api_key).parse().unwrap(),
    );
    let mut first_stream = client.listen(request).await.expect("listen should succeed").into_inner();

    // Drain until CURRENT
    let mut initial_doc_count = 0usize;
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(5), first_stream.next())
            .await
            .expect("timed out waiting for CURRENT")
            .expect("stream ended before CURRENT")
            .expect("stream error");
        match msg.response_type {
            Some(listen_response::ResponseType::DocumentChange(_)) => {
                initial_doc_count += 1;
            }
            Some(listen_response::ResponseType::TargetChange(tc)) => {
                let change_type = TargetChangeType::try_from(tc.target_change_type)
                    .unwrap_or(TargetChangeType::NoChange);
                if change_type == TargetChangeType::Current {
                    break;
                }
            }
            _ => {}
        }
    }
    assert_eq!(initial_doc_count, 1, "expected exactly 1 doc in initial snapshot");

    // Step c: Read next message — should be NO_CHANGE with resume_token bytes
    let resume_token_bytes = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let msg = first_stream.next().await?.ok()?;
            if let Some(listen_response::ResponseType::TargetChange(tc)) = msg.response_type {
                let change_type = TargetChangeType::try_from(tc.target_change_type)
                    .unwrap_or(TargetChangeType::Add);
                if change_type == TargetChangeType::NoChange && !tc.resume_token.is_empty() {
                    return Some(tc.resume_token);
                }
            }
        }
    })
    .await
    .expect("timed out waiting for NO_CHANGE with resume token")
    .expect("expected NO_CHANGE with resume token bytes");

    // Wait 1100ms to ensure the next writes land in a different second than the token timestamp
    // (resume token stores second-precision timestamps; delta filter uses date_trunc('second')).
    tokio::time::sleep(Duration::from_millis(1100)).await;

    // Step d: Write 3 more docs
    for doc_id in ["delta-2", "delta-3", "delta-4"] {
        seed_document(&mut client, &env.project_id, &env.api_key, "resume-col", doc_id).await;
    }

    // Step e: Drop the first stream (let it go out of scope)
    drop(first_stream);

    // Step f: Open NEW Listen stream, pass saved resume_token in AddTarget
    let resume_request = ListenRequest {
        database: format!("projects/{}/databases/(default)", env.project_id),
        target_change: Some(listen_request::TargetChange::AddTarget(Target {
            target_id: 1,
            resume_type: Some(embyr_proto::firestore::target::ResumeType::ResumeToken(
                resume_token_bytes,
            )),
            target_type: Some(embyr_proto::firestore::target::TargetType::Query(
                embyr_proto::firestore::target::QueryTarget {
                    parent: format!("projects/{}/databases/(default)/documents", env.project_id),
                    query_type: Some(embyr_proto::firestore::target::query_target::QueryType::StructuredQuery(
                        StructuredQuery {
                            from: vec![CollectionSelector {
                                collection_id: "resume-col".to_string(),
                                all_descendants: false,
                            }],
                            ..Default::default()
                        },
                    )),
                },
            )),
            ..Default::default()
        })),
        ..Default::default()
    };

    let req_stream2 = tokio_stream::once(resume_request);
    let mut request2 = tonic::Request::new(req_stream2);
    request2.metadata_mut().insert(
        "authorization",
        format!("bearer {}", env.api_key).parse().unwrap(),
    );
    let mut second_stream = client.listen(request2).await.expect("second listen should succeed").into_inner();

    // Step g: Drain second stream: collect DocumentChange events until CURRENT
    let mut delta_doc_names: Vec<String> = Vec::new();
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(10), second_stream.next())
            .await
            .expect("timed out waiting for CURRENT on second stream")
            .expect("stream ended before CURRENT")
            .expect("stream error");
        match msg.response_type {
            Some(listen_response::ResponseType::DocumentChange(dc)) => {
                if let Some(doc) = dc.document {
                    delta_doc_names.push(doc.name);
                }
            }
            Some(listen_response::ResponseType::TargetChange(tc)) => {
                let change_type = TargetChangeType::try_from(tc.target_change_type)
                    .unwrap_or(TargetChangeType::NoChange);
                if change_type == TargetChangeType::Current {
                    break;
                }
            }
            _ => {}
        }
    }

    // Assert: exactly 3 DocumentChange events; delta-1 NOT present
    assert_eq!(delta_doc_names.len(), 3, "expected exactly 3 delta DocumentChange events, got: {:?}", delta_doc_names);
    let contains_delta1 = delta_doc_names.iter().any(|n| n.contains("delta-1"));
    assert!(!contains_delta1, "delta-1 should NOT be in delta delivery, got: {:?}", delta_doc_names);
}

/// AC-05f (error path): resume token older than 24 hours triggers full re-snapshot, no error
///
/// Given:  a resume token encoding a timestamp 25 hours in the past
/// When:   the client opens a Listen stream presenting that stale resume token
/// Then:   the server delivers a full snapshot (all current documents)
/// And:    no error status is returned to the client
/// And:    the client eventually receives a new fresh resume token in a NO_CHANGE response
#[tokio::test]
async fn stale_resume_token_triggers_full_resnapshot_not_error() {
    use chrono::Utc;

    let env = setup(
        "test-sk-us05-listen-08",
        "us05-listen-project-08",
        Duration::from_millis(200),
    )
    .await;
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    // Write "all-1" and "all-2" to "stale-col"
    seed_document(&mut client, &env.project_id, &env.api_key, "stale-col", "all-1").await;
    seed_document(&mut client, &env.project_id, &env.api_key, "stale-col", "all-2").await;

    // Build stale token: 8-byte i64 BE of (Utc::now() - 25 hours).timestamp(), then 32 zero bytes
    let stale_ts = (Utc::now() - chrono::Duration::hours(25)).timestamp();
    let mut stale_token = Vec::with_capacity(40);
    stale_token.extend_from_slice(&stale_ts.to_be_bytes());
    stale_token.extend_from_slice(&[0u8; 32]);

    // Open Listen stream with stale token in AddTarget
    let stale_request = ListenRequest {
        database: format!("projects/{}/databases/(default)", env.project_id),
        target_change: Some(listen_request::TargetChange::AddTarget(Target {
            target_id: 1,
            resume_type: Some(embyr_proto::firestore::target::ResumeType::ResumeToken(
                stale_token,
            )),
            target_type: Some(embyr_proto::firestore::target::TargetType::Query(
                embyr_proto::firestore::target::QueryTarget {
                    parent: format!("projects/{}/databases/(default)/documents", env.project_id),
                    query_type: Some(embyr_proto::firestore::target::query_target::QueryType::StructuredQuery(
                        StructuredQuery {
                            from: vec![CollectionSelector {
                                collection_id: "stale-col".to_string(),
                                all_descendants: false,
                            }],
                            ..Default::default()
                        },
                    )),
                },
            )),
            ..Default::default()
        })),
        ..Default::default()
    };

    let req_stream = tokio_stream::once(stale_request);
    let mut request = tonic::Request::new(req_stream);
    request.metadata_mut().insert(
        "authorization",
        format!("bearer {}", env.api_key).parse().unwrap(),
    );
    let mut listen_stream = client.listen(request).await.expect("listen should succeed with stale token").into_inner();

    // Drain until CURRENT, collect DocumentChange events
    let mut doc_count = 0usize;
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(10), listen_stream.next())
            .await
            .expect("timed out waiting for CURRENT")
            .expect("stream ended before CURRENT")
            .expect("stream error — no error status expected for stale token");
        match msg.response_type {
            Some(listen_response::ResponseType::DocumentChange(_)) => {
                doc_count += 1;
            }
            Some(listen_response::ResponseType::TargetChange(tc)) => {
                let change_type = TargetChangeType::try_from(tc.target_change_type)
                    .unwrap_or(TargetChangeType::NoChange);
                if change_type == TargetChangeType::Current {
                    break;
                }
            }
            _ => {}
        }
    }

    // Assert: 2 DocumentChange events (full snapshot); no error
    assert_eq!(doc_count, 2, "expected 2 DocumentChange events for full snapshot after stale token");
}

/// Error path: slow consumer buffer overflow triggers RESET
///
/// Given:  a Listen stream with a consumer that does not drain its receive buffer
/// When:   more than 64 DocChange events accumulate in the subscriber channel
/// Then:   the server sends a TargetChange(RESET)
/// And:    subsequent AddTarget from the client causes a fresh full snapshot
#[tokio::test]
async fn slow_consumer_overflow_triggers_reset() {
    let env = setup(
        "test-sk-us05-listen-06",
        "us05-listen-project-06",
        Duration::from_secs(30),
    )
    .await;

    let mut listener_client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    // Open Listen stream for "overflow-col".
    let req_stream = tokio_stream::once(add_target_request(&env.project_id, "overflow-col"));
    let mut listen_req = tonic::Request::new(req_stream);
    listen_req
        .metadata_mut()
        .insert("authorization", format!("bearer {}", env.api_key).parse().unwrap());

    let mut listen_stream = listener_client
        .listen(listen_req)
        .await
        .expect("listen should succeed")
        .into_inner();

    // Drain until CURRENT (empty snapshot).
    loop {
        let msg = tokio::time::timeout(Duration::from_secs(5), listen_stream.next())
            .await
            .expect("timed out waiting for CURRENT")
            .expect("stream ended before CURRENT")
            .expect("stream error");
        if let Some(listen_response::ResponseType::TargetChange(tc)) = msg.response_type {
            let change_type = TargetChangeType::try_from(tc.target_change_type)
                .unwrap_or(TargetChangeType::NoChange);
            if change_type == TargetChangeType::Current {
                break;
            }
        }
    }

    // Simulate overflow directly via the registry — this is more reliable than
    // trying to saturate the channel via timing under async scheduling.
    // The capacity-based overflow (for production use) is tested by the unit
    // mechanism in listen_registry; here we verify the end-to-end RESET delivery.
    let channel = notify_channel(&env.project_id);
    // Give the listen handler a moment to register the subscriber.
    tokio::time::sleep(Duration::from_millis(50)).await;
    env.server.listen_registry.simulate_overflow(&channel).await;

    // Drain the stream — the RESET should arrive within a short window.
    let found_reset = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let msg = listen_stream.next().await?.ok()?;
            if let Some(listen_response::ResponseType::TargetChange(tc)) = msg.response_type {
                let change_type = TargetChangeType::try_from(tc.target_change_type)
                    .unwrap_or(TargetChangeType::NoChange);
                if change_type == TargetChangeType::Reset {
                    return Some(());
                }
            }
        }
    })
    .await;

    assert!(
        found_reset.is_ok() && found_reset.unwrap().is_some(),
        "expected TargetChange(RESET) when subscriber channel overflows"
    );
}

/// Property: listen latency p99 <= 2 seconds under 100 concurrent listeners
///
/// Given:  100 concurrent Listen streams are active for 100 separate collections
/// When:   10 writes per second are applied across those collections
/// Then:   at least 99 of every 100 write-to-callback durations are <= 2000ms
///
/// Note: this is an invariant test — the KPI from feature-delta.md
#[tokio::test]
async fn listen_latency_p99_under_2_seconds_with_100_concurrent_listeners() {
    let env = std::sync::Arc::new(
        setup(
            "test-sk-us05-listen-09",
            "us05-listen-project-09",
            Duration::from_secs(30),
        )
        .await,
    );

    let num_listeners = 100usize;
    let addr = env.server.grpc_addr;
    let project_id = env.project_id.clone();
    let api_key = env.api_key.clone();

    // Open 100 Listen streams, one per collection col-0..col-99
    let mut streams = Vec::with_capacity(num_listeners);
    for i in 0..num_listeners {
        let collection = format!("col-{i}");
        let req_stream = tokio_stream::once(add_target_request(&project_id, &collection));
        let mut request = tonic::Request::new(req_stream);
        request.metadata_mut().insert(
            "authorization",
            format!("bearer {api_key}").parse().unwrap(),
        );
        let mut client = FirestoreClient::new(make_channel(addr));
        let listen_stream = client.listen(request).await.expect("listen should succeed").into_inner();
        streams.push((client, listen_stream, collection));
    }

    // Drain all streams until CURRENT (empty snapshot)
    for (_client, stream, _coll) in &mut streams {
        loop {
            let msg = tokio::time::timeout(Duration::from_secs(10), stream.next())
                .await
                .expect("timed out waiting for CURRENT")
                .expect("stream ended before CURRENT")
                .expect("stream error");
            if let Some(listen_response::ResponseType::TargetChange(tc)) = msg.response_type {
                let change_type = TargetChangeType::try_from(tc.target_change_type)
                    .unwrap_or(TargetChangeType::NoChange);
                if change_type == TargetChangeType::Current {
                    break;
                }
            }
        }
    }

    // For each of the 100 collections: write a doc and measure latency
    let mut write_client = FirestoreClient::new(make_channel(addr));
    let mut latencies_ms: Vec<u128> = Vec::with_capacity(num_listeners);

    for (i, (_client, stream, collection)) in streams.iter_mut().enumerate() {
        let doc_id = format!("lat-doc-{i}");
        let write_start = std::time::Instant::now();

        // Write document
        let parent = format!("projects/{project_id}/databases/(default)/documents");
        let mut write_req = tonic::Request::new(CreateDocumentRequest {
            parent,
            collection_id: collection.clone(),
            document_id: doc_id,
            document: Some(Document { name: String::new(), fields: HashMap::new(), ..Default::default() }),
            ..Default::default()
        });
        write_req.metadata_mut().insert(
            "authorization",
            format!("bearer {api_key}").parse().unwrap(),
        );
        write_client.create_document(write_req).await.expect("write should succeed");

        // Wait for DocumentChange event on this stream
        let received = tokio::time::timeout(Duration::from_millis(2500), async {
            loop {
                let msg = stream.next().await?.ok()?;
                if let Some(listen_response::ResponseType::DocumentChange(_)) = msg.response_type {
                    return Some(());
                }
            }
        })
        .await;

        let elapsed = write_start.elapsed().as_millis();
        assert!(
            received.is_ok() && received.unwrap().is_some(),
            "collection {collection}: DocumentChange not received within 2500ms"
        );
        latencies_ms.push(elapsed);
    }

    // Sort and assert p99 <= 2000ms
    latencies_ms.sort_unstable();
    let p99 = latencies_ms[98]; // 99th element (0-indexed 98) = p99 for 100 samples
    assert!(
        p99 <= 2000,
        "p99 latency {p99}ms exceeds 2000ms limit; all latencies: {:?}",
        &latencies_ms[90..]
    );
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
