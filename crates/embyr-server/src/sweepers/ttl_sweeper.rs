//! `TtlSweeper` — background deletion of documents whose TTL field has
//! expired (go-live finding P5; Firestore-style TTL).
//!
//! For every ENABLED policy of an ACTIVE project (`ttl_policies`, migration
//! 0050) it runs a typed `field < now` collection-group query against the
//! customer database and deletes the hits in batches through the backend
//! commit path (`begin_transaction` + `commit_transaction`), NOT raw SQL: the
//! commit path emits the same `pg_notify` per written path as any client
//! delete, so live Listen subscribers see the removal. TTL deletes bypass
//! security rules (like Firestore) and never touch subcollections of the
//! deleted document.
//!
//! Shape mirrors `TransactionSweeper` (one advisory-locked cycle per tick,
//! per-project DSN resolution without an api_key) with two differences from
//! the other sweepers, both about the small system pool: the first tick is
//! delayed (no startup stampede) and the single connection that carries the
//! session-level advisory lock ALSO runs the policy query, so a cycle pins
//! exactly one system-pool connection and never waits for a second.
//!
//! `backend_mode = 'agent'` projects are skipped (counted + logged): the
//! agent's `RunQuery` cannot express a collection-group query with a limit.
//!
//! ponytail: sequential over projects and policies, one commit per batch of
//! <= `BATCH_SIZE`. Add bounded concurrency across projects if a sweep ever
//! exceeds its interval.

use std::sync::Arc;
use std::time::Duration;

use sqlx::{PgPool, Row};

use embyr_core::domain::document::{CollectionPath, FirestoreDocument};
use embyr_core::domain::project::ProjectId;
use embyr_core::domain::transaction::TransactionOptions;
use embyr_core::domain::ttl::{
    expired_documents_query, in_collection_group, is_expired, next_batch_limit,
};
use embyr_core::error::CoreError;
use embyr_core::storage::backend_adapter::{BackendAdapter, Write, WritePrecondition};

use crate::adapters::aws_secret_fetcher::AwsSecretFetcher;
use crate::adapters::customer_db_connect::{resolve_dsn_without_api_key, PgConnectInfo};
use crate::adapters::gcp_secret_fetcher::GcpSecretFetcher;
use crate::adapters::postgres_backend::PostgresBackendAdapter;
use crate::adapters::system_db::{SweeperProjectRow, SystemDb};

use super::advisory_lock_key;

/// `pg_try_advisory_lock` key namespace; `pub` so tests can prove single-flight.
pub const LOCK_KEY_NAME: &str = "embyr_ttl_sweep";

/// First tick is this long after start (capped at the interval): the other
/// sweepers fire immediately at startup and share the small system pool.
const INITIAL_DELAY: Duration = Duration::from_secs(60);

/// Wall-clock bound per project per cycle; a slow customer database cannot
/// stall the sweep of the others. Unfinished work resumes next cycle.
const PROJECT_BUDGET: Duration = Duration::from_secs(300);

/// Connections in the short-lived per-project customer pool.
const TENANT_POOL_CONNECTIONS: u32 = 2;
const TENANT_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);

/// Everything `run_cycle` needs besides the system pool.
#[derive(Clone)]
pub struct TtlSweepConfig {
    pub aws_secret_fetcher: Option<Arc<AwsSecretFetcher>>,
    pub gcp_secret_fetcher: Option<Arc<GcpSecretFetcher>>,
    pub encryption_key: [u8; 32],
    pub encryption_key_previous: Option<[u8; 32]>,
    /// Per-policy, per-cycle deletion cap (`EMBYR_TTL_MAX_DELETES_PER_RUN`).
    pub max_deletes_per_policy: u64,
}

/// One active project with its enabled policies: `(collection_group, field_path)`.
pub struct ProjectPolicies {
    pub project: SweeperProjectRow,
    pub policies: Vec<(String, String)>,
}

fn record_error() {
    metrics::counter!("embyr_ttl_sweep_errors_total").increment(1);
    super::record_sweeper_error("ttl_sweeper");
}

/// Spawn the sweeper; the composition root holds the `JoinHandle`.
pub fn spawn(
    system_db: Arc<SystemDb>,
    interval: Duration,
    cfg: TtlSweepConfig,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let start = tokio::time::Instant::now() + INITIAL_DELAY.min(interval);
        let mut tick = tokio::time::interval_at(start, interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;

            let lock_key = advisory_lock_key(LOCK_KEY_NAME);
            // Session affinity: lock and unlock must share one connection.
            let Ok(mut lock_conn) = system_db.pool().acquire().await else {
                continue;
            };
            let locked: Option<bool> = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
                .bind(lock_key)
                .fetch_one(&mut *lock_conn)
                .await
                .ok();
            if locked != Some(true) {
                continue; // another replica is sweeping (or the probe failed)
            }

            match fetch_policies(&mut *lock_conn).await {
                Ok(projects) => {
                    sweep(projects, &cfg).await;
                    mark_run();
                }
                Err(e) => {
                    tracing::warn!(error = %e, "TtlSweeper: failed to load policies");
                    record_error();
                }
            }

            let unlocked: Option<bool> = sqlx::query_scalar("SELECT pg_advisory_unlock($1)")
                .bind(lock_key)
                .fetch_one(&mut *lock_conn)
                .await
                .ok();
            if unlocked != Some(true) {
                // Never hand a connection that may still hold the lock back to the pool.
                lock_conn.close_on_drop();
            }
        }
    })
}

/// One full cycle without the lock/interval wrapper — the testable driving
/// port. Returns the number of documents deleted.
pub async fn run_cycle(pool: &PgPool, cfg: &TtlSweepConfig) -> u64 {
    match fetch_policies(pool).await {
        Ok(projects) => {
            let deleted = sweep(projects, cfg).await;
            mark_run();
            deleted
        }
        Err(e) => {
            tracing::warn!(error = %e, "TtlSweeper: failed to load policies");
            record_error();
            0
        }
    }
}

fn mark_run() {
    metrics::gauge!("embyr_ttl_last_run_timestamp_seconds")
        .set(chrono::Utc::now().timestamp() as f64);
}

/// Enabled policies of ACTIVE projects only (suspended/deleted are skipped),
/// grouped per project (rows arrive ordered by project id).
pub async fn fetch_policies<'c, E: sqlx::PgExecutor<'c>>(
    exec: E,
) -> Result<Vec<ProjectPolicies>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT p.id, p.backend_mode, p.backend_secret_arn, p.backend_secret_gcp, \
                p.backend_pg_dsn_enc, t.collection_group, t.field_path \
         FROM ttl_policies t JOIN projects p ON p.id = t.project_id \
         WHERE t.enabled AND p.status = 'active' \
         ORDER BY p.id, t.collection_group",
    )
    .fetch_all(exec)
    .await?;

    let mut out: Vec<ProjectPolicies> = Vec::new();
    for r in rows {
        let id: String = r.try_get("id")?;
        let policy = (r.try_get("collection_group")?, r.try_get("field_path")?);
        match out.last_mut() {
            Some(last) if last.project.id == id => last.policies.push(policy),
            _ => out.push(ProjectPolicies {
                project: SweeperProjectRow {
                    id,
                    backend_mode: r.try_get("backend_mode")?,
                    backend_secret_arn: r.try_get("backend_secret_arn")?,
                    backend_secret_gcp: r.try_get("backend_secret_gcp")?,
                    backend_pg_dsn_enc: r.try_get("backend_pg_dsn_enc")?,
                },
                policies: vec![policy],
            }),
        }
    }
    Ok(out)
}

/// Sweep every project in turn; a failing or slow project never stops the rest.
async fn sweep(projects: Vec<ProjectPolicies>, cfg: &TtlSweepConfig) -> u64 {
    let mut total = 0;
    for pp in &projects {
        match tokio::time::timeout(PROJECT_BUDGET, sweep_project(pp, cfg)).await {
            Ok(n) => total += n,
            Err(_) => {
                tracing::warn!(project_id = %pp.project.id, "TtlSweeper: project exceeded its time budget, resuming next cycle");
                record_error();
            }
        }
    }
    total
}

async fn sweep_project(pp: &ProjectPolicies, cfg: &TtlSweepConfig) -> u64 {
    let project = &pp.project;
    if project.backend_mode == "agent" {
        tracing::warn!(project_id = %project.id, "TtlSweeper: TTL is not supported for backend_mode=agent projects, skipping");
        metrics::counter!("embyr_ttl_skipped_projects_total", "reason" => "agent_mode").increment(1);
        return 0;
    }
    let Ok(project_id) = ProjectId::new(project.id.clone()) else {
        record_error();
        return 0;
    };
    let info = PgConnectInfo {
        backend_mode: project.backend_mode.clone(),
        backend_secret_arn: project.backend_secret_arn.clone(),
        backend_secret_gcp: project.backend_secret_gcp.clone(),
        backend_pg_dsn_enc: project.backend_pg_dsn_enc.clone(),
    };
    let Some(dsn) = resolve_dsn_without_api_key(
        &project.id,
        &info,
        cfg.aws_secret_fetcher.as_deref(),
        cfg.gcp_secret_fetcher.as_deref(),
        &cfg.encryption_key,
        cfg.encryption_key_previous.as_ref(),
    )
    .await
    else {
        record_error();
        return 0;
    };
    let adapter = match PostgresBackendAdapter::with_pool_config(
        &dsn,
        TENANT_POOL_CONNECTIONS,
        TENANT_ACQUIRE_TIMEOUT,
    )
    .await
    {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!(project_id = %project.id, error = %e, "TtlSweeper: failed to connect to customer database");
            record_error();
            return 0;
        }
    };

    let mut total = 0;
    for (group, field) in &pp.policies {
        match sweep_policy(&adapter, &project_id, group, field, cfg.max_deletes_per_policy).await {
            Ok(n) => {
                if n > 0 {
                    tracing::info!(project_id = %project.id, collection_group = %group, deleted = n, "TtlSweeper: deleted expired documents");
                }
                total += n;
            }
            Err(e) => {
                tracing::warn!(project_id = %project.id, collection_group = %group, error = %e, "TtlSweeper: policy sweep failed");
                record_error();
            }
        }
    }
    adapter.pool().close().await;
    total
}

/// Delete expired documents of one policy, `BATCH_SIZE` at a time, until the
/// query comes back short, a batch deletes nothing, or the per-run cap is hit.
pub async fn sweep_policy(
    adapter: &dyn BackendAdapter,
    project_id: &ProjectId,
    group: &str,
    field: &str,
    cap: u64,
) -> Result<u64, CoreError> {
    let now = chrono::Utc::now();
    let now = (now.timestamp(), now.timestamp_subsec_nanos() as i32);
    let collection = CollectionPath {
        project_id: project_id.clone(),
        collection_path: group.to_string(),
    };

    let mut deleted = 0u64;
    while let Some(limit) = next_batch_limit(cap, deleted) {
        let query = expired_documents_query(group, field, now, limit);
        let docs = adapter.run_query(&collection, &query, None).await?;
        let fetched = docs.len();
        let eligible: Vec<FirestoreDocument> = docs
            .into_iter()
            .filter(|d| in_collection_group(&d.path.collection_path, group) && is_expired(&d.fields, field, now))
            .collect();

        let n = delete_batch(adapter, project_id, &eligible).await?;
        metrics::counter!("embyr_ttl_deleted_total").increment(n);
        deleted += n;
        if n == 0 || fetched < limit as usize {
            break;
        }
    }
    Ok(deleted)
}

/// Delete `docs` in one commit, guarded per document by its `update_time` (a
/// document rewritten since the query — e.g. its TTL pushed out — is kept).
/// If the guard trips, retry one by one; documents that changed are skipped.
async fn delete_batch(
    adapter: &dyn BackendAdapter,
    project_id: &ProjectId,
    docs: &[FirestoreDocument],
) -> Result<u64, CoreError> {
    if docs.is_empty() {
        return Ok(0);
    }
    match commit(adapter, project_id, docs.iter().map(delete_write).collect()).await {
        Ok(()) => return Ok(docs.len() as u64),
        Err(CoreError::TransactionAborted) => {}
        Err(e) => return Err(e),
    }
    let mut n = 0;
    for d in docs {
        match commit(adapter, project_id, vec![delete_write(d)]).await {
            Ok(()) => n += 1,
            Err(CoreError::TransactionAborted) => {} // changed or gone: re-evaluated next cycle
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

fn delete_write(d: &FirestoreDocument) -> Write {
    Write::Delete {
        path: d.path.clone(),
        version: None,
        precondition: Some(WritePrecondition::UpdateTime(d.update_time.0, d.update_time.1)),
    }
}

async fn commit(
    adapter: &dyn BackendAdapter,
    project_id: &ProjectId,
    writes: Vec<Write>,
) -> Result<(), CoreError> {
    let txn = adapter
        .begin_transaction(project_id, TransactionOptions::ReadWrite)
        .await?;
    adapter.commit_transaction(project_id, &txn, writes).await.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use async_trait::async_trait;
    use embyr_core::domain::document::{DocumentPath, WriteResult};
    use embyr_core::domain::field_value::FieldValue;
    use embyr_core::domain::query::{QueryFilter, StructuredQuery};
    use embyr_core::domain::transaction::TransactionId;

    /// In-memory adapter: `run_query` honours `field < now` + `limit`;
    /// `commit_transaction` deletes, optionally aborting multi-write commits
    /// (a concurrently modified document) and/or one permanently "stale" doc.
    struct Stub {
        docs: Mutex<Vec<FirestoreDocument>>,
        abort_multi_write_commits: bool,
        stale_doc_id: Option<&'static str>,
        commit_sizes: Mutex<Vec<usize>>,
    }

    impl Stub {
        fn new(docs: Vec<FirestoreDocument>) -> Self {
            Stub {
                docs: Mutex::new(docs),
                abort_multi_write_commits: false,
                stale_doc_id: None,
                commit_sizes: Mutex::new(vec![]),
            }
        }
        fn remaining(&self) -> Vec<String> {
            self.docs.lock().unwrap().iter().map(|d| d.path.document_id.clone()).collect()
        }
    }

    fn pid() -> ProjectId {
        ProjectId::new("ttl-test").unwrap()
    }

    fn doc(id: &str, collection_path: &str, exp: Option<FieldValue>) -> FirestoreDocument {
        let mut fields = BTreeMap::new();
        if let Some(v) = exp {
            fields.insert("exp".to_string(), v);
        }
        FirestoreDocument {
            path: DocumentPath {
                project_id: pid(),
                collection_path: collection_path.to_string(),
                document_id: id.to_string(),
            },
            fields,
            create_time: (1, 0),
            update_time: (2, 0),
            version: 1,
        }
    }

    fn past() -> Option<FieldValue> {
        Some(FieldValue::Timestamp(1_000, 0))
    }

    #[async_trait]
    impl BackendAdapter for Stub {
        async fn get_document(&self, _: &DocumentPath, _: Option<&TransactionId>) -> Result<Option<FirestoreDocument>, CoreError> {
            unimplemented!()
        }
        async fn create_document(&self, _: &DocumentPath, _: BTreeMap<String, FieldValue>) -> Result<WriteResult, CoreError> {
            unimplemented!()
        }
        async fn update_document(&self, _: &DocumentPath, _: BTreeMap<String, FieldValue>, _: Option<WritePrecondition>) -> Result<WriteResult, CoreError> {
            unimplemented!()
        }
        async fn delete_document(&self, _: &DocumentPath, _: Option<WritePrecondition>) -> Result<(), CoreError> {
            unimplemented!()
        }
        async fn run_query(&self, _: &CollectionPath, q: &StructuredQuery, _: Option<&TransactionId>) -> Result<Vec<FirestoreDocument>, CoreError> {
            assert!(q.all_descendants, "TTL must query the collection group");
            let Some(QueryFilter::Field(f)) = &q.filter else { panic!("filter") };
            let FieldValue::Timestamp(s, n) = f.value else { panic!("timestamp bound") };
            Ok(self
                .docs
                .lock()
                .unwrap()
                .iter()
                .filter(|d| is_expired(&d.fields, &f.field_path, (s, n)))
                .take(q.limit.unwrap() as usize)
                .cloned()
                .collect())
        }
        async fn begin_transaction(&self, _: &ProjectId, _: TransactionOptions) -> Result<TransactionId, CoreError> {
            Ok(TransactionId(vec![0; 16]))
        }
        async fn commit_transaction(&self, _: &ProjectId, _: &TransactionId, writes: Vec<Write>) -> Result<Vec<WriteResult>, CoreError> {
            self.commit_sizes.lock().unwrap().push(writes.len());
            if self.abort_multi_write_commits && writes.len() > 1 {
                return Err(CoreError::TransactionAborted);
            }
            let ids: Vec<String> = writes
                .iter()
                .map(|w| match w {
                    Write::Delete { path, precondition: Some(WritePrecondition::UpdateTime(2, 0)), .. } => path.document_id.clone(),
                    other => panic!("TTL must issue update_time-guarded deletes, got {other:?}"),
                })
                .collect();
            if let Some(stale) = self.stale_doc_id {
                if ids.iter().any(|i| i == stale) {
                    return Err(CoreError::TransactionAborted);
                }
            }
            self.docs.lock().unwrap().retain(|d| !ids.contains(&d.path.document_id));
            Ok(vec![])
        }
        async fn rollback_transaction(&self, _: &ProjectId, _: &TransactionId) -> Result<(), CoreError> {
            unimplemented!()
        }
        async fn probe(&self) -> Result<(), CoreError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn deletes_only_expired_timestamps_in_batches_of_500() {
        let mut docs: Vec<_> = (0..1_200).map(|i| doc(&format!("e{i}"), "sessions", past())).collect();
        docs.push(doc("future", "sessions", Some(FieldValue::Timestamp(i64::MAX / 2, 0))));
        docs.push(doc("missing", "sessions", None));
        docs.push(doc("null", "sessions", Some(FieldValue::Null)));
        docs.push(doc("string", "sessions", Some(FieldValue::String("2001-01-01".into()))));
        docs.push(doc("nested", "users/u1/sessions", past()));
        let stub = Stub::new(docs);

        let n = sweep_policy(&stub, &pid(), "sessions", "exp", 100_000).await.unwrap();

        assert_eq!(n, 1_201); // 1200 + the one in users/u1/sessions (same collection group)
        assert_eq!(*stub.commit_sizes.lock().unwrap(), vec![500, 500, 201]);
        assert_eq!(stub.remaining(), vec!["future", "missing", "null", "string"]);
    }

    #[tokio::test]
    async fn per_run_cap_bounds_deletions_and_the_rest_waits_for_the_next_run() {
        let docs: Vec<_> = (0..1_000).map(|i| doc(&format!("e{i}"), "sessions", past())).collect();
        let stub = Stub::new(docs);

        assert_eq!(sweep_policy(&stub, &pid(), "sessions", "exp", 700).await.unwrap(), 700);
        assert_eq!(*stub.commit_sizes.lock().unwrap(), vec![500, 200]);
        assert_eq!(stub.remaining().len(), 300);

        assert_eq!(sweep_policy(&stub, &pid(), "sessions", "exp", 700).await.unwrap(), 300);
        assert!(stub.remaining().is_empty());
    }

    #[tokio::test]
    async fn never_deletes_outside_the_configured_group_even_if_the_query_over_matches() {
        // An adapter whose group predicate over-matches (LIKE wildcard): the
        // sibling collection's expired doc is returned but must be kept.
        let stub = Stub::new(vec![doc("keep", "userXsessions", past()), doc("go", "user_sessions", past())]);
        let n = sweep_policy(&stub, &pid(), "user_sessions", "exp", 100).await.unwrap();
        assert_eq!(n, 1);
        assert_eq!(stub.remaining(), vec!["keep"]);
    }

    #[tokio::test]
    async fn aborted_batch_falls_back_to_one_by_one_and_keeps_a_changed_document() {
        let docs: Vec<_> = (0..5).map(|i| doc(&format!("e{i}"), "sessions", past())).collect();
        let mut stub = Stub::new(docs);
        stub.abort_multi_write_commits = true;
        stub.stale_doc_id = Some("e2");

        let n = sweep_policy(&stub, &pid(), "sessions", "exp", 100).await.unwrap();

        assert_eq!(n, 4);
        assert_eq!(stub.remaining(), vec!["e2"]); // terminates despite the permanently stale doc
    }

    #[tokio::test]
    async fn nothing_expired_is_a_noop() {
        let stub = Stub::new(vec![doc("a", "sessions", Some(FieldValue::Timestamp(i64::MAX / 2, 0)))]);
        assert_eq!(sweep_policy(&stub, &pid(), "sessions", "exp", 100).await.unwrap(), 0);
        assert!(stub.commit_sizes.lock().unwrap().is_empty());
    }
}
