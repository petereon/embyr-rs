// SCAFFOLD: false
//! US-02 item 6 — `SigninRateLimitSweeper` genuinely bounds
//! `signin_rate_limits` table growth against an IP-rotating attacker
//! (admin-signin-hardening, ADR-076 § "signin_rate_limit_sweeper — new 4th
//! background sweeper").
//!
//! DISTILL's own explicit deferral (feature-delta.md § Deferred Test —
//! Sweeper) reserved this test for DELIVER, mirroring
//! `tests/soft_delete_purge_sweeper/acceptance/us01_purge_credentials_after_grace_window.rs`'s
//! own shape: `start_system_db` helper pattern, `run_cycle` direct-invocation
//! convention, concurrent-serialization test on the sweeper's own advisory
//! lock key.
//!
//! Driving port: `signin_rate_limit_sweeper::run_cycle` — the pure function
//! IS its own driving port (layered-test discipline, mirrors both existing
//! sweepers' own established test shape).

use std::sync::Arc;

use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt},
};

use embyr_server::adapters::system_db::SystemDb;
use embyr_server::sweepers::signin_rate_limit_sweeper;

async fn start_system_db() -> (ContainerAsync<Postgres>, Arc<SystemDb>) {
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
    let db = Arc::new(SystemDb::new(&url).await.expect("SystemDb::new failed"));
    db.migrate().await.expect("system DB migrations failed");
    (container, db)
}

/// Seed one `signin_rate_limits` row with `last_refill` set `hours_ago` hours
/// in the past (bound via `make_interval`, never string-interpolated).
async fn insert_bucket_row(sys_pool: &sqlx::PgPool, source_key: &str, hours_ago: f64) {
    sqlx::query(
        "INSERT INTO signin_rate_limits (source_key, tokens, last_refill) \
         VALUES ($1, 150.0, now() - make_interval(hours => $2::int))",
    )
    .bind(source_key)
    .bind(hours_ago)
    .execute(sys_pool)
    .await
    .expect("insert signin_rate_limits row");
}

async fn row_exists(sys_pool: &sqlx::PgPool, source_key: &str) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM signin_rate_limits WHERE source_key = $1)",
    )
    .bind(source_key)
    .fetch_one(sys_pool)
    .await
    .expect("query row existence")
}

// ─── Stale rows older than 24h retention are purged ────────────────────────

#[tokio::test]
async fn stale_rows_older_than_retention_are_purged() {
    let (_container, system_db) = start_system_db().await;

    insert_bucket_row(system_db.pool(), "203.0.113.1", 25.0).await;

    signin_rate_limit_sweeper::run_cycle(&system_db).await;

    assert!(
        !row_exists(system_db.pool(), "203.0.113.1").await,
        "a row older than the 24h retention window must be purged"
    );
}

// ─── Fresh rows within retention are untouched ─────────────────────────────

#[tokio::test]
async fn fresh_rows_within_retention_are_untouched() {
    let (_container, system_db) = start_system_db().await;

    insert_bucket_row(system_db.pool(), "203.0.113.2", 1.0).await;

    signin_rate_limit_sweeper::run_cycle(&system_db).await;

    assert!(
        row_exists(system_db.pool(), "203.0.113.2").await,
        "a row well inside the 24h retention window must be left untouched"
    );
}

// ─── Concurrent instances are serialized by advisory lock ──────────────────

#[tokio::test]
async fn concurrent_sweep_attempts_are_serialized_by_advisory_lock() {
    let (_container, system_db) = start_system_db().await;

    let lock_key =
        embyr_server::sweepers::advisory_lock_key(signin_rate_limit_sweeper::LOCK_KEY_NAME);

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
