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
//! Step 01-01 (walking skeleton) implements the happy-reachable path: config
//! parsing, a timeout-bound connect, migrate(), a before/after
//! `_sqlx_migrations` count comparison to distinguish "freshly applied" from
//! "already up to date", and the DML-role-absent informational note.
//! Connection-failure classification (`error_report::classify()`) and the
//! grant step are filled in by later steps (01-02, 05-01) — see
//! `tests/customer_db_onboarding/acceptance/cdo0{1..11}_*.rs`.

mod config;
mod error_report;

use std::time::Duration;

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

    let pool = match tokio::time::timeout(
        Duration::from_secs(10),
        sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .connect(&cfg.dsn),
    )
    .await
    {
        Ok(Ok(pool)) => pool,
        Ok(Err(_)) => {
            // Connection-failure classification (distinct message shapes)
            // is 01-02's scope. This step only needs the happy-reachable
            // path to succeed without hanging.
            eprintln!("embyr-db-prep: failed to connect to the database");
            std::process::exit(1);
        }
        Err(_) => {
            eprintln!("embyr-db-prep: connection attempt timed out");
            std::process::exit(1);
        }
    };

    let adapter = PostgresBackendAdapter::new_from_pool(pool);

    let applied_before = applied_migration_count(&adapter).await;

    if let Err(e) = adapter.migrate().await {
        // Full presentable classification is 01-02's scope
        // (error_report::classify()) — not needed for this step's ATs.
        eprintln!("embyr-db-prep: migration failed: {e}");
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

    match cfg.dml_role_dsn {
        None => {
            println!(
                "note: EMBYR_DB_PREP_DML_ROLE_DSN not set -- read-verification access was not established"
            );
        }
        Some(_) => {
            // TODO(05-01): discover_current_user() + grant_schema_readiness_read().
        }
    }

    std::process::exit(0);
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
