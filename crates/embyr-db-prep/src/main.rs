//! embyr-db-prep — customer-run standalone database-preparation binary.
//!
//! Lets a customer's DBA apply `migrations/customer/` against their own
//! Postgres, under their own elevated, database-scoped credentials, without
//! ever handing embyr's SaaS a DDL-capable connection string (JOB-15,
//! feature `customer-db-onboarding`, ADR-022, ADR-023).
//!
//! Mirrors `embyr-agent`'s wire -> probe -> use shape (`crates/embyr-agent/src/main.rs`).
//! Depends only on `embyr-pg-storage`, `embyr-core`, `sqlx`, `tokio` — no
//! tonic, no rustls-server, no aws-sdk, no axum (ADR-022 § Decision 2,
//! supply-chain minimization for a customer-run binary).
//!
//! DESIGN-specified sequence (`docs/product/architecture/brief.md` §
//! Component Decomposition — customer-db-onboarding):
//!   1. `DbPrepConfig::from_env()` — `EMBYR_DB_PREP_DSN` (required, elevated)
//!      + `EMBYR_DB_PREP_DML_ROLE_DSN` (optional, ADR-023 revised).
//!   2. Connectivity probe (SELECT 1, timeout-bound) — before migrate() is
//!      ever attempted (AC-01-05: distinct "connection failed" message).
//!   3. `PostgresBackendAdapter::migrate()` — the sole embed point for
//!      `migrations/customer/` workspace-wide (ADR-022).
//!   4. Classify the migrate result via `error_report::classify()` into one
//!      of 3 named message shapes (AC-01-01..04).
//!   5. If `EMBYR_DB_PREP_DML_ROLE_DSN` is present: open a brief connection
//!      with it, `discover_current_user()`, then
//!      `grant_schema_readiness_read(role_name)` against the elevated
//!      connection (ADR-023 revised). Absent/unreachable → skip with an
//!      informational (non-fatal) note; migration success reporting is
//!      unaffected.
//!
//! Step 01-01 (walking skeleton) implemented the happy-reachable path: config
//! parsing, a timeout-bound connect, migrate(), a before/after
//! `_sqlx_migrations` count comparison to distinguish "freshly applied" from
//! "already up to date", and the DML-role-absent informational note.
//! Step 01-02 added the connect-phase liveness probe and
//! `error_report::classify()` wiring (AC-01-04, AC-01-05): connect-phase
//! failures (pool open or probe) short-circuit to a hardcoded message and
//! never reach `classify()`; only a `migrate()`-phase failure does. Step
//! 05-01 wired the grant step: `discover_current_user()` +
//! `grant_schema_readiness_read()` — see
//! `tests/customer_db_onboarding/acceptance/cdo0{1..11}_*.rs`.

mod config;
mod error_report;

use std::time::Duration;

use embyr_core::storage::backend_adapter::BackendAdapter;
use embyr_pg_storage::backend_adapter::PostgresBackendAdapter;

#[tokio::main]
async fn main() {
    let cfg = match config::DbPrepConfig::from_env() {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    // Connect phase: pool open + a liveness probe (SELECT 1), both
    // timeout-bound. Any failure here (unreachable host, refused
    // connection, probe timeout) short-circuits to a hardcoded,
    // structurally distinct "connection failed" message -- migrate() and
    // error_report::classify() are NEVER reached from this branch, which is
    // what makes AC-01-05's negative assertion (no "privilege"/"permission
    // denied" wording) structurally guaranteed rather than incidental.
    let pool = match tokio::time::timeout(
        Duration::from_secs(10),
        sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .connect(&cfg.dsn),
    )
    .await
    {
        Ok(Ok(pool)) => pool,
        _ => connection_failed_and_exit(&cfg.dsn),
    };

    let adapter = PostgresBackendAdapter::new_from_pool(pool);

    match tokio::time::timeout(Duration::from_secs(10), adapter.probe()).await {
        Ok(Ok(())) => {}
        _ => connection_failed_and_exit(&cfg.dsn),
    }

    let applied_before = applied_migration_count(&adapter).await;

    // Migrate phase: the pool is open and the probe succeeded, so any
    // failure from here on is classified via error_report::classify() --
    // this is the only path that reaches it.
    if let Err(e) = adapter.migrate().await {
        eprintln!("embyr-db-prep: {}", error_report::classify(&e, &cfg.dsn));
        std::process::exit(1);
    }

    let applied_after = applied_migration_count(&adapter).await;

    if applied_before == applied_after && applied_before > 0 {
        println!("database already up to date (schema version {applied_after} applied)");
    } else {
        println!(
            "database ready for embyr onboarding (schema version {applied_after} of {applied_after} applied)"
        );
    }

    // collection-group-query-index (ADR-080 § Component Boundaries): after
    // migrate() succeeds, build the two collection-group indexes then
    // backfill collection_id for any pre-existing NULL rows. Both calls are
    // idempotent -- a no-op in practice for a fresh project (zero
    // pre-existing rows / indexes already present), real work for a DBA
    // re-running this tool against an existing, already-provisioned
    // database. Runs unconditionally, matching this codebase's existing
    // preference for one code path over a conditional one.
    if let Err(e) = adapter.ensure_collection_group_indexes().await {
        eprintln!("embyr-db-prep: failed to build collection-group indexes: {e}");
        std::process::exit(1);
    }
    match adapter
        .backfill_collection_id(
            cfg.backfill_batch_size,
            Duration::from_millis(cfg.backfill_throttle_ms),
        )
        .await
    {
        Ok(summary) => {
            println!(
                "collection_id backfill complete: {} documents backfilled",
                summary.rows_backfilled
            );
        }
        Err(e) => {
            eprintln!("embyr-db-prep: collection_id backfill failed: {e}");
            std::process::exit(1);
        }
    }

    match &cfg.dml_role_dsn {
        None => {
            println!(
                "note: EMBYR_DB_PREP_DML_ROLE_DSN not set -- read-verification access was not established"
            );
        }
        Some(dml_dsn) => {
            grant_dml_role_read_access(&adapter, dml_dsn).await;
        }
    }

    std::process::exit(0);
}

/// Discover the DML role's own name (via a brief connection using its own
/// DSN) and grant it `SELECT` on `_sqlx_migrations` from the elevated
/// connection (ADR-023 revised § Mechanism, step 4).
///
/// Best-effort: a connect failure against `dml_dsn` is treated identically
/// to the DSN-absent case (informational skip note, not a hard failure) --
/// the migration itself already succeeded by the time this runs, so a
/// transient DML-role connectivity problem must not fail the whole run.
async fn grant_dml_role_read_access(elevated_adapter: &PostgresBackendAdapter, dml_dsn: &str) {
    let dml_adapter = match tokio::time::timeout(
        Duration::from_secs(10),
        PostgresBackendAdapter::new(dml_dsn),
    )
    .await
    {
        Ok(Ok(adapter)) => adapter,
        _ => {
            println!(
                "note: could not connect using EMBYR_DB_PREP_DML_ROLE_DSN -- \
                 read-verification access was not established"
            );
            return;
        }
    };

    let role_name = match dml_adapter.discover_current_user().await {
        Ok(name) => name,
        Err(e) => {
            eprintln!("embyr-db-prep: failed to discover DML role name: {e}");
            return;
        }
    };
    drop(dml_adapter);

    match elevated_adapter.grant_schema_readiness_read(&role_name).await {
        Ok(()) => println!("read-verification access granted to role {role_name}"),
        Err(e) => eprintln!("embyr-db-prep: failed to grant read-verification access: {e}"),
    }
}

/// Print the hardcoded, structurally-distinct connect-phase failure message
/// and exit non-zero. Used for both a failed/timed-out pool open and a
/// failed/timed-out liveness probe -- neither ever calls
/// `error_report::classify()`, which is what makes AC-01-05's message
/// structurally incapable of containing privilege/permission wording.
fn connection_failed_and_exit(dsn: &str) -> ! {
    eprintln!("embyr-db-prep: connection failed: {} unreachable", host_from_dsn(dsn));
    std::process::exit(1);
}

/// Extract the `host[:port]` component from a `postgres://user:pass@host:port/db`
/// connection string, for use in the connect-phase failure message. Falls
/// back to the raw DSN if the shape is unexpected -- never panics.
fn host_from_dsn(dsn: &str) -> &str {
    let after_scheme = dsn.split_once("://").map_or(dsn, |(_, rest)| rest);
    let host_and_path = after_scheme
        .split_once('@')
        .map_or(after_scheme, |(_, rest)| rest);
    host_and_path.split('/').next().unwrap_or(host_and_path)
}

/// Count successfully-applied rows in sqlx's own `_sqlx_migrations`
/// bookkeeping table. Treats "relation does not exist" (fresh database,
/// table not yet created by `migrate()`) as a count of 0.
async fn applied_migration_count(adapter: &PostgresBackendAdapter) -> i64 {
    match sqlx::query_scalar::<_, i64>("SELECT count(*) FROM _sqlx_migrations WHERE success")
        .fetch_one(adapter.pool())
        .await
    {
        Ok(count) => count,
        Err(e) if e.to_string().contains("does not exist") => 0,
        Err(e) => panic!("embyr-db-prep: failed to query _sqlx_migrations: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_from_dsn_extracts_host_and_port() {
        assert_eq!(
            host_from_dsn("postgres://elena_dba:secret@pg-prod.example.internal:5432/meridian_embyr"),
            "pg-prod.example.internal:5432"
        );
    }

    #[test]
    fn host_from_dsn_falls_back_to_raw_dsn_when_shape_is_unexpected() {
        assert_eq!(host_from_dsn("not-a-dsn"), "not-a-dsn");
    }
}
