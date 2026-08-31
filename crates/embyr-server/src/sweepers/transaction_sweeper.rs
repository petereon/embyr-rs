//! `TransactionSweeper` — background task proactively reclaiming abandoned
//! `'active'` `transactions` rows across every PG-reachable customer database
//! (ADR-054, customer-db-transaction-sweeper Slice 01/US-01).
//!
//! Closes a confirmed resource leak: `begin_transaction` inserts a
//! `transactions` row; only a LATER call referencing the same
//! `transaction_id` ever reactively expires it
//! (`crates/embyr-pg-storage/src/backend_adapter.rs::commit_transaction`,
//! its own 60-second check). `firestore-batch-write`'s own per-write loop
//! (ADR-048) synthesizes a fresh `transaction_id` per write, so that reactive
//! check structurally never fires for it — this sweeper generalizes the
//! identical semantic proactively, cross-project.
//!
//! Shape mirrors `CapUsageRefresher` (ADR-020) almost exactly — interval
//! loop + one cycle-level `pg_try_advisory_lock`/`pg_advisory_unlock` pair
//! held on a single `PoolConnection` for session affinity — with one
//! wrinkle: the inner loop body connects to a DIFFERENT customer database
//! per iteration instead of querying `SystemDb` only (ADR-054 § D5).
//!
//! Raw SQL against `transactions` runs directly against
//! `PostgresBackendAdapter::pool()` (ADR-054 § D1) — `transactions` sits
//! outside `BackendAdapter`'s own document-CRUD trait surface, so no trait
//! method is added; zero `AgentBackendAdapter` change of any kind, since
//! `backend_mode = 'agent'` is excluded at the `SystemDb` enumeration query
//! itself (`SystemDb::list_pg_reachable_projects`), never per-adapter.

use std::sync::Arc;
use std::time::Duration;

use crate::adapters::aws_secret_fetcher::AwsSecretFetcher;
use crate::adapters::encryption::decrypt_with_rotation;
use crate::adapters::gcp_secret_fetcher::GcpSecretFetcher;
use crate::adapters::postgres_backend::PostgresBackendAdapter;
use crate::adapters::system_db::{SweeperProjectRow, SystemDb};

use super::advisory_lock_key;

/// `commit_transaction`'s own reactive-expiry window
/// (`crates/embyr-pg-storage/src/backend_adapter.rs::commit_transaction`,
/// `chrono::Duration::seconds(60)`) — deliberately identical, not invented
/// (ADR-054 § D2). A compile-time constant, not an env var: making it
/// independently configurable would let it drift out of lockstep with that
/// existing check, silently breaking the "proactive generalization of an
/// already-established semantic" design intent.
const ABANDONMENT_THRESHOLD_SECS: i64 = 60;

/// The `pg_try_advisory_lock` key namespace for this sweeper (ADR-054 § D5)
/// — distinct from `CapUsageRefresher`'s own `"embyr_cap_check"`. `pub`
/// (not private): exposed so integration tests can prove the real Postgres
/// advisory-lock mechanism against the sweeper's own actual lock key,
/// without duplicating the FNV-1a computation in test code.
pub const LOCK_KEY_NAME: &str = "embyr_transaction_sweep";

/// Spawn the `TransactionSweeper` background task. Returns a `JoinHandle`
/// the composition root should hold for the process lifetime (mirrors
/// `cap_usage_refresher::spawn`'s exact fire-and-forget shape).
///
/// Registers (but never increments here) `embyr_transaction_sweeper_purged_total`
/// — Slice 01's own scope is reclaim only; the purge counter is registered
/// now so it always appears on `/metrics` from process start, incremented
/// starting in Slice 02.
pub fn spawn(
    system_db: Arc<SystemDb>,
    aws_secret_fetcher: Option<Arc<AwsSecretFetcher>>,
    gcp_secret_fetcher: Option<Arc<GcpSecretFetcher>>,
    encryption_key: [u8; 32],
    encryption_key_previous: Option<[u8; 32]>,
    interval: Duration,
) -> tokio::task::JoinHandle<()> {
    metrics::counter!("embyr_transaction_sweeper_purged_total").increment(0);

    tokio::spawn(async move {
        let mut tick = tokio::time::interval(interval);
        loop {
            tick.tick().await;

            let lock_key = advisory_lock_key(LOCK_KEY_NAME);

            // Session-affinity requirement identical to CapUsageRefresher's
            // own — see that module's doc comment for the full rationale.
            let Ok(mut lock_conn) = system_db.pool().acquire().await else {
                continue;
            };
            let locked: Option<bool> = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
                .bind(lock_key)
                .fetch_one(&mut *lock_conn)
                .await
                .ok();

            if locked != Some(true) {
                // Lock held by another instance — skip this cycle, retry
                // next tick (ADR-054 § D5: not required for correctness,
                // the reclaim SQL is idempotent; avoids redundant work).
                continue;
            }

            run_cycle(
                &system_db,
                aws_secret_fetcher.as_deref(),
                gcp_secret_fetcher.as_deref(),
                &encryption_key,
                encryption_key_previous.as_ref(),
            )
            .await;

            let _: Option<bool> = sqlx::query_scalar("SELECT pg_advisory_unlock($1)")
                .bind(lock_key)
                .fetch_one(&mut *lock_conn)
                .await
                .ok();
        }
    })
}

/// One sweep cycle: enumerate every PG-reachable project
/// (`SystemDb::list_pg_reachable_projects`), then sweep each one in turn —
/// sequentially, never concurrently (ADR-054 § D5: satisfies DISCUSS's own
/// "never a full concurrent fan-out" constraint as a direct consequence of
/// this plain loop shape, not an extra guard). A connect failure, query
/// failure, or unresolvable DSN for one project never aborts the cycle for
/// the rest (`sweep_one_project` never returns an error).
///
/// `pub` (not `pub(crate)`): this cycle function IS the sweeper's own
/// testable driving port — `spawn`'s own interval/lock wrapping is proven
/// separately by the walking-skeleton test; the remaining acceptance
/// scenarios invoke this directly for deterministic, fast assertions
/// (mirrors "pure domain function IS its own driving port").
pub async fn run_cycle(
    system_db: &Arc<SystemDb>,
    aws_secret_fetcher: Option<&AwsSecretFetcher>,
    gcp_secret_fetcher: Option<&GcpSecretFetcher>,
    encryption_key: &[u8; 32],
    encryption_key_previous: Option<&[u8; 32]>,
) {
    let projects = system_db
        .list_pg_reachable_projects()
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "TransactionSweeper: failed to enumerate PG-reachable projects");
            Vec::new()
        });

    for project in &projects {
        sweep_one_project(
            project,
            aws_secret_fetcher,
            gcp_secret_fetcher,
            encryption_key,
            encryption_key_previous,
        )
        .await;
    }
}

/// Resolve `project`'s DSN, connect, and reclaim its orphaned `'active'`
/// rows. Every failure path (unresolvable DSN, connect failure, query
/// failure) logs a `tracing::warn!` and returns — never aborts the cycle for
/// other projects (ADR-054 § D5, mirrors `CapUsageRefresher`'s own
/// per-account continue-on-error discipline).
async fn sweep_one_project(
    project: &SweeperProjectRow,
    aws_secret_fetcher: Option<&AwsSecretFetcher>,
    gcp_secret_fetcher: Option<&GcpSecretFetcher>,
    encryption_key: &[u8; 32],
    encryption_key_previous: Option<&[u8; 32]>,
) {
    let Some(dsn) = resolve_dsn_without_api_key(
        project,
        aws_secret_fetcher,
        gcp_secret_fetcher,
        encryption_key,
        encryption_key_previous,
    )
    .await
    else {
        return;
    };

    let adapter = match PostgresBackendAdapter::new(&dsn).await {
        Ok(adapter) => adapter,
        Err(e) => {
            tracing::warn!(
                project_id = %project.id,
                error = %e,
                "TransactionSweeper: failed to connect to customer database"
            );
            return;
        }
    };

    // ADR-054 § D2: no bind parameters — the threshold is a compile-time
    // constant, deliberately identical to `commit_transaction`'s own
    // hardcoded 60s check. Interpolated (not bound) so the SQL literal can
    // never drift from `ABANDONMENT_THRESHOLD_SECS`.
    let reclaim_sql = format!(
        "UPDATE transactions SET status = 'expired' \
         WHERE status = 'active' AND started_at < now() - interval '{ABANDONMENT_THRESHOLD_SECS} seconds'"
    );
    let result = sqlx::query(&reclaim_sql).execute(adapter.pool()).await;

    match result {
        Ok(query_result) => {
            let reclaimed = query_result.rows_affected();
            if reclaimed > 0 {
                metrics::counter!("embyr_transaction_sweeper_reclaimed_total")
                    .increment(reclaimed);
            }
        }
        Err(e) => {
            tracing::warn!(
                project_id = %project.id,
                error = %e,
                "TransactionSweeper: reclaim query failed"
            );
        }
    }
}

/// Resolve `project`'s customer-DB DSN without ever holding a live api_key
/// (ADR-054 § D3, DISCUSS § System Constraints). `None` on any failure
/// (missing arn/resource-name, fetcher not configured, fetch error,
/// `backend_pg_dsn_enc IS NULL`, decrypt/AEAD failure, malformed UTF-8) —
/// every `None` path has already logged a `tracing::warn!` naming the
/// reason, so the caller can skip silently.
async fn resolve_dsn_without_api_key(
    project: &SweeperProjectRow,
    aws_secret_fetcher: Option<&AwsSecretFetcher>,
    gcp_secret_fetcher: Option<&GcpSecretFetcher>,
    encryption_key: &[u8; 32],
    encryption_key_previous: Option<&[u8; 32]>,
) -> Option<String> {
    match project.backend_mode.as_str() {
        "aws_secret" => resolve_aws_secret_dsn(project, aws_secret_fetcher).await,
        "gcp_secret" => resolve_gcp_secret_dsn(project, gcp_secret_fetcher).await,
        "direct_pg" => resolve_direct_pg_dsn(project, encryption_key, encryption_key_previous),
        other => {
            tracing::warn!(
                project_id = %project.id,
                backend_mode = %other,
                "TransactionSweeper: unreachable backend_mode in enumeration result"
            );
            None
        }
    }
}

async fn resolve_aws_secret_dsn(
    project: &SweeperProjectRow,
    aws_secret_fetcher: Option<&AwsSecretFetcher>,
) -> Option<String> {
    let Some(arn) = project.backend_secret_arn.as_deref() else {
        tracing::warn!(project_id = %project.id, "TransactionSweeper: aws_secret project missing backend_secret_arn, skipping");
        return None;
    };
    let Some(fetcher) = aws_secret_fetcher else {
        tracing::warn!(project_id = %project.id, "TransactionSweeper: aws_secret_fetcher not configured, skipping");
        return None;
    };
    match fetcher.get_dsn(arn).await {
        Ok(dsn) => Some(dsn),
        Err(e) => {
            tracing::warn!(project_id = %project.id, error = %e, "TransactionSweeper: aws secret fetch failed, skipping");
            None
        }
    }
}

async fn resolve_gcp_secret_dsn(
    project: &SweeperProjectRow,
    gcp_secret_fetcher: Option<&GcpSecretFetcher>,
) -> Option<String> {
    let Some(resource_name) = project.backend_secret_gcp.as_deref() else {
        tracing::warn!(project_id = %project.id, "TransactionSweeper: gcp_secret project missing backend_secret_gcp, skipping");
        return None;
    };
    let Some(fetcher) = gcp_secret_fetcher else {
        tracing::warn!(project_id = %project.id, "TransactionSweeper: gcp_secret_fetcher not configured, skipping");
        return None;
    };
    match fetcher.get_dsn(resource_name).await {
        Ok(dsn) => Some(dsn),
        Err(e) => {
            tracing::warn!(project_id = %project.id, error = %e, "TransactionSweeper: gcp secret fetch failed, skipping");
            None
        }
    }
}

fn resolve_direct_pg_dsn(
    project: &SweeperProjectRow,
    encryption_key: &[u8; 32],
    encryption_key_previous: Option<&[u8; 32]>,
) -> Option<String> {
    let Some(enc) = project.backend_pg_dsn_enc.as_deref() else {
        // ADR-055: accepted, documented coverage gap — silent skip, not an error.
        tracing::warn!(project_id = %project.id, "TransactionSweeper: direct_pg project has backend_pg_dsn_enc IS NULL, skipping (ADR-055)");
        return None;
    };
    let plaintext = match decrypt_with_rotation(encryption_key, encryption_key_previous, enc) {
        Ok(bytes) => bytes,
        Err(e) => {
            tracing::warn!(project_id = %project.id, error = %e, "TransactionSweeper: backend_pg_dsn_enc decrypt failed, skipping");
            return None;
        }
    };
    match String::from_utf8(plaintext) {
        Ok(dsn) => Some(dsn),
        Err(_) => {
            tracing::warn!(project_id = %project.id, "TransactionSweeper: decrypted DSN is not valid UTF-8, skipping");
            None
        }
    }
}
