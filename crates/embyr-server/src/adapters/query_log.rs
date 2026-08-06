//! PostgresQueryLogAdapter — implements IQueryLogWriter.
//!
//! Bounded channel (1000) + background consumer task inserts into the
//! partitioned `query_logs` table. Fire-and-forget: a full channel silently
//! drops the entry. Insert failures are logged but never fatal to the caller.
//!
//! QueryLogSweeper: daily tokio task that acquires `pg_try_advisory_lock`
//! (lock id 0x656d627972716c73) before dropping expired partitions, preventing
//! concurrent sweep races across multiple embyr-server instances.

use std::sync::Arc;

use tokio::sync::mpsc;

use embyr_core::admin::query_log::{IQueryLogWriter, OperationType, OpStatus, QueryLogEntry};

use crate::adapters::system_db::SystemDb;

// ---------------------------------------------------------------------------
// PostgresQueryLogAdapter
// ---------------------------------------------------------------------------

/// Postgres-backed query log writer.
///
/// Receives entries via a bounded in-process channel and persists them to
/// the partitioned `query_logs` table from a background tokio task.
pub struct PostgresQueryLogAdapter {
    sender: mpsc::Sender<QueryLogEntry>,
}

impl PostgresQueryLogAdapter {
    /// Creates the adapter and spawns the background consumer task.
    pub fn new(system_db: Arc<SystemDb>) -> Self {
        let (tx, mut rx) = mpsc::channel::<QueryLogEntry>(1000);

        tokio::spawn(async move {
            while let Some(entry) = rx.recv().await {
                let op_str = match entry.op {
                    OperationType::Read => "read",
                    OperationType::Write => "write",
                    OperationType::Delete => "delete",
                    OperationType::Listen => "listen",
                    OperationType::RunQuery => "run_query",
                };
                let status_str = match entry.status {
                    OpStatus::Ok => "ok",
                    OpStatus::Error => "error",
                };
                let created_at = chrono::DateTime::<chrono::Utc>::from(entry.timestamp);
                let project_id_str = entry.project_id.to_string();
                let latency_ms = entry.latency_ms as i32;

                let pool = system_db.pool();

                // Resolve account_id — required by the query_logs schema.
                let account_id_result: Result<Option<uuid::Uuid>, _> =
                    sqlx::query_scalar("SELECT account_id FROM projects WHERE id = $1")
                        .bind(&project_id_str)
                        .fetch_optional(pool)
                        .await;

                let account_id = match account_id_result {
                    Ok(Some(id)) => id,
                    _ => continue, // project not found or DB error — drop entry
                };

                // Ensure the daily partition exists before inserting.
                let date = created_at.format("%Y_%m_%d").to_string();
                let date_str = created_at.format("%Y-%m-%d").to_string();
                let next_date = (created_at + chrono::Duration::days(1))
                    .format("%Y-%m-%d")
                    .to_string();
                let partition_name = format!("query_logs_{date}");

                let create_partition_sql = format!(
                    "CREATE TABLE IF NOT EXISTS {partition_name} PARTITION OF query_logs \
                     FOR VALUES FROM ('{date_str}') TO ('{next_date}')"
                );

                if let Err(e) = sqlx::query(&create_partition_sql).execute(pool).await {
                    // Non-fatal: concurrent creation race — try the insert anyway.
                    tracing::warn!("create partition {partition_name}: {e}");
                }

                let result = sqlx::query(
                    "INSERT INTO query_logs \
                     (project_id, account_id, op, collection_path, latency_ms, status, created_at) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7)",
                )
                .bind(&project_id_str)
                .bind(account_id)
                .bind(op_str)
                .bind(&entry.collection_path)
                .bind(latency_ms)
                .bind(status_str)
                .bind(created_at)
                .execute(pool)
                .await;

                if let Err(e) = result {
                    tracing::warn!(
                        "query_log insert failed for project {project_id_str}: {e}"
                    );
                }
            }
        });

        Self { sender: tx }
    }
}

impl IQueryLogWriter for PostgresQueryLogAdapter {
    fn record(&self, entry: QueryLogEntry) {
        // Fire-and-forget: if the channel is full, drop silently.
        let _ = self.sender.try_send(entry);
    }
}

// ---------------------------------------------------------------------------
// QueryLogSweeper
// ---------------------------------------------------------------------------

/// Daily tokio task that drops expired `query_logs_*` partitions.
///
/// Acquires `pg_try_advisory_lock(0x656d627972716c73)` before dropping
/// partitions to prevent concurrent sweep races across instances.
pub struct QueryLogSweeper {
    system_db: Arc<SystemDb>,
}

impl QueryLogSweeper {
    pub fn new(system_db: Arc<SystemDb>) -> Self {
        Self { system_db }
    }

    /// Spawns the background daily sweep task. Call once at startup.
    pub fn start(self) {
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(86_400));
            loop {
                interval.tick().await;
                self.run_sweep().await;
            }
        });
    }

    async fn run_sweep(&self) {
        // Advisory lock id — "embyRqls" encoded as little-endian i64.
        const SWEEP_ADVISORY_LOCK: i64 = 0x656d627972716c73_i64;

        let pool = self.system_db.pool();

        let locked: bool =
            sqlx::query_scalar::<_, bool>("SELECT pg_try_advisory_lock($1)")
                .bind(SWEEP_ADVISORY_LOCK)
                .fetch_one(pool)
                .await
                .unwrap_or(false);

        if !locked {
            tracing::debug!("query_log sweep: advisory lock not acquired, skipping");
            return;
        }

        // Max retention across all live projects (default 90 days when none set).
        let max_retention: Option<i32> = sqlx::query_scalar::<_, Option<i32>>(
            "SELECT MAX(COALESCE(log_retention_days, 90)) \
             FROM projects WHERE status != 'deleted'",
        )
        .fetch_one(pool)
        .await
        .unwrap_or(None);

        let retention_days = max_retention.unwrap_or(90);
        let cutoff =
            chrono::Utc::now() - chrono::Duration::days(retention_days as i64);
        let cutoff_date = cutoff.format("%Y_%m_%d").to_string();

        // Find all query_logs_* partitions whose date prefix is older than cutoff.
        let partitions: Vec<String> = sqlx::query_scalar(
            "SELECT tablename FROM pg_tables \
             WHERE schemaname = 'public' \
               AND tablename LIKE 'query_logs_%' \
               AND tablename < $1",
        )
        .bind(format!("query_logs_{cutoff_date}"))
        .fetch_all(pool)
        .await
        .unwrap_or_default();

        for partition in partitions {
            if !partition.starts_with("query_logs_") {
                continue; // extra safety guard
            }
            let drop_sql = format!("DROP TABLE IF EXISTS {partition}");
            match sqlx::query(&drop_sql).execute(pool).await {
                Ok(_) => {
                    tracing::info!("dropped expired query_log partition: {partition}");
                }
                Err(e) => {
                    tracing::error!("failed to drop partition {partition}: {e}");
                }
            }
        }

        // Release the advisory lock.
        let _ = sqlx::query_scalar::<_, bool>("SELECT pg_advisory_unlock($1)")
            .bind(SWEEP_ADVISORY_LOCK)
            .fetch_one(pool)
            .await;
    }
}
