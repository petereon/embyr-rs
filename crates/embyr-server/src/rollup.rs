//! hourly-metrics-rollup (ADR-084): request-path hook and background flush of
//! the in-process [`RollupBuffer`] into `hourly_project_metrics`.
//!
//! The request path only ever touches the buffer (a short mutex section, no
//! `.await`, no DB). The flush task drains it every `EMBYR_ROLLUP_FLUSH_SECS`
//! with a multi-row additive upsert; a failed flush puts the data back.

use std::future::Future;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use chrono::Utc;
use embyr_core::metrics_rollup::{
    truncate_to_hour, Acc, Op, RollupBuffer, RollupKey, BUCKET_COUNT, DEFAULT_MAX_KEYS,
};

use crate::adapters::system_db::SystemDb;
use crate::middleware::obs_helpers as obs;

/// Distinct (project, hour, op) keys upserted per statement.
const FLUSH_CHUNK_KEYS: usize = 500;

/// How long the flusher waits for a system-DB connection before skipping this
/// flush (data stays buffered). Short on purpose: the flusher must yield the
/// pool to requests rather than queue for the pool's full 5s acquire timeout.
const FLUSH_ACQUIRE_TIMEOUT: Duration = Duration::from_millis(500);

/// Postgres `undefined_table`: the rollup table does not exist (yet), e.g. a
/// rolling deploy where migration 0047 has not been applied.
const PG_UNDEFINED_TABLE: &str = "42P01";

/// Env-tunable knobs (defaults per ADR-084).
#[derive(Debug, Clone, Copy)]
pub struct RollupConfig {
    pub flush_interval: Duration,
    pub max_keys: usize,
    pub retention_days: i64,
}

impl RollupConfig {
    pub fn from_env() -> Self {
        fn var<T: std::str::FromStr + PartialOrd + Default>(name: &str, default: T) -> T {
            std::env::var(name)
                .ok()
                .and_then(|v| v.parse::<T>().ok())
                .filter(|v| *v > T::default())
                .unwrap_or(default)
        }
        Self {
            flush_interval: Duration::from_secs(var("EMBYR_ROLLUP_FLUSH_SECS", 10u64)),
            max_keys: var("EMBYR_ROLLUP_MAX_KEYS", DEFAULT_MAX_KEYS),
            retention_days: var("EMBYR_ROLLUP_RETENTION_DAYS", 14i64),
        }
    }
}

/// Op class of a gRPC method, `None` for methods that are not sampled
/// (transaction control and the `Listen`/`Write` streams — AC-HMR-12).
pub fn op_for_method(method: &str) -> Option<Op> {
    match method {
        obs::METHOD_GET_DOCUMENT
        | obs::METHOD_LIST_DOCUMENTS
        | obs::METHOD_LIST_COLLECTION_IDS
        | obs::METHOD_BATCH_GET_DOCUMENTS => Some(Op::Read),
        obs::METHOD_CREATE_DOCUMENT
        | obs::METHOD_UPDATE_DOCUMENT
        | obs::METHOD_COMMIT
        | obs::METHOD_BATCH_WRITE => Some(Op::Write),
        obs::METHOD_DELETE_DOCUMENT => Some(Op::Delete),
        obs::METHOD_RUN_QUERY | obs::METHOD_RUN_AGGREGATION_QUERY => Some(Op::RunQuery),
        _ => None,
    }
}

tokio::task_local! {
    /// Filled by [`mark_authenticated`] once a request's credentials verified.
    static AUTHENTICATED_PROJECT: Arc<OnceLock<String>>;
}

/// Called by the handler after `authenticate` succeeded: only authenticated
/// requests are attributed to a project, so a caller who merely knows a
/// project id cannot pollute its metrics. No-op outside [`observe`].
pub fn mark_authenticated(project_id: &str) {
    let _ = AUTHENTICATED_PROJECT.try_with(|slot| {
        let _ = slot.set(project_id.to_string());
    });
}

/// Run a gRPC handler future and record it in `buffer` (project known only
/// if the handler authenticated). Never fails or delays the request.
pub async fn observe<T, F>(
    buffer: &RollupBuffer,
    method: &'static str,
    fut: F,
) -> Result<tonic::Response<T>, tonic::Status>
where
    F: Future<Output = Result<tonic::Response<T>, tonic::Status>>,
{
    let Some(op) = op_for_method(method) else {
        return fut.await;
    };
    let slot = Arc::new(OnceLock::new());
    let start = Instant::now();
    let result = AUTHENTICATED_PROJECT.scope(Arc::clone(&slot), fut).await;
    if let Some(project_id) = slot.get() {
        let latency_ms = start.elapsed().as_secs_f64() * 1000.0;
        // ponytail: units = 1 per request; per-response document counts need
        // handler-body hooks (follow-up).
        let kept = buffer.record(
            project_id,
            truncate_to_hour(Utc::now()),
            op,
            latency_ms,
            1,
            result.is_err(),
        );
        if !kept {
            metrics::counter!("embyr_rollup_dropped_total").increment(1);
        }
    }
    result
}

fn upsert_sql() -> &'static str {
    static SQL: OnceLock<String> = OnceLock::new();
    SQL.get_or_init(|| {
        let buckets: Vec<String> = (0..BUCKET_COUNT).map(|i| format!("b{i:02}")).collect();
        let counters: Vec<String> = ["requests", "units", "errors"]
            .iter()
            .map(|s| s.to_string())
            .chain(buckets)
            .collect();
        let cols = counters.join(", ");
        let arrays: Vec<String> =
            (4..4 + counters.len()).map(|i| format!("${i}::bigint[]")).collect();
        let sets: Vec<String> = counters
            .iter()
            .map(|c| format!("{c} = hourly_project_metrics.{c} + EXCLUDED.{c}"))
            .collect();
        // The join skips projects deleted mid-flush instead of failing the chunk on the FK;
        // `WHERE true` sidesteps Postgres' documented INSERT..SELECT..FROM / ON CONFLICT ambiguity.
        format!(
            "INSERT INTO hourly_project_metrics (project_id, hour, op, {cols}) \
             SELECT u.project_id, u.hour, u.op, {ucols} \
             FROM UNNEST($1::text[], $2::timestamptz[], $3::smallint[], {arrays}) \
               AS u(project_id, hour, op, {cols}) \
             JOIN projects p ON p.id = u.project_id \
             WHERE true \
             ON CONFLICT (project_id, hour, op) DO UPDATE SET {sets}",
            ucols = counters.iter().map(|c| format!("u.{c}")).collect::<Vec<_>>().join(", "),
            arrays = arrays.join(", "),
            sets = sets.join(", "),
        )
    })
}

fn clamp(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

async fn upsert_chunk(
    system_db: &SystemDb,
    chunk: &[(RollupKey, Acc)],
) -> Result<(), sqlx::Error> {
    let projects: Vec<String> = chunk.iter().map(|(k, _)| k.project_id.clone()).collect();
    let hours: Vec<chrono::DateTime<Utc>> = chunk.iter().map(|(k, _)| k.hour).collect();
    let ops: Vec<i16> = chunk.iter().map(|(k, _)| k.op as i16).collect();
    let mut columns: Vec<Vec<i64>> = vec![
        chunk.iter().map(|(_, a)| clamp(a.requests)).collect(),
        chunk.iter().map(|(_, a)| clamp(a.units)).collect(),
        chunk.iter().map(|(_, a)| clamp(a.errors)).collect(),
    ];
    for i in 0..BUCKET_COUNT {
        columns.push(chunk.iter().map(|(_, a)| clamp(a.buckets[i])).collect());
    }
    let mut query = sqlx::query(upsert_sql()).bind(projects).bind(hours).bind(ops);
    for column in columns {
        query = query.bind(column);
    }
    // Exactly one connection, held for this chunk only.
    let mut conn = tokio::time::timeout(FLUSH_ACQUIRE_TIMEOUT, system_db.pool().acquire())
        .await
        .map_err(|_| sqlx::Error::PoolTimedOut)??;
    query.execute(&mut *conn).await.map(|_| ())
}

fn is_missing_table(e: &sqlx::Error) -> bool {
    e.as_database_error().and_then(|d| d.code()).as_deref() == Some(PG_UNDEFINED_TABLE)
}

/// Drain the system DB's buffer into `hourly_project_metrics` (additive
/// upsert, safe across instances). On failure the unflushed entries go back
/// into the (bounded) buffer; the error is logged and counted, never raised
/// to a request.
pub async fn flush(system_db: &SystemDb) -> Result<(), sqlx::Error> {
    let buffer = system_db.rollup();
    let entries = buffer.drain();
    for (done, chunk) in entries.chunks(FLUSH_CHUNK_KEYS).enumerate() {
        if let Err(e) = upsert_chunk(system_db, chunk).await {
            let unflushed = entries[done * FLUSH_CHUNK_KEYS..].to_vec();
            let dropped = buffer.restore(unflushed);
            if dropped > 0 {
                metrics::counter!("embyr_rollup_dropped_total").increment(dropped);
            }
            if is_missing_table(&e) {
                // Quiet: log once, keep buffering (bounded) until the table exists.
                static LOGGED: std::sync::atomic::AtomicBool =
                    std::sync::atomic::AtomicBool::new(false);
                if !LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                    tracing::warn!("rollup table missing (migration 0047 pending?); buffering");
                }
            } else {
                tracing::warn!(error = %e, dropped, "rollup flush failed; data kept for next flush");
            }
            metrics::counter!("embyr_rollup_flush_total", "outcome" => "error").increment(1);
            return Err(e);
        }
    }
    if !entries.is_empty() {
        metrics::counter!("embyr_rollup_flush_total", "outcome" => "ok").increment(1);
    }
    Ok(())
}

/// Handle to the background flush task.
pub struct Flusher {
    stop: tokio::sync::oneshot::Sender<()>,
    join: tokio::task::JoinHandle<()>,
}

impl Flusher {
    /// Stop the task after one final flush (graceful shutdown, AC-HMR-06).
    pub async fn shutdown(self) {
        let _ = self.stop.send(());
        let _ = self.join.await;
    }
}

/// Spawn the flush loop (first flush after one `interval`).
pub fn spawn_flusher(system_db: Arc<SystemDb>, interval: Duration) -> Flusher {
    let (stop, mut stop_rx) = tokio::sync::oneshot::channel::<()>();
    let join = tokio::spawn(async move {
        let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + interval, interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = tick.tick() => { let _ = flush(&system_db).await; }
                _ = &mut stop_rx => {
                    let _ = flush(&system_db).await;
                    break;
                }
            }
        }
    });
    Flusher { stop, join }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sampled_methods_map_to_op_classes_and_streams_are_skipped() {
        assert_eq!(op_for_method(obs::METHOD_GET_DOCUMENT), Some(Op::Read));
        assert_eq!(op_for_method(obs::METHOD_COMMIT), Some(Op::Write));
        assert_eq!(op_for_method(obs::METHOD_DELETE_DOCUMENT), Some(Op::Delete));
        assert_eq!(op_for_method(obs::METHOD_RUN_QUERY), Some(Op::RunQuery));
        assert_eq!(op_for_method(obs::METHOD_LISTEN), None);
        assert_eq!(op_for_method(obs::METHOD_WRITE), None);
        assert_eq!(op_for_method(obs::METHOD_BEGIN_TRANSACTION), None);
    }

    #[test]
    fn upsert_sql_has_one_array_per_column_and_adds_every_counter() {
        let sql = upsert_sql();
        assert!(sql.contains("$19::bigint[]") && !sql.contains("$20"));
        assert!(sql.contains("b12 = hourly_project_metrics.b12 + EXCLUDED.b12"));
        assert!(sql.contains("ON CONFLICT (project_id, hour, op)"));
    }

    #[tokio::test]
    async fn observe_records_only_authenticated_requests_with_error_flag() {
        let buf = RollupBuffer::new(10);
        let ok: Result<tonic::Response<()>, tonic::Status> = observe(&buf, obs::METHOD_GET_DOCUMENT, async {
            mark_authenticated("proj");
            Ok(tonic::Response::new(()))
        })
        .await;
        assert!(ok.is_ok());
        let err: Result<tonic::Response<()>, tonic::Status> =
            observe(&buf, obs::METHOD_GET_DOCUMENT, async {
                mark_authenticated("proj");
                Err(tonic::Status::internal("x"))
            })
            .await;
        assert!(err.is_err());
        // Unauthenticated request: not attributed to any project.
        let _: Result<tonic::Response<()>, tonic::Status> =
            observe(&buf, obs::METHOD_GET_DOCUMENT, async {
                Err(tonic::Status::unauthenticated("bad key"))
            })
            .await;
        // Unsampled method: never recorded even if authenticated.
        let _: Result<tonic::Response<()>, tonic::Status> =
            observe(&buf, obs::METHOD_LISTEN, async {
                mark_authenticated("proj");
                Ok(tonic::Response::new(()))
            })
            .await;
        let drained = buf.drain();
        assert_eq!(drained.len(), 1);
        let acc = &drained[0].1;
        assert_eq!((acc.requests, acc.errors), (2, 1));
    }
}
