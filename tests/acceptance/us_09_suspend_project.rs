// SCAFFOLD: false
//! US-09 — Suspend a non-paying project
//!
//! As Sam, I want to POST /admin/v1/projects/{id}/suspend and have all SDK requests
//! immediately rejected, so that I can enforce SLAs without deleting customer data.
//!
//! Driving port: Admin HTTP port (:9090) for suspend/activate/delete;
//!               gRPC data port (:8080) for observing enforcement
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
    _cust: ContainerAsync<Postgres>,
    server: embyr_server::TestServer,
    project_id: String,
    api_key: String,
    admin_key: String,
    cust_url: String,
}

async fn setup_with_project(project_id: &str) -> TestEnv {
    let (_sys, sys_url) = start_postgres().await;
    let (_cust, cust_url) = start_postgres().await;
    let db = Arc::new(SystemDb::new(&sys_url).await.unwrap());
    db.migrate().await.unwrap();
    let server =
        start_test_server_with_keepalive(Arc::clone(&db), std::time::Duration::from_secs(30))
            .await;
    let admin_key = "test-admin-key-secret".to_string();

    // Provision project
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{}/admin/v1/projects", server.admin_addr))
        .header("Authorization", format!("Bearer {}", admin_key))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "project_id": project_id,
            "dsn": cust_url,
            "backend_mode": "direct_pg"
        }))
        .send()
        .await
        .expect("provision request failed");
    assert_eq!(resp.status(), 201, "setup: provision must return 201");
    let body: serde_json::Value = resp.json().await.unwrap();
    let api_key = body["api_key"].as_str().unwrap().to_string();

    TestEnv {
        _sys,
        _cust,
        server,
        project_id: project_id.to_string(),
        api_key,
        admin_key,
        cust_url,
    }
}

/// Make a GetDocument gRPC call. Returns the tonic Status code.
async fn grpc_get_document(env: &TestEnv) -> Result<(), tonic::Status> {
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
            env.project_id
        ),
        ..Default::default()
    });
    request.metadata_mut().insert(
        "authorization",
        MetadataValue::try_from(format!("Bearer {}", env.api_key)).unwrap(),
    );

    client.get_document(request).await.map(|_| ())
}

/// AC-09a: suspension causes SDK requests to return permission-denied within 1 second
///
/// Given:  a provisioned active project that SDK calls currently succeed for
/// When:   POST /admin/v1/projects/{id}/suspend is called with admin credentials
/// Then:   the suspend call returns 200 OK
/// And:    the next GetDocument SDK call returns PERMISSION_DENIED
/// And:    the time from suspend response to first rejected SDK call is <= 1 second
#[tokio::test]
async fn suspend_causes_sdk_permission_denied_within_one_second() {
    let env = setup_with_project("suspend-test-09a").await;
    let client = reqwest::Client::new();

    // Verify SDK call succeeds before suspend (not_found is fine — auth succeeded)
    let result = grpc_get_document(&env).await;
    match &result {
        Err(s) if s.code() == tonic::Code::PermissionDenied => {
            panic!("SDK call must not be PERMISSION_DENIED before suspend, got: {s:?}");
        }
        _ => {} // not_found or ok both indicate auth worked
    }

    // Record time before suspend
    let time_before = std::time::Instant::now();

    // Suspend the project
    let resp = client
        .post(format!(
            "http://{}/admin/v1/projects/{}/suspend",
            env.server.admin_addr, env.project_id
        ))
        .header("Authorization", format!("Bearer {}", env.admin_key))
        .send()
        .await
        .expect("suspend request failed");
    assert_eq!(resp.status(), 200, "suspend must return 200 OK");

    // Make a gRPC SDK call — must be PERMISSION_DENIED
    let result = grpc_get_document(&env).await;
    let elapsed = time_before.elapsed();

    assert!(
        elapsed.as_secs() < 1,
        "suspension must take effect within 1 second; elapsed={elapsed:?}"
    );

    match result {
        Err(s) if s.code() == tonic::Code::PermissionDenied => {
            // Expected
        }
        other => panic!(
            "expected PERMISSION_DENIED after suspend, got: {other:?}"
        ),
    }
}

/// AC-09b: activate restores SDK access
///
/// Given:  a suspended project
/// When:   POST /admin/v1/projects/{id}/activate is called
/// Then:   the activate call returns 200 OK
/// And:    subsequent SDK calls for that project succeed again
#[tokio::test]
async fn activate_restores_sdk_access() {
    let env = setup_with_project("activate-test-09b").await;
    let client = reqwest::Client::new();

    // Suspend
    let resp = client
        .post(format!(
            "http://{}/admin/v1/projects/{}/suspend",
            env.server.admin_addr, env.project_id
        ))
        .header("Authorization", format!("Bearer {}", env.admin_key))
        .send()
        .await
        .expect("suspend request failed");
    assert_eq!(resp.status(), 200, "suspend must return 200 OK");

    // Verify SDK call is PERMISSION_DENIED
    let result = grpc_get_document(&env).await;
    match result {
        Err(s) if s.code() == tonic::Code::PermissionDenied => {}
        other => panic!("expected PERMISSION_DENIED after suspend, got: {other:?}"),
    }

    // Activate
    let resp = client
        .post(format!(
            "http://{}/admin/v1/projects/{}/activate",
            env.server.admin_addr, env.project_id
        ))
        .header("Authorization", format!("Bearer {}", env.admin_key))
        .send()
        .await
        .expect("activate request failed");
    assert_eq!(resp.status(), 200, "activate must return 200 OK");

    // Make gRPC SDK call — must succeed (not_found or ok are both auth-success)
    let result = grpc_get_document(&env).await;
    match result {
        Err(s) if s.code() == tonic::Code::PermissionDenied => {
            panic!("SDK call must not be PERMISSION_DENIED after activate, got: {s:?}");
        }
        _ => {} // not_found or ok indicate auth succeeded
    }
}

/// AC-09c (edge case): suspending an already-suspended project is idempotent (200)
///
/// Given:  a project is already suspended
/// When:   POST /admin/v1/projects/{id}/suspend is called again
/// Then:   the response status is 200 OK (idempotent, not 409)
#[tokio::test]
async fn suspend_already_suspended_project_is_idempotent() {
    let env = setup_with_project("idempotent-test-09c").await;
    let client = reqwest::Client::new();

    // First suspend
    let resp = client
        .post(format!(
            "http://{}/admin/v1/projects/{}/suspend",
            env.server.admin_addr, env.project_id
        ))
        .header("Authorization", format!("Bearer {}", env.admin_key))
        .send()
        .await
        .expect("first suspend request failed");
    assert_eq!(resp.status(), 200, "first suspend must return 200 OK");

    // Second suspend — must be idempotent
    let resp = client
        .post(format!(
            "http://{}/admin/v1/projects/{}/suspend",
            env.server.admin_addr, env.project_id
        ))
        .header("Authorization", format!("Bearer {}", env.admin_key))
        .send()
        .await
        .expect("second suspend request failed");
    assert_eq!(
        resp.status(),
        200,
        "second suspend must return 200 (idempotent)"
    );
}

/// AC-09d: DELETE returns 200; GET returns 404 immediately; data purged after retention window
///
/// Given:  a provisioned project
/// When:   DELETE /admin/v1/projects/{id} is called
/// Then:   the response status is 200 OK
/// And:    GET /admin/v1/projects/{id} returns 404 immediately
/// And:    the project record status is "deleted" in the system DB
#[tokio::test]
async fn delete_project_returns_200_and_get_returns_404() {
    let env = setup_with_project("delete-test-09d").await;
    let client = reqwest::Client::new();

    // Delete the project
    let resp = client
        .delete(format!(
            "http://{}/admin/v1/projects/{}",
            env.server.admin_addr, env.project_id
        ))
        .header("Authorization", format!("Bearer {}", env.admin_key))
        .send()
        .await
        .expect("delete request failed");
    assert_eq!(resp.status(), 200, "delete must return 200 OK");

    // GET must return 404 immediately
    let resp = client
        .get(format!(
            "http://{}/admin/v1/projects/{}",
            env.server.admin_addr, env.project_id
        ))
        .header("Authorization", format!("Bearer {}", env.admin_key))
        .send()
        .await
        .expect("GET request failed");
    assert_eq!(
        resp.status(),
        404,
        "GET on deleted project must return 404"
    );
}

/// Error path: SDK calls for a deleted project return NOT_FOUND
///
/// Given:  a project has been soft-deleted
/// When:   a GetDocument SDK call is attempted for that project
/// Then:   the call returns status NOT_FOUND
#[tokio::test]
async fn sdk_call_for_deleted_project_returns_not_found() {
    let env = setup_with_project("sdk-deleted-09e").await;
    let client = reqwest::Client::new();

    // Verify SDK call succeeds before deletion (not_found is fine — auth worked)
    let result = grpc_get_document(&env).await;
    match &result {
        Err(s) if s.code() == tonic::Code::PermissionDenied || s.code() == tonic::Code::NotFound && s.message().contains("project not found") => {
            panic!("unexpected status before delete: {result:?}");
        }
        _ => {}
    }

    // Delete the project
    let resp = client
        .delete(format!(
            "http://{}/admin/v1/projects/{}",
            env.server.admin_addr, env.project_id
        ))
        .header("Authorization", format!("Bearer {}", env.admin_key))
        .send()
        .await
        .expect("delete request failed");
    assert_eq!(resp.status(), 200, "delete must return 200 OK");

    // Make gRPC SDK call — must be NOT_FOUND
    let result = grpc_get_document(&env).await;
    match result {
        Err(s) if s.code() == tonic::Code::NotFound => {
            // Expected
        }
        other => panic!(
            "expected NOT_FOUND after delete, got: {other:?}"
        ),
    }
}
