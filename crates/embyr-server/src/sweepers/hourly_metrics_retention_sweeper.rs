//! `HourlyMetricsRetentionSweeper` — hourly purge of `hourly_project_metrics`
//! rows older than the retention window (hourly-metrics-rollup, ADR-084).
//! Mirrors its sibling sweepers' spawn/run_cycle shape, minus the advisory lock.

use std::sync::Arc;
use std::time::Duration;

use crate::adapters::system_db::SystemDb;

/// Spawn the sweeper; the composition root holds the `JoinHandle`.
///
/// No advisory lock and a deferred first tick: the purge is one idempotent
/// `DELETE` (concurrent instances are harmless), so the sweeper never holds a
/// connection while waiting for a second one, and it does not join the
/// startup stampede of the other sweepers on the small system pool (5
/// sweepers each pinning a lock connection exhausted a 5-connection pool and
/// 500'd early requests).
pub fn spawn(
    system_db: Arc<SystemDb>,
    interval: Duration,
    retention_days: i64,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + interval, interval);
        loop {
            tick.tick().await;
            run_cycle(&system_db, retention_days).await;
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
