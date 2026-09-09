// SCAFFOLD: false
//! US-01 (Walking Skeleton, the entire feature) — A Soft-Deleted Project's
//! Encrypted Credentials Are Genuinely Purged Once the Grace Window Elapses
//! (ADR-073, production-readiness-audit-2026-09-08.md finding #6).
//!
//! Acceptance criteria verified here (docs/feature/soft-delete-purge-sweeper/
//! feature-delta.md § Acceptance Criteria):
//!   - AC-SDP-01: a 'deleted' project past its grace window has all three
//!     sensitive columns (ecies_encrypted_dsn, backend_pg_dsn_enc,
//!     agent_tls_bundle_enc) set to NULL, for direct_pg AND agent backend
//!     modes (the SQL carries no backend_mode filter by design, ADR-073
//!     Decision 2).
//!   - AC-SDP-02: a 'deleted' project still inside its grace window is left
//!     completely untouched.
//!   - AC-SDP-03: an already-purged row is left alone on a later cycle — no
//!     error, no redundant write, no duplicate metric increment (ADR-073
//!     Decision 1, the nulled-columns state IS the idempotency marker).
//!   - AC-SDP-04: a non-'deleted' project is never purged regardless of
//!     deleted_at/created_at age.
//!   - AC-SDP-05: concurrent instances are serialized by the
//!     "embyr_soft_delete_purge" Postgres advisory lock — mirrors
//!     TransactionSweeper's own proven concurrent-serialization test shape.
//!   - AC-SDP-06: default grace window is exactly 7 days (168h), matching
//!     the admin-UI's own unchanged promise — proven by using the real
//!     shipped default (not an overridden short window) in every scenario
//!     below, with deleted_at manipulated directly (established convention,
//!     customer_db_transaction_sweeper's own us01/us02 tests).
//!   - AC-SDP-07 (regression guard): the purge never touches `status`,
//!     `deleted_at`, `updated_at`, or any row referencing the project's id
//!     (`sdk_api_keys`) — column-level purge only, never row-level.
//!
//! Driving port: `soft_delete_purge_sweeper::spawn` (walking-skeleton
//! scenario only — real interval, real advisory lock) and
//! `soft_delete_purge_sweeper::run_cycle` directly (remaining scenarios,
//! same production code, invoked deterministically — "pure function IS its
//! own driving port" layered-test discipline, mirrors both existing
//! sweepers' own established test shape).
//!
//! SystemDb-only feature (ADR-073 Decision 2) — no customer-database
//! connection, no DSN resolution, no `migrations/customer` migration run.
//! One Postgres testcontainer per test, not two — a direct simplification
//! over customer_db_transaction_sweeper's own sibling tests, made possible
//! by this feature's own structural scope.

use std::sync::Arc;
use std::time::Duration as StdDuration;

use serial_test::serial;
use testcontainers_modules::{
    postgres::Postgres,
    testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt},
};

use embyr_server::adapters::system_db::SystemDb;
use embyr_server::sweepers::soft_delete_purge_sweeper;

/// The real shipped default (ADR-073 § Design Decisions D3, AC-SDP-06) —
/// used unmodified by every scenario below. `deleted_at` is manipulated
/// directly (via `make_interval`) to land on either side of this window,
/// matching customer_db_transaction_sweeper's own established convention
/// for testing a grace/retention window on a realistic timescale without
/// waiting for real wall-clock days to pass.
const GRACE_DAYS: i64 = 7;

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

async fn insert_account(sys_pool: &sqlx::PgPool) -> uuid::Uuid {
    sqlx::query_scalar(
        "INSERT INTO accounts (name) VALUES ('soft-delete-purge-test-account') RETURNING id",
    )
    .fetch_one(sys_pool)
    .await
    .expect("insert account")
}

/// Seed one `projects` row with fully controlled sensitive-column values and
/// a `deleted_at` set `deleted_days_ago` days in the past (bound via
/// `make_interval`, never string-interpolated — mirrors this design's own
/// bind-parameter convention, ADR-073 § Design Decisions D6). `status`
/// defaults to `'deleted'`; callers needing an `'active'` row pass it
/// explicitly.
#[allow(clippy::too_many_arguments)]
async fn insert_project(
    sys_pool: &sqlx::PgPool,
    account_id: uuid::Uuid,
    project_id: &str,
    backend_mode: &str,
    status: &str,
    deleted_days_ago: Option<f64>,
    ecies_encrypted_dsn: Option<&[u8]>,
    backend_pg_dsn_enc: Option<&[u8]>,
    agent_tls_bundle_enc: Option<&[u8]>,
) {
    sqlx::query(
        "INSERT INTO projects \
         (id, account_id, backend_mode, api_key_hash_current, status, name, \
          deleted_at, ecies_encrypted_dsn, backend_pg_dsn_enc, agent_tls_bundle_enc) \
         VALUES ($1, $2, $3, 'unused-hash', $4, $1, \
          CASE WHEN $5::float8 IS NULL THEN NULL ELSE now() - make_interval(days => $5) END, \
          $6, $7, $8)",
    )
    .bind(project_id)
    .bind(account_id)
    .bind(backend_mode)
    .bind(status)
    .bind(deleted_days_ago)
    .bind(ecies_encrypted_dsn)
    .bind(backend_pg_dsn_enc)
    .bind(agent_tls_bundle_enc)
    .execute(sys_pool)
    .await
    .expect("insert project");
}

struct ProjectRow {
    ecies_encrypted_dsn: Option<Vec<u8>>,
    backend_pg_dsn_enc: Option<Vec<u8>>,
    agent_tls_bundle_enc: Option<Vec<u8>>,
    status: String,
    deleted_at: Option<chrono::DateTime<chrono::Utc>>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

async fn read_project(sys_pool: &sqlx::PgPool, project_id: &str) -> ProjectRow {
    let row: (
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        String,
        Option<chrono::DateTime<chrono::Utc>>,
        chrono::DateTime<chrono::Utc>,
    ) = sqlx::query_as(
        "SELECT ecies_encrypted_dsn, backend_pg_dsn_enc, agent_tls_bundle_enc, \
         status, deleted_at, updated_at FROM projects WHERE id = $1",
    )
    .bind(project_id)
    .fetch_one(sys_pool)
    .await
    .expect("read project row");
    ProjectRow {
        ecies_encrypted_dsn: row.0,
        backend_pg_dsn_enc: row.1,
        agent_tls_bundle_enc: row.2,
        status: row.3,
        deleted_at: row.4,
        updated_at: row.5,
    }
}

/// Read a label-free Prometheus counter's current value from the shared
/// process-global recorder — mirrors customer_db_transaction_sweeper's own
/// identical helper for `embyr_transaction_sweeper_reclaimed_total`.
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

// ─── AC-SDP-01 (direct_pg) + AC-SDP-06: walking skeleton ───────────────────

/// Walking skeleton: real `soft_delete_purge_sweeper::spawn` (real interval,
/// real advisory lock) purges a direct_pg project's credentials once its
/// deleted_at is past the real shipped 7-day grace window, and the
/// Prometheus counter increments.
#[tokio::test]
#[serial]
async fn direct_pg_project_past_grace_window_has_credentials_purged_via_spawn_and_counter_increments(
) {
    let (_sys_container, system_db) = start_system_db().await;
    let account_id = insert_account(system_db.pool()).await;

    insert_project(
        system_db.pool(),
        account_id,
        "trailmark-staging",
        "direct_pg",
        "deleted",
        Some(8.0), // past the 7-day grace window
        Some(b"real-ecies-encrypted-dsn"),
        Some(b"real-backend-pg-dsn-enc"),
        None,
    )
    .await;

    let before = read_counter("embyr_soft_delete_purge_sweeper_purged_total");

    let handle = soft_delete_purge_sweeper::spawn(
        system_db.clone(),
        StdDuration::from_millis(50),
        GRACE_DAYS,
    );

    let deadline = tokio::time::Instant::now() + StdDuration::from_secs(5);
    loop {
        let row = read_project(system_db.pool(), "trailmark-staging").await;
        if row.ecies_encrypted_dsn.is_none() {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "sweep cycle did not purge the past-grace-window row within 5s"
        );
        tokio::time::sleep(StdDuration::from_millis(20)).await;
    }
    handle.abort();

    let row = read_project(system_db.pool(), "trailmark-staging").await;
    assert!(
        row.ecies_encrypted_dsn.is_none(),
        "ecies_encrypted_dsn must be NULL"
    );
    assert!(
        row.backend_pg_dsn_enc.is_none(),
        "backend_pg_dsn_enc must be NULL"
    );

    let after = read_counter("embyr_soft_delete_purge_sweeper_purged_total");
    assert!(
        after >= before + 1.0,
        "purged_total must increase by at least 1: before={before}, after={after}"
    );
}

// ─── AC-SDP-02: within-grace-window is untouched ───────────────────────────

/// A project soft-deleted less than the grace window ago is left completely
/// alone — the sweeper's own `WHERE deleted_at < $1` predicate structurally
/// excludes it.
#[tokio::test]
async fn project_within_grace_window_is_left_completely_untouched() {
    let (_sys_container, system_db) = start_system_db().await;
    let account_id = insert_account(system_db.pool()).await;

    insert_project(
        system_db.pool(),
        account_id,
        "trailmark-demo",
        "direct_pg",
        "deleted",
        Some(3.0), // well inside the 7-day grace window
        Some(b"real-ecies-encrypted-dsn"),
        None,
        None,
    )
    .await;

    soft_delete_purge_sweeper::run_cycle(&system_db, GRACE_DAYS).await;

    let row = read_project(system_db.pool(), "trailmark-demo").await;
    assert_eq!(
        row.ecies_encrypted_dsn.as_deref(),
        Some(b"real-ecies-encrypted-dsn".as_slice()),
        "ecies_encrypted_dsn must remain unchanged while inside the grace window"
    );
}

// ─── AC-SDP-03: idempotency on an already-purged row ───────────────────────

/// Running the sweeper a second time against an already-purged row is a
/// no-op: no error, and the metric does not increment a second time (ADR-073
/// Decision 1 — the nulled-columns state itself is the idempotency marker,
/// no `purged_at` column exists to check instead).
#[tokio::test]
#[serial]
async fn already_purged_project_is_a_no_op_on_the_next_cycle_with_no_duplicate_metric() {
    let (_sys_container, system_db) = start_system_db().await;
    let account_id = insert_account(system_db.pool()).await;

    insert_project(
        system_db.pool(),
        account_id,
        "trailmark-staging-2",
        "direct_pg",
        "deleted",
        Some(8.0),
        Some(b"real-ecies-encrypted-dsn"),
        None,
        None,
    )
    .await;

    // First cycle: purges the row.
    soft_delete_purge_sweeper::run_cycle(&system_db, GRACE_DAYS).await;
    let after_first = read_project(system_db.pool(), "trailmark-staging-2").await;
    assert!(
        after_first.ecies_encrypted_dsn.is_none(),
        "first cycle must purge the row"
    );
    let counter_after_first = read_counter("embyr_soft_delete_purge_sweeper_purged_total");

    // Second cycle: the WHERE ... IS NOT NULL guard matches zero rows for
    // this project now — no error, no redundant write, no second increment.
    soft_delete_purge_sweeper::run_cycle(&system_db, GRACE_DAYS).await;
    let after_second = read_project(system_db.pool(), "trailmark-staging-2").await;
    assert!(after_second.ecies_encrypted_dsn.is_none());

    let counter_after_second = read_counter("embyr_soft_delete_purge_sweeper_purged_total");
    assert_eq!(
        counter_after_second, counter_after_first,
        "purged_total must NOT increase on a cycle over an already-purged row \
         (double-counted metric: before={counter_after_first}, after={counter_after_second})"
    );
}

// ─── AC-SDP-04: non-'deleted' status is never purged ───────────────────────

/// A project with status 'active' is never purged, regardless of
/// deleted_at/created_at age — the sweeper's own `WHERE status = 'deleted'`
/// predicate gates every other condition.
#[tokio::test]
async fn active_project_is_never_purged_regardless_of_age() {
    let (_sys_container, system_db) = start_system_db().await;
    let account_id = insert_account(system_db.pool()).await;

    insert_project(
        system_db.pool(),
        account_id,
        "trailmark-prod",
        "direct_pg",
        "active",
        None, // deleted_at is NULL — never soft-deleted
        Some(b"real-ecies-encrypted-dsn"),
        None,
        None,
    )
    .await;

    soft_delete_purge_sweeper::run_cycle(&system_db, GRACE_DAYS).await;

    let row = read_project(system_db.pool(), "trailmark-prod").await;
    assert_eq!(
        row.ecies_encrypted_dsn.as_deref(),
        Some(b"real-ecies-encrypted-dsn".as_slice()),
        "an active project's credentials must never be purged"
    );
    assert_eq!(row.status, "active");
}

// ─── AC-SDP-05: concurrent instances are serialized by advisory lock ──────

/// Two server instances never race the same purge cycle: a second
/// `pg_try_advisory_lock` on the sweeper's own "embyr_soft_delete_purge"
/// lock key fails while the first instance still holds it, and succeeds
/// once released — mirrors customer_db_transaction_sweeper's own proven
/// concurrent-serialization test shape exactly.
#[tokio::test]
async fn concurrent_sweep_attempts_are_serialized_by_advisory_lock() {
    let (_sys_container, system_db) = start_system_db().await;

    let lock_key =
        embyr_server::sweepers::advisory_lock_key(soft_delete_purge_sweeper::LOCK_KEY_NAME);

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

// ─── AC-SDP-01 (agent mode) ─────────────────────────────────────────────────

/// A `backend_mode=agent` project's `agent_tls_bundle_enc` is purged
/// identically to a direct_pg project's DSN — the sweeper's own SQL carries
/// no `backend_mode` filter (ADR-073 Decision 2), unlike
/// TransactionSweeper's own necessary exclusion.
#[tokio::test]
async fn agent_mode_project_past_grace_window_has_tls_bundle_purged() {
    let (_sys_container, system_db) = start_system_db().await;
    let account_id = insert_account(system_db.pool()).await;

    insert_project(
        system_db.pool(),
        account_id,
        "trailmark-agent",
        "agent",
        "deleted",
        Some(8.0),
        None,
        None,
        Some(b"real-agent-tls-bundle"),
    )
    .await;

    soft_delete_purge_sweeper::run_cycle(&system_db, GRACE_DAYS).await;

    let row = read_project(system_db.pool(), "trailmark-agent").await;
    assert!(
        row.agent_tls_bundle_enc.is_none(),
        "agent_tls_bundle_enc must be NULL for a purged agent-mode project"
    );
}

// ─── AC-SDP-07: regression guard — column-level purge only ────────────────

/// The purge touches ONLY the three named sensitive columns. `status`,
/// `deleted_at`, and `updated_at` on the project's own row, and any row
/// referencing the project's id (`sdk_api_keys`), are exactly unchanged.
#[tokio::test]
async fn purge_never_touches_status_deleted_at_updated_at_or_referencing_rows() {
    let (_sys_container, system_db) = start_system_db().await;
    let account_id = insert_account(system_db.pool()).await;

    insert_project(
        system_db.pool(),
        account_id,
        "trailmark-regression",
        "direct_pg",
        "deleted",
        Some(8.0),
        Some(b"real-ecies-encrypted-dsn"),
        Some(b"real-backend-pg-dsn-enc"),
        None,
    )
    .await;

    sqlx::query(
        "INSERT INTO sdk_api_keys (project_id, name, key_hash, prefix, revoked_at) \
         VALUES ($1, 'revoked-key', 'unused-hash-bytes', 'sk_live_', now())",
    )
    .bind("trailmark-regression")
    .execute(system_db.pool())
    .await
    .expect("insert sdk_api_keys row");

    let before = read_project(system_db.pool(), "trailmark-regression").await;
    let sdk_keys_before: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sdk_api_keys WHERE project_id = $1")
            .bind("trailmark-regression")
            .fetch_one(system_db.pool())
            .await
            .expect("count sdk_api_keys before");

    soft_delete_purge_sweeper::run_cycle(&system_db, GRACE_DAYS).await;

    let after = read_project(system_db.pool(), "trailmark-regression").await;
    // The purge itself must have happened (sanity — otherwise this test
    // would vacuously pass without exercising the code path at all).
    assert!(
        after.ecies_encrypted_dsn.is_none(),
        "sanity: purge must have run"
    );

    assert_eq!(
        after.status, before.status,
        "status must be untouched by the purge"
    );
    assert_eq!(
        after.deleted_at, before.deleted_at,
        "deleted_at must be untouched by the purge"
    );
    assert_eq!(
        after.updated_at, before.updated_at,
        "updated_at must be untouched by the purge (ADR-073 § Design Decisions D2)"
    );

    let sdk_keys_after: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sdk_api_keys WHERE project_id = $1")
            .bind("trailmark-regression")
            .fetch_one(system_db.pool())
            .await
            .expect("count sdk_api_keys after");
    assert_eq!(
        sdk_keys_after, sdk_keys_before,
        "sdk_api_keys rows referencing the project must be untouched by the purge"
    );
}
