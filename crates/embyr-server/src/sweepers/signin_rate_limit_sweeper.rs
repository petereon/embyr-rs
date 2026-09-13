//! `SigninRateLimitSweeper` — background purge of stale `signin_rate_limits`
//! rows (admin-signin-hardening, ADR-076 § "signin_rate_limit_sweeper — new
//! 4th background sweeper").
//!
//! Unlike `rate_buckets` (row count bounded by provisioned-project count),
//! `signin_rate_limits` rows are created for *any* source IP that ever calls
//! the signin route — an attacker rotating source IPs would otherwise grow
//! this table without bound, becoming a storage-exhaustion vector in its own
//! right. Mirrors `SoftDeletePurgeSweeper`'s exact spawn/run_cycle/
//! advisory-lock shape.

use std::sync::Arc;
use std::time::Duration;

use crate::adapters::system_db::SystemDb;

use super::advisory_lock_key;

/// The `pg_try_advisory_lock` key namespace for this sweeper — distinct from
/// the three existing lock keys (`"embyr_cap_check"`,
/// `"embyr_transaction_sweep"`, `"embyr_soft_delete_purge"`).
pub const LOCK_KEY_NAME: &str = "embyr_signin_rate_limit_sweep";

/// 24h retention — comfortably beyond the bucket's own refill-to-full time
/// (well under 15 minutes at capacity=150/refill=10 per minute), so a
/// swept-then-reinserted row never hands a returning attacker more tokens
/// than a continuously-tracked row would have.
const RETENTION_HOURS: i64 = 24;

/// Whether this instance should run the purge cycle: only when
/// `pg_try_advisory_lock` genuinely returned `true`.
fn should_run_cycle(locked: Option<bool>) -> bool {
    locked == Some(true)
}

/// Spawn the `SigninRateLimitSweeper` background task. Returns a `JoinHandle`
/// the composition root should hold for the process lifetime (mirrors
/// `soft_delete_purge_sweeper::spawn`'s exact shape).
pub fn spawn(system_db: Arc<SystemDb>, interval: Duration) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(interval);
        loop {
            tick.tick().await;

            let lock_key = advisory_lock_key(LOCK_KEY_NAME);

            let Ok(mut lock_conn) = system_db.pool().acquire().await else {
                continue;
            };
            let locked: Option<bool> = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
                .bind(lock_key)
                .fetch_one(&mut *lock_conn)
                .await
                .ok();

            if !should_run_cycle(locked) {
                continue;
            }

            run_cycle(&system_db).await;

            let _: Option<bool> = sqlx::query_scalar("SELECT pg_advisory_unlock($1)")
                .bind(lock_key)
                .fetch_one(&mut *lock_conn)
                .await
                .ok();
        }
    })
}

/// One purge cycle: delete `signin_rate_limits` rows whose `last_refill` is
/// older than the retention window.
///
/// `pub` (not `pub(crate)`): this cycle function IS the sweeper's own
/// testable driving port, mirroring `soft_delete_purge_sweeper::run_cycle`.
pub async fn run_cycle(system_db: &Arc<SystemDb>) {
    let cutoff = chrono::Utc::now() - chrono::Duration::hours(RETENTION_HOURS);
    let result = sqlx::query("DELETE FROM signin_rate_limits WHERE last_refill < $1")
        .bind(cutoff)
        .execute(system_db.pool())
        .await;
    match result {
        Ok(qr) if qr.rows_affected() > 0 => {
            metrics::counter!("embyr_signin_rate_limit_sweeper_purged_total")
                .increment(qr.rows_affected());
            tracing::info!(
                rows_purged = qr.rows_affected(),
                "SigninRateLimitSweeper: purged stale rows"
            );
        }
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "SigninRateLimitSweeper: purge query failed"),
    }
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
