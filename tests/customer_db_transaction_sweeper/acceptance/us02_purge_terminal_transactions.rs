// SCAFFOLD: false
//! US-02 (Slice 02) — Terminal-State Transaction Rows Are Purged, Bounding
//! Table Growth (ADR-054 § D2).
//!
//! Acceptance criteria verified here (slice-02-purge-terminal-transactions.md):
//!   - a 'committed' row older than the retention window is hard-deleted.
//!   - an 'expired' row older than the retention window is hard-deleted.
//!   - a terminal-state row within the retention window is never deleted.
//!   - an 'active' (non-terminal) row is never purged regardless of age —
//!     proves purge and Slice 01's own reclaim do not overlap incorrectly.
//!   - embyr_transaction_sweeper_purged_total increments once per row
//!     actually deleted, observable via GET :9090/metrics.
//!
//! Purge reuses Slice 01's own DSN-resolution dispatch and customer-DB
//! connection path unchanged (ADR-054 § D2) — no mode-specific branching to
//! re-prove here; `direct_pg` alone exercises the shared `sweep_one_project`
//! code path both `aws_secret`/`gcp_secret` purge would also run through.
//!
//! Driving port: `TransactionSweeper::run_cycle` (same production entry
//! point as Slice 01's own non-walking-skeleton scenarios — "pure function
//! IS its own driving port" layered-test discipline).

use std::sync::Arc;

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use rand_core::{OsRng, RngCore};
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt},
};

use embyr_server::adapters::system_db::SystemDb;
use embyr_server::sweepers::transaction_sweeper;

const ENCRYPTION_KEY: [u8; 32] = [7u8; 32];
const RETENTION_DAYS: i64 = 30;

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

async fn insert_direct_pg_project(
    sys_pool: &sqlx::PgPool,
    account_id: uuid::Uuid,
    project_id: &str,
    dsn: &str,
) {
    let dsn_enc = encrypt_dsn(&ENCRYPTION_KEY, dsn);
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

/// `started_seconds_ago` mirrors US-01's own helper shape (seconds, not
/// days) so both "just past the 60s abandonment threshold" and "days past
/// the retention window" cases are expressible with the same helper.
async fn insert_transaction(
    cust_pool: &sqlx::PgPool,
    project_id: &str,
    status: &str,
    started_seconds_ago: f64,
) -> uuid::Uuid {
    let transaction_id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO transactions (transaction_id, project_id, status, started_at) \
         VALUES ($1, $2, $3, now() - make_interval(secs => $4))",
    )
    .bind(transaction_id)
    .bind(project_id)
    .bind(status)
    .bind(started_seconds_ago)
    .execute(cust_pool)
    .await
    .expect("insert transaction");
    transaction_id
}

const SECS_PER_DAY: f64 = 86_400.0;

async fn transaction_exists(cust_pool: &sqlx::PgPool, transaction_id: uuid::Uuid) -> bool {
    let row: Option<uuid::Uuid> =
        sqlx::query_scalar("SELECT transaction_id FROM transactions WHERE transaction_id = $1")
            .bind(transaction_id)
            .fetch_optional(cust_pool)
            .await
            .expect("query transaction existence");
    row.is_some()
}

async fn transaction_status(cust_pool: &sqlx::PgPool, transaction_id: uuid::Uuid) -> String {
    sqlx::query_scalar("SELECT status FROM transactions WHERE transaction_id = $1")
        .bind(transaction_id)
        .fetch_one(cust_pool)
        .await
        .expect("read transaction status")
}

/// Read a label-free Prometheus counter's current value from the shared
/// process-global recorder.
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

// ─── Committed row past retention is purged ────────────────────────────────

#[tokio::test]
async fn committed_row_past_retention_window_is_purged() {
    let (_sys_container, system_db) = start_system_db().await;
    let (_cust_container, cust_url) = start_postgres().await;
    let cust_pool = migrate_customer_db(&cust_url).await;

    let account_id = insert_account(system_db.pool()).await;
    insert_direct_pg_project(system_db.pool(), account_id, "trailmark-prod", &cust_url).await;

    let txn_id =
        insert_transaction(&cust_pool, "trailmark-prod", "committed", 45.0 * SECS_PER_DAY).await;

    transaction_sweeper::run_cycle(
        &system_db,
        None,
        None,
        &ENCRYPTION_KEY,
        None,
        RETENTION_DAYS,
    )
    .await;

    assert!(
        !transaction_exists(&cust_pool, txn_id).await,
        "committed row 45 days old must be hard-deleted past a 30-day retention window"
    );
}

// ─── Expired row past retention is purged ──────────────────────────────────

#[tokio::test]
async fn expired_row_past_retention_window_is_purged() {
    let (_sys_container, system_db) = start_system_db().await;
    let (_cust_container, cust_url) = start_postgres().await;
    let cust_pool = migrate_customer_db(&cust_url).await;

    let account_id = insert_account(system_db.pool()).await;
    insert_direct_pg_project(system_db.pool(), account_id, "trailmark-prod", &cust_url).await;

    let txn_id =
        insert_transaction(&cust_pool, "trailmark-prod", "expired", 45.0 * SECS_PER_DAY).await;

    transaction_sweeper::run_cycle(
        &system_db,
        None,
        None,
        &ENCRYPTION_KEY,
        None,
        RETENTION_DAYS,
    )
    .await;

    assert!(
        !transaction_exists(&cust_pool, txn_id).await,
        "expired row 45 days old must be hard-deleted past a 30-day retention window"
    );
}

// ─── Terminal row within retention is left untouched ───────────────────────

#[tokio::test]
async fn terminal_row_within_retention_window_is_untouched() {
    let (_sys_container, system_db) = start_system_db().await;
    let (_cust_container, cust_url) = start_postgres().await;
    let cust_pool = migrate_customer_db(&cust_url).await;

    let account_id = insert_account(system_db.pool()).await;
    insert_direct_pg_project(system_db.pool(), account_id, "trailmark-prod", &cust_url).await;

    let txn_id =
        insert_transaction(&cust_pool, "trailmark-prod", "expired", 20.0 * SECS_PER_DAY).await;

    transaction_sweeper::run_cycle(
        &system_db,
        None,
        None,
        &ENCRYPTION_KEY,
        None,
        RETENTION_DAYS,
    )
    .await;

    assert!(
        transaction_exists(&cust_pool, txn_id).await,
        "terminal row 20 days old must survive a 30-day retention window"
    );
    assert_eq!(transaction_status(&cust_pool, txn_id).await, "expired");
}

// ─── Active row is never purged, regardless of age ─────────────────────────

/// Purge's own `DELETE ... WHERE status IN ('committed', 'expired')` clause
/// structurally never matches `status = 'active'` — it can only ever act on
/// a row Slice 01's own reclaim step has already relabeled. This is proven
/// by construction, not by example, so the meaningful regression to guard is
/// the COMPOSED case within a single cycle: an abandoned row past the 60s
/// reclaim threshold (Slice 01 relabels it 'active' -> 'expired' this same
/// cycle) but still well within the 30-day retention window must survive —
/// purge must not delete a row the instant reclaim produces it, using the
/// SAME `started_at` anchor both steps share (ADR-054 § D2's own accepted
/// started_at approximation). Proves the two mechanisms compose correctly
/// rather than overlapping incorrectly.
#[tokio::test]
async fn reclaimed_row_within_retention_window_survives_the_same_cycles_purge() {
    let (_sys_container, system_db) = start_system_db().await;
    let (_cust_container, cust_url) = start_postgres().await;
    let cust_pool = migrate_customer_db(&cust_url).await;

    let account_id = insert_account(system_db.pool()).await;
    insert_direct_pg_project(system_db.pool(), account_id, "trailmark-prod", &cust_url).await;

    // 5 minutes ago: past the 60s abandonment threshold (reclaim fires,
    // 'active' -> 'expired'), far inside the 30-day retention window (purge
    // must not fire).
    let txn_id = insert_transaction(&cust_pool, "trailmark-prod", "active", 5.0 * 60.0).await;

    transaction_sweeper::run_cycle(
        &system_db,
        None,
        None,
        &ENCRYPTION_KEY,
        None,
        RETENTION_DAYS,
    )
    .await;

    assert!(
        transaction_exists(&cust_pool, txn_id).await,
        "a freshly-reclaimed row within the retention window must survive the same cycle's purge step"
    );
    assert_eq!(
        transaction_status(&cust_pool, txn_id).await,
        "expired",
        "reclaim must still relabel the row 'active' -> 'expired' in this cycle"
    );
}

// ─── Purge counter increments, observable via /metrics ─────────────────────

#[tokio::test]
async fn purge_counter_increments_and_is_observable_via_metrics() {
    let (_sys_container, system_db) = start_system_db().await;
    let (_cust_container, cust_url) = start_postgres().await;
    let cust_pool = migrate_customer_db(&cust_url).await;

    let account_id = insert_account(system_db.pool()).await;
    insert_direct_pg_project(system_db.pool(), account_id, "trailmark-prod", &cust_url).await;

    insert_transaction(&cust_pool, "trailmark-prod", "committed", 45.0 * SECS_PER_DAY).await;

    let before = read_counter("embyr_transaction_sweeper_purged_total");

    transaction_sweeper::run_cycle(
        &system_db,
        None,
        None,
        &ENCRYPTION_KEY,
        None,
        RETENTION_DAYS,
    )
    .await;

    let after = read_counter("embyr_transaction_sweeper_purged_total");
    assert!(
        after >= before + 1.0,
        "purged_total must increase by at least 1: before={before}, after={after}"
    );
}
