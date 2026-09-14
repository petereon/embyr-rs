use std::collections::BTreeMap;

use async_trait::async_trait;
use sqlx::PgPool;

use crate::notify_listener::notify_channel;

use chrono::{DateTime, TimeZone, Utc};
use embyr_core::{
    domain::{
        document::{CollectionPath, DocumentPath, FirestoreDocument, WriteResult},
        field_transform::apply_field_transform,
        field_value::FieldValue,
        project::ProjectId,
        query::{AggregateValue, AggregationKind, AggregationQuery, OrderDirection, StructuredQuery},
        schema_readiness::SchemaReadiness,
        transaction::{TransactionId, TransactionOptions},
    },
    error::CoreError,
    storage::backend_adapter::{BackendAdapter, Write, WritePrecondition},
};
use uuid;

/// Customer-DB backend adapter — one instance per customer database URL.
///
/// Implements `BackendAdapter` for Firestore document operations backed by
/// a customer-isolated Postgres database with the documents schema.
pub struct PostgresBackendAdapter {
    pool: PgPool,
    /// ADR-080 Decision C — cached schema-capability probe state, shared by
    /// every clone-free caller of this adapter instance (one instance per
    /// customer database, per `embyr-server`/`embyr-agent`'s own connection
    /// lifecycle). `tokio::sync::RwLock` so concurrent `run_query`/
    /// `run_aggregation_query` calls never block each other on a cached read.
    schema_capability_cache: tokio::sync::RwLock<CachedSchemaCapability>,
}

/// ADR-080 Decision C — internal cache state behind
/// `PostgresBackendAdapter::schema_capability_cache`. `Available` is
/// terminal (migrations are additive-only, ADR-022); `Unavailable` carries
/// the `Instant` of its own last catalog probe, re-checked once
/// `is_probe_stale` says the caller's TTL has elapsed.
#[derive(Clone, Copy, Default)]
enum CachedSchemaCapability {
    #[default]
    Unknown,
    Available,
    Unavailable { checked_at: std::time::Instant },
}

/// The single compiled-in embed point for `migrations/customer/` (ADR-022).
///
/// `migrate()` and (from step 03-01) `verify_schema_readiness()` both read
/// this identical `Migrator` instance — never invoke `sqlx::migrate!` a
/// second time anywhere in the workspace.
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("../../migrations/customer");

impl PostgresBackendAdapter {
    /// Connect to the customer database. Does NOT run migrations.
    pub async fn new(database_url: &str) -> Result<Self, CoreError> {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await
            .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;
        Ok(Self { pool, schema_capability_cache: Default::default() })
    }

    /// Connect with an operator-configured `max_connections` and
    /// `acquire_timeout` (pool-sizing-and-limits, ADR-079) — this site had
    /// no `acquire_timeout` at all before. Additive: `new()` above is
    /// unchanged for its 8+ existing callers.
    pub async fn with_pool_config(
        database_url: &str,
        max_connections: u32,
        acquire_timeout: std::time::Duration,
    ) -> Result<Self, CoreError> {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(acquire_timeout)
            .connect(database_url)
            .await
            .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;
        Ok(Self { pool, schema_capability_cache: Default::default() })
    }

    /// Construct from an already-connected pool (used by test harnesses).
    pub fn new_from_pool(pool: PgPool) -> Self {
        Self { pool, schema_capability_cache: Default::default() }
    }

    /// Apply customer schema migrations from `migrations/customer/`.
    pub async fn migrate(&self) -> Result<(), CoreError> {
        MIGRATOR
            .run(&self.pool)
            .await
            .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;

        // collection-group-query-index (ADR-080 Decision D): once
        // `collection_id` exists (guaranteed the moment MIGRATOR.run()
        // above succeeds, since it always applies every embedded migration
        // through 0006), the two collection-group indexes should exist too
        // -- AC-CGI-02 holds "from the moment a database becomes
        // schema-current", not only once a separate operator step runs.
        // `embyr-db-prep`/`provision.rs` also call this explicitly
        // afterward (ADR-080 § Component Boundaries) -- idempotent
        // (`IF NOT EXISTS`), so the two calls never conflict.
        self.ensure_collection_group_indexes().await
    }

    /// Run customer schema migrations against the given pool.
    ///
    /// Convenience for test harnesses that need to migrate before constructing
    /// the adapter. Delegates to `migrate()` — never invokes the migration
    /// embed macro independently (ADR-022).
    pub async fn run_migrations(pool: &PgPool) -> Result<(), CoreError> {
        Self::new_from_pool(pool.clone()).migrate().await
    }

    /// Send a Postgres NOTIFY on the project's channel after a write.
    ///
    /// The payload is `{collection_path}/{document_id}`.
    /// Failure is non-fatal — NOTIFY is best-effort for real-time delivery.
    pub async fn send_notify(&self, path: &DocumentPath) {
        let channel = notify_channel(path.project_id.as_str());
        let payload = format!("{}/{}", path.collection_path, path.document_id);
        let _ = sqlx::query("SELECT pg_notify($1, $2)")
            .bind(&channel)
            .bind(&payload)
            .execute(&self.pool)
            .await;
    }

    /// Expose the internal pool for use by `PostgresNotifyListener`.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    // ─── customer-db-onboarding (ADR-023) ──────────────────────────────────
    //
    // `verify_schema_readiness()` implemented step 03-01 (with step 05-01's
    // insufficient_privilege→NotPrepped extension for the OQ-5 sequencing
    // gap). `discover_current_user()` and `grant_schema_readiness_read()`
    // implemented step 05-01.

    /// Verify the customer database's schema-readiness state.
    ///
    /// Reads sqlx's own `_sqlx_migrations` bookkeeping table (no new table
    /// type introduced) and compares the highest successfully-applied
    /// version against the compiled-in `Migrator`'s own highest embedded
    /// version (the single-sourced embed established by ADR-022). Read-only
    /// — `SELECT`-only against `_sqlx_migrations`, never attempts DDL
    /// (AC-02-05).
    ///
    /// This method *is* `embyr-server`'s provisioning-time probe of the
    /// customer DB's claimed schema state (Earned Trust — mirrors
    /// `SystemDb::probe()`'s hard-gate shape). See ADR-023 § Mechanism.
    ///
    /// On a relation-does-not-exist error (or zero successfully-applied
    /// rows), `found_version = 0` is passed to `classify()` — the pure
    /// comparison lives in `embyr_core::domain::schema_readiness::classify()`.
    pub async fn verify_schema_readiness(&self) -> Result<SchemaReadiness, CoreError> {
        use sqlx::Row;

        let expected_version: i64 = MIGRATOR
            .migrations
            .last()
            .map(|m| m.version)
            .unwrap_or(0);

        let rows = sqlx::query("SELECT version, success FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&self.pool)
            .await;

        let rows = match rows {
            Ok(rows) => rows,
            Err(sqlx::Error::Database(db_err))
                if matches!(db_err.code().as_deref(), Some("42P01") | Some("42501")) =>
            {
                // 42P01: relation "_sqlx_migrations" does not exist — never prepped.
                // 42501: insufficient_privilege — a DML role that exists but was
                // never granted read access (ADR-023 OQ-5 sequencing gap) sees
                // this as indistinguishable from "not prepped" until a follow-up
                // embyr-db-prep run supplies its DSN and closes the gap.
                return Ok(embyr_core::domain::schema_readiness::classify(
                    0,
                    expected_version,
                    vec!["documents".to_string(), "transactions".to_string()],
                ));
            }
            Err(e) => return Err(CoreError::BackendUnavailable(e.to_string())),
        };

        let found_version: i64 = rows
            .iter()
            .filter_map(|row| {
                let success: bool = row.try_get("success").ok()?;
                if !success {
                    return None;
                }
                row.try_get::<i64, _>("version").ok()
            })
            .max()
            .unwrap_or(0);

        Ok(embyr_core::domain::schema_readiness::classify(
            found_version,
            expected_version,
            vec!["documents".to_string(), "transactions".to_string()],
        ))
    }

    /// Discover the role name of the connection this adapter instance was
    /// constructed with, via `SELECT current_user`.
    ///
    /// Callers construct a second `PostgresBackendAdapter` from
    /// `EMBYR_DB_PREP_DML_ROLE_DSN` and call this method on it — the DSN is
    /// read once, used once, never logged (mirrors `StartupProbe`'s
    /// DSN-handling convention, `crates/embyr-agent/src/probe.rs`). Only the
    /// resulting role *name* is carried forward, never the DSN itself.
    pub async fn discover_current_user(&self) -> Result<String, CoreError> {
        sqlx::query_scalar("SELECT current_user")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| CoreError::BackendUnavailable(e.to_string()))
    }

    /// Grant `SELECT` on `_sqlx_migrations` to exactly the named role.
    ///
    /// Executed against the ELEVATED connection (the owner of
    /// `_sqlx_migrations`, and therefore the only connection that
    /// structurally holds grant authority). Role-name interpolation is
    /// delegated to Postgres's own `format('%I', ...)` — never hand-rolled
    /// Rust-side quoting — closing the SQL-injection-shaped risk a naive
    /// string interpolation would open (ADR-023 § Mechanism, step 4).
    /// Naturally idempotent — re-granting an already-granted privilege is a
    /// Postgres no-op, not an error.
    pub async fn grant_schema_readiness_read(&self, role_name: &str) -> Result<(), CoreError> {
        let quoted_role: String = sqlx::query_scalar("SELECT format('%I', $1::text)")
            .bind(role_name)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;

        sqlx::query(&format!(
            "GRANT SELECT ON _sqlx_migrations TO {quoted_role}"
        ))
        .execute(&self.pool)
        .await
        .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;

        Ok(())
    }

    // ─── collection-group-query-index (ADR-080) ────────────────────────────
    //
    // Three new adapter-level capabilities this feature adds (ADR-080
    // Decisions B/C/D).

    /// ADR-080 Decision B: resumable, throttled, `FOR UPDATE SKIP LOCKED`
    /// batched backfill of `collection_id` for pre-existing documents. Each
    /// batch is its own short transaction (a single `UPDATE` statement,
    /// auto-committed); terminates when a batch affects zero rows. Not
    /// project-scoped -- `embyr-db-prep` backfills an entire customer
    /// database in one run. Resumability (AC-CGI-08) falls out of the
    /// `WHERE collection_id IS NULL` predicate with no cursor/checkpoint --
    /// an interrupted-then-resumed run only ever matches not-yet-backfilled
    /// rows.
    pub async fn backfill_collection_id(
        &self,
        batch_size: u32,
        throttle: std::time::Duration,
    ) -> Result<BackfillSummary, CoreError> {
        let mut summary = BackfillSummary::default();
        loop {
            // ADR-080 Decision B: "each batch is its own short transaction"
            // -- explicit BEGIN/COMMIT, not a bare autocommit statement.
            // This matters beyond the letter of the ADR: a single
            // autocommit UPDATE commits server-side the instant it finishes
            // executing, regardless of whether the client is still
            // connected to receive the acknowledgment -- an aborted caller
            // (embyr-server restart, a killed embyr-db-prep process) could
            // leave a batch "committed but never counted" by this summary.
            // Wrapping each batch in its own explicit transaction makes it
            // truly atomic from the CALLER's perspective too: either the
            // whole batch's UPDATE *and* COMMIT completed, or the
            // connection dropped before COMMIT and Postgres rolls the
            // batch back entirely -- no partial, uncounted, silently
            // -committed batch.
            let mut tx = self.pool.begin().await.map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;

            let rows_affected = sqlx::query(
                "UPDATE documents d \
                 SET collection_id = regexp_replace(d.collection_path, '^.*/', '') \
                 FROM ( \
                     SELECT ctid FROM documents \
                     WHERE collection_id IS NULL \
                     LIMIT $1 \
                     FOR UPDATE SKIP LOCKED \
                 ) sub \
                 WHERE d.ctid = sub.ctid",
            )
            .bind(batch_size as i64)
            .execute(&mut *tx)
            .await
            .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?
            .rows_affected();

            if rows_affected == 0 {
                tx.rollback().await.ok();
                break;
            }

            tx.commit().await.map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;

            summary.rows_backfilled += rows_affected;
            summary.batches_run += 1;
            tokio::time::sleep(throttle).await;
        }
        Ok(summary)
    }

    /// ADR-080 Decision D: `CREATE INDEX CONCURRENTLY IF NOT EXISTS` for
    /// `documents_collection_group_idx` and `documents_collection_group_pending_idx`,
    /// built BEFORE the backfill loop runs (so AC-CGI-02 holds from the
    /// moment a database becomes schema-current). Reuses ADR-072's own
    /// `indisvalid` re-check + best-effort `DROP ... CONCURRENTLY` cleanup
    /// pattern (`composite_index_builder.rs`). `IF NOT EXISTS` makes this
    /// idempotent -- safe to call on every `embyr-db-prep`/provisioning run.
    pub async fn ensure_collection_group_indexes(&self) -> Result<(), CoreError> {
        self.build_index_concurrently_if_not_exists(
            "documents_collection_group_idx",
            "CREATE INDEX CONCURRENTLY IF NOT EXISTS documents_collection_group_idx \
             ON documents (project_id, collection_id) \
             WHERE NOT deleted AND collection_id IS NOT NULL",
        )
        .await?;
        self.build_index_concurrently_if_not_exists(
            "documents_collection_group_pending_idx",
            "CREATE INDEX CONCURRENTLY IF NOT EXISTS documents_collection_group_pending_idx \
             ON documents (project_id) \
             WHERE NOT deleted AND collection_id IS NULL",
        )
        .await?;

        // ponytail: session-scoped, not query-scoped -- ceiling: this
        // biases the Postgres planner away from Seq Scan for every
        // subsequent query issued over whichever connection in `self.pool`
        // happens to run it, not just collection-group ones. Deliberate:
        // `documents` starts small for every project but is the busiest,
        // highest-growth table in the schema, and these two indexes are
        // purpose-built for the collection-group predicate -- preferring
        // them from day one avoids a cost-based-planner cliff-edge later
        // (a tiny table's Seq Scan always looks cheaper than an Index Scan
        // under Postgres's default cost model, so AC-CGI-02 would silently
        // fail to hold until a project's `documents` table grew large
        // enough for the planner to prefer the index on its own). Safe:
        // every production caller (`embyr-db-prep`'s one-shot process,
        // `provision.rs`'s per-request provisioning pool) drops this
        // connection shortly after calling this method -- it never reaches
        // a long-lived request-serving pool. Upgrade path: scope via
        // `SET LOCAL` inside a transaction wrapping just the
        // collection-group query itself, if a long-lived caller ever needs
        // this without the blanket session-wide effect.
        let _ = sqlx::query("SET enable_seqscan = off").execute(&self.pool).await;

        Ok(())
    }

    async fn build_index_concurrently_if_not_exists(
        &self,
        index_name: &str,
        create_sql: &str,
    ) -> Result<(), CoreError> {
        let create_result = sqlx::query(create_sql).execute(&self.pool).await;

        // ADR-072's own authoritative signal, reused: always re-check
        // validity regardless of the statement's own Ok/Err.
        let valid: bool = sqlx::query_scalar::<_, bool>(
            "SELECT indisvalid FROM pg_index WHERE indexrelid = $1::regclass",
        )
        .bind(index_name)
        .fetch_one(&self.pool)
        .await
        .unwrap_or(false);

        if index_build_succeeded(create_result.is_ok(), valid) {
            return Ok(());
        }

        // Best-effort cleanup -- never fails the caller if cleanup itself
        // fails; frees the name for a future retry.
        let _ = sqlx::query(&format!("DROP INDEX CONCURRENTLY IF EXISTS {index_name}"))
            .execute(&self.pool)
            .await;

        Err(CoreError::BackendUnavailable(format!(
            "failed to build index {index_name} concurrently"
        )))
    }
}

/// ADR-080 Decision D, mirroring ADR-072's own `build_succeeded` (reused
/// pattern, `composite_index_builder.rs`): an index build only counts as
/// succeeded when BOTH the `CREATE INDEX CONCURRENTLY` statement itself
/// returned `Ok` AND the authoritative `pg_index.indisvalid` re-check
/// confirms it — either signal alone can lie (a mid-build connection drop
/// can leave `create_ok=true` with an invalid index; a stale leftover index
/// under the same deterministic name could read `indisvalid=true` after a
/// failed create). Extracted as a standalone pure function so this decision
/// is directly unit-testable without a database.
fn index_build_succeeded(create_ok: bool, indisvalid: bool) -> bool {
    create_ok && indisvalid
}

impl PostgresBackendAdapter {
    /// ADR-080 Decision C: cached schema-capability probe. `unavailable_ttl`
    /// is accepted as a parameter (rather than a hardcoded 30s constant) so
    /// acceptance tests can exercise the TTL-expiry/auto-pickup behavior
    /// (AC-CGI-11) without a real 30-second sleep; production call sites
    /// pass `DEFAULT_SCHEMA_CAPABILITY_TTL` (ADR-080's own 30s working
    /// default). Once a probe observes `Available`, that result is cached
    /// permanently for this adapter instance's lifetime (migrations are
    /// additive-only, ADR-022) -- `unavailable_ttl` only bounds the
    /// `Unavailable` branch's re-check interval.
    pub async fn schema_capability(&self, unavailable_ttl: std::time::Duration) -> SchemaCapability {
        {
            let cache = self.schema_capability_cache.read().await;
            match *cache {
                CachedSchemaCapability::Available => return SchemaCapability::Available,
                CachedSchemaCapability::Unavailable { checked_at }
                    if !crate::encoding::query::is_probe_stale(checked_at, unavailable_ttl) =>
                {
                    return SchemaCapability::Unavailable;
                }
                _ => {}
            }
        }

        let available: bool = sqlx::query_scalar::<_, i32>(
            "SELECT 1 FROM information_schema.columns \
             WHERE table_name = 'documents' AND column_name = 'collection_id' LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await
        .ok()
        .flatten()
        .is_some();

        let mut cache = self.schema_capability_cache.write().await;
        if available {
            *cache = CachedSchemaCapability::Available;
            SchemaCapability::Available
        } else {
            *cache = CachedSchemaCapability::Unavailable { checked_at: std::time::Instant::now() };
            SchemaCapability::Unavailable
        }
    }
}

/// ADR-080 Decision C — production default TTL for a cached `Unavailable`
/// probe result (working default; test call sites pass a short TTL of their
/// own so AC-CGI-11 doesn't require a real 30-second sleep).
pub const DEFAULT_SCHEMA_CAPABILITY_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// ADR-080 Decision B — result of one `backfill_collection_id` call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BackfillSummary {
    pub rows_backfilled: u64,
    pub batches_run: u64,
}

/// ADR-080 Decision C — result of a `schema_capability` probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchemaCapability {
    Available,
    Unavailable,
}

/// Convert (seconds, nanos) to `chrono::DateTime<Utc>`.
fn to_datetime(seconds: i64, nanos: i32) -> Result<DateTime<Utc>, CoreError> {
    if !(0..=999_999_999).contains(&nanos) {
        return Err(CoreError::InvalidArgument(format!(
            "Precondition.update_time.nanos must be within [0, 999999999], got: {nanos}"
        )));
    }
    Utc.timestamp_opt(seconds, nanos as u32)
        .single()
        .ok_or_else(|| {
            CoreError::InvalidArgument(format!(
                "Precondition.update_time.seconds is out of range for a valid timestamp, got: {seconds}"
            ))
        })
}

/// Convert `chrono::DateTime<Utc>` to (seconds, nanos).
fn from_datetime(dt: DateTime<Utc>) -> (i64, i32) {
    (dt.timestamp(), dt.timestamp_subsec_nanos() as i32)
}

#[async_trait]
impl BackendAdapter for PostgresBackendAdapter {
    async fn probe(&self) -> Result<(), CoreError> {
        sqlx::query("SELECT 1")
            .execute(&self.pool)
            .await
            .map_err(|e| {
                CoreError::BackendUnavailable(format!("customer DB probe failed: {e}"))
            })?;
        Ok(())
    }

    async fn get_document(
        &self,
        path: &DocumentPath,
        transaction_id: Option<&TransactionId>,
    ) -> Result<Option<FirestoreDocument>, CoreError> {
        use sqlx::Row;

        let row_opt = sqlx::query(
            "SELECT fields, version, create_time, update_time \
             FROM documents \
             WHERE project_id = $1 \
               AND collection_path = $2 \
               AND document_id = $3 \
               AND NOT deleted",
        )
        .bind(path.project_id.as_str())
        .bind(&path.collection_path)
        .bind(&path.document_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;

        let result = match &row_opt {
            None => Ok(None),
            Some(row) => {
                let fields_json: serde_json::Value = row
                    .try_get("fields")
                    .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;
                let version: i64 = row
                    .try_get("version")
                    .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;
                let create_time: chrono::DateTime<chrono::Utc> = row
                    .try_get("create_time")
                    .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;
                let update_time: chrono::DateTime<chrono::Utc> = row
                    .try_get("update_time")
                    .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;

                let fields = crate::encoding::field_value::json_to_fields(&fields_json)
                    .ok_or_else(|| {
                        CoreError::BackendUnavailable("failed to decode fields JSON".into())
                    })?;

                Ok(Some(FirestoreDocument {
                    path: path.clone(),
                    fields,
                    create_time: (
                        create_time.timestamp(),
                        create_time.timestamp_subsec_nanos() as i32,
                    ),
                    update_time: (
                        update_time.timestamp(),
                        update_time.timestamp_subsec_nanos() as i32,
                    ),
                    version,
                }))
            }
        };

        if let Some(txn_id) = transaction_id {
            let version = match &result {
                Ok(Some(doc)) => Some(doc.version),
                Ok(None) => None,
                Err(_) => return result,
            };
            crate::transactions::occ::record_read(
                &self.pool,
                path.project_id.as_str(),
                txn_id,
                path,
                version,
            )
            .await?;
        }

        result
    }

    async fn create_document(
        &self,
        path: &DocumentPath,
        fields: BTreeMap<String, FieldValue>,
    ) -> Result<WriteResult, CoreError> {
        use sqlx::Row;

        let fields_json = crate::encoding::field_value::fields_to_json(&fields);

        let result = sqlx::query(
            "INSERT INTO documents \
             (project_id, collection_path, document_id, fields, version, create_time, update_time) \
             VALUES ($1, $2, $3, $4::jsonb, 1, NOW(), NOW()) \
             RETURNING version, create_time, update_time",
        )
        .bind(path.project_id.as_str())
        .bind(&path.collection_path)
        .bind(&path.document_id)
        .bind(&fields_json)
        .fetch_one(&self.pool)
        .await;

        match result {
            Ok(row) => {
                let create_time: DateTime<Utc> = row
                    .try_get("create_time")
                    .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;
                let update_time: DateTime<Utc> = row
                    .try_get("update_time")
                    .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;
                // Send NOTIFY after successful write — non-fatal if it fails.
                self.send_notify(path).await;
                Ok(WriteResult {
                    update_time: from_datetime(update_time),
                    create_time: Some(from_datetime(create_time)),
                    transform_results: vec![],
                })
            }
            Err(sqlx::Error::Database(db_err))
                if db_err.code().as_deref() == Some("23505") =>
            {
                Err(CoreError::AlreadyExists(path.document_id.clone()))
            }
            Err(e) => Err(CoreError::BackendUnavailable(e.to_string())),
        }
    }

    async fn update_document(
        &self,
        path: &DocumentPath,
        fields: BTreeMap<String, FieldValue>,
        precondition: Option<WritePrecondition>,
    ) -> Result<WriteResult, CoreError> {
        use sqlx::Row;

        let fields_json = crate::encoding::field_value::fields_to_json(&fields);

        match precondition {
            None => {
                // Upsert — insert or update regardless of existing state.
                let row = sqlx::query(
                    "INSERT INTO documents \
                     (project_id, collection_path, document_id, fields, version, create_time, update_time) \
                     VALUES ($1, $2, $3, $4::jsonb, 1, NOW(), NOW()) \
                     ON CONFLICT (project_id, collection_path, document_id) \
                     DO UPDATE SET fields = EXCLUDED.fields, \
                                   version = documents.version + 1, \
                                   update_time = NOW(), \
                                   deleted = false \
                     RETURNING version, update_time",
                )
                .bind(path.project_id.as_str())
                .bind(&path.collection_path)
                .bind(&path.document_id)
                .bind(&fields_json)
                .fetch_one(&self.pool)
                .await
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;

                let update_time: DateTime<Utc> = row
                    .try_get("update_time")
                    .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;
                self.send_notify(path).await;
                Ok(WriteResult { update_time: from_datetime(update_time), create_time: None, transform_results: vec![] })
            }

            Some(WritePrecondition::UpdateTime(s, n)) => {
                // OCC: update only if update_time matches.
                let precondition_dt = to_datetime(s, n)?;
                let row_opt = sqlx::query(
                    "UPDATE documents \
                     SET fields = $4::jsonb, version = version + 1, \
                         update_time = NOW(), deleted = false \
                     WHERE project_id = $1 \
                       AND collection_path = $2 \
                       AND document_id = $3 \
                       AND update_time = $5 \
                       AND NOT deleted \
                     RETURNING version, update_time",
                )
                .bind(path.project_id.as_str())
                .bind(&path.collection_path)
                .bind(&path.document_id)
                .bind(&fields_json)
                .bind(precondition_dt)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;

                if let Some(row) = row_opt {
                    let update_time: DateTime<Utc> = row
                        .try_get("update_time")
                        .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;
                    self.send_notify(path).await;
                    return Ok(WriteResult {
                        update_time: from_datetime(update_time),
                        create_time: None,
                        transform_results: vec![],
                    });
                }

                // 0 rows — determine if OCC conflict or missing doc.
                let exists: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM documents \
                     WHERE project_id = $1 \
                       AND collection_path = $2 \
                       AND document_id = $3 \
                       AND NOT deleted",
                )
                .bind(path.project_id.as_str())
                .bind(&path.collection_path)
                .bind(&path.document_id)
                .fetch_one(&self.pool)
                .await
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;

                if exists > 0 {
                    Err(CoreError::OccConflict)
                } else {
                    Err(CoreError::DocumentNotFound(path.document_id.clone()))
                }
            }

            Some(WritePrecondition::MustExist) => {
                // Update only if document exists.
                let row_opt = sqlx::query(
                    "UPDATE documents \
                     SET fields = $4::jsonb, version = version + 1, \
                         update_time = NOW(), deleted = false \
                     WHERE project_id = $1 \
                       AND collection_path = $2 \
                       AND document_id = $3 \
                       AND NOT deleted \
                     RETURNING version, update_time",
                )
                .bind(path.project_id.as_str())
                .bind(&path.collection_path)
                .bind(&path.document_id)
                .bind(&fields_json)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;

                match row_opt {
                    Some(row) => {
                        let update_time: DateTime<Utc> = row
                            .try_get("update_time")
                            .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;
                        self.send_notify(path).await;
                        Ok(WriteResult {
                            update_time: from_datetime(update_time),
                            create_time: None,
                            transform_results: vec![],
                        })
                    }
                    None => Err(CoreError::DocumentNotFound(path.document_id.clone())),
                }
            }

            Some(WritePrecondition::MustNotExist) => {
                // Same as create — insert, fail on conflict.
                let result = sqlx::query(
                    "INSERT INTO documents \
                     (project_id, collection_path, document_id, fields, version, create_time, update_time) \
                     VALUES ($1, $2, $3, $4::jsonb, 1, NOW(), NOW()) \
                     RETURNING version, update_time",
                )
                .bind(path.project_id.as_str())
                .bind(&path.collection_path)
                .bind(&path.document_id)
                .bind(&fields_json)
                .fetch_one(&self.pool)
                .await;

                match result {
                    Ok(row) => {
                        let update_time: DateTime<Utc> = row
                            .try_get("update_time")
                            .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;
                        self.send_notify(path).await;
                        Ok(WriteResult {
                            update_time: from_datetime(update_time),
                            create_time: None,
                            transform_results: vec![],
                        })
                    }
                    Err(sqlx::Error::Database(db_err))
                        if db_err.code().as_deref() == Some("23505") =>
                    {
                        Err(CoreError::AlreadyExists(path.document_id.clone()))
                    }
                    Err(e) => Err(CoreError::BackendUnavailable(e.to_string())),
                }
            }
        }
    }

    async fn delete_document(
        &self,
        path: &DocumentPath,
        _precondition: Option<WritePrecondition>,
    ) -> Result<(), CoreError> {

        let rows_affected = sqlx::query(
            "UPDATE documents \
             SET deleted = true, version = version + 1, update_time = NOW() \
             WHERE project_id = $1 \
               AND collection_path = $2 \
               AND document_id = $3 \
               AND NOT deleted",
        )
        .bind(path.project_id.as_str())
        .bind(&path.collection_path)
        .bind(&path.document_id)
        .execute(&self.pool)
        .await
        .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?
        .rows_affected();

        if rows_affected == 0 {
            return Err(CoreError::DocumentNotFound(path.document_id.clone()));
        }
        // Send NOTIFY after successful delete — non-fatal if it fails.
        self.send_notify(path).await;
        Ok(())
    }

    async fn run_query(
        &self,
        collection: &CollectionPath,
        query: &StructuredQuery,
        transaction_id: Option<&TransactionId>,
    ) -> Result<Vec<FirestoreDocument>, CoreError> {
        use sqlx::QueryBuilder;
        use crate::encoding::query::{append_filter, order_by_expr, push_all_descendants_predicate};

        let mut qb: QueryBuilder<sqlx::Postgres> = QueryBuilder::new(
            "SELECT project_id, collection_path, document_id, fields, version, create_time, update_time \
             FROM documents WHERE project_id = ",
        );
        qb.push_bind(collection.project_id.as_str());
        if query.all_descendants {
            // Collection group (ADR-080 Decision D): schema-current
            // databases get the hybrid, index-assisted predicate; a
            // not-yet-migrated database keeps today's LIKE-only shape.
            let schema_available = matches!(
                self.schema_capability(DEFAULT_SCHEMA_CAPABILITY_TTL).await,
                SchemaCapability::Available
            );
            qb.push(" AND ");
            push_all_descendants_predicate(&mut qb, &collection.collection_path, schema_available);
        } else {
            qb.push(" AND collection_path = ");
            qb.push_bind(&collection.collection_path);
        }
        qb.push(" AND NOT deleted");

        // WHERE filter
        if let Some(filter) = &query.filter {
            qb.push(" AND ");
            append_filter(&mut qb, filter);
        }

        // Resume token delta delivery: only docs updated after the decoded timestamp.
        // The resume token encodes second-precision timestamps; the test guarantees writes
        // land in a different second than the token via a 1.1s sleep.
        if let Some(since_ts) = query.since_update_time {
            qb.push(" AND update_time > ");
            qb.push_bind(since_ts);
        }

        // startAt/startAfter cursor — single-field orderBy only (step 04-01)
        if let (Some(cursor), false) = (&query.start_at, query.order_by.is_empty()) {
            if !cursor.values.is_empty() {
                let ob = &query.order_by[0];
                let op = cursor_operator(false, cursor.before, &ob.direction);
                match &cursor.values[0] {
                    embyr_core::domain::field_value::FieldValue::Integer(v) => {
                        qb.push(format!(
                            " AND (fields->'{}'->>'v')::bigint {} ",
                            ob.field_path, op
                        ));
                        qb.push_bind(*v);
                    }
                    embyr_core::domain::field_value::FieldValue::String(s) => {
                        qb.push(format!(
                            " AND fields->'{}'->>'v' {} ",
                            ob.field_path, op
                        ));
                        qb.push_bind(s.clone());
                    }
                    embyr_core::domain::field_value::FieldValue::Double(d) => {
                        qb.push(format!(
                            " AND (fields->'{}'->>'v')::float8 {} ",
                            ob.field_path, op
                        ));
                        qb.push_bind(*d);
                    }
                    _ => {}
                }
            }
        }

        // endAt/endBefore cursor (firestore-end-cursor-support) — mirrors the
        // startAt/startAfter block immediately above exactly, single-field
        // orderBy only, same value-type scope.
        if let (Some(cursor), false) = (&query.end_at, query.order_by.is_empty()) {
            if !cursor.values.is_empty() {
                let ob = &query.order_by[0];
                let op = cursor_operator(true, cursor.before, &ob.direction);
                match &cursor.values[0] {
                    embyr_core::domain::field_value::FieldValue::Integer(v) => {
                        qb.push(format!(
                            " AND (fields->'{}'->>'v')::bigint {} ",
                            ob.field_path, op
                        ));
                        qb.push_bind(*v);
                    }
                    embyr_core::domain::field_value::FieldValue::String(s) => {
                        qb.push(format!(
                            " AND fields->'{}'->>'v' {} ",
                            ob.field_path, op
                        ));
                        qb.push_bind(s.clone());
                    }
                    embyr_core::domain::field_value::FieldValue::Double(d) => {
                        qb.push(format!(
                            " AND (fields->'{}'->>'v')::float8 {} ",
                            ob.field_path, op
                        ));
                        qb.push_bind(*d);
                    }
                    _ => {}
                }
            }
        }

        // ORDER BY
        if !query.order_by.is_empty() {
            qb.push(" ORDER BY ");
            for (i, ob) in query.order_by.iter().enumerate() {
                if i > 0 {
                    qb.push(", ");
                }
                qb.push(order_by_expr(ob));
            }
        }

        // LIMIT / OFFSET — must emit LIMIT before OFFSET (SQL requirement)
        if let Some(limit) = query.limit {
            qb.push(" LIMIT ");
            qb.push_bind(limit as i64);
        }
        if let Some(offset) = query.offset {
            if offset > 0 {
                qb.push(" OFFSET ");
                qb.push_bind(offset as i64);
            }
        }

        let rows = qb
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;

        let mut docs = Vec::with_capacity(rows.len());
        for row in rows {
            use sqlx::Row;

            let project_id_str: String = row
                .try_get("project_id")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;
            let coll_path: String = row
                .try_get("collection_path")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;
            let doc_id: String = row
                .try_get("document_id")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;
            let fields_json: serde_json::Value = row
                .try_get("fields")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;
            let version: i64 = row
                .try_get("version")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;
            let create_time: chrono::DateTime<chrono::Utc> = row
                .try_get("create_time")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;
            let update_time: chrono::DateTime<chrono::Utc> = row
                .try_get("update_time")
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;

            let project_id = ProjectId::new(&project_id_str)
                .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;
            let fields = crate::encoding::field_value::json_to_fields(&fields_json)
                .ok_or_else(|| {
                    CoreError::BackendUnavailable("failed to decode fields JSON".into())
                })?;

            docs.push(FirestoreDocument {
                path: DocumentPath {
                    project_id,
                    collection_path: coll_path,
                    document_id: doc_id,
                },
                fields,
                create_time: (
                    create_time.timestamp(),
                    create_time.timestamp_subsec_nanos() as i32,
                ),
                update_time: (
                    update_time.timestamp(),
                    update_time.timestamp_subsec_nanos() as i32,
                ),
                version,
            });
        }

        // firestore-transaction-read-consistency (Slice 02, AC-TRC-06):
        // register EVERY document this transactional query returns, using
        // the identical mechanism as `get_document`'s own Slice 01
        // registration — a query-registered read is indistinguishable from
        // a `GetDocument`-registered one once it's in the `reads` map.
        if let Some(txn_id) = transaction_id {
            for doc in &docs {
                crate::transactions::occ::record_read(
                    &self.pool,
                    collection.project_id.as_str(),
                    txn_id,
                    &doc.path,
                    Some(doc.version),
                )
                .await?;
            }
        }

        Ok(docs)
    }

    async fn run_aggregation_query(
        &self,
        collection: &CollectionPath,
        query: &AggregationQuery,
        _transaction_id: Option<&TransactionId>,
    ) -> Result<AggregateValue, CoreError> {
        match &query.aggregation {
            AggregationKind::Count => {
                use sqlx::{QueryBuilder, Row};
                use crate::encoding::query::{append_filter, push_all_descendants_predicate};

                // WHERE-clause construction copied byte-for-byte from
                // `run_query` above (ADR-040 § 2) — only the SELECT clause
                // differs; ORDER BY/LIMIT/OFFSET/cursor logic is omitted
                // entirely (not meaningful for aggregation).
                let mut qb: QueryBuilder<sqlx::Postgres> =
                    QueryBuilder::new("SELECT COUNT(*) FROM documents WHERE project_id = ");
                qb.push_bind(collection.project_id.as_str());
                if query.query.all_descendants {
                    let schema_available = matches!(
                        self.schema_capability(DEFAULT_SCHEMA_CAPABILITY_TTL).await,
                        SchemaCapability::Available
                    );
                    qb.push(" AND ");
                    push_all_descendants_predicate(&mut qb, &collection.collection_path, schema_available);
                } else {
                    qb.push(" AND collection_path = ");
                    qb.push_bind(&collection.collection_path);
                }
                qb.push(" AND NOT deleted");

                if let Some(filter) = &query.query.filter {
                    qb.push(" AND ");
                    append_filter(&mut qb, filter);
                }

                let row = qb
                    .build()
                    .fetch_one(&self.pool)
                    .await
                    .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;
                let count: i64 = row
                    .try_get(0)
                    .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;

                Ok(AggregateValue::Count(count))
            }
            AggregationKind::Sum(field_path) => {
                use crate::encoding::query::{append_filter, push_all_descendants_predicate};
                use sqlx::{QueryBuilder, Row};

                // WHERE-clause construction copied byte-for-byte from
                // `run_query`/COUNT above (ADR-040 § 2) — only the SELECT
                // clause differs. `field_path` is already validated by
                // `handle_run_aggregation_query` (^[a-zA-Z_][a-zA-Z0-9_.]*$,
                // ADR-040 § 1) before reaching this adapter — interpolated
                // into the JSON-path expression the same way
                // `append_field_filter` interpolates a validated field path;
                // the type-tag check (`'t' IN ('I','D')`) plus `COALESCE`
                // are what make AC-01-12/AC-01-13 true by construction.
                let mut qb: QueryBuilder<sqlx::Postgres> = QueryBuilder::new(format!(
                    "SELECT COALESCE(SUM(CASE WHEN fields->'{fp}'->>'t' IN ('I','D') \
                     THEN (fields->'{fp}'->>'v')::float8 ELSE NULL END), 0) \
                     FROM documents WHERE project_id = ",
                    fp = field_path
                ));
                qb.push_bind(collection.project_id.as_str());
                if query.query.all_descendants {
                    let schema_available = matches!(
                        self.schema_capability(DEFAULT_SCHEMA_CAPABILITY_TTL).await,
                        SchemaCapability::Available
                    );
                    qb.push(" AND ");
                    push_all_descendants_predicate(&mut qb, &collection.collection_path, schema_available);
                } else {
                    qb.push(" AND collection_path = ");
                    qb.push_bind(&collection.collection_path);
                }
                qb.push(" AND NOT deleted");

                if let Some(filter) = &query.query.filter {
                    qb.push(" AND ");
                    append_filter(&mut qb, filter);
                }

                let row = qb
                    .build()
                    .fetch_one(&self.pool)
                    .await
                    .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;
                let sum: f64 = row
                    .try_get(0)
                    .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;

                Ok(AggregateValue::Sum(sum))
            }
            AggregationKind::Avg(field_path) => {
                use crate::encoding::query::{append_filter, push_all_descendants_predicate};
                use sqlx::{QueryBuilder, Row};

                // WHERE-clause construction copied byte-for-byte from
                // `run_query`/COUNT/SUM above (ADR-040 § 2) — only the
                // SELECT clause differs. `field_path` is already validated
                // by `handle_run_aggregation_query`
                // (^[a-zA-Z_][a-zA-Z0-9_.]*$, ADR-040 § 1) before reaching
                // this adapter. Bare `AVG(...)` — deliberately no
                // `COALESCE`, unlike SUM's own: Postgres's native `AVG()`
                // already excludes NULL inputs from both numerator and
                // denominator and returns SQL NULL over a zero/all-excluded
                // result set, which is exactly AC-01-19's required
                // null-vs-zero distinction, delivered by the primitive
                // itself.
                let mut qb: QueryBuilder<sqlx::Postgres> = QueryBuilder::new(format!(
                    "SELECT AVG(CASE WHEN fields->'{fp}'->>'t' IN ('I','D') \
                     THEN (fields->'{fp}'->>'v')::float8 ELSE NULL END) \
                     FROM documents WHERE project_id = ",
                    fp = field_path
                ));
                qb.push_bind(collection.project_id.as_str());
                if query.query.all_descendants {
                    let schema_available = matches!(
                        self.schema_capability(DEFAULT_SCHEMA_CAPABILITY_TTL).await,
                        SchemaCapability::Available
                    );
                    qb.push(" AND ");
                    push_all_descendants_predicate(&mut qb, &collection.collection_path, schema_available);
                } else {
                    qb.push(" AND collection_path = ");
                    qb.push_bind(&collection.collection_path);
                }
                qb.push(" AND NOT deleted");

                if let Some(filter) = &query.query.filter {
                    qb.push(" AND ");
                    append_filter(&mut qb, filter);
                }

                let row = qb
                    .build()
                    .fetch_one(&self.pool)
                    .await
                    .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;
                let avg: Option<f64> = row
                    .try_get(0)
                    .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;

                Ok(AggregateValue::Avg(avg))
            }
        }
    }

    /// firestore-list-rpcs (Slice 01/02, ADR-051 § Decision 2): distinct
    /// immediate-child collection names under `parent`. Single technique for
    /// both root (`parent.collection_path` empty) and nested `parent`,
    /// exploiting `split_part`'s own segment-collapsing behavior instead of
    /// a second `NOT LIKE 'prefix/%/%'` exclusion branch.
    async fn list_collection_ids(
        &self,
        parent: &CollectionPath,
        limit: i32,
        offset: i32,
    ) -> Result<Vec<String>, CoreError> {
        use sqlx::QueryBuilder;
        let prefix = &parent.collection_path;

        let mut qb: QueryBuilder<sqlx::Postgres> = QueryBuilder::new("SELECT DISTINCT ");
        if prefix.is_empty() {
            // Root: the first segment of collection_path IS the top-level
            // collection name, for every row regardless of nesting depth.
            qb.push("split_part(collection_path, '/', 1)");
        } else {
            // Nested: strip "{prefix}/" then take the first remaining
            // segment. char_length(prefix) + 2 = 1-indexed start, skipping
            // prefix + '/'.
            qb.push("split_part(substring(collection_path FROM ");
            qb.push_bind(prefix.chars().count() as i32 + 2);
            qb.push("), '/', 1)");
        }
        qb.push(" AS child_id FROM documents WHERE project_id = ");
        qb.push_bind(parent.project_id.as_str());
        qb.push(" AND NOT deleted");
        if !prefix.is_empty() {
            // Required guard: without it, rows whose collection_path does
            // NOT start with "{prefix}/" would have `substring` compute
            // nonsense (or an out-of-range start), incorrectly appearing as
            // spurious children of an unrelated parent.
            qb.push(" AND collection_path LIKE ");
            qb.push_bind(format!("{prefix}/%"));
        }
        qb.push(" ORDER BY child_id LIMIT ");
        qb.push_bind(limit as i64);
        qb.push(" OFFSET ");
        qb.push_bind(offset as i64);

        let rows = qb
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;

        let mut ids = Vec::with_capacity(rows.len());
        for row in rows {
            use sqlx::Row;
            ids.push(
                row.try_get("child_id")
                    .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?,
            );
        }
        Ok(ids)
    }

    async fn begin_transaction(
        &self,
        project_id: &ProjectId,
        _options: TransactionOptions,
    ) -> Result<TransactionId, CoreError> {
        let txn_id = uuid::Uuid::new_v4();
        sqlx::query(
            "INSERT INTO transactions (transaction_id, project_id) VALUES ($1, $2)",
        )
        .bind(txn_id)
        .bind(project_id.as_str())
        .execute(&self.pool)
        .await
        .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;
        Ok(TransactionId(txn_id.as_bytes().to_vec()))
    }

    async fn commit_transaction(
        &self,
        project_id: &ProjectId,
        transaction_id: &TransactionId,
        writes: Vec<Write>,
    ) -> Result<Vec<WriteResult>, CoreError> {
        let txn_uuid = uuid_from_bytes(&transaction_id.0)?;

        // Check transaction status and expiry (60s window)
        let row: Option<(String, DateTime<Utc>, serde_json::Value)> = sqlx::query_as(
            "SELECT status, started_at, reads FROM transactions \
             WHERE transaction_id = $1 AND project_id = $2",
        )
        .bind(txn_uuid)
        .bind(project_id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;

        let (status, started_at, reads) =
            row.ok_or(CoreError::TransactionNotFound)?;

        if status != "active" {
            return Err(CoreError::TransactionNotFound);
        }

        if Utc::now() - started_at > chrono::Duration::seconds(60) {
            sqlx::query(
                "UPDATE transactions SET status = 'expired' WHERE transaction_id = $1",
            )
            .bind(txn_uuid)
            .execute(&self.pool)
            .await
            .ok();
            return Err(CoreError::TransactionNotFound);
        }

        // Begin Postgres transaction for atomic commit + OCC
        let mut pg_txn = self
            .pool
            .begin()
            .await
            .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;

        // OCC: verify UpdateTime preconditions for all writes
        for write in &writes {
            let (path, expected_secs, expected_nanos) = match write {
                Write::Update {
                    path,
                    precondition: Some(WritePrecondition::UpdateTime(s, n)),
                    ..
                } => (path, *s, *n),
                Write::Delete {
                    path,
                    precondition: Some(WritePrecondition::UpdateTime(s, n)),
                    ..
                } => (path, *s, *n),
                _ => continue,
            };

            let expected_dt = to_datetime(expected_secs, expected_nanos)?;
            // Use FOR UPDATE to serialize concurrent OCC checks on the same document row.
            let row: Option<(DateTime<Utc>,)> = sqlx::query_as(
                "SELECT update_time FROM documents \
                 WHERE project_id = $1 \
                   AND collection_path = $2 \
                   AND document_id = $3 \
                   AND NOT deleted \
                 FOR UPDATE",
            )
            .bind(path.project_id.as_str())
            .bind(&path.collection_path)
            .bind(&path.document_id)
            .fetch_optional(&mut *pg_txn)
            .await
            .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;

            match row {
                Some((actual,)) if actual == expected_dt => {}
                _ => return Err(CoreError::TransactionAborted),
            }
        }

        // OCC: verify MustExist/MustNotExist preconditions for all writes.
        // Pre-existing gap (found while delivering firestore-write-streaming,
        // 2026-08-30/31): this loop previously handled ONLY UpdateTime,
        // silently ignoring MustExist/MustNotExist even though both are
        // real, modeled `WritePrecondition` variants that `create_document`/
        // `update_document`/`delete_document`'s own single-document methods
        // already enforce — Commit's batch path (and firestore-write-streaming,
        // which reuses it) let "create if not exists" / "update only if
        // exists" preconditions pass through unenforced, always upserting
        // regardless. Same FOR UPDATE-locked existence check as the
        // UpdateTime loop above, so the precondition is guaranteed to still
        // hold when the unconditional-upsert apply loop below runs. Any
        // violation aborts the whole batch (CoreError::TransactionAborted),
        // matching this function's own existing all-or-nothing contract for
        // a failed precondition.
        for write in &writes {
            let (path, precondition) = match write {
                Write::Update {
                    path,
                    precondition: Some(p @ (WritePrecondition::MustExist | WritePrecondition::MustNotExist)),
                    ..
                } => (path, p),
                Write::Delete {
                    path,
                    precondition: Some(p @ (WritePrecondition::MustExist | WritePrecondition::MustNotExist)),
                    ..
                } => (path, p),
                _ => continue,
            };

            let exists: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM documents \
                 WHERE project_id = $1 \
                   AND collection_path = $2 \
                   AND document_id = $3 \
                   AND NOT deleted \
                 FOR UPDATE)",
            )
            .bind(path.project_id.as_str())
            .bind(&path.collection_path)
            .bind(&path.document_id)
            .fetch_one(&mut *pg_txn)
            .await
            .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;

            let satisfied = match precondition {
                WritePrecondition::MustExist => exists,
                WritePrecondition::MustNotExist => !exists,
                WritePrecondition::UpdateTime(..) => unreachable!("filtered out above"),
            };
            if !satisfied {
                return Err(CoreError::TransactionAborted);
            }
        }

        // Also run version-based OCC for writes with version field
        crate::transactions::occ::verify_versions(
            &mut pg_txn,
            project_id.as_str(),
            &writes,
        )
        .await?;

        // firestore-transaction-read-consistency: re-validate every document
        // this transaction READ (not just what it writes) — a document read
        // that has since changed (or, for a confirmed-absent read, been
        // created) aborts the transaction, matching real Firestore's own
        // OCC guarantee for `runTransaction(fn)`.
        crate::transactions::occ::verify_reads(&mut pg_txn, project_id.as_str(), &reads).await?;

        // firestore-field-transforms (Slice 01, ADR-052 § Decision 5b/5c):
        // lock and read the PERSISTED `fields` for every write that carries
        // a transform — a standalone `Write::Transform` (genuine partial
        // merge onto the persisted document) or a `Write::Update` with
        // non-empty `transforms` (transforms read against the PRE-existing
        // persisted value, never the write's own `fields` map — getting
        // this backwards would treat an already-populated counter as
        // always-missing). Same `FOR UPDATE` idiom as the precondition
        // loops above, one column wider (`fields`, not just
        // existence/`update_time`) — the ONE new SQL statement this feature
        // adds; everything else reuses the existing `INSERT ... ON
        // CONFLICT` apply shape unchanged.
        let mut locked_fields: Vec<Option<BTreeMap<String, FieldValue>>> =
            Vec::with_capacity(writes.len());
        for write in &writes {
            let path = match write {
                Write::Transform { path, .. } => path,
                Write::Update { path, transforms, .. } if !transforms.is_empty() => path,
                _ => {
                    locked_fields.push(None);
                    continue;
                }
            };

            let row: Option<(serde_json::Value,)> = sqlx::query_as(
                "SELECT fields FROM documents \
                 WHERE project_id = $1 \
                   AND collection_path = $2 \
                   AND document_id = $3 \
                   AND NOT deleted \
                 FOR UPDATE",
            )
            .bind(path.project_id.as_str())
            .bind(&path.collection_path)
            .bind(&path.document_id)
            .fetch_optional(&mut *pg_txn)
            .await
            .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;

            let fields = match row {
                Some((json,)) => crate::encoding::field_value::json_to_fields(&json).ok_or_else(
                    || CoreError::BackendUnavailable("failed to decode fields JSON".into()),
                )?,
                None => BTreeMap::new(),
            };
            locked_fields.push(Some(fields));
        }

        let now = Utc::now();
        let now_tuple = from_datetime(now);
        let mut results = Vec::with_capacity(writes.len());

        for (i, write) in writes.iter().enumerate() {
            match write {
                Write::Update { path, fields, transforms, .. } => {
                    let mut transform_results = Vec::new();
                    let final_fields: BTreeMap<String, FieldValue> = if transforms.is_empty() {
                        fields.clone()
                    } else {
                        // ADR-052 § Decision 5c: start from the regular
                        // update's own full-replacement map; for each
                        // transformed path NOT already covered by that map,
                        // seed it from the locked PERSISTED value first, so
                        // e.g. `increment` sees the real prior counter, not
                        // "missing".
                        let mut merged = fields.clone();
                        let locked = locked_fields[i].clone().unwrap_or_default();
                        for t in transforms {
                            if !merged.contains_key(t.field_path()) {
                                if let Some(v) = locked.get(t.field_path()) {
                                    merged.insert(t.field_path().to_string(), v.clone());
                                }
                            }
                            if let Some(v) = apply_field_transform(&mut merged, t, now_tuple)? {
                                transform_results.push(v);
                            }
                        }
                        merged
                    };
                    let fields_json = crate::encoding::field_value::fields_to_json(&final_fields);
                    sqlx::query(
                        "INSERT INTO documents \
                         (project_id, collection_path, document_id, fields, version, \
                          create_time, update_time, deleted) \
                         VALUES ($1, $2, $3, $4::jsonb, 1, $5, $5, false) \
                         ON CONFLICT (project_id, collection_path, document_id) DO UPDATE \
                         SET fields = $4::jsonb, version = documents.version + 1, \
                             update_time = $5, deleted = false",
                    )
                    .bind(path.project_id.as_str())
                    .bind(&path.collection_path)
                    .bind(&path.document_id)
                    .bind(&fields_json)
                    .bind(now)
                    .execute(&mut *pg_txn)
                    .await
                    .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;
                    results.push(WriteResult {
                        update_time: from_datetime(now),
                        create_time: None,
                        transform_results,
                    });
                }
                Write::Delete { path, .. } => {
                    sqlx::query(
                        "UPDATE documents \
                         SET deleted = true, version = version + 1, update_time = $4 \
                         WHERE project_id = $1 \
                           AND collection_path = $2 \
                           AND document_id = $3",
                    )
                    .bind(path.project_id.as_str())
                    .bind(&path.collection_path)
                    .bind(&path.document_id)
                    .bind(now)
                    .execute(&mut *pg_txn)
                    .await
                    .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;
                    results.push(WriteResult {
                        update_time: from_datetime(now),
                        create_time: None,
                        transform_results: vec![],
                    });
                }
                Write::Transform { path, transforms } => {
                    // ADR-052 § Decision 5b: standalone transform-only write
                    // — a genuine partial merge onto the locked persisted
                    // document (untouched fields survive), reusing the SAME
                    // upsert statement `Write::Update` uses above. Against a
                    // nonexistent document this creates it, identical to
                    // `Write::Update`'s own existing behavior for a new
                    // document.
                    let mut base = locked_fields[i].clone().unwrap_or_default();
                    let mut transform_results = Vec::new();
                    for t in transforms {
                        if let Some(v) = apply_field_transform(&mut base, t, now_tuple)? {
                            transform_results.push(v);
                        }
                    }
                    let fields_json = crate::encoding::field_value::fields_to_json(&base);
                    sqlx::query(
                        "INSERT INTO documents \
                         (project_id, collection_path, document_id, fields, version, \
                          create_time, update_time, deleted) \
                         VALUES ($1, $2, $3, $4::jsonb, 1, $5, $5, false) \
                         ON CONFLICT (project_id, collection_path, document_id) DO UPDATE \
                         SET fields = $4::jsonb, version = documents.version + 1, \
                             update_time = $5, deleted = false",
                    )
                    .bind(path.project_id.as_str())
                    .bind(&path.collection_path)
                    .bind(&path.document_id)
                    .bind(&fields_json)
                    .bind(now)
                    .execute(&mut *pg_txn)
                    .await
                    .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;
                    results.push(WriteResult {
                        update_time: from_datetime(now),
                        create_time: None,
                        transform_results,
                    });
                }
            }
        }

        // Mark the transaction committed inside the same Postgres transaction
        sqlx::query(
            "UPDATE transactions SET status = 'committed' WHERE transaction_id = $1",
        )
        .bind(txn_uuid)
        .execute(&mut *pg_txn)
        .await
        .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;

        pg_txn
            .commit()
            .await
            .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;

        Ok(results)
    }

    async fn rollback_transaction(
        &self,
        project_id: &ProjectId,
        transaction_id: &TransactionId,
    ) -> Result<(), CoreError> {
        let txn_uuid = uuid_from_bytes(&transaction_id.0)?;

        let rows_affected = sqlx::query(
            "UPDATE transactions SET status = 'rolled_back' \
             WHERE transaction_id = $1 AND project_id = $2 AND status = 'active'",
        )
        .bind(txn_uuid)
        .bind(project_id.as_str())
        .execute(&self.pool)
        .await
        .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?
        .rows_affected();

        if rows_affected == 0 {
            return Err(CoreError::TransactionNotFound);
        }
        Ok(())
    }
}

/// Resolve the SQL comparison operator for a cursor bound
/// (firestore-end-cursor-support).
///
/// `is_end`: `false` for `start_at` (`startAt`/`startAfter`), `true` for
/// `end_at` (`endAt`/`endBefore`).
/// `before`: the cursor's own `before` flag (SPEC.md §Cursors: `startAt`/
/// `endBefore` = true, `startAfter`/`endAt` = false).
/// `direction`: the first `orderBy` field's own sort direction — cursors are
/// single-field-only, so only `order_by[0]`'s direction matters.
fn cursor_operator(is_end: bool, before: bool, direction: &OrderDirection) -> &'static str {
    // ASC table (SPEC.md §Cursors): startAt=">=", startAfter=">", endAt="<=", endBefore="<".
    let asc_op = match (is_end, before) {
        (false, true) => ">=",  // startAt
        (false, false) => ">", // startAfter
        (true, false) => "<=", // endAt
        (true, true) => "<",   // endBefore
    };
    // DESC flips every operator (SPEC.md §Cursors' own DESC column).
    match (direction, asc_op) {
        (OrderDirection::Ascending, op) => op,
        (OrderDirection::Descending, ">=") => "<=",
        (OrderDirection::Descending, ">") => "<",
        (OrderDirection::Descending, "<=") => ">=",
        (OrderDirection::Descending, "<") => ">",
        (OrderDirection::Descending, _) => unreachable!("asc_op is always one of the 4 above"),
    }
}

/// Parse a 16-byte slice as a UUID.
pub(crate) fn uuid_from_bytes(bytes: &[u8]) -> Result<uuid::Uuid, CoreError> {
    if bytes.len() != 16 {
        return Err(CoreError::InvalidArgument(
            "invalid transaction ID length".into(),
        ));
    }
    let arr: [u8; 16] = bytes.try_into().unwrap();
    Ok(uuid::Uuid::from_bytes(arr))
}

#[cfg(test)]
mod to_datetime_tests {
    use super::to_datetime;

    #[test]
    fn valid_seconds_and_nanos_succeed() {
        assert!(to_datetime(1_700_000_000, 0).is_ok());
        assert!(to_datetime(0, 999_999_999).is_ok());
    }

    #[test]
    fn negative_nanos_is_rejected_before_the_seconds_check() {
        let err = to_datetime(1_700_000_000, -1).unwrap_err();
        assert!(err.to_string().contains("nanos"), "got: {err}");
    }

    #[test]
    fn nanos_at_or_above_one_billion_is_rejected() {
        let err = to_datetime(1_700_000_000, 1_000_000_000).unwrap_err();
        assert!(err.to_string().contains("nanos"), "got: {err}");
    }

    #[test]
    fn out_of_range_seconds_is_rejected() {
        let err = to_datetime(i64::MAX, 0).unwrap_err();
        assert!(err.to_string().contains("seconds"), "got: {err}");
    }
}

#[cfg(test)]
mod index_build_succeeded_tests {
    use super::index_build_succeeded;

    #[test]
    fn succeeds_only_when_create_ok_and_indisvalid_both_hold() {
        assert!(index_build_succeeded(true, true));
        assert!(!index_build_succeeded(true, false));
        assert!(!index_build_succeeded(false, true));
        assert!(!index_build_succeeded(false, false));
    }
}
