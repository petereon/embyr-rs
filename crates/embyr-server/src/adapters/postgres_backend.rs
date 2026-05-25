use std::collections::BTreeMap;

use async_trait::async_trait;
use sqlx::PgPool;

use embyr_core::{
    domain::{
        document::{CollectionPath, DocumentPath, FirestoreDocument, WriteResult},
        field_value::FieldValue,
        project::ProjectId,
        query::StructuredQuery,
        transaction::{TransactionId, TransactionOptions},
    },
    error::CoreError,
    storage::backend_adapter::{BackendAdapter, Write},
};

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
        _path: &DocumentPath,
        _fields: BTreeMap<String, FieldValue>,
    ) -> Result<WriteResult, CoreError> {
        todo!("step 03-01")
    }

    async fn update_document(
        &self,
        _path: &DocumentPath,
        _fields: BTreeMap<String, FieldValue>,
        _version: Option<i64>,
    ) -> Result<WriteResult, CoreError> {
        todo!("step 03-01")
    }

    async fn delete_document(
        &self,
        _path: &DocumentPath,
        _version: Option<i64>,
    ) -> Result<(), CoreError> {
        todo!("step 03-01")
    }

    async fn run_query(
        &self,
        _collection: &CollectionPath,
        _query: &StructuredQuery,
        _transaction_id: Option<&TransactionId>,
    ) -> Result<Vec<FirestoreDocument>, CoreError> {
        todo!("step 04-01")
    }

    async fn begin_transaction(
        &self,
        _project_id: &ProjectId,
        _options: TransactionOptions,
    ) -> Result<TransactionId, CoreError> {
        todo!("step 06-01")
    }

    async fn commit_transaction(
        &self,
        _project_id: &ProjectId,
        _transaction_id: &TransactionId,
        _writes: Vec<Write>,
    ) -> Result<Vec<WriteResult>, CoreError> {
        todo!("step 06-01")
    }

    async fn rollback_transaction(
        &self,
        _project_id: &ProjectId,
        _transaction_id: &TransactionId,
    ) -> Result<(), CoreError> {
        todo!("step 06-01")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use testcontainers_modules::postgres::Postgres;
    use testcontainers_modules::testcontainers::{runners::AsyncRunner, ContainerAsync, ImageExt};

    async fn start_postgres() -> (ContainerAsync<Postgres>, String) {
        let container = Postgres::default()
            .with_tag("15-alpine")
            .start()
            .await
            .expect("Failed to start Postgres container");
        let port = container
            .get_host_port_ipv4(5432)
            .await
            .expect("Failed to get port");
        let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
        (container, url)
    }

    #[tokio::test]
    async fn probe_returns_ok_after_migrate() {
        let (_c, url) = start_postgres().await;
        let db = PostgresBackendAdapter::new(&url).await.unwrap();
        db.migrate().await.unwrap();
        assert!(db.probe().await.is_ok());
    }

    #[tokio::test]
    async fn get_document_returns_none_for_missing() {
        let (_c, url) = start_postgres().await;
        let db = PostgresBackendAdapter::new(&url).await.unwrap();
        db.migrate().await.unwrap();

        let path = DocumentPath {
            project_id: embyr_core::domain::project::ProjectId::new("test-proj").unwrap(),
            collection_path: "messages".into(),
            document_id: "nonexistent".into(),
        };
        let result = db.get_document(&path).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn get_document_returns_some_for_seeded_document() {
        let (_c, url) = start_postgres().await;
        let db = PostgresBackendAdapter::new(&url).await.unwrap();
        db.migrate().await.unwrap();

        // Seed a document directly via SQL
        let pool = sqlx::PgPool::connect(&url).await.unwrap();
        let fields_json = serde_json::json!({
            "greeting": {"t": "S", "v": "hello"},
            "count": {"t": "I", "v": 42}
        });
        sqlx::query(
            "INSERT INTO documents (project_id, collection_path, document_id, fields) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind("test-proj")
        .bind("messages")
        .bind("doc1")
        .bind(&fields_json)
        .execute(&pool)
        .await
        .unwrap();

        let path = DocumentPath {
            project_id: embyr_core::domain::project::ProjectId::new("test-proj").unwrap(),
            collection_path: "messages".into(),
            document_id: "doc1".into(),
        };
        let doc = db.get_document(&path).await.unwrap().expect("document should exist");
        assert_eq!(doc.path.document_id, "doc1");
        assert_eq!(
            doc.fields.get("greeting").unwrap(),
            &embyr_core::domain::field_value::FieldValue::String("hello".into())
        );
        assert_eq!(
            doc.fields.get("count").unwrap(),
            &embyr_core::domain::field_value::FieldValue::Integer(42)
        );
    }
}
