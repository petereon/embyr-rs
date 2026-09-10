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
        Ok(Self { pool })
    }

    /// Construct from an already-connected pool (used by test harnesses).
    pub fn new_from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Apply customer schema migrations from `migrations/customer/`.
    pub async fn migrate(&self) -> Result<(), CoreError> {
        MIGRATOR
            .run(&self.pool)
            .await
            .map_err(|e| CoreError::BackendUnavailable(e.to_string()))
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
        use crate::encoding::query::{append_filter, order_by_expr};

        let mut qb: QueryBuilder<sqlx::Postgres> = QueryBuilder::new(
            "SELECT project_id, collection_path, document_id, fields, version, create_time, update_time \
             FROM documents WHERE project_id = ",
        );
        qb.push_bind(collection.project_id.as_str());
        if query.all_descendants {
            // Collection group: match collection_path exactly OR as a nested sub-collection.
            qb.push(" AND (collection_path = ");
            qb.push_bind(&collection.collection_path);
            qb.push(" OR collection_path LIKE ");
            qb.push_bind(format!("%/{}", collection.collection_path));
            qb.push(")");
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
                use crate::encoding::query::append_filter;

                // WHERE-clause construction copied byte-for-byte from
                // `run_query` above (ADR-040 § 2) — only the SELECT clause
                // differs; ORDER BY/LIMIT/OFFSET/cursor logic is omitted
                // entirely (not meaningful for aggregation).
                let mut qb: QueryBuilder<sqlx::Postgres> =
                    QueryBuilder::new("SELECT COUNT(*) FROM documents WHERE project_id = ");
                qb.push_bind(collection.project_id.as_str());
                if query.query.all_descendants {
                    qb.push(" AND (collection_path = ");
                    qb.push_bind(&collection.collection_path);
                    qb.push(" OR collection_path LIKE ");
                    qb.push_bind(format!("%/{}", collection.collection_path));
                    qb.push(")");
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
                use crate::encoding::query::append_filter;
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
                    qb.push(" AND (collection_path = ");
                    qb.push_bind(&collection.collection_path);
                    qb.push(" OR collection_path LIKE ");
                    qb.push_bind(format!("%/{}", collection.collection_path));
                    qb.push(")");
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
                use crate::encoding::query::append_filter;
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
                    qb.push(" AND (collection_path = ");
                    qb.push_bind(&collection.collection_path);
                    qb.push(" OR collection_path LIKE ");
                    qb.push_bind(format!("%/{}", collection.collection_path));
                    qb.push(")");
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
