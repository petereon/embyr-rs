// SCAFFOLD: false
//! US-01 (Slice 01, Walking Skeleton) — Orphaned Transaction Rows Are
//! Reclaimed Across Every Reachable Customer Database (ADR-054, ADR-055).
//!
//! Acceptance criteria verified here (slice-01-reclaim-orphaned-transactions.md):
//!   - direct_pg project, backend_pg_dsn_enc populated: orphaned 'active' row
//!     past the abandonment threshold is marked 'expired'.
//!   - gcp_secret project: orphaned row reclaimed with zero live api_key
//!     involved (DSN resolved via GcpSecretFetcher + ARN/resource-name alone).
//!   - a row within the abandonment threshold is never modified.
//!   - a direct_pg project with backend_pg_dsn_enc IS NULL is skipped without
//!     aborting the cycle for other projects (ADR-055).
//!   - backend_mode=agent projects never appear in the sweep's enumeration
//!     query.
//!   - concurrent sweep attempts are serialized by a Postgres advisory lock.
//!   - embyr_transaction_sweeper_reclaimed_total increments once per row
//!     actually reclaimed.
//!
//! Driving port: `TransactionSweeper::spawn` (the walking-skeleton test only
//! — real interval + advisory lock, real customer Postgres) and
//! `TransactionSweeper::run_cycle` directly (remaining scenarios — same
//! production code, invoked deterministically per the "pure function IS its
//! own driving port" layered-test discipline; `SystemDb::list_pg_reachable_projects`
//! directly for the enumeration-only scenario).

use std::sync::Arc;

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use rand_core::{OsRng, RngCore};
use serial_test::serial;
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt},
};

use embyr_server::adapters::system_db::SystemDb;
use embyr_server::sweepers::transaction_sweeper;

const ENCRYPTION_KEY: [u8; 32] = [7u8; 32];

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

async fn start_system_db() -> (ContainerAsync<Postgres>, Arc<SystemDb>) {
    let (container, url) = start_postgres().await;
    let db = Arc::new(SystemDb::new(&url).await.expect("SystemDb::new failed"));
    db.migrate().await.expect("system DB migrations failed");
    (container, db)
}

async fn migrate_customer_db(url: &str) -> sqlx::PgPool {
    let pool = sqlx::PgPool::connect(url)
        .await
        .expect("connect customer db");
    sqlx::migrate!("../../migrations/customer")
        .run(&pool)
        .await
        .expect("customer DB migrations failed");
    pool
}

fn encrypt_dsn(key: &[u8; 32], dsn: &str) -> Vec<u8> {
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let cipher = Aes256Gcm::new_from_slice(key).expect("valid key");
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher.encrypt(nonce, dsn.as_bytes()).expect("encrypt dsn");
    let mut enc = nonce_bytes.to_vec();
    enc.extend_from_slice(&ct);
    enc
}

async fn insert_account(sys_pool: &sqlx::PgPool) -> uuid::Uuid {
    sqlx::query_scalar("INSERT INTO accounts (name) VALUES ('sweeper-test-account') RETURNING id")
        .fetch_one(sys_pool)
        .await
        .expect("insert account")
}

/// Insert a `direct_pg` project row with `backend_pg_dsn_enc` populated
/// (or NULL when `dsn` is `None`, ADR-055's own coverage-gap scenario).
async fn insert_direct_pg_project(
    sys_pool: &sqlx::PgPool,
    account_id: uuid::Uuid,
    project_id: &str,
    dsn: Option<&str>,
) {
    let dsn_enc = dsn.map(|d| encrypt_dsn(&ENCRYPTION_KEY, d));
    sqlx::query(
        "INSERT INTO projects \
         (id, account_id, backend_mode, api_key_hash_current, status, name, backend_pg_dsn_enc) \
         VALUES ($1, $2, 'direct_pg', 'unused-hash', 'active', $1, $3)",
    )
    .bind(project_id)
    .bind(account_id)
    .bind(dsn_enc)
    .execute(sys_pool)
    .await
    .expect("insert direct_pg project");
}

async fn insert_gcp_secret_project(
    sys_pool: &sqlx::PgPool,
    account_id: uuid::Uuid,
    project_id: &str,
    resource_name: &str,
) {
    sqlx::query(
        "INSERT INTO projects \
         (id, account_id, backend_mode, api_key_hash_current, status, name, backend_secret_gcp) \
         VALUES ($1, $2, 'gcp_secret', 'unused-hash', 'active', $1, $3)",
    )
    .bind(project_id)
    .bind(account_id)
    .bind(resource_name)
    .execute(sys_pool)
    .await
    .expect("insert gcp_secret project");
}

async fn insert_agent_project(sys_pool: &sqlx::PgPool, account_id: uuid::Uuid, project_id: &str) {
    sqlx::query(
        "INSERT INTO projects \
         (id, account_id, backend_mode, api_key_hash_current, status, name) \
         VALUES ($1, $2, 'agent', 'unused-hash', 'active', $1)",
    )
    .bind(project_id)
    .bind(account_id)
    .execute(sys_pool)
    .await
    .expect("insert agent project");
}

async fn insert_transaction(
    cust_pool: &sqlx::PgPool,
    project_id: &str,
    status: &str,
    started_seconds_ago: i64,
) -> uuid::Uuid {
    let transaction_id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO transactions (transaction_id, project_id, status, started_at) \
         VALUES ($1, $2, $3, now() - make_interval(secs => $4))",
    )
    .bind(transaction_id)
    .bind(project_id)
    .bind(status)
    .bind(started_seconds_ago as f64)
    .execute(cust_pool)
    .await
    .expect("insert transaction");
    transaction_id
}

async fn transaction_status(cust_pool: &sqlx::PgPool, transaction_id: uuid::Uuid) -> String {
    sqlx::query_scalar("SELECT status FROM transactions WHERE transaction_id = $1")
        .bind(transaction_id)
        .fetch_one(cust_pool)
        .await
        .expect("read transaction status")
}

/// Read a label-free Prometheus counter's current value from the shared
/// process-global recorder (`embyr_transaction_sweeper_reclaimed_total`
/// carries no `project_id` label, DISCUSS § System Constraints).
fn read_counter(name: &str) -> f64 {
    let body = embyr_server::observability::get_or_install_prometheus_handle().render();
    let target = format!("{name} ");
    for line in body.lines() {
        if line.starts_with('#') {
            continue;
        }
        if let Some(value) = line.strip_prefix(&target) {
            return value.trim().parse().unwrap_or(0.0);
        }
    }
    0.0
}

// ─── direct_pg reclaim (walking skeleton) ──────────────────────────────────

/// Walking skeleton: real `TransactionSweeper::spawn` (real interval, real
/// advisory lock) against a real direct_pg customer database reclaims an
/// orphaned row and increments the Prometheus counter.
#[tokio::test]
#[serial]
async fn direct_pg_orphaned_transaction_is_reclaimed_via_spawn_and_counter_increments() {
    let (_sys_container, system_db) = start_system_db().await;
    let (_cust_container, cust_url) = start_postgres().await;
    let cust_pool = migrate_customer_db(&cust_url).await;

    let account_id = insert_account(system_db.pool()).await;
    insert_direct_pg_project(system_db.pool(), account_id, "trailmark-prod", Some(&cust_url)).await;

    let txn_id = insert_transaction(&cust_pool, "trailmark-prod", "active", 20 * 60).await;

    let before = read_counter("embyr_transaction_sweeper_reclaimed_total");

    let handle = transaction_sweeper::spawn(
        system_db.clone(),
        None,
        None,
        ENCRYPTION_KEY,
        None,
        std::time::Duration::from_millis(50),
        30,
    );

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if transaction_status(&cust_pool, txn_id).await == "expired" {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "sweep cycle did not reclaim the orphaned row within 5s"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    handle.abort();

    assert_eq!(transaction_status(&cust_pool, txn_id).await, "expired");
    let after = read_counter("embyr_transaction_sweeper_reclaimed_total");
    assert!(
        after >= before + 1.0,
        "reclaimed_total must increase by at least 1: before={before}, after={after}"
    );
}

// ─── gcp_secret reclaim without any api_key ────────────────────────────────

mod gcp_mock {
    use axum::{extract::Path, extract::State, response::IntoResponse};
    use base64::Engine as _;
    use std::{collections::HashMap, sync::Arc, sync::Mutex};

    #[derive(Clone, Default)]
    pub struct MockGcpState {
        secrets: Arc<Mutex<HashMap<String, String>>>,
    }

    impl MockGcpState {
        pub fn add_secret(&self, resource_name: &str, dsn: &str) {
            self.secrets
                .lock()
                .unwrap()
                .insert(resource_name.to_string(), dsn.to_string());
        }
    }

    async fn access_secret_version(
        State(state): State<MockGcpState>,
        Path(path): Path<String>,
    ) -> axum::response::Response {
        let resource_name = path.trim_end_matches("/versions/latest:access").to_string();
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
            None => (axum::http::StatusCode::NOT_FOUND, "not found").into_response(),
        }
    }

    pub async fn start() -> (MockGcpState, String) {
        let state = MockGcpState::default();
        let app = axum::Router::new()
            .route("/v1/*path", axum::routing::get(access_secret_version))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (state, format!("http://127.0.0.1:{port}"))
    }
}

/// gcp_secret: orphaned row is reclaimed with zero live api_key involved —
/// DSN resolved purely via `GcpSecretFetcher::get_dsn(resource_name)`.
#[tokio::test]
#[serial]
async fn gcp_secret_orphaned_transaction_is_reclaimed_without_api_key() {
    let (mock_state, base_url) = gcp_mock::start().await;
    let (_sys_container, system_db) = start_system_db().await;
    let (_cust_container, cust_url) = start_postgres().await;
    let cust_pool = migrate_customer_db(&cust_url).await;

    let resource_name = "projects/acme/secrets/acme-orders-dsn";
    mock_state.add_secret(resource_name, &cust_url);

    let account_id = insert_account(system_db.pool()).await;
    insert_gcp_secret_project(system_db.pool(), account_id, "acme-orders", resource_name).await;

    let txn_id = insert_transaction(&cust_pool, "acme-orders", "active", 3 * 60 * 60).await;

    let gcp_fetcher = Arc::new(embyr_server::adapters::gcp_secret_fetcher::GcpSecretFetcher::new(
        &base_url,
        "test-token",
        300,
    ));

    transaction_sweeper::run_cycle(&system_db, None, Some(&gcp_fetcher), &ENCRYPTION_KEY, None, 30)
        .await;

    assert_eq!(transaction_status(&cust_pool, txn_id).await, "expired");
}

// ─── Threshold guardrail ────────────────────────────────────────────────────

/// A row started within the abandonment threshold must never be modified —
/// the sweep cycle's `UPDATE ... WHERE started_at < now() - interval` clause
/// does not match it.
#[tokio::test]
async fn transaction_within_abandonment_threshold_is_left_untouched() {
    let (_sys_container, system_db) = start_system_db().await;
    let (_cust_container, cust_url) = start_postgres().await;
    let cust_pool = migrate_customer_db(&cust_url).await;

    let account_id = insert_account(system_db.pool()).await;
    insert_direct_pg_project(system_db.pool(), account_id, "trailmark-prod", Some(&cust_url)).await;

    let txn_id = insert_transaction(&cust_pool, "trailmark-prod", "active", 10).await;

    transaction_sweeper::run_cycle(&system_db, None, None, &ENCRYPTION_KEY, None, 30).await;

    assert_eq!(transaction_status(&cust_pool, txn_id).await, "active");
}

// ─── ADR-055 coverage gap: NULL backend_pg_dsn_enc ─────────────────────────

/// A direct_pg project with `backend_pg_dsn_enc IS NULL` is skipped without
/// aborting the cycle — another project's orphaned row is still reclaimed in
/// the same cycle.
#[tokio::test]
async fn direct_pg_project_with_null_dsn_enc_is_skipped_without_aborting_cycle() {
    let (_sys_container, system_db) = start_system_db().await;
    let (_cust_container, cust_url) = start_postgres().await;
    let cust_pool = migrate_customer_db(&cust_url).await;

    let account_id = insert_account(system_db.pool()).await;
    // acme-legacy: direct_pg, no DSN ever re-submitted — backend_pg_dsn_enc IS NULL.
    insert_direct_pg_project(system_db.pool(), account_id, "acme-legacy", None).await;
    // trailmark-prod: reachable, has an orphaned row eligible for reclaim.
    insert_direct_pg_project(system_db.pool(), account_id, "trailmark-prod", Some(&cust_url)).await;

    let txn_id = insert_transaction(&cust_pool, "trailmark-prod", "active", 20 * 60).await;

    // Must not panic/error despite acme-legacy being unreachable.
    transaction_sweeper::run_cycle(&system_db, None, None, &ENCRYPTION_KEY, None, 30).await;

    assert_eq!(transaction_status(&cust_pool, txn_id).await, "expired");
}

// ─── backend_mode=agent exclusion ──────────────────────────────────────────

/// `backend_mode=agent` projects never appear in the sweep's own enumeration
/// query — excluded at the SQL `WHERE` clause, not filtered per-row.
#[tokio::test]
async fn agent_mode_projects_never_appear_in_sweep_enumeration() {
    let (_sys_container, system_db) = start_system_db().await;
    let account_id = insert_account(system_db.pool()).await;

    insert_agent_project(system_db.pool(), account_id, "agent-project").await;
    insert_direct_pg_project(system_db.pool(), account_id, "trailmark-prod", Some("postgres://unused/db"))
        .await;

    let rows = system_db
        .list_pg_reachable_projects()
        .await
        .expect("list_pg_reachable_projects");

    assert!(
        rows.iter().all(|r| r.id != "agent-project"),
        "backend_mode=agent project must never appear in enumeration: {rows:?}"
    );
    assert!(
        rows.iter().any(|r| r.id == "trailmark-prod"),
        "PG-reachable project must appear in enumeration: {rows:?}"
    );
}

// ─── Concurrent sweep serialization ────────────────────────────────────────

/// Two server instances never race the same sweep cycle: a second
/// `pg_try_advisory_lock` on the sweeper's own lock key fails while the
/// first instance still holds it, and succeeds once released.
#[tokio::test]
async fn concurrent_sweep_attempts_are_serialized_by_advisory_lock() {
    let (_sys_container, system_db) = start_system_db().await;

    let lock_key = embyr_server::sweepers::advisory_lock_key(transaction_sweeper::LOCK_KEY_NAME);

    let mut conn_a = system_db.pool().acquire().await.expect("acquire conn a");
    let mut conn_b = system_db.pool().acquire().await.expect("acquire conn b");

    let locked_a: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
        .bind(lock_key)
        .fetch_one(&mut *conn_a)
        .await
        .expect("try lock a");
    assert!(locked_a, "first instance must acquire the lock");

    let locked_b: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
        .bind(lock_key)
        .fetch_one(&mut *conn_b)
        .await
        .expect("try lock b");
    assert!(
        !locked_b,
        "second instance must be refused the lock while the first holds it"
    );

    let _: bool = sqlx::query_scalar("SELECT pg_advisory_unlock($1)")
        .bind(lock_key)
        .fetch_one(&mut *conn_a)
        .await
        .expect("unlock a");

    let locked_b_retry: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
        .bind(lock_key)
        .fetch_one(&mut *conn_b)
        .await
        .expect("try lock b retry");
    assert!(
        locked_b_retry,
        "second instance must acquire the lock once the first releases it"
    );
}
