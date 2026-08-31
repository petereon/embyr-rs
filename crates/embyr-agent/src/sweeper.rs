//! Background transaction sweeper — deletes expired and terminal transaction records.
//!
//! Production: spawned by `server::run()` with a 30-second interval.
//! Tests: instantiated directly and driven via `sweep_once()`.

/// Background sweeper that periodically deletes expired and terminal transaction records.
///
/// Each sweep runs two independent policies:
///   1. Reclaim: `started_at < NOW() - ttl_secs * INTERVAL '1 second'` AND `status = 'active'`
///   2. Purge: `status IN ('committed', 'expired', 'rolled_back')` AND
///      `started_at < NOW() - retention_days * INTERVAL '1 day'` (ADR-058)
pub struct AgentTransactionSweeper {
    pool: sqlx::PgPool,
    ttl_secs: i64,
    retention_days: i64,
    interval: std::time::Duration,
}

impl AgentTransactionSweeper {
    /// Construct a new sweeper.
    ///
    /// - `pool`: Postgres connection pool
    /// - `ttl_secs`: transaction TTL in seconds (60 in production)
    /// - `retention_days`: terminal-status row retention window in days (30 in production)
    /// - `interval`: how often to run a sweep (30s in production, shorter in tests)
    pub fn new(
        pool: sqlx::PgPool,
        ttl_secs: i64,
        retention_days: i64,
        interval: std::time::Duration,
    ) -> Self {
        Self { pool, ttl_secs, retention_days, interval }
    }

    /// Run one sweep: reclaim abandoned active records, then purge terminal records
    /// past their retention window.
    ///
    /// Returns the total number of rows deleted (reclaimed + purged).
    pub async fn sweep_once(&self) -> Result<u64, sqlx::Error> {
        let reclaimed = sqlx::query(
            "DELETE FROM transactions \
             WHERE started_at < NOW() - $1 * INTERVAL '1 second' \
               AND status = 'active'",
        )
        .bind(self.ttl_secs)
        .execute(&self.pool)
        .await?
        .rows_affected();

        let purged = sqlx::query(
            "DELETE FROM transactions \
             WHERE status IN ('committed', 'expired', 'rolled_back') \
               AND started_at < NOW() - $1 * INTERVAL '1 day'",
        )
        .bind(self.retention_days)
        .execute(&self.pool)
        .await?
        .rows_affected();

        Ok(reclaimed + purged)
    }

    /// Spawn as a background tokio task that runs `sweep_once` on every `interval`.
    pub fn spawn(self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(self.interval).await;
                if let Err(e) = self.sweep_once().await {
                    tracing::warn!("transaction sweep failed: {e}");
                }
            }
        })
    }
}
