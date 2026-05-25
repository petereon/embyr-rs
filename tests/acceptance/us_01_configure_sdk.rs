// SCAFFOLD: true
//! US-01 — Configure SDK to use embyr
//!
//! As Alex (SDK Developer), I want to change only apiKey and host in
//! firebase.initializeApp, so that my existing Firebase app connects to embyr
//! without other code changes.
//!
//! Driving port: gRPC data port (:8080) — health endpoint and first SDK call
//! Red classification: MISSING_FUNCTIONALITY

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

/// Set up two Postgres containers (system DB + customer DB), run migrations,
/// provision a test project with a seeded document, and return a running server.
///
/// Container handles are returned to keep them alive for the test duration.
async fn setup_test_environment() -> (
    ContainerAsync<Postgres>,
    ContainerAsync<Postgres>,
    Arc<SystemDb>,
    String,
    String,
    embyr_server::TestServer,
) {
    let (_sys_container, sys_url) = start_postgres().await;
    let (_cust_container, cust_url) = start_postgres().await;

    // Run system DB migrations
    let system_db = Arc::new(SystemDb::new(&sys_url).await.unwrap());
    system_db.migrate().await.unwrap();

    // Run customer DB migrations
    let cust_pool = sqlx::PgPool::connect(&cust_url).await.unwrap();
    sqlx::migrate!("../../migrations/customer")
        .run(&cust_pool)
        .await
        .unwrap();

    // Compute credentials for the test project
    let project_id = "sdk-test-01";
    let api_key = "test-api-key-us01-abc123";
    let api_key_hash = argon2::hash_api_key(api_key.as_bytes()).unwrap();
    let pub_key = ecies::derive_public_key(api_key.as_bytes());
    let encrypted_dsn = ecies::encrypt(&pub_key, cust_url.as_bytes()).unwrap();

    // Insert project row into system DB
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

    // Seed a document directly into the customer DB
    let fields_json = serde_json::json!({
        "msg": {"t": "S", "v": "hi"}
    });
    sqlx::query(
        "INSERT INTO documents \
         (project_id, collection_path, document_id, fields) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(project_id)
    .bind("items")
    .bind("item1")
    .bind(&fields_json)
    .execute(&cust_pool)
    .await
    .unwrap();

    let server = start_test_server(system_db.clone()).await;

    (
        _sys_container,
        _cust_container,
        system_db,
        project_id.to_string(),
        api_key.to_string(),
        server,
    )
}

/// AC-01a: Firebase-compatible gRPC client connects to embyr after pointing host at it
///
/// Given:  an embyr server is running with a provisioned project
/// When:   the client connects using the project API key and the embyr gRPC host
/// Then:   the first SDK operation (GetDocument) succeeds without errors
#[tokio::test]
async fn grpc_client_connects_after_pointing_host_at_embyr() {
    let (_sc, _cc, _sdb, project_id, api_key, server) = setup_test_environment().await;

    let channel = tonic::transport::Channel::from_shared(
        format!("http://{}", server.grpc_addr),
    )
    .unwrap()
    .connect()
    .await
    .unwrap();
    let mut client = FirestoreClient::new(channel);

    let mut request = tonic::Request::new(GetDocumentRequest {
        name: format!(
            "projects/{project_id}/databases/(default)/documents/items/item1"
        ),
        ..Default::default()
    });
    request
        .metadata_mut()
        .insert("authorization", format!("bearer {api_key}").parse().unwrap());

    let response = client
        .get_document(request)
        .await
        .expect("GetDocument must succeed");
    let doc = response.into_inner();

    assert!(
        doc.name.ends_with("items/item1"),
        "unexpected document name: {}",
        doc.name
    );

    let msg_val = doc.fields.get("msg").expect("msg field missing");
    match &msg_val.value_type {
        Some(ValueType::StringValue(s)) => {
            assert_eq!(s, "hi", "msg value mismatch");
        }
        other => panic!("expected StringValue(\"hi\"), got {other:?}"),
    }
}

/// AC-01b: Incorrect host causes first SDK operation to return transport error, not panic
///
/// Given:  a Firebase-compatible gRPC client configured with a non-existent host
/// When:   the client attempts a GetDocument call
/// Then:   the call returns a connection error (not a panic or unhandled exception)
/// And:    the error is classifiable as a transport-level failure
#[tokio::test]
async fn incorrect_host_causes_transport_error_not_crash() {
    // Port 1 is privileged / unreachable — connection must fail, not panic
    let channel = tonic::transport::Channel::from_shared("http://127.0.0.1:1")
        .unwrap()
        .connect_timeout(std::time::Duration::from_millis(500))
        .connect_lazy();
    let mut client = FirestoreClient::new(channel);

    let request = tonic::Request::new(GetDocumentRequest {
        name: "projects/fake/databases/(default)/documents/col/doc".into(),
        ..Default::default()
    });

    let result = client.get_document(request).await;
    assert!(result.is_err(), "expected transport error, got Ok");

    let status = result.unwrap_err();
    assert!(
        matches!(
            status.code(),
            tonic::Code::Unavailable | tonic::Code::Unknown | tonic::Code::Internal
        ),
        "unexpected status code: {:?}",
        status.code()
    );
}

/// AC-01c: /healthz on REST port returns 200 OK while server is healthy
///
/// Given:  an embyr server is running
/// When:   a client sends GET /healthz to the REST port
/// Then:   the response status is 200 OK
/// And:    the response arrives within 100 milliseconds
#[tokio::test]
async fn healthz_returns_200_while_server_healthy() {
    let (_sc, _cc, _sdb, _pid, _key, server) = setup_test_environment().await;

    let client = reqwest::Client::new();
    let start = std::time::Instant::now();
    let resp = client
        .get(format!("http://{}/healthz", server.rest_addr))
        .timeout(std::time::Duration::from_millis(200))
        .send()
        .await
        .expect("healthz request must succeed");
    let elapsed = start.elapsed();

    assert_eq!(resp.status(), 200, "expected 200 OK");
    assert!(
        elapsed.as_millis() < 100,
        "healthz must respond within 100ms, took {}ms",
        elapsed.as_millis()
    );
}
