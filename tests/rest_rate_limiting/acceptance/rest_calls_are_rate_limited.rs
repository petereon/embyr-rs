// SCAFFOLD: true
//! REST rate-limiting gap fix — every REST-port (:8081) endpoint that
//! carries a `:project_id` path param must be gated by the SAME
//! `Arc<RateLimiter>` instance the gRPC data-plane already uses.
//!
//! Prior state (confirmed by grep, documented in ADR-043 Decision 7):
//! `grep -rln "rate_limiter" crates/embyr-server/src/rest/` returned NOTHING
//! — none of the `accounts:<verb>` REST identity-bridge routes were
//! rate-limited, unlike every gRPC handler method.
//!
//! Driving port: REST `:8081` — `POST /v1/projects/{project_id}/accounts:signUp`
//! (the accounts-bridge dispatch route; any accounts:<verb> arm exercises the
//! same shared middleware).
//! Driven internal: in-process `RateLimiter` fixture (`start_test_server_with_rate_limit`)
//! — simplest fixture that proves wiring; the token-bucket algorithm itself
//! is already covered by `tests/acceptance/us_14_rate_limiting.rs` and the
//! `distributed_rate_limiting` suite.

use std::sync::Arc;

use embyr_server::{adapters::system_db::SystemDb, start_test_server_with_rate_limit};
use serial_test::serial;
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

/// AC-REST-RL-01: a REST call to a project whose token bucket is exhausted
/// is rejected with 429 BEFORE any handler-specific logic runs.
///
/// MODIFIED for security finding X4: the gate now charges only requests whose
/// `?key=` is already verified (held by the credential cache) — it used to
/// charge every request, even key-less ones for a project that does not
/// exist, which let anyone drain a known project's bucket. So this test now
/// provisions a real project, sends one warm-up call with its real key (not
/// charged: the key is not yet cached; the handler verifies it and caches
/// it), and then proves the NEXT call with that key is rejected with 429
/// before the handler runs (no body would otherwise be accepted either way).
#[tokio::test]
#[serial]
async fn rest_call_rejected_with_429_when_bucket_exhausted() {
    let (_sys_container, sys_url) = start_postgres().await;
    let system_db = Arc::new(SystemDb::new(&sys_url).await.unwrap());
    system_db.migrate().await.unwrap();

    let project_id = "rest-rl-exhausted-project";
    let api_key = "rest-rl-exhausted-key";
    let api_key_hash = embyr_core::auth::argon2::hash_api_key(api_key.as_bytes()).unwrap();
    let encrypted_dsn = embyr_core::auth::ecies::encrypt(
        &embyr_core::auth::ecies::derive_public_key(api_key.as_bytes()),
        sys_url.as_bytes(),
    )
    .unwrap();
    sqlx::query(
        "INSERT INTO projects \
         (id, status, backend_mode, api_key_hash_current, ecies_encrypted_dsn) \
         VALUES ($1, 'active', 'direct_pg', $2, $3)",
    )
    .bind(project_id)
    .bind(&api_key_hash)
    .bind(&encrypted_dsn)
    .execute(system_db.pool())
    .await
    .unwrap();

    // capacity=0.0 -> the in-process bucket is permanently empty (tokens are
    // capped at capacity on every refill), so every check() rejects.
    let server = start_test_server_with_rate_limit(system_db, 0.0, 0.0, true).await;

    let client = reqwest::Client::new();
    let url = format!(
        "http://{}/v1/projects/{project_id}/accounts:signUp?key={api_key}",
        server.rest_addr
    );
    let warm_up = client
        .post(&url)
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("warm-up POST accounts:signUp");
    assert_ne!(warm_up.status().as_u16(), 429, "an unverified key must not spend tokens");
    let response = client
        .post(&url)
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("POST accounts:signUp");

    assert_eq!(response.status().as_u16(), 429, "expected 429 Too Many Requests");

    let body: serde_json::Value = response.json().await.expect("valid JSON body");
    assert_eq!(body["error"]["code"], 429);
    assert_eq!(body["error"]["status"], "RESOURCE_EXHAUSTED");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("rest-rl-exhausted-project"),
        "message should name the rate-limited project, got: {body}"
    );
}

/// AC-REST-RL-02 (regression): a REST call to a project with tokens
/// available is NOT rejected by the new gate — it reaches the handler's own
/// logic, which returns its normal 401 `INVALID_API_KEY` for a request with
/// no `?key=` param.
#[tokio::test]
#[serial]
async fn rest_call_reaches_handler_when_tokens_available() {
    let (_sys_container, sys_url) = start_postgres().await;
    let system_db = Arc::new(SystemDb::new(&sys_url).await.unwrap());
    system_db.migrate().await.unwrap();

    let server = start_test_server_with_rate_limit(system_db, 1000.0, 1000.0, true).await;

    let client = reqwest::Client::new();
    let url = format!(
        "http://{}/v1/projects/rest-rl-available-project/accounts:signUp",
        server.rest_addr
    );
    let response = client
        .post(&url)
        .json(&serde_json::json!({}))
        .send()
        .await
        .expect("POST accounts:signUp");

    assert_ne!(
        response.status().as_u16(),
        429,
        "valid traffic must not be rejected by the rate-limit gate"
    );
    assert_eq!(response.status().as_u16(), 401, "expected handler's own INVALID_API_KEY response");

    let body: serde_json::Value = response.json().await.expect("valid JSON body");
    assert_eq!(body["reason"], "INVALID_API_KEY");
}
