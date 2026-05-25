// SCAFFOLD: true
//! Walking Skeleton — Slice 01
//!
//! Purpose: prove end-to-end wiring through the four fundamental layers:
//!   transport (gRPC) → auth (Argon2id) → storage adapter (direct_pg) → encoding (Firestore proto)
//!
//! Strategy: in-process tonic test client against an in-process embyr server
//! bound on ephemeral ports; real Testcontainers Postgres for both system DB
//! and customer DB (see docs/architecture/atdd-infrastructure-policy.md).
//!
//! Red classification: MISSING_FUNCTIONALITY — test body panics before any
//! production code exists; import errors are structurally impossible because
//! this file references no production modules directly.

use embyr_core::auth::{argon2, ecies};
use embyr_proto::firestore::{firestore_client::FirestoreClient, value::ValueType, GetDocumentRequest};
use embyr_server::{adapters::system_db::SystemDb, start_test_server};
use std::sync::Arc;
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

/// Walking skeleton: SDK developer retrieves a document they previously wrote
///
/// Given:  a project is provisioned with a direct-pg backend
/// And:    a document has been written to that project
/// When:   a Firebase-compatible gRPC client calls GetDocument with the correct API key
/// Then:   the client receives the document with identical field values
/// And:    the document name ends with the expected resource path suffix
///
/// Tags: @walking_skeleton @us_01 @us_03 @real_io @driving_port
#[tokio::test]
async fn sdk_developer_retrieves_written_document_via_grpc() {
    // 1. Spin up two independent Postgres containers: system DB + customer DB
    let (_sys_container, sys_url) = start_postgres().await;
    let (_cust_container, cust_url) = start_postgres().await;

    // 2. Run system DB migrations (projects table, etc.)
    let system_db = Arc::new(SystemDb::new(&sys_url).await.unwrap());
    system_db.migrate().await.unwrap();

    // 3. Run customer DB migrations (documents table)
    let cust_pool = sqlx::PgPool::connect(&cust_url).await.unwrap();
    sqlx::migrate!("../../migrations/customer")
        .run(&cust_pool)
        .await
        .unwrap();

    // 4. Compute credentials
    let api_key = "test-sk-walking-skeleton-01";
    let project_id = "acme-test";
    let api_key_hash = argon2::hash_api_key(api_key.as_bytes()).unwrap();
    let pub_key = ecies::derive_public_key(api_key.as_bytes());
    let encrypted_dsn = ecies::encrypt(&pub_key, cust_url.as_bytes()).unwrap();

    // 5. Insert project row into system DB
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

    // 6. Seed a document directly into the customer DB
    let fields_json = serde_json::json!({
        "greeting": {"t": "S", "v": "hello"}
    });
    sqlx::query(
        "INSERT INTO documents \
         (project_id, collection_path, document_id, fields) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(project_id)
    .bind("messages")
    .bind("greeting")
    .bind(&fields_json)
    .execute(&cust_pool)
    .await
    .unwrap();

    // 7. Start in-process gRPC server
    let server = start_test_server(system_db).await;

    // 8. Build tonic channel to the ephemeral server port
    let endpoint = format!("http://{}", server.grpc_addr);
    let channel = tonic::transport::Channel::from_shared(endpoint)
        .unwrap()
        .connect()
        .await
        .unwrap();
    let mut client = FirestoreClient::new(channel);

    // 9. Call GetDocument with the correct API key in authorization metadata
    let doc_name = format!(
        "projects/{project_id}/databases/(default)/documents/messages/greeting"
    );
    let mut request = tonic::Request::new(GetDocumentRequest {
        name: doc_name.clone(),
        ..Default::default()
    });
    request.metadata_mut().insert(
        "authorization",
        format!("bearer {api_key}").parse().unwrap(),
    );

    let response = client
        .get_document(request)
        .await
        .expect("GetDocument should succeed");
    let doc = response.into_inner();

    // 10. Assert: document name ends with expected suffix
    assert!(
        doc.name.ends_with("messages/greeting"),
        "unexpected document name: {}",
        doc.name
    );

    // 11. Assert: greeting field contains StringValue("hello")
    let greeting_val = doc.fields.get("greeting").expect("greeting field missing");
    match &greeting_val.value_type {
        Some(ValueType::StringValue(s)) => {
            assert_eq!(s, "hello", "greeting value mismatch");
        }
        other => panic!("expected StringValue(\"hello\"), got {other:?}"),
    }
}
