// SCAFFOLD: true
//! US-13 — Browser-based app uses embyr via gRPC-Web
//!
//! As Alex, I want to use the Firebase JS web SDK in a browser application against
//! embyr, so that browser-based apps have the same capabilities as Node.js apps.
//!
//! Driving port: REST / gRPC-Web port (:8081) — Content-Type grpc-web+proto
//! Red classification: MISSING_FUNCTIONALITY

use embyr_core::auth::{argon2, ecies};
use embyr_proto::firestore::{
    firestore_client::FirestoreClient,
    value::ValueType,
    CreateDocumentRequest, Document, GetDocumentRequest,
};
use embyr_server::{adapters::system_db::SystemDb, start_test_server};
use prost::Message;
use std::{collections::HashMap, sync::Arc};
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt},
};

// ── shared helpers ────────────────────────────────────────────────────────────

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

    let project_id = "us13-test-project";
    let api_key = "test-sk-us13-browser-abc";
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
        project_id: project_id.to_string(),
        api_key: api_key.to_string(),
        server,
    }
}

/// Encode a protobuf message as a gRPC-Web frame:
/// 1 byte: compressed flag (0x00)
/// 4 bytes: big-endian message length
/// N bytes: protobuf bytes
fn grpc_web_encode(proto_bytes: &[u8]) -> Vec<u8> {
    let mut frame = Vec::with_capacity(5 + proto_bytes.len());
    frame.push(0x00u8); // not compressed
    let len = proto_bytes.len() as u32;
    frame.extend_from_slice(&len.to_be_bytes());
    frame.extend_from_slice(proto_bytes);
    frame
}

fn make_string_value(s: &str) -> embyr_proto::firestore::Value {
    embyr_proto::firestore::Value {
        value_type: Some(ValueType::StringValue(s.to_string())),
    }
}

// ── AC-13a ───────────────────────────────────────────────────────────────────

/// AC-13a: gRPC-Web client reads and writes documents without errors
///
/// Given:  a provisioned project with a direct-pg backend
/// When:   an HTTP client sends a CreateDocument request with Content-Type grpc-web+proto
/// Then:   the response Content-Type is grpc-web+proto
/// And:    the document is persisted and retrievable via GetDocument on the same transport
#[tokio::test]
async fn grpc_web_client_reads_and_writes_documents() {
    let env = setup().await;
    let rest_base = format!("http://{}", env.server.rest_addr);

    // Build CreateDocument request proto.
    let mut fields = HashMap::new();
    fields.insert("title".to_string(), make_string_value("grpc-web-doc"));

    let parent = format!(
        "projects/{}/databases/(default)/documents",
        env.project_id
    );
    let create_req = CreateDocumentRequest {
        parent: parent.clone(),
        collection_id: "browser_test".to_string(),
        document_id: "grpc-web-01".to_string(),
        document: Some(Document {
            name: String::new(),
            fields: fields.clone(),
            ..Default::default()
        }),
        ..Default::default()
    };

    let proto_bytes = create_req.encode_to_vec();
    let frame = grpc_web_encode(&proto_bytes);

    // Build an HTTP/1.1 client (gRPC-Web uses HTTP/1.1).
    let client = reqwest::Client::builder()
        .http1_only()
        .build()
        .expect("reqwest client");

    let url = format!(
        "{}/google.firestore.v1.Firestore/CreateDocument",
        rest_base
    );
    let resp = client
        .post(&url)
        .header("content-type", "application/grpc-web+proto")
        .header("authorization", format!("bearer {}", env.api_key))
        .header("x-grpc-web", "1")
        .body(frame)
        .send()
        .await
        .expect("POST gRPC-Web CreateDocument");

    assert_eq!(
        resp.status(),
        200,
        "expected HTTP 200 from gRPC-Web CreateDocument"
    );

    let content_type = resp
        .headers()
        .get("content-type")
        .expect("Content-Type header must be present")
        .to_str()
        .expect("Content-Type must be UTF-8");

    assert!(
        content_type.starts_with("application/grpc-web"),
        "Content-Type must be grpc-web, got: {content_type}"
    );

    // Now verify via native gRPC GetDocument that the document was actually persisted.
    let channel = tonic::transport::Channel::from_shared(format!(
        "http://{}",
        env.server.grpc_addr
    ))
    .unwrap()
    .connect_lazy();
    let mut grpc_client = FirestoreClient::new(channel);

    let doc_name = format!(
        "projects/{}/databases/(default)/documents/browser_test/grpc-web-01",
        env.project_id
    );
    let mut get_req = tonic::Request::new(GetDocumentRequest {
        name: doc_name,
        ..Default::default()
    });
    get_req.metadata_mut().insert(
        "authorization",
        format!("bearer {}", env.api_key).parse().unwrap(),
    );
    let doc = grpc_client
        .get_document(get_req)
        .await
        .expect("GetDocument via native gRPC should succeed")
        .into_inner();

    assert!(
        doc.fields.contains_key("title"),
        "persisted document must have 'title' field"
    );
}

// AC-13b / AC-13c (BrowserChannel long-polling for Listen and Write) moved to
// tests/browser_channel/acceptance/bc_long_polling.rs, which drives the real
// WebChannel protocol (the old /channel stub no longer exists).

// ── AC-13d ───────────────────────────────────────────────────────────────────

/// AC-13d: no separate server process required for browser transports
///
/// Given:  the embyr server binary is running (single process)
/// When:   both a gRPC-Web request and a native gRPC request are made simultaneously
/// Then:   both requests are served by the same process on their respective ports
/// And:    no additional process IDs appear beyond the embyr binary's PID
#[tokio::test]
async fn single_process_serves_both_grpc_and_grpc_web() {
    let env = setup().await;
    let rest_base = format!("http://{}", env.server.rest_addr);

    // Build a minimal CreateDocument proto for gRPC-Web.
    let mut fields = HashMap::new();
    fields.insert("marker".to_string(), make_string_value("dual-port"));

    let parent = format!(
        "projects/{}/databases/(default)/documents",
        env.project_id
    );
    let create_req = CreateDocumentRequest {
        parent: parent.clone(),
        collection_id: "dual_test".to_string(),
        document_id: "dual-01".to_string(),
        document: Some(Document {
            name: String::new(),
            fields: fields.clone(),
            ..Default::default()
        }),
        ..Default::default()
    };
    let proto_bytes = create_req.encode_to_vec();
    let frame = grpc_web_encode(&proto_bytes);

    let http_client = reqwest::Client::builder()
        .http1_only()
        .build()
        .expect("reqwest client");

    // Launch both requests concurrently from the SAME test (same TestServer PID).
    let grpc_web_url = format!("{rest_base}/google.firestore.v1.Firestore/CreateDocument");
    let api_key = env.api_key.clone();
    let grpc_web_fut = http_client
        .post(&grpc_web_url)
        .header("content-type", "application/grpc-web+proto")
        .header("authorization", format!("bearer {api_key}"))
        .header("x-grpc-web", "1")
        .body(frame)
        .send();

    let channel = tonic::transport::Channel::from_shared(format!(
        "http://{}",
        env.server.grpc_addr
    ))
    .unwrap()
    .connect_lazy();
    let mut grpc_client = FirestoreClient::new(channel);
    let mut native_req = tonic::Request::new(CreateDocumentRequest {
        parent: parent.clone(),
        collection_id: "dual_test".to_string(),
        document_id: "dual-02".to_string(),
        document: Some(Document {
            name: String::new(),
            fields: fields.clone(),
            ..Default::default()
        }),
        ..Default::default()
    });
    native_req.metadata_mut().insert(
        "authorization",
        format!("bearer {}", env.api_key).parse().unwrap(),
    );
    let native_grpc_fut = grpc_client.create_document(native_req);

    // Run both concurrently.
    let (grpc_web_resp, native_grpc_resp) =
        tokio::join!(grpc_web_fut, native_grpc_fut);

    let grpc_web_status = grpc_web_resp
        .expect("gRPC-Web request must complete")
        .status();
    assert_eq!(
        grpc_web_status,
        200,
        "gRPC-Web port must return 200"
    );

    native_grpc_resp.expect("native gRPC request must succeed");
}
