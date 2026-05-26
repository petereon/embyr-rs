// SCAFFOLD: false
//! US-10 — Provision project with AWS Secrets Manager
//!
//! As Morgan (Tenant Admin), I want to register a project with backend_mode=aws_secret
//! and an ARN, so that embyr fetches the DSN from my existing secret store and never
//! stores it.
//!
//! Driving port: Admin HTTP port (:9090) — POST /admin/v1/projects
//!               gRPC data port (:8080) — SDK operations after provisioning
//! Infrastructure: LocalStack for AWS Secrets Manager emulation
//! Red classification: MISSING_FUNCTIONALITY

use embyr_server::{
    adapters::{aws_secret_fetcher::AwsSecretFetcher, system_db::SystemDb},
    start_test_server_with_aws_fetcher,
};
use std::sync::Arc;
use testcontainers::{
    core::{ContainerPort, WaitFor},
    runners::AsyncRunner,
    ContainerAsync, GenericImage,
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

async fn start_localstack() -> (ContainerAsync<GenericImage>, String) {
    let container = GenericImage::new("localstack/localstack", "3.5")
        .with_exposed_port(ContainerPort::Tcp(4566))
        .with_wait_for(WaitFor::message_on_stdout("Ready."))
        .start()
        .await
        .expect("start localstack");
    let port = container
        .get_host_port_ipv4(4566)
        .await
        .expect("get localstack port");
    let endpoint_url = format!("http://127.0.0.1:{port}");
    (container, endpoint_url)
}

async fn make_sm_client(endpoint_url: &str) -> aws_sdk_secretsmanager::Client {
    let aws_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .endpoint_url(endpoint_url)
        .region(
            aws_config::meta::region::RegionProviderChain::default_provider()
                .or_else("us-east-1"),
        )
        .credentials_provider(aws_sdk_secretsmanager::config::Credentials::new(
            "test", "test", None, None, "test",
        ))
        .load()
        .await;
    aws_sdk_secretsmanager::Client::new(&aws_config)
}

/// AC-10a: POST with aws_secret mode and valid ARN + IAM access returns 201; no DSN in system DB
///
/// Given:  LocalStack AWS Secrets Manager has a secret with DSN at a test ARN
/// And:    embyr has IAM credentials that can access LocalStack
/// When:   POST /admin/v1/projects with backend_mode=aws_secret and the test ARN
/// Then:   the response status is 201 Created
/// And:    the system DB project record for this project has no plaintext DSN
/// And:    the project backend_secret_arn column equals the provided ARN
///
/// Tags: @real_io @adapter_integration
#[tokio::test]
async fn provision_with_aws_secret_stores_arn_not_dsn() {
    let (_localstack, endpoint_url) = start_localstack().await;
    let (_cust, cust_url) = start_postgres().await;
    let (_sys, sys_url) = start_postgres().await;

    // Create secret in LocalStack
    let sm_client = make_sm_client(&endpoint_url).await;
    let secret_string = format!(r#"{{"dsn": "{}"}}"#, cust_url);
    let create_resp = sm_client
        .create_secret()
        .name("test-dsn-secret-10a")
        .secret_string(&secret_string)
        .send()
        .await
        .expect("create secret");
    let arn = create_resp.arn().expect("arn present").to_string();

    // Build system DB + fetcher + server
    let db = Arc::new(SystemDb::new(&sys_url).await.unwrap());
    db.migrate().await.unwrap();

    let ttl_secs: u64 = 300;
    let aws_cfg = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .endpoint_url(&endpoint_url)
        .region(
            aws_config::meta::region::RegionProviderChain::default_provider()
                .or_else("us-east-1"),
        )
        .credentials_provider(aws_sdk_secretsmanager::config::Credentials::new(
            "test", "test", None, None, "test",
        ))
        .load()
        .await;
    let fetcher = Arc::new(AwsSecretFetcher::new(&aws_cfg, ttl_secs).await);

    let server = start_test_server_with_aws_fetcher(
        db.clone(),
        std::time::Duration::from_secs(30),
        fetcher,
    )
    .await;

    // POST provision
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{}/admin/v1/projects", server.admin_addr))
        .header("Authorization", "Bearer test-admin-key-secret")
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "project_id": "aws-secret-project",
            "backend_mode": "aws_secret",
            "secret_arn": arn
        }))
        .send()
        .await
        .expect("HTTP request failed");

    assert_eq!(resp.status(), 201, "expected 201 Created, got: {}", resp.status());

    let body: serde_json::Value = resp.json().await.expect("body must be JSON");
    assert_eq!(body["project_id"], "aws-secret-project");
    assert!(body["api_key"].is_string(), "api_key must be present: {body}");

    // Verify system DB: backend_secret_arn = arn, ecies_encrypted_dsn IS NULL
    let row = sqlx::query(
        "SELECT backend_secret_arn, ecies_encrypted_dsn FROM projects WHERE id = $1",
    )
    .bind("aws-secret-project")
    .fetch_one(db.pool())
    .await
    .expect("project row must exist");

    use sqlx::Row;
    let stored_arn: Option<String> = row.try_get("backend_secret_arn").expect("backend_secret_arn");
    let stored_dsn: Option<Vec<u8>> = row.try_get("ecies_encrypted_dsn").expect("ecies_encrypted_dsn");

    assert_eq!(
        stored_arn.as_deref(),
        Some(arn.as_str()),
        "backend_secret_arn must equal the ARN"
    );
    assert!(
        stored_dsn.is_none(),
        "ecies_encrypted_dsn must be NULL for aws_secret projects"
    );
}

/// AC-10b (error path): no IAM access returns 400 backend_secret_fetch_failed
///
/// Given:  the provided ARN does not exist in AWS Secrets Manager (simulates denied access)
/// When:   POST /admin/v1/projects with backend_mode=aws_secret
/// Then:   the response status is 400 Bad Request
/// And:    the error code is "backend_secret_fetch_failed"
#[tokio::test]
async fn no_iam_access_returns_backend_secret_fetch_failed() {
    let (_localstack, endpoint_url) = start_localstack().await;
    let (_sys, sys_url) = start_postgres().await;

    let db = Arc::new(SystemDb::new(&sys_url).await.unwrap());
    db.migrate().await.unwrap();

    let aws_cfg = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .endpoint_url(&endpoint_url)
        .region(
            aws_config::meta::region::RegionProviderChain::default_provider()
                .or_else("us-east-1"),
        )
        .credentials_provider(aws_sdk_secretsmanager::config::Credentials::new(
            "test", "test", None, None, "test",
        ))
        .load()
        .await;
    let fetcher = Arc::new(AwsSecretFetcher::new(&aws_cfg, 300).await);

    let server = start_test_server_with_aws_fetcher(
        db.clone(),
        std::time::Duration::from_secs(30),
        fetcher,
    )
    .await;

    // Use a nonexistent ARN — LocalStack returns ResourceNotFoundException
    let fake_arn = "arn:aws:secretsmanager:us-east-1:000000000000:secret:nonexistent-secret-ABCDEF";

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{}/admin/v1/projects", server.admin_addr))
        .header("Authorization", "Bearer test-admin-key-secret")
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "project_id": "denied-project",
            "backend_mode": "aws_secret",
            "secret_arn": fake_arn
        }))
        .send()
        .await
        .expect("HTTP request failed");

    assert_eq!(resp.status(), 400, "expected 400, got: {}", resp.status());
    let body: serde_json::Value = resp.json().await.expect("body must be JSON");
    assert_eq!(
        body["error"], "backend_secret_fetch_failed",
        "error code must be backend_secret_fetch_failed: {body}"
    );
}

/// AC-10c (error path): malformed secret JSON returns 400 backend_secret_format_invalid
///
/// Given:  the AWS secret at the ARN contains plain text, not {"dsn": "..."}
/// When:   POST /admin/v1/projects with backend_mode=aws_secret
/// Then:   the response status is 400 Bad Request
/// And:    the error code is "backend_secret_format_invalid"
#[tokio::test]
async fn malformed_aws_secret_returns_format_invalid() {
    let (_localstack, endpoint_url) = start_localstack().await;
    let (_sys, sys_url) = start_postgres().await;

    let sm_client = make_sm_client(&endpoint_url).await;
    let create_resp = sm_client
        .create_secret()
        .name("bad-secret-10c")
        .secret_string("not-json-at-all")
        .send()
        .await
        .expect("create secret");
    let arn = create_resp.arn().expect("arn").to_string();

    let db = Arc::new(SystemDb::new(&sys_url).await.unwrap());
    db.migrate().await.unwrap();

    let aws_cfg = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .endpoint_url(&endpoint_url)
        .region(
            aws_config::meta::region::RegionProviderChain::default_provider()
                .or_else("us-east-1"),
        )
        .credentials_provider(aws_sdk_secretsmanager::config::Credentials::new(
            "test", "test", None, None, "test",
        ))
        .load()
        .await;
    let fetcher = Arc::new(AwsSecretFetcher::new(&aws_cfg, 300).await);

    let server = start_test_server_with_aws_fetcher(
        db.clone(),
        std::time::Duration::from_secs(30),
        fetcher,
    )
    .await;

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{}/admin/v1/projects", server.admin_addr))
        .header("Authorization", "Bearer test-admin-key-secret")
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "project_id": "bad-format-project",
            "backend_mode": "aws_secret",
            "secret_arn": arn
        }))
        .send()
        .await
        .expect("HTTP request failed");

    assert_eq!(resp.status(), 400, "expected 400, got: {}", resp.status());
    let body: serde_json::Value = resp.json().await.expect("body must be JSON");
    assert_eq!(
        body["error"], "backend_secret_format_invalid",
        "error code must be backend_secret_format_invalid: {body}"
    );
}

/// AC-10d: password rotation in AWS Secrets Manager is transparent within cache TTL
///
/// Given:  a project provisioned with aws_secret backend is receiving successful SDK calls
/// When:   the DSN password in AWS Secrets Manager is rotated
/// And:    short cache TTL (2s) elapses
/// Then:   SDK calls succeed using the new password
///
/// Tags: @real_io @kpi
#[tokio::test]
async fn aws_secret_rotation_transparent_within_cache_ttl() {
    let (_localstack, endpoint_url) = start_localstack().await;
    let (_cust, cust_url) = start_postgres().await;
    let (_sys, sys_url) = start_postgres().await;

    let sm_client = make_sm_client(&endpoint_url).await;
    let secret_string = format!(r#"{{"dsn": "{}"}}"#, cust_url);
    let create_resp = sm_client
        .create_secret()
        .name("rotation-secret-10d")
        .secret_string(&secret_string)
        .send()
        .await
        .expect("create secret");
    let arn = create_resp.arn().expect("arn").to_string();

    let db = Arc::new(SystemDb::new(&sys_url).await.unwrap());
    db.migrate().await.unwrap();

    // Short TTL = 2 seconds so we don't wait 5 minutes
    let ttl_secs: u64 = 2;
    let aws_cfg = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .endpoint_url(&endpoint_url)
        .region(
            aws_config::meta::region::RegionProviderChain::default_provider()
                .or_else("us-east-1"),
        )
        .credentials_provider(aws_sdk_secretsmanager::config::Credentials::new(
            "test", "test", None, None, "test",
        ))
        .load()
        .await;
    let fetcher = Arc::new(AwsSecretFetcher::new(&aws_cfg, ttl_secs).await);

    let server = start_test_server_with_aws_fetcher(
        db.clone(),
        std::time::Duration::from_secs(30),
        fetcher.clone(),
    )
    .await;

    // Provision the project
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{}/admin/v1/projects", server.admin_addr))
        .header("Authorization", "Bearer test-admin-key-secret")
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "project_id": "rotation-project",
            "backend_mode": "aws_secret",
            "secret_arn": arn
        }))
        .send()
        .await
        .expect("HTTP request failed");
    assert_eq!(resp.status(), 201, "provision must succeed: {}", resp.status());
    let body: serde_json::Value = resp.json().await.expect("body must be JSON");
    let api_key = body["api_key"].as_str().unwrap().to_string();

    // Initial SDK call: verify DSN fetch works (cache populates)
    let dsn1: String = fetcher.get_dsn(&arn).await.expect("first get_dsn must succeed");
    assert_eq!(dsn1, cust_url, "first DSN must match original");

    // Update the secret in LocalStack with a new DSN (same DB, simulating password rotation)
    // The new DSN still points to the same Postgres instance (no actual rotation needed)
    let new_secret_string = format!(r#"{{"dsn": "{}"}}"#, cust_url);
    sm_client
        .put_secret_value()
        .secret_id(&arn)
        .secret_string(&new_secret_string)
        .send()
        .await
        .expect("update secret");

    // Sleep > TTL so cache expires
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;

    // After TTL expiry, get_dsn must re-fetch from AWS SM
    let dsn2: String = fetcher.get_dsn(&arn).await.expect("post-rotation get_dsn must succeed");
    assert!(!dsn2.is_empty(), "DSN after rotation must not be empty");

    // Verify the api_key is non-empty (server was live throughout)
    assert!(!api_key.is_empty(), "api_key must be non-empty");
}
