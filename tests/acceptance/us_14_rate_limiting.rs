// SCAFFOLD: true
//! US-14 — Rate limiting protects service per project
//!
//! As Sam, I want to configure per-project request rate limits, so that one
//! project cannot starve others or cause system-wide instability.
//!
//! Driving port: gRPC data port (:8080) — any Firestore RPC subject to rate limiting
//! Red classification: MISSING_FUNCTIONALITY

use std::{sync::Arc, time::Duration};

use embyr_core::auth::{argon2, ecies};
use embyr_proto::firestore::{
    firestore_client::FirestoreClient, GetDocumentRequest,
};
use embyr_server::{
    adapters::system_db::SystemDb,
    start_test_server_with_rate_limit,
};
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

async fn provision_project(
    _system_db: &SystemDb,
    sys_url: &str,
    project_id: &str,
    api_key: &str,
    cust_url: &str,
) {
    let cust_pool = sqlx::PgPool::connect(cust_url).await.unwrap();
    sqlx::migrate!("../../migrations/customer")
        .run(&cust_pool)
        .await
        .unwrap();

    let api_key_hash = argon2::hash_api_key(api_key.as_bytes()).unwrap();
    let pub_key = ecies::derive_public_key(api_key.as_bytes());
    let encrypted_dsn = ecies::encrypt(&pub_key, cust_url.as_bytes()).unwrap();

    let sys_pool = sqlx::PgPool::connect(sys_url).await.unwrap();
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
}

fn make_channel(addr: std::net::SocketAddr) -> tonic::transport::Channel {
    tonic::transport::Channel::from_shared(format!("http://{addr}"))
        .unwrap()
        .connect_lazy()
}

fn make_get_request(project_id: &str, api_key: &str) -> tonic::Request<GetDocumentRequest> {
    let name = format!(
        "projects/{project_id}/databases/(default)/documents/test/doc1"
    );
    let mut req = tonic::Request::new(GetDocumentRequest { name, ..Default::default() });
    req.metadata_mut().insert(
        "authorization",
        format!("bearer {api_key}").parse().unwrap(),
    );
    req
}

/// AC-14a: burst above default_burst returns RESOURCE_EXHAUSTED for excess requests
///
/// Given:  a project with default_burst=100 configured
/// When:   200 requests are sent concurrently to the gRPC data port
/// Then:   approximately the first 100 succeed (or return NOT_FOUND — not rate limited)
/// And:    the excess requests return status RESOURCE_EXHAUSTED
#[tokio::test]
#[serial]
async fn burst_above_default_returns_resource_exhausted() {
    let (_sys_container, sys_url) = start_postgres().await;
    let (_cust_container, cust_url) = start_postgres().await;

    let system_db = Arc::new(SystemDb::new(&sys_url).await.unwrap());
    system_db.migrate().await.unwrap();

    let project_id = "us14-rl-project-01";
    let api_key = "test-sk-us14-rl-01";

    provision_project(&system_db, &sys_url, project_id, api_key, &cust_url).await;

    // capacity=100, refill=0.1/s (effectively no refill during burst), enabled
    let server = start_test_server_with_rate_limit(system_db, 100.0, 0.1, true).await;

    // Warm up: authenticate once so the credential cache is populated.
    // This avoids counting Argon2id race-conditions toward rate limit differences.
    {
        let mut warmup_client = FirestoreClient::new(make_channel(server.grpc_addr));
        let _ = warmup_client.get_document(make_get_request(project_id, api_key)).await;
        // Wait briefly for warm-up to complete and tokens to settle
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let num_requests = 200_usize;
    let addr = server.grpc_addr;
    let project_id_str = project_id.to_string();
    let api_key_str = api_key.to_string();

    // Share a single channel so all 200 requests go through the same HTTP/2
    // multiplexed connection — ensures they all reach the rate-limiter check
    // rather than failing at the transport layer.
    let shared_channel = make_channel(addr);

    let mut join_set = tokio::task::JoinSet::new();
    for _ in 0..num_requests {
        let project_id = project_id_str.clone();
        let api_key = api_key_str.clone();
        let ch = shared_channel.clone();
        join_set.spawn(async move {
            let mut client = FirestoreClient::new(ch);
            client.get_document(make_get_request(&project_id, &api_key)).await
        });
    }

    let mut resource_exhausted = 0usize;
    let mut allowed = 0usize;
    while let Some(result) = join_set.join_next().await {
        match result.unwrap() {
            Ok(_) => allowed += 1,
            Err(status) if status.code() == tonic::Code::ResourceExhausted => {
                resource_exhausted += 1;
            }
            Err(status) if status.code() == tonic::Code::NotFound => {
                // Document doesn't exist but auth + rate limit passed — counts as allowed
                allowed += 1;
            }
            Err(_) => {
                allowed += 1; // other errors (auth cache miss races) don't count as rate limited
            }
        }
    }

    assert!(
        resource_exhausted >= 80,
        "expected at least 80 RESOURCE_EXHAUSTED responses (capacity=100, requests=200), got {resource_exhausted} exhausted / {allowed} allowed"
    );
    assert!(
        allowed >= 80,
        "expected at least 80 allowed responses (capacity=100, requests=200), got {allowed} allowed / {resource_exhausted} exhausted"
    );
}

/// AC-14b: rate limiting disabled — no requests rejected
///
/// Given:  a project with ratelimit.enabled=false
/// When:   200 requests are sent concurrently
/// Then:   all 200 requests succeed (no RESOURCE_EXHAUSTED)
#[tokio::test]
#[serial]
async fn rate_limiting_disabled_no_requests_rejected() {
    let (_sys_container, sys_url) = start_postgres().await;
    let (_cust_container, cust_url) = start_postgres().await;

    let system_db = Arc::new(SystemDb::new(&sys_url).await.unwrap());
    system_db.migrate().await.unwrap();

    let project_id = "us14-rl-project-02";
    let api_key = "test-sk-us14-rl-02";

    provision_project(&system_db, &sys_url, project_id, api_key, &cust_url).await;

    // disabled rate limiter — all requests must pass through
    let server = start_test_server_with_rate_limit(system_db, 0.0, 0.0, false).await;

    // Warm up: prime credential cache with one Argon2id before concurrent burst.
    {
        let mut warmup_client = FirestoreClient::new(make_channel(server.grpc_addr));
        let _ = warmup_client.get_document(make_get_request(project_id, api_key)).await;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let num_requests = 200_usize;
    let addr = server.grpc_addr;
    let project_id_str = project_id.to_string();
    let api_key_str = api_key.to_string();

    let mut join_set = tokio::task::JoinSet::new();
    for _ in 0..num_requests {
        let addr = addr;
        let project_id = project_id_str.clone();
        let api_key = api_key_str.clone();
        join_set.spawn(async move {
            let mut client = FirestoreClient::new(make_channel(addr));
            client.get_document(make_get_request(&project_id, &api_key)).await
        });
    }

    let mut resource_exhausted = 0usize;
    while let Some(result) = join_set.join_next().await {
        if let Err(status) = result.unwrap() {
            if status.code() == tonic::Code::ResourceExhausted {
                resource_exhausted += 1;
            }
        }
    }

    assert_eq!(
        resource_exhausted, 0,
        "rate limiting disabled: expected 0 RESOURCE_EXHAUSTED out of {num_requests} requests, got {resource_exhausted}"
    );
}

/// AC-14c (property): p99 latency increase from rate limiting is under 0.5ms on hot path
///
/// Given:  rate limiting is enabled with default parameters
/// When:   requests at 50% of the rate limit are sent
/// Then:   the p99 additional latency from the rate limiter check is < 0.5 milliseconds
///
/// Note: verified by directly measuring RateLimiter::check() timing (microbenchmark)
/// Tags: @kpi
#[tokio::test]
#[serial]
async fn rate_limiter_adds_less_than_half_ms_to_p99_latency() {
    use embyr_server::middleware::rate_limit::RateLimiter;

    // Never-exhausted limiter: capacity and refill are f64::MAX
    let rl = RateLimiter::new(f64::MAX, f64::MAX);

    let samples = 10_000_usize;
    let mut latencies = Vec::with_capacity(samples);

    for _ in 0..samples {
        let start = std::time::Instant::now();
        rl.check("bench-project").await.unwrap();
        latencies.push(start.elapsed());
    }

    latencies.sort();
    let p99 = latencies[samples * 99 / 100];

    assert!(
        p99 < Duration::from_micros(500),
        "rate limiter p99 latency {p99:?} exceeds 500µs budget"
    );
}

/// Error path: different projects use independent rate limit buckets
///
/// Given:  project-A has a tight rate limit (capacity=10) and project-B has the same server
/// When:   project-A exhausts its rate limit (20 requests sent)
/// Then:   project-A calls return RESOURCE_EXHAUSTED for excess requests
/// And:    project-B requests continue to succeed (or return NOT_FOUND)
#[tokio::test]
#[serial]
async fn rate_limit_exhaustion_is_isolated_per_project() {
    let (_sys_container_a, sys_url) = start_postgres().await;
    let (_cust_container_a, cust_url_a) = start_postgres().await;
    let (_cust_container_b, cust_url_b) = start_postgres().await;

    let system_db = Arc::new(SystemDb::new(&sys_url).await.unwrap());
    system_db.migrate().await.unwrap();

    let project_a = "us14-rl-project-a";
    let api_key_a = "test-sk-us14-rl-a";
    let project_b = "us14-rl-project-b";
    let api_key_b = "test-sk-us14-rl-b";

    provision_project(&system_db, &sys_url, project_a, api_key_a, &cust_url_a).await;
    provision_project(&system_db, &sys_url, project_b, api_key_b, &cust_url_b).await;

    // capacity=12, refill=0.01/s (very slow — effectively static during test).
    // 12 gives headroom for 2 warm-up requests (one per project) before the
    // main burst; project-A gets 20 sequential requests and will still exhaust
    // (20 > 12 − 1 warm-up), project-B gets 10 sequential requests and must
    // not exhaust (10 ≤ 12 − 1 warm-up = 11).
    let server = start_test_server_with_rate_limit(system_db, 12.0, 0.01, true).await;

    // Warm up both projects so credential cache is primed
    {
        let mut c = FirestoreClient::new(make_channel(server.grpc_addr));
        let _ = c.get_document(make_get_request(project_a, api_key_a)).await;
        let _ = c.get_document(make_get_request(project_b, api_key_b)).await;
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    let addr = server.grpc_addr;

    // Send 20 sequential requests to project-A: first 10 should pass, next 10 exhausted.
    // Sequential to make token consumption deterministic.
    let mut a_allowed = 0usize;
    let mut a_exhausted = 0usize;
    for _ in 0..20 {
        let mut client = FirestoreClient::new(make_channel(addr));
        match client.get_document(make_get_request(project_a, api_key_a)).await {
            Ok(_) => a_allowed += 1,
            Err(s) if s.code() == tonic::Code::ResourceExhausted => a_exhausted += 1,
            Err(s) if s.code() == tonic::Code::NotFound => a_allowed += 1,
            Err(_) => a_allowed += 1,
        }
    }

    // Send 10 requests to project-B: all should pass
    let mut b_resource_exhausted = 0usize;
    for _ in 0..10 {
        let mut client = FirestoreClient::new(make_channel(addr));
        if let Err(s) = client.get_document(make_get_request(project_b, api_key_b)).await {
            if s.code() == tonic::Code::ResourceExhausted {
                b_resource_exhausted += 1;
            }
        }
    }

    assert!(
        a_exhausted >= 8,
        "project-A: expected at least 8 RESOURCE_EXHAUSTED (capacity=10, sent=20), got {a_exhausted} exhausted / {a_allowed} allowed"
    );
    assert_eq!(
        b_resource_exhausted, 0,
        "project-B must not be rate-limited by project-A exhausting its bucket, got {b_resource_exhausted} RESOURCE_EXHAUSTED"
    );
}
