//! `HourlyMetricsRetentionSweeper` — hourly purge of `hourly_project_metrics`
//! rows older than the retention window (hourly-metrics-rollup, ADR-084).
//! Mirrors `SigninRateLimitSweeper`'s spawn/run_cycle/advisory-lock shape.

use std::sync::Arc;
use std::time::Duration;

use crate::adapters::system_db::SystemDb;

use super::advisory_lock_key;

/// The `pg_try_advisory_lock` key namespace for this sweeper.
pub const LOCK_KEY_NAME: &str = "embyr_hourly_metrics_retention";

/// Whether this instance should run the purge cycle: only when
/// `pg_try_advisory_lock` genuinely returned `true`.
fn should_run_cycle(locked: Option<bool>) -> bool {
    locked == Some(true)
}

/// Spawn the sweeper; the composition root holds the `JoinHandle`.
pub fn spawn(
    system_db: Arc<SystemDb>,
    interval: Duration,
    retention_days: i64,
) -> tokio::task::JoinHandle<()> {
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

            run_cycle(&system_db, retention_days).await;

            let _: Option<bool> = sqlx::query_scalar("SELECT pg_advisory_unlock($1)")
                .bind(lock_key)
                .fetch_one(&mut *lock_conn)
                .await
                .ok();
        }
    })
}

/// One purge cycle: delete rows whose `hour` is older than `retention_days`.
///
/// `pub`: this cycle function IS the sweeper's testable driving port.
pub async fn run_cycle(system_db: &Arc<SystemDb>, retention_days: i64) {
    let cutoff = chrono::Utc::now() - chrono::Duration::days(retention_days);
    let result = sqlx::query("DELETE FROM hourly_project_metrics WHERE hour < $1")
        .bind(cutoff)
        .execute(system_db.pool())
        .await;

    match result {
        Ok(done) => {
            let purged = done.rows_affected();
            if purged > 0 {
                metrics::counter!("embyr_hourly_metrics_retention_purged_total").increment(purged);
                tracing::info!(
                    rows_purged = purged,
                    "HourlyMetricsRetentionSweeper: purged expired rollup rows"
                );
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "HourlyMetricsRetentionSweeper: purge query failed");
            super::record_sweeper_error("hourly_metrics_retention_sweeper");
        }
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
