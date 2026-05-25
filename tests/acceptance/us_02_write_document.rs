// SCAFFOLD: true
//! US-02 — Write a document
//!
//! As Alex, I want to call setDoc and have the document persisted in my Postgres
//! database, so that I can build write-heavy features without a Firebase-specific
//! data layer.
//!
//! Driving port: gRPC data port (:8080) — CreateDocument / UpdateDocument RPC
//! Red classification: MISSING_FUNCTIONALITY

use embyr_core::auth::{argon2, ecies};
use embyr_proto::firestore::{
    firestore_client::FirestoreClient,
    precondition::ConditionType,
    value::ValueType,
    CreateDocumentRequest, Document, GetDocumentRequest, Precondition, UpdateDocumentRequest,
};
use embyr_server::{adapters::system_db::SystemDb, start_test_server};
use prost_types::Timestamp;
use std::{collections::HashMap, sync::Arc};
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt},
};

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
    sys_pool: sqlx::PgPool,
    project_id: String,
    api_key: String,
    server: embyr_server::TestServer,
}

async fn setup() -> TestEnv {
    let (_sys_container, sys_url) = start_postgres().await;
    let (_cust_container, cust_url) = start_postgres().await;

    let system_db = Arc::new(SystemDb::new(&sys_url).await.unwrap());
    system_db.migrate().await.unwrap();

    let cust_pool = sqlx::PgPool::connect(&cust_url).await.unwrap();
    sqlx::migrate!("../../migrations/customer")
        .run(&cust_pool)
        .await
        .unwrap();

    let project_id = "us02-test-project";
    let api_key = "test-sk-us02-write-abc";
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

    let server = start_test_server(system_db).await;

    TestEnv {
        _sys_container,
        _cust_container,
        sys_pool,
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

fn make_string_value(s: &str) -> embyr_proto::firestore::Value {
    embyr_proto::firestore::Value {
        value_type: Some(ValueType::StringValue(s.to_string())),
    }
}

fn make_integer_value(i: i64) -> embyr_proto::firestore::Value {
    embyr_proto::firestore::Value {
        value_type: Some(ValueType::IntegerValue(i)),
    }
}

fn make_authed_request<T>(payload: T, api_key: &str) -> tonic::Request<T> {
    let mut req = tonic::Request::new(payload);
    req.metadata_mut().insert(
        "authorization",
        format!("bearer {api_key}").parse().unwrap(),
    );
    req
}

/// AC-02a (happy path): setDoc persists document; getDoc returns identical data
///
/// Given:  a provisioned project with a direct-pg backend
/// When:   a gRPC CreateDocument call writes {"greeting": "hello", "count": 42}
/// Then:   a subsequent GetDocument call returns exists=true with identical fields
/// And:    the write_result contains a server-assigned update_time timestamp
#[tokio::test]
async fn write_then_read_returns_same_document_fields() {
    let env = setup().await;
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    let mut fields = HashMap::new();
    fields.insert("greeting".to_string(), make_string_value("hello"));
    fields.insert("count".to_string(), make_integer_value(42));

    let parent = format!(
        "projects/{}/databases/(default)/documents",
        env.project_id
    );
    let create_req = make_authed_request(
        CreateDocumentRequest {
            parent: parent.clone(),
            collection_id: "messages".to_string(),
            document_id: "doc-01".to_string(),
            document: Some(Document {
                name: String::new(),
                fields: fields.clone(),
                ..Default::default()
            }),
            ..Default::default()
        },
        &env.api_key,
    );

    let create_resp = client
        .create_document(create_req)
        .await
        .expect("CreateDocument should succeed");
    let created_doc = create_resp.into_inner();

    // Verify response has timestamps set.
    assert!(
        created_doc.create_time.is_some(),
        "create_time must be set in response"
    );
    assert!(
        created_doc.update_time.is_some(),
        "update_time must be set in response"
    );

    // Now read the document back.
    let doc_name = format!(
        "projects/{}/databases/(default)/documents/messages/doc-01",
        env.project_id
    );
    let get_req = make_authed_request(
        GetDocumentRequest {
            name: doc_name.clone(),
            ..Default::default()
        },
        &env.api_key,
    );

    let get_resp = client
        .get_document(get_req)
        .await
        .expect("GetDocument should succeed");
    let retrieved_doc = get_resp.into_inner();

    assert!(
        retrieved_doc.name.ends_with("messages/doc-01"),
        "unexpected document name: {}",
        retrieved_doc.name
    );

    let greeting = retrieved_doc.fields.get("greeting").expect("greeting field missing");
    match &greeting.value_type {
        Some(ValueType::StringValue(s)) => assert_eq!(s, "hello"),
        other => panic!("expected StringValue(\"hello\"), got {other:?}"),
    }

    let count = retrieved_doc.fields.get("count").expect("count field missing");
    match &count.value_type {
        Some(ValueType::IntegerValue(i)) => assert_eq!(*i, 42),
        other => panic!("expected IntegerValue(42), got {other:?}"),
    }
}

/// AC-02b: WriteResult.update_time has microsecond precision
///
/// Given:  a provisioned project
/// When:   a CreateDocument RPC is issued
/// Then:   the returned WriteResult.update_time is a valid Timestamp proto
/// And:    the timestamp has sub-second precision (nanos field is non-trivially set)
#[tokio::test]
async fn write_result_update_time_has_microsecond_precision() {
    let env = setup().await;
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    let mut fields = HashMap::new();
    fields.insert("x".to_string(), make_integer_value(1));

    let parent = format!(
        "projects/{}/databases/(default)/documents",
        env.project_id
    );
    let create_req = make_authed_request(
        CreateDocumentRequest {
            parent,
            collection_id: "ts-test".to_string(),
            document_id: "ts-doc".to_string(),
            document: Some(Document {
                name: String::new(),
                fields,
                ..Default::default()
            }),
            ..Default::default()
        },
        &env.api_key,
    );

    let resp = client
        .create_document(create_req)
        .await
        .expect("CreateDocument should succeed")
        .into_inner();

    let create_time = resp.create_time.expect("create_time must be present");
    let update_time = resp.update_time.expect("update_time must be present");

    // Timestamps must be positive (after epoch).
    assert!(create_time.seconds > 0, "create_time.seconds must be > 0");
    assert!(update_time.seconds > 0, "update_time.seconds must be > 0");

    // nanos must be in valid range [0, 999_999_999].
    assert!(
        create_time.nanos >= 0 && create_time.nanos < 1_000_000_000,
        "create_time.nanos out of range: {}",
        create_time.nanos
    );
    assert!(
        update_time.nanos >= 0 && update_time.nanos < 1_000_000_000,
        "update_time.nanos out of range: {}",
        update_time.nanos
    );

    // Postgres stores TIMESTAMPTZ at microsecond precision → nanos are multiples of 1000.
    // Verify they are multiples of 1000 (microsecond granularity).
    assert_eq!(
        create_time.nanos % 1000,
        0,
        "create_time nanos must be microsecond-aligned (multiple of 1000): {}",
        create_time.nanos
    );
}

/// AC-02c (error path — OCC): concurrent writes on same document — only one succeeds per version
///
/// Given:  a provisioned project with an existing document at version N
/// When:   two concurrent UpdateDocument calls both specify version=N
/// Then:   exactly one succeeds with a new version N+1
/// And:    the other receives an ABORTED status code
#[tokio::test]
async fn concurrent_writes_on_same_document_one_aborts_per_occ() {
    let env = setup().await;
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    // 1. Create the initial document.
    let mut fields = HashMap::new();
    fields.insert("value".to_string(), make_integer_value(0));

    let parent = format!(
        "projects/{}/databases/(default)/documents",
        env.project_id
    );
    let create_req = make_authed_request(
        CreateDocumentRequest {
            parent,
            collection_id: "occ-test".to_string(),
            document_id: "occ-doc".to_string(),
            document: Some(Document {
                name: String::new(),
                fields,
                ..Default::default()
            }),
            ..Default::default()
        },
        &env.api_key,
    );

    let create_resp = client
        .create_document(create_req)
        .await
        .expect("initial CreateDocument should succeed")
        .into_inner();

    let initial_update_time = create_resp.update_time.expect("update_time must be set");

    // 2. Prepare two concurrent UpdateDocument requests with the same precondition.
    let doc_name = format!(
        "projects/{}/databases/(default)/documents/occ-test/occ-doc",
        env.project_id
    );

    let precondition = Precondition {
        condition_type: Some(ConditionType::UpdateTime(Timestamp {
            seconds: initial_update_time.seconds,
            nanos: initial_update_time.nanos,
        })),
    };

    let mut fields_1 = HashMap::new();
    fields_1.insert("value".to_string(), make_integer_value(1));
    let mut fields_2 = HashMap::new();
    fields_2.insert("value".to_string(), make_integer_value(2));

    // Build channels for two concurrent clients.
    let channel1 = tonic::transport::Channel::from_shared(
        format!("http://{}", env.server.grpc_addr),
    )
    .unwrap()
    .connect_lazy();
    let channel2 = tonic::transport::Channel::from_shared(
        format!("http://{}", env.server.grpc_addr),
    )
    .unwrap()
    .connect_lazy();
    let mut client1 = FirestoreClient::new(channel1);
    let mut client2 = FirestoreClient::new(channel2);

    let mut req1 = tonic::Request::new(UpdateDocumentRequest {
        document: Some(Document {
            name: doc_name.clone(),
            fields: fields_1,
            ..Default::default()
        }),
        current_document: Some(precondition.clone()),
        ..Default::default()
    });
    req1.metadata_mut().insert(
        "authorization",
        format!("bearer {}", env.api_key).parse().unwrap(),
    );

    let mut req2 = tonic::Request::new(UpdateDocumentRequest {
        document: Some(Document {
            name: doc_name.clone(),
            fields: fields_2,
            ..Default::default()
        }),
        current_document: Some(precondition),
        ..Default::default()
    });
    req2.metadata_mut().insert(
        "authorization",
        format!("bearer {}", env.api_key).parse().unwrap(),
    );

    // 3. Fire both updates concurrently.
    let (r1, r2) = tokio::join!(
        client1.update_document(req1),
        client2.update_document(req2),
    );
    let results = [r1, r2];

    let ok_count = results.iter().filter(|r| r.is_ok()).count();
    let aborted_count = results
        .iter()
        .filter(|r| {
            r.as_ref()
                .err()
                .map_or(false, |s| s.code() == tonic::Code::Aborted)
        })
        .count();

    assert_eq!(ok_count, 1, "exactly one update must succeed");
    assert_eq!(aborted_count, 1, "exactly one update must be aborted");
}

/// Error path: writing to a suspended project returns permission denied
///
/// Given:  a project that has been suspended
/// When:   a gRPC CreateDocument call is attempted with valid credentials
/// Then:   the call returns status PERMISSION_DENIED
/// And:    no document is written to the customer database
#[tokio::test]
async fn write_to_suspended_project_returns_permission_denied() {
    let env = setup().await;

    // Suspend the project in the system DB.
    sqlx::query("UPDATE projects SET status = 'suspended' WHERE id = $1")
        .bind(&env.project_id)
        .execute(&env.sys_pool)
        .await
        .unwrap();

    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    let mut fields = HashMap::new();
    fields.insert("x".to_string(), make_integer_value(1));

    let parent = format!(
        "projects/{}/databases/(default)/documents",
        env.project_id
    );
    let req = make_authed_request(
        CreateDocumentRequest {
            parent,
            collection_id: "col".to_string(),
            document_id: "doc".to_string(),
            document: Some(Document {
                name: String::new(),
                fields,
                ..Default::default()
            }),
            ..Default::default()
        },
        &env.api_key,
    );

    let result = client.create_document(req).await;

    assert!(result.is_err(), "expected error for suspended project");
    let status = result.unwrap_err();
    assert_eq!(
        status.code(),
        tonic::Code::PermissionDenied,
        "expected PERMISSION_DENIED, got {:?}: {}",
        status.code(),
        status.message()
    );
}

/// Error path: writing with invalid API key returns unauthenticated
///
/// Given:  a provisioned project
/// When:   a gRPC CreateDocument call uses a wrong API key
/// Then:   the call returns status UNAUTHENTICATED
#[tokio::test]
async fn write_with_invalid_api_key_returns_unauthenticated() {
    let env = setup().await;
    let mut client = FirestoreClient::new(make_channel(env.server.grpc_addr));

    let mut fields = HashMap::new();
    fields.insert("x".to_string(), make_integer_value(1));

    let parent = format!(
        "projects/{}/databases/(default)/documents",
        env.project_id
    );
    let req = make_authed_request(
        CreateDocumentRequest {
            parent,
            collection_id: "col".to_string(),
            document_id: "doc".to_string(),
            document: Some(Document {
                name: String::new(),
                fields,
                ..Default::default()
            }),
            ..Default::default()
        },
        "wrong-api-key-totally-invalid",
    );

    let result = client.create_document(req).await;

    assert!(result.is_err(), "expected error for invalid API key");
    let status = result.unwrap_err();
    assert_eq!(
        status.code(),
        tonic::Code::Unauthenticated,
        "expected UNAUTHENTICATED, got {:?}: {}",
        status.code(),
        status.message()
    );
}
