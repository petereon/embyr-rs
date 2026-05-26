use sqlx::PgPool;

/// Records per-project daily operation counts in the system DB.
///
/// Metrics are best-effort: failures are silently dropped to avoid
/// impacting the request path.
pub struct MetricsAdapter {
    pool: PgPool,
}

impl MetricsAdapter {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Increment read_ops by `count` for `project_id` on today's date.
    ///
    /// Uses an upsert so the first request of the day creates the row and
    /// subsequent requests accumulate into it.
    pub async fn record_read(&self, project_id: &str, count: i64) {
        let _ = sqlx::query(
            "INSERT INTO daily_project_metrics (project_id, date, read_ops)
             VALUES ($1, CURRENT_DATE, $2)
             ON CONFLICT (project_id, date) DO UPDATE SET
               read_ops = daily_project_metrics.read_ops + EXCLUDED.read_ops",
        )
        .bind(project_id)
        .bind(count)
        .execute(&self.pool)
        .await;
    }

    /// Increment write_ops by `count` for `project_id` on today's date.
    pub async fn record_write(&self, project_id: &str, count: i64) {
        let _ = sqlx::query(
            "INSERT INTO daily_project_metrics (project_id, date, write_ops)
             VALUES ($1, CURRENT_DATE, $2)
             ON CONFLICT (project_id, date) DO UPDATE SET
               write_ops = daily_project_metrics.write_ops + EXCLUDED.write_ops",
        )
        .bind(project_id)
        .bind(count)
        .execute(&self.pool)
        .await;
    }
}
