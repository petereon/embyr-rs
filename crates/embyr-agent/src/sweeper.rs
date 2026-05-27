//! Background transaction sweeper — deletes expired transaction records.
//!
//! Production: spawned by `server::run()` with a 30-second interval.
//! Tests: instantiated directly and driven via `sweep_once()`.

/// Background sweeper that periodically deletes expired transaction records.
///
/// A transaction is considered expired when:
///   `started_at < NOW() - ttl_secs * INTERVAL '1 second'` AND `status = 'active'`
pub struct AgentTransactionSweeper {
    pool: sqlx::PgPool,
    ttl_secs: i64,
    interval: std::time::Duration,
}

impl AgentTransactionSweeper {
    /// Construct a new sweeper.
    ///
    /// - `pool`: Postgres connection pool
    /// - `ttl_secs`: transaction TTL in seconds (60 in production)
    /// - `interval`: how often to run a sweep (30s in production, shorter in tests)
    pub fn new(pool: sqlx::PgPool, ttl_secs: i64, interval: std::time::Duration) -> Self {
        Self { pool, ttl_secs, interval }
    }

    /// Run one sweep: delete active transaction records whose TTL has elapsed.
    ///
    /// Returns the number of rows deleted.
    pub async fn sweep_once(&self) -> Result<u64, sqlx::Error> {
        let result = sqlx::query(
            "DELETE FROM transactions \
             WHERE started_at < NOW() - $1 * INTERVAL '1 second' \
               AND status = 'active'",
        )
        .bind(self.ttl_secs)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
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
