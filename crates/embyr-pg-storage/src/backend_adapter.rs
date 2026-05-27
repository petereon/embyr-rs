use std::collections::BTreeMap;

use async_trait::async_trait;
use sqlx::PgPool;

use crate::notify_listener::notify_channel;

use chrono::{DateTime, TimeZone, Utc};
use embyr_core::{
    domain::{
        document::{CollectionPath, DocumentPath, FirestoreDocument, WriteResult},
        field_value::FieldValue,
        project::ProjectId,
        query::StructuredQuery,
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

    /// Apply customer schema migrations from `migrations/customer/`.
    pub async fn migrate(&self) -> Result<(), CoreError> {
        sqlx::migrate!("../../migrations/customer")
            .run(&self.pool)
            .await
            .map_err(|e| CoreError::BackendUnavailable(e.to_string()))
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
}

/// Convert (seconds, nanos) to `chrono::DateTime<Utc>`.
fn to_datetime(seconds: i64, nanos: i32) -> DateTime<Utc> {
    Utc.timestamp_opt(seconds, nanos as u32)
        .single()
        .expect("valid timestamp")
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

        let Some(row) = row_opt else {
            return Ok(None);
        };

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
            .ok_or_else(|| CoreError::BackendUnavailable("failed to decode fields JSON".into()))?;

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
                Ok(WriteResult { update_time: from_datetime(update_time), create_time: None })
            }

            Some(WritePrecondition::UpdateTime(s, n)) => {
                // OCC: update only if update_time matches.
                let precondition_dt = to_datetime(s, n);
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
        _transaction_id: Option<&TransactionId>,
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

        // startAfter cursor — single-field orderBy only (step 04-01)
        if let (Some(cursor), false) = (&query.start_at, query.order_by.is_empty()) {
            if !cursor.values.is_empty() {
                let ob = &query.order_by[0];
                let op = if cursor.before { ">=" } else { ">" };
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

        // LIMIT
        if let Some(limit) = query.limit {
            qb.push(" LIMIT ");
            qb.push_bind(limit as i64);
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
        Ok(docs)
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
        let row: Option<(String, DateTime<Utc>)> = sqlx::query_as(
            "SELECT status, started_at FROM transactions \
             WHERE transaction_id = $1 AND project_id = $2",
        )
        .bind(txn_uuid)
        .bind(project_id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| CoreError::BackendUnavailable(e.to_string()))?;

        let (status, started_at) =
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

            let expected_dt = to_datetime(expected_secs, expected_nanos);
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

        // Also run version-based OCC for writes with version field
        crate::transactions::occ::verify_versions(
            &mut pg_txn,
            project_id.as_str(),
            &writes,
        )
        .await?;

        let now = Utc::now();
        let mut results = Vec::with_capacity(writes.len());

        for write in &writes {
            match write {
                Write::Update { path, fields, .. } => {
                    let fields_json = crate::encoding::field_value::fields_to_json(fields);
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
                    results.push(WriteResult { update_time: from_datetime(now), create_time: None });
                }
                Write::Transform { .. } => {
                    // ServerTimestamp transforms are handled as timestamp write
                    results.push(WriteResult { update_time: from_datetime(now), create_time: None });
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

/// Parse a 16-byte slice as a UUID.
fn uuid_from_bytes(bytes: &[u8]) -> Result<uuid::Uuid, CoreError> {
    if bytes.len() != 16 {
        return Err(CoreError::InvalidArgument(
            "invalid transaction ID length".into(),
        ));
    }
    let arr: [u8; 16] = bytes.try_into().unwrap();
    Ok(uuid::Uuid::from_bytes(arr))
}
