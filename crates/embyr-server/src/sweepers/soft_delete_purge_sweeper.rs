//! `SoftDeletePurgeSweeper` — background purge of the 3 sensitive
//! encrypted-credential columns (`ecies_encrypted_dsn`, `backend_pg_dsn_enc`,
//! `agent_tls_bundle_enc`) on a `'deleted'` project's own row, once the
//! configured grace window (default 168h/7 days, matching the admin-UI's own
//! unchanged promise) has elapsed (ADR-073, production-readiness-audit-
//! 2026-09-08.md finding #6).
//!
//! `SystemDb`-only — no customer-database connection, no DSN resolution
//! (unlike `TransactionSweeper`): the target columns live on the `projects`
//! row itself in the system DB. Mirrors `CapUsageRefresher`'s shape (ADR-073
//! Decision 2), not `TransactionSweeper`'s per-project connection loop.
//!
//! Idempotent by construction (ADR-073 Decision 1): the purge `UPDATE`'s own
//! `WHERE ... IS NOT NULL` guard means an already-purged row matches zero
//! rows on a later cycle — no new `purged_at` marker column.

use std::sync::Arc;
use std::time::Duration;

use crate::adapters::system_db::SystemDb;

use super::advisory_lock_key;

/// The `pg_try_advisory_lock` key namespace for this sweeper (ADR-073 §
/// Design Decisions D4) — distinct from `"embyr_cap_check"` and
/// `"embyr_transaction_sweep"`. `pub`: exposed so integration tests can prove
/// the real Postgres advisory-lock mechanism against this sweeper's own
/// actual lock key, mirroring `TransactionSweeper::LOCK_KEY_NAME`.
pub const LOCK_KEY_NAME: &str = "embyr_soft_delete_purge";

/// `EMBYR_ACCOUNT_DELETE_GRACE_DAYS` — account-deletion grace window, default 7
/// days (168h). Read from the environment where needed (handler + sweeper) so
/// no composition-root signature has to change; invalid values fall back to 7.
pub fn account_delete_grace_days() -> i64 {
    std::env::var("EMBYR_ACCOUNT_DELETE_GRACE_DAYS")
        .ok()
        .and_then(|v| v.parse::<i64>().ok())
        .filter(|d| *d >= 0)
        .unwrap_or(7)
}

/// Whether this instance should run the purge cycle: only when
/// `pg_try_advisory_lock` genuinely returned `true` — `Some(false)` (lock
/// held by another instance) and `None` (the probe query itself failed) both
/// mean "skip, retry next tick".
fn should_run_cycle(locked: Option<bool>) -> bool {
    locked == Some(true)
}

/// Spawn the `SoftDeletePurgeSweeper` background task. Returns a `JoinHandle`
/// the composition root should hold for the process lifetime (mirrors
/// `cap_usage_refresher::spawn`'s exact fire-and-forget shape).
pub fn spawn(
    system_db: Arc<SystemDb>,
    interval: Duration,
    grace_days: i64,
) -> tokio::task::JoinHandle<()> {
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

            if !should_run_cycle(locked) {
                // Lock held by another instance (or the probe query itself
                // failed) — skip this cycle, retry next tick.
                continue;
            }

            run_cycle(&system_db, grace_days).await;
            run_account_cycle(&system_db, account_delete_grace_days()).await;

            let _: Option<bool> = sqlx::query_scalar("SELECT pg_advisory_unlock($1)")
                .bind(lock_key)
                .fetch_one(&mut *lock_conn)
                .await
                .ok();
        }
    })
}

/// One purge cycle: a single `SystemDb`-only `UPDATE` nulling the three
/// sensitive credential columns on every `'deleted'` project row whose
/// `deleted_at` is at least `grace_days` in the past and which still has at
/// least one non-`NULL` sensitive column (ADR-073 § Design Decisions D6). No
/// `backend_mode` filter — every mode, including `agent`, is in scope.
///
/// `pub` (not `pub(crate)`): this cycle function IS the sweeper's own
/// testable driving port, mirroring `TransactionSweeper::run_cycle` — the
/// walking-skeleton test proves `spawn`'s interval/lock wrapping separately;
/// the remaining acceptance scenarios invoke this directly for
/// deterministic, fast assertions.
pub async fn run_cycle(system_db: &Arc<SystemDb>, grace_days: i64) {
    let cutoff = chrono::Utc::now() - chrono::Duration::days(grace_days);

    let result = sqlx::query(
        "UPDATE projects \
         SET ecies_encrypted_dsn = NULL, \
             backend_pg_dsn_enc = NULL, \
             agent_tls_bundle_enc = NULL, \
             agent_tls_bundle_srv_enc = NULL \
         WHERE status = 'deleted' \
           AND deleted_at < $1 \
           AND (ecies_encrypted_dsn IS NOT NULL \
                OR backend_pg_dsn_enc IS NOT NULL \
                OR agent_tls_bundle_enc IS NOT NULL \
                OR agent_tls_bundle_srv_enc IS NOT NULL)",
    )
    .bind(cutoff)
    .execute(system_db.pool())
    .await;

    match result {
        Ok(query_result) => {
            let purged = query_result.rows_affected();
            if purged > 0 {
                metrics::counter!("embyr_soft_delete_purge_sweeper_purged_total")
                    .increment(purged);
                tracing::info!(
                    rows_purged = purged,
                    "SoftDeletePurgeSweeper: purged sensitive credential columns for \
                     soft-deleted projects past grace window"
                );
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "SoftDeletePurgeSweeper: purge query failed");
            super::record_sweeper_error("soft_delete_purge_sweeper");
        }
    }
}

/// Dependent-row deletes for one soft-deleted account, in FK order. `$1` =
/// account id. The `accounts` row itself survives as a tombstone (projects keep
/// an FK to it): its name is blanked, which is also the "already purged" marker.
const ACCOUNT_PURGE_STEPS: [&str; 10] = [
    "DELETE FROM sessions WHERE account_id = $1",
    "DELETE FROM admin_api_keys WHERE account_id = $1",
    "DELETE FROM service_accounts WHERE account_id = $1",
    "DELETE FROM invitations WHERE account_id = $1",
    "DELETE FROM oidc_providers WHERE account_id = $1",
    "DELETE FROM subscriptions WHERE account_id = $1",
    "DELETE FROM mfa_recovery_codes WHERE user_id IN (SELECT id FROM users WHERE account_id = $1)",
    "DELETE FROM account_members WHERE account_id = $1",
    "DELETE FROM users WHERE account_id = $1",
    "UPDATE accounts SET name = '' WHERE id = $1",
];

/// Account-deletion purge cycle: every account soft-deleted more than
/// `grace_days` ago and not yet tombstoned (`name <> ''`) has its dependent
/// rows removed in one transaction. Idempotent; a failed account is retried
/// next cycle. `pub` for the same testable-driving-port reason as `run_cycle`.
pub async fn run_account_cycle(system_db: &Arc<SystemDb>, grace_days: i64) {
    let cutoff = embyr_core::admin::account_deletion::purge_cutoff(chrono::Utc::now(), grace_days);

    let due: Vec<uuid::Uuid> = match sqlx::query_scalar(
        "SELECT id FROM accounts WHERE deleted_at < $1 AND name <> ''",
    )
    .bind(cutoff)
    .fetch_all(system_db.pool())
    .await
    {
        Ok(ids) => ids,
        Err(e) => {
            tracing::warn!(error = %e, "SoftDeletePurgeSweeper: account purge query failed");
            super::record_sweeper_error("soft_delete_purge_sweeper");
            return;
        }
    };

    for account_id in due {
        match purge_account(system_db, account_id).await {
            Ok(()) => {
                metrics::counter!("embyr_account_purge_sweeper_purged_total").increment(1);
                tracing::info!(%account_id, "SoftDeletePurgeSweeper: purged deleted account");
            }
            Err(e) => {
                tracing::warn!(%account_id, error = %e, "SoftDeletePurgeSweeper: account purge failed");
                super::record_sweeper_error("soft_delete_purge_sweeper");
            }
        }
    }
}

async fn purge_account(system_db: &SystemDb, account_id: uuid::Uuid) -> Result<(), sqlx::Error> {
    let mut tx = system_db.pool().begin().await?;
    for stmt in ACCOUNT_PURGE_STEPS {
        sqlx::query(stmt).bind(account_id).execute(&mut *tx).await?;
    }
    tx.commit().await
}

#[cfg(test)]
mod tests {
    use super::should_run_cycle;

    #[test]
    fn only_a_genuinely_acquired_lock_runs_the_cycle() {
        assert!(should_run_cycle(Some(true)));
        assert!(!should_run_cycle(Some(false)));
        assert!(!should_run_cycle(None));
    }
}
