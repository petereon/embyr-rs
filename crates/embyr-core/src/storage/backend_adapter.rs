use std::collections::BTreeMap;

use async_trait::async_trait;

use crate::{
    domain::{
        document::{CollectionPath, DocumentPath, FirestoreDocument, WriteResult},
        field_value::FieldValue,
        project::ProjectId,
        query::StructuredQuery,
        transaction::{TransactionId, TransactionOptions},
    },
    error::CoreError,
};

/// Field-level server-side transform to apply at commit time.
#[derive(Debug, Clone)]
pub enum FieldTransform {
    /// Write the server's current timestamp to the named field path.
    ServerTimestamp(String),
}

/// A single mutation in a transaction commit batch.
#[derive(Debug, Clone)]
pub enum Write {
    Update {
        path: DocumentPath,
        fields: BTreeMap<String, FieldValue>,
        /// If `Some(v)`, only succeeds when document version == v (OCC).
        version: Option<i64>,
    },
    Delete {
        path: DocumentPath,
        /// If `Some(v)`, only succeeds when document version == v (OCC).
        version: Option<i64>,
    },
    Transform {
        path: DocumentPath,
        transforms: Vec<FieldTransform>,
    },
}

/// Driven port: the contract every backend must implement.
///
/// Trait is object-safe: use `Box<dyn BackendAdapter>` in adapter registries.
#[async_trait]
pub trait BackendAdapter: Send + Sync {
    async fn get_document(
        &self,
        path: &DocumentPath,
    ) -> Result<Option<FirestoreDocument>, CoreError>;

    async fn create_document(
        &self,
        path: &DocumentPath,
        fields: BTreeMap<String, FieldValue>,
    ) -> Result<WriteResult, CoreError>;

    async fn update_document(
        &self,
        path: &DocumentPath,
        fields: BTreeMap<String, FieldValue>,
        version: Option<i64>,
    ) -> Result<WriteResult, CoreError>;

    async fn delete_document(
        &self,
        path: &DocumentPath,
        version: Option<i64>,
    ) -> Result<(), CoreError>;

    async fn run_query(
        &self,
        collection: &CollectionPath,
        query: &StructuredQuery,
        transaction_id: Option<&TransactionId>,
    ) -> Result<Vec<FirestoreDocument>, CoreError>;

    async fn begin_transaction(
        &self,
        project_id: &ProjectId,
        options: TransactionOptions,
    ) -> Result<TransactionId, CoreError>;

    async fn commit_transaction(
        &self,
        project_id: &ProjectId,
        transaction_id: &TransactionId,
        writes: Vec<Write>,
    ) -> Result<Vec<WriteResult>, CoreError>;

    async fn rollback_transaction(
        &self,
        project_id: &ProjectId,
        transaction_id: &TransactionId,
    ) -> Result<(), CoreError>;

    /// Liveness check — returns `Ok(())` when the backend is reachable.
    async fn probe(&self) -> Result<(), CoreError>;
}

/// Type alias for a heap-allocated backend adapter.
pub type DynBackendAdapter = Box<dyn BackendAdapter>;
