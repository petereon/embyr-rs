//! PostgresQueryLogAdapter — implements IQueryLogWriter.
//!
//! Bounded channel (`QUERY_LOG_CHANNEL_CAPACITY`) + background consumer task
//! inserts into the partitioned `query_logs` table. Fire-and-forget: a full
//! channel silently drops the entry. Insert failures are logged but never fatal
//! to the caller.
//!
//! QueryLogSweeper: daily tokio task that acquires `pg_try_advisory_lock`
//! (lock id `SWEEP_ADVISORY_LOCK`) before dropping expired partitions and
//! row-purging projects whose own `log_retention_days` is shorter than the
//! partition horizon, preventing concurrent sweep races across multiple
//! embyr-server instances.

use std::sync::Arc;

use tokio::sync::mpsc;

use embyr_core::admin::query_log::{IQueryLogWriter, OperationType, OpStatus, QueryLogEntry};

use crate::adapters::system_db::SystemDb;

/// Bounded channel capacity for the background log consumer.
/// When full, new entries are silently dropped (fire-and-forget contract).
const QUERY_LOG_CHANNEL_CAPACITY: usize = 1_000;

/// Sweep task interval in seconds (24 hours).
const SWEEP_INTERVAL_SECS: u64 = 86_400;

/// Instance default for a project whose `log_retention_days` is NULL.
const DEFAULT_RETENTION_DAYS: i32 = 90;
/// Admin API's validated range for `log_retention_days` (admin/handlers/projects.rs).
const MAX_RETENTION_DAYS: i32 = 3650;
/// Rows deleted per statement, and per project per sweep run. A project whose
/// backlog exceeds the cap is finished by the next run.
const PURGE_BATCH_ROWS: i64 = 5_000;
const PURGE_MAX_ROWS_PER_PROJECT: i64 = 100_000;

/// Effective retention for a project: NULL means the instance default; values
/// outside the validated 1..=3650 range (should not exist) are clamped.
fn resolve_retention_days(configured: Option<i32>) -> i32 {
    configured
        .unwrap_or(DEFAULT_RETENTION_DAYS)
        .clamp(1, MAX_RETENTION_DAYS)
}

/// `(partition horizon, projects needing a row-level purge)` from
/// `(project_id, log_retention_days, is_deleted)` rows.
///
/// The horizon is the longest retention among live projects (the partition
/// drop must never remove rows a live project still keeps; default when
/// there are none). Any project, deleted or not, keeping logs for less than
/// the horizon needs its own rows deleted: the partition drop alone would
/// hold them for the whole horizon.
fn plan_retention(projects: &[(String, Option<i32>, bool)]) -> (i32, Vec<(&str, i32)>) {
    let horizon = projects
        .iter()
        .filter(|(_, _, deleted)| !deleted)
        .map(|(_, days, _)| resolve_retention_days(*days))
        .max()
        .unwrap_or(DEFAULT_RETENTION_DAYS);
    let purge = projects
        .iter()
        .map(|(id, days, _)| (id.as_str(), resolve_retention_days(*days)))
        .filter(|(_, days)| *days < horizon)
        .collect();
    (horizon, purge)
}

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
        let (tx, mut rx) = mpsc::channel::<QueryLogEntry>(QUERY_LOG_CHANNEL_CAPACITY);

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

                // Safety: partition_name is derived from chrono date formatting
                // ("%Y_%m_%d") and date_str/next_date from "%Y-%m-%d" — both
                // produce only digits, underscores, and hyphens. User input never
                // reaches these strings, so there is no SQL injection risk.
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
                tokio::time::interval(std::time::Duration::from_secs(SWEEP_INTERVAL_SECS));
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

        sweep(pool).await;

        let _ = sqlx::query_scalar::<_, bool>("SELECT pg_advisory_unlock($1)")
            .bind(SWEEP_ADVISORY_LOCK)
            .fetch_one(pool)
            .await;
    }
}

/// One locked sweep: partition drop at the horizon, then the per-project row purge.
async fn sweep(pool: &sqlx::PgPool) {
    let projects: Vec<(String, Option<i32>, bool)> = match sqlx::query_as(
        "SELECT id, log_retention_days, status = 'deleted' FROM projects",
    )
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(e) => {
            // Never guess the horizon: a wrong (default) one could drop a
            // longer-retention project's partitions.
            tracing::warn!("query_log sweep: cannot read project retentions, skipping: {e}");
            crate::sweepers::record_sweeper_error("query_log_sweeper");
            return;
        }
    };
    let (retention_days, per_project) = plan_retention(&projects);
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

    // Row-level purge for projects with a shorter retention than the horizon.
    for (project_id, days) in per_project {
        purge_project(pool, project_id, days).await;
    }
}

/// Delete `project_id`'s rows older than `days`, in `PURGE_BATCH_ROWS` batches
/// (bounded by `PURGE_MAX_ROWS_PER_PROJECT`). Selects by the primary key so
/// each batch is one indexed scan on `(project_id, created_at)`.
async fn purge_project(pool: &sqlx::PgPool, project_id: &str, days: i32) {
    let cutoff = chrono::Utc::now() - chrono::Duration::days(days as i64);
    let mut purged: i64 = 0;
    while purged < PURGE_MAX_ROWS_PER_PROJECT {
        let result = sqlx::query(
            "DELETE FROM query_logs WHERE (id, created_at) IN ( \
                 SELECT id, created_at FROM query_logs \
                 WHERE project_id = $1 AND created_at < $2 LIMIT $3)",
        )
        .bind(project_id)
        .bind(cutoff)
        .bind(PURGE_BATCH_ROWS)
        .execute(pool)
        .await;
        match result {
            Ok(done) => {
                let n = done.rows_affected() as i64;
                purged += n;
                metrics::counter!("embyr_query_logs_purged_total").increment(n as u64);
                if n < PURGE_BATCH_ROWS {
                    break;
                }
            }
            Err(e) => {
                tracing::warn!("query_log retention purge failed for project {project_id}: {e}");
                crate::sweepers::record_sweeper_error("query_log_sweeper");
                return;
            }
        }
    }
    if purged > 0 {
        tracing::info!(project_id, days, rows_purged = purged, "query_log retention purge");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(id: &str, days: Option<i32>, deleted: bool) -> (String, Option<i32>, bool) {
        (id.to_string(), days, deleted)
    }

    #[test]
    fn null_retention_is_the_default_and_out_of_range_is_clamped() {
        assert_eq!(resolve_retention_days(None), 90);
        assert_eq!(resolve_retention_days(Some(7)), 7);
        assert_eq!(resolve_retention_days(Some(0)), 1);
        assert_eq!(resolve_retention_days(Some(-5)), 1);
        assert_eq!(resolve_retention_days(Some(100_000)), 3650);
    }

    #[test]
    fn horizon_is_the_longest_live_retention_and_shorter_projects_get_a_row_purge() {
        let projects = [
            p("short", Some(7), false),
            p("default", None, false),
            p("long", Some(365), false),
            p("gone", Some(3), true),
            p("gone-long", Some(3650), true),
        ];
        let (horizon, purge) = plan_retention(&projects);
        assert_eq!(horizon, 365, "deleted project must not stretch the horizon");
        assert_eq!(purge, vec![("short", 7), ("default", 90), ("gone", 3)]);
    }

    #[test]
    fn no_live_projects_means_default_horizon() {
        assert_eq!(plan_retention(&[]), (90, vec![]));
        assert_eq!(plan_retention(&[p("gone", Some(3), true)]).0, 90);
    }
}
