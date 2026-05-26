// SCAFFOLD: false
//! US-08 — Verify and monitor project
//!
//! As Sam, I want to GET /admin/v1/projects/{id} and query daily_project_metrics,
//! so that I can verify project health and produce usage reports.
//!
//! Driving port: Admin HTTP port (:9090) — GET /admin/v1/projects/{id}
//! Red classification: MISSING_FUNCTIONALITY

use embyr_server::{adapters::system_db::SystemDb, start_test_server_with_keepalive};
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

struct TestEnv {
    _sys: ContainerAsync<Postgres>,
    server: embyr_server::TestServer,
    admin_key: String,
    sys_pool: sqlx::PgPool,
}

async fn setup() -> TestEnv {
    let (_sys, sys_url) = start_postgres().await;
    let db = Arc::new(SystemDb::new(&sys_url).await.unwrap());
    db.migrate().await.unwrap();
    let sys_pool = sqlx::PgPool::connect(&sys_url).await.unwrap();
    let server =
        start_test_server_with_keepalive(db, std::time::Duration::from_secs(30)).await;
    TestEnv {
        _sys,
        server,
        admin_key: "test-admin-key-secret".into(),
        sys_pool,
    }
}

/// Provision a project with a customer DB. Returns the api_key.
async fn provision_project(
    env: &TestEnv,
    project_id: &str,
    cust_url: &str,
) -> String {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{}/admin/v1/projects", env.server.admin_addr))
        .header("Authorization", format!("Bearer {}", env.admin_key))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "project_id": project_id,
            "dsn": cust_url,
            "backend_mode": "direct_pg"
        }))
        .send()
        .await
        .expect("provision request failed");
    assert_eq!(resp.status(), 201, "provision must return 201");
    let body: serde_json::Value = resp.json().await.unwrap();
    body["api_key"].as_str().unwrap().to_string()
}

/// AC-08a: GET /admin/v1/projects/{id} returns status, backend_mode, auth_mode (no raw credentials)
///
/// Given:  a provisioned project "my-project" with backend_mode=direct_pg
/// When:   GET /admin/v1/projects/my-project is called with admin Bearer token
/// Then:   the response status is 200 OK
/// And:    the body contains status="active", backend_mode="direct_pg", auth_mode="key"
/// And:    the body does NOT contain any DSN, plaintext key, or encrypted bytes
#[tokio::test]
async fn get_project_returns_status_and_mode_without_credentials() {
    let (_cust, cust_url) = start_postgres().await;
    let env = setup().await;

    provision_project(&env, "monitor-test", &cust_url).await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!(
            "http://{}/admin/v1/projects/monitor-test",
            env.server.admin_addr
        ))
        .header("Authorization", format!("Bearer {}", env.admin_key))
        .send()
        .await
        .expect("GET request failed");

    assert_eq!(resp.status(), 200, "expected 200 OK");

    let body: serde_json::Value = resp.json().await.expect("body must be JSON");
    assert_eq!(body["status"], "active", "status must be active");
    assert_eq!(body["backend_mode"], "direct_pg", "backend_mode must be direct_pg");
    assert_eq!(body["auth_mode"], "key", "auth_mode must be key");

    // Must NOT contain raw credential fields
    let body_str = body.to_string();
    assert!(
        !body_str.contains("dsn"),
        "response must not contain 'dsn': {body_str}"
    );
    assert!(
        !body_str.contains("encrypted"),
        "response must not contain 'encrypted': {body_str}"
    );
    assert!(
        !body_str.contains("hash"),
        "response must not contain 'hash': {body_str}"
    );
}

/// AC-08b: daily_project_metrics has a row for the current day after one SDK request
///
/// Given:  a provisioned project
/// And:    at least one GetDocument SDK call has been made for that project
/// When:   the daily_project_metrics table is queried for today
/// Then:   a row exists for (project_id, today)
/// And:    read_ops > 0 (traffic was recorded)
#[tokio::test]
async fn metrics_row_exists_after_sdk_request() {
    let (_cust, cust_url) = start_postgres().await;
    let env = setup().await;

    let project_id = "metrics-test";
    let api_key = provision_project(&env, project_id, &cust_url).await;

    // Make a gRPC SDK call — GetDocument (will return not-found, but auth + metrics still fires)
    use embyr_proto::firestore::firestore_client::FirestoreClient;
    use embyr_proto::firestore::GetDocumentRequest;
    use tonic::metadata::MetadataValue;

    let grpc_url = format!("http://{}", env.server.grpc_addr);
    let channel = tonic::transport::Channel::from_shared(grpc_url)
        .unwrap()
        .connect()
        .await
        .expect("gRPC connect failed");

    let mut client = FirestoreClient::new(channel);
    let mut request = tonic::Request::new(GetDocumentRequest {
        name: format!(
            "projects/{}/databases/(default)/documents/col/doc-1",
            project_id
        ),
        ..Default::default()
    });
    request.metadata_mut().insert(
        "authorization",
        MetadataValue::try_from(format!("Bearer {}", api_key)).unwrap(),
    );

    // Ignore the result — not_found is fine; we only care that auth succeeded
    let _ = client.get_document(request).await;

    // Query daily_project_metrics from system DB
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT read_ops FROM daily_project_metrics WHERE project_id=$1 AND date=CURRENT_DATE",
    )
    .bind(project_id)
    .fetch_optional(&env.sys_pool)
    .await
    .expect("metrics query failed");

    assert!(
        row.is_some(),
        "expected a metrics row for project {project_id} on today's date"
    );
    let (read_ops,) = row.unwrap();
    assert!(
        read_ops > 0,
        "read_ops must be > 0 after a GetDocument call, got {read_ops}"
    );
}

/// AC-08c (error path): GET on a non-existent project returns 404
///
/// Given:  no project with ID "does-not-exist" has been provisioned
/// When:   GET /admin/v1/projects/does-not-exist is called
/// Then:   the response status is 404 Not Found
#[tokio::test]
async fn get_nonexistent_project_returns_404() {
    let env = setup().await;

    let client = reqwest::Client::new();
    let resp = client
        .get(format!(
            "http://{}/admin/v1/projects/does-not-exist",
            env.server.admin_addr
        ))
        .header("Authorization", format!("Bearer {}", env.admin_key))
        .send()
        .await
        .expect("GET request failed");

    assert_eq!(resp.status(), 404, "expected 404 for non-existent project");
}

/// AC-08c (error path): GET on a deleted project returns 404
///
/// Given:  a project has been soft-deleted (status='deleted')
/// When:   GET /admin/v1/projects/{id} is called for the deleted project
/// Then:   the response status is 404 Not Found
#[tokio::test]
async fn get_deleted_project_returns_404() {
    let (_cust, cust_url) = start_postgres().await;
    let env = setup().await;

    provision_project(&env, "deleted-project", &cust_url).await;

    // Soft-delete by setting status='deleted'
    sqlx::query("UPDATE projects SET status='deleted', updated_at=now() WHERE id=$1")
        .bind("deleted-project")
        .execute(&env.sys_pool)
        .await
        .expect("soft-delete failed");

    let client = reqwest::Client::new();
    let resp = client
        .get(format!(
            "http://{}/admin/v1/projects/deleted-project",
            env.server.admin_addr
        ))
        .header("Authorization", format!("Bearer {}", env.admin_key))
        .send()
        .await
        .expect("GET request failed");

    assert_eq!(resp.status(), 404, "expected 404 for deleted project");
}
