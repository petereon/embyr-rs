// SCAFFOLD: false
//! US-11 — Provision project with GCP Secret Manager
//!
//! As Morgan, I want to register a project with backend_mode=gcp_secret and a
//! GCP secret resource name, so that embyr fetches the DSN via workload identity
//! without storing it.
//!
//! Driving port: Admin HTTP port (:9090) — POST /admin/v1/projects
//! Red classification: MISSING_FUNCTIONALITY

use base64::Engine as _;
use embyr_server::{
    adapters::{gcp_secret_fetcher::GcpSecretFetcher, system_db::SystemDb},
    start_test_server_with_gcp_fetcher,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner as ModulesAsyncRunner, ContainerAsync as ModulesContainerAsync, ImageExt},
};

async fn start_postgres() -> (ModulesContainerAsync<Postgres>, String) {
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

// ── In-process GCP Secret Manager mock ─────────────────────────────────────

#[derive(Clone)]
struct MockGcpState {
    /// resource_name → DSN value
    secrets: Arc<Mutex<HashMap<String, String>>>,
    /// resource_names that should return 403
    deny_set: Arc<Mutex<std::collections::HashSet<String>>>,
    /// call log of resource_name paths that were accessed
    calls: Arc<Mutex<Vec<String>>>,
}

impl MockGcpState {
    fn new() -> Self {
        Self {
            secrets: Arc::new(Mutex::new(HashMap::new())),
            deny_set: Arc::new(Mutex::new(std::collections::HashSet::new())),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn add_secret(&self, resource_name: &str, dsn: &str) {
        self.secrets
            .lock()
            .unwrap()
            .insert(resource_name.to_string(), dsn.to_string());
    }

    fn deny(&self, resource_name: &str) {
        self.deny_set
            .lock()
            .unwrap()
            .insert(resource_name.to_string());
    }

    fn call_log_snapshot(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

async fn access_secret_version(
    axum::extract::State(state): axum::extract::State<MockGcpState>,
    axum::extract::Path(path): axum::extract::Path<String>,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    // path is like "projects/my-proj/secrets/my-secret/versions/latest:access"
    // Strip the trailing "/versions/latest:access" to get the resource_name
    let resource_name = path
        .trim_end_matches("/versions/latest:access")
        .to_string();

    state
        .calls
        .lock()
        .unwrap()
        .push(resource_name.clone());

    let deny_set = state.deny_set.lock().unwrap();
    if deny_set.contains(&resource_name) {
        return (
            axum::http::StatusCode::FORBIDDEN,
            axum::Json(serde_json::json!({"error": "access denied"})),
        )
            .into_response();
    }
    drop(deny_set);

    let secrets = state.secrets.lock().unwrap();
    match secrets.get(&resource_name) {
        Some(dsn) => {
            let json_dsn = serde_json::json!({"dsn": dsn}).to_string();
            let encoded =
                base64::engine::general_purpose::STANDARD.encode(json_dsn.as_bytes());
            (
                axum::http::StatusCode::OK,
                axum::Json(serde_json::json!({"payload": {"data": encoded}})),
            )
                .into_response()
        }
        None => (
            axum::http::StatusCode::NOT_FOUND,
            axum::Json(serde_json::json!({"error": "not found"})),
        )
            .into_response(),
    }
}

/// Start an in-process mock GCP Secret Manager server.
/// Returns (state, base_url).
async fn start_mock_gcp_sm() -> (MockGcpState, String) {
    let state = MockGcpState::new();
    let app = axum::Router::new()
        .route(
            "/v1/*path",
            axum::routing::get(access_secret_version),
        )
        .with_state(state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let base_url = format!("http://127.0.0.1:{port}");
    (state, base_url)
}

// ── AC-11a ──────────────────────────────────────────────────────────────────

/// AC-11a (symmetric to AC-10a): POST with gcp_secret returns 201; system DB has no DSN
///
/// Given:  a GCP Secret Manager emulator has a secret with DSN at a test resource name
/// And:    embyr has workload identity credentials that can access the emulator
/// When:   POST /admin/v1/projects with backend_mode=gcp_secret and the test resource name
/// Then:   the response status is 201 Created
/// And:    the system DB project record has no plaintext DSN
/// And:    the project backend_secret_gcp column equals the provided resource name
///
/// Tags: @real_io @adapter_integration
#[tokio::test]
async fn provision_with_gcp_secret_stores_resource_name_not_dsn() {
    let (mock_state, base_url) = start_mock_gcp_sm().await;
    let (_cust, cust_url) = start_postgres().await;
    let (_sys, sys_url) = start_postgres().await;

    let resource_name = "projects/my-gcp-proj/secrets/my-dsn-secret";
    mock_state.add_secret(resource_name, &cust_url);

    let db = Arc::new(SystemDb::new(&sys_url).await.unwrap());
    db.migrate().await.unwrap();

    let fetcher = Arc::new(GcpSecretFetcher::new(&base_url, "test-token", 300));

    let server =
        start_test_server_with_gcp_fetcher(db.clone(), std::time::Duration::from_secs(30), fetcher)
            .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{}/admin/v1/projects", server.admin_addr))
        .header("Authorization", "Bearer test-admin-key-secret")
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "project_id": "gcp-secret-project-11a",
            "backend_mode": "gcp_secret",
            "gcp_resource_name": resource_name
        }))
        .send()
        .await
        .expect("HTTP request failed");

    assert_eq!(
        resp.status(),
        201,
        "expected 201 Created, got: {}",
        resp.status()
    );

    let body: serde_json::Value = resp.json().await.expect("body must be JSON");
    assert_eq!(body["project_id"], "gcp-secret-project-11a");
    assert!(body["api_key"].is_string(), "api_key must be present: {body}");

    // Verify system DB: backend_secret_gcp = resource_name, ecies_encrypted_dsn IS NULL
    let row = sqlx::query(
        "SELECT backend_secret_gcp, ecies_encrypted_dsn FROM projects WHERE id = $1",
    )
    .bind("gcp-secret-project-11a")
    .fetch_one(db.pool())
    .await
    .expect("project row must exist");

    use sqlx::Row;
    let stored_gcp: Option<String> =
        row.try_get("backend_secret_gcp").expect("backend_secret_gcp");
    let stored_dsn: Option<Vec<u8>> =
        row.try_get("ecies_encrypted_dsn").expect("ecies_encrypted_dsn");

    assert_eq!(
        stored_gcp.as_deref(),
        Some(resource_name),
        "backend_secret_gcp must equal the resource name"
    );
    assert!(
        stored_dsn.is_none(),
        "ecies_encrypted_dsn must be NULL for gcp_secret projects"
    );
}

// ── AC-11b ──────────────────────────────────────────────────────────────────

/// AC-11b: no plaintext DSN in embyr system DB for gcp_secret project
///
/// Given:  a project provisioned with gcp_secret backend
/// When:   the system DB projects row is inspected for this project
/// Then:   the backend_pg_creds_enc column is NULL
/// And:    the backend_secret_gcp column contains the GCP resource name
#[tokio::test]
async fn gcp_secret_project_has_no_dsn_in_system_db() {
    let (mock_state, base_url) = start_mock_gcp_sm().await;
    let (_cust, cust_url) = start_postgres().await;
    let (_sys, sys_url) = start_postgres().await;

    let resource_name = "projects/my-gcp-proj/secrets/dsn-11b";
    mock_state.add_secret(resource_name, &cust_url);

    let db = Arc::new(SystemDb::new(&sys_url).await.unwrap());
    db.migrate().await.unwrap();

    let fetcher = Arc::new(GcpSecretFetcher::new(&base_url, "test-token", 300));

    let server =
        start_test_server_with_gcp_fetcher(db.clone(), std::time::Duration::from_secs(30), fetcher)
            .await;

    let client = reqwest::Client::new();
    client
        .post(format!("http://{}/admin/v1/projects", server.admin_addr))
        .header("Authorization", "Bearer test-admin-key-secret")
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "project_id": "gcp-secret-project-11b",
            "backend_mode": "gcp_secret",
            "gcp_resource_name": resource_name
        }))
        .send()
        .await
        .expect("HTTP request failed");

    // Primary assertion: system DB has no encrypted DSN for this project
    let row = sqlx::query(
        "SELECT backend_secret_gcp, ecies_encrypted_dsn FROM projects WHERE id = $1",
    )
    .bind("gcp-secret-project-11b")
    .fetch_one(db.pool())
    .await
    .expect("project row must exist");

    use sqlx::Row;
    let stored_dsn: Option<Vec<u8>> =
        row.try_get("ecies_encrypted_dsn").expect("ecies_encrypted_dsn");
    let stored_gcp: Option<String> =
        row.try_get("backend_secret_gcp").expect("backend_secret_gcp");

    assert!(
        stored_dsn.is_none(),
        "backend_pg_creds_enc (ecies_encrypted_dsn) must be NULL: found {stored_dsn:?}"
    );
    assert_eq!(
        stored_gcp.as_deref(),
        Some(resource_name),
        "backend_secret_gcp must contain the resource name"
    );
}

// ── AC-11c ──────────────────────────────────────────────────────────────────

/// AC-11c: GCP audit logs record embyr AccessSecretVersion calls
///
/// Given:  a project provisioned with gcp_secret backend
/// When:   an SDK call triggers a credential cache miss (first call after TTL expiry)
/// Then:   an AccessSecretVersion call appears in GCP Cloud Audit Logs for the secret
///
/// Note: this is verified by contract smoke — the emulator records call logs
/// Tags: @real_io @kpi
#[tokio::test]
async fn gcp_access_secret_version_is_audited() {
    let (mock_state, base_url) = start_mock_gcp_sm().await;
    let (_cust, cust_url) = start_postgres().await;
    let (_sys, sys_url) = start_postgres().await;

    let resource_name = "projects/my-gcp-proj/secrets/dsn-11c";
    mock_state.add_secret(resource_name, &cust_url);

    let db = Arc::new(SystemDb::new(&sys_url).await.unwrap());
    db.migrate().await.unwrap();

    // TTL = 1 second so cache expires quickly
    let fetcher = Arc::new(GcpSecretFetcher::new(&base_url, "test-token", 1));

    let server =
        start_test_server_with_gcp_fetcher(db.clone(), std::time::Duration::from_secs(30), fetcher.clone())
            .await;

    // Provision the project — triggers fetch_fresh (always a real call)
    let client = reqwest::Client::new();
    client
        .post(format!("http://{}/admin/v1/projects", server.admin_addr))
        .header("Authorization", "Bearer test-admin-key-secret")
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "project_id": "gcp-audit-project",
            "backend_mode": "gcp_secret",
            "gcp_resource_name": resource_name
        }))
        .send()
        .await
        .expect("HTTP request failed");

    // Wait for TTL to expire then trigger get_dsn (cache miss → AccessSecretVersion call)
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let _dsn: String = fetcher
        .get_dsn(resource_name)
        .await
        .expect("get_dsn after TTL expiry must succeed");

    // Assert: AccessSecretVersion was called (at least once for provision + once for cache miss)
    let calls: Vec<String> = fetcher.call_log_snapshot();
    assert!(
        calls.iter().any(|c: &String| c.contains("dsn-11c")),
        "call_log must contain an AccessSecretVersion call for the resource: {calls:?}"
    );

    // Also verify mock server recorded the call
    let mock_calls = mock_state.call_log_snapshot();
    assert!(
        mock_calls.iter().any(|c| c.contains("dsn-11c")),
        "mock server call log must contain resource name: {mock_calls:?}"
    );
}

// ── AC-11d (error path) ─────────────────────────────────────────────────────

/// Error path: GCP workload identity unavailable returns 400
///
/// Given:  embyr does not have workload identity access configured
/// When:   POST /admin/v1/projects with backend_mode=gcp_secret
/// Then:   the response status is 400 Bad Request
/// And:    the error indicates the secret could not be fetched
#[tokio::test]
async fn missing_gcp_workload_identity_returns_400() {
    let (mock_state, base_url) = start_mock_gcp_sm().await;
    let (_sys, sys_url) = start_postgres().await;

    let resource_name = "projects/my-gcp-proj/secrets/denied-secret";
    // Configure the mock to return 403 for this resource
    mock_state.deny(resource_name);

    let db = Arc::new(SystemDb::new(&sys_url).await.unwrap());
    db.migrate().await.unwrap();

    let fetcher = Arc::new(GcpSecretFetcher::new(&base_url, "test-token", 300));

    let server =
        start_test_server_with_gcp_fetcher(db.clone(), std::time::Duration::from_secs(30), fetcher)
            .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{}/admin/v1/projects", server.admin_addr))
        .header("Authorization", "Bearer test-admin-key-secret")
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "project_id": "denied-gcp-project",
            "backend_mode": "gcp_secret",
            "gcp_resource_name": resource_name
        }))
        .send()
        .await
        .expect("HTTP request failed");

    assert_eq!(
        resp.status(),
        400,
        "expected 400, got: {}",
        resp.status()
    );
    let body: serde_json::Value = resp.json().await.expect("body must be JSON");
    assert_eq!(
        body["error"], "backend_secret_fetch_failed",
        "error code must be backend_secret_fetch_failed: {body}"
    );
}
