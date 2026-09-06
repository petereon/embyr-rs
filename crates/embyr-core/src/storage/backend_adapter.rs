use std::collections::BTreeMap;

use async_trait::async_trait;

use crate::{
    domain::{
        document::{CollectionPath, DocumentPath, FirestoreDocument, WriteResult},
        field_value::FieldValue,
        project::ProjectId,
        query::{AggregateValue, AggregationQuery, StructuredQuery},
        transaction::{TransactionId, TransactionOptions},
    },
    error::CoreError,
};

/// Precondition for write operations, matching Firestore protocol semantics.
#[derive(Debug, Clone)]
pub enum WritePrecondition {
    /// Document must exist (applies to updates/deletes).
    MustExist,
    /// Document must NOT exist (prevents overwrite).
    MustNotExist,
    /// Document's update_time must equal this value for OCC (seconds, nanos).
    UpdateTime(i64, i32),
}

/// Field-level server-side transform to apply at commit time.
///
/// 6 variants (ADR-052 § Decision 1) — Slice 01/02 (firestore-field-transforms)
/// implement real `apply_field_transform` logic for `ServerTimestamp`/
/// `Increment`/`Maximum`/`Minimum`; the remaining 2 (`AppendMissingElements`/
/// `RemoveAllFromArray`) exist so the enum/translation shape is complete,
/// but fail closed at apply time until Slice 03 extends them.
#[derive(Debug, Clone)]
pub enum FieldTransform {
    /// Write the server's current timestamp to the named field path.
    ServerTimestamp(String),
    /// Add `FieldValue` (Integer or Double) to the field's current value.
    Increment(String, FieldValue),
    /// Set the field to the greater of its current value and the given value.
    Maximum(String, FieldValue),
    /// Set the field to the lesser of its current value and the given value.
    Minimum(String, FieldValue),
    /// Append the given elements, skipping any already present.
    AppendMissingElements(String, Vec<FieldValue>),
    /// Remove every element matching any of the given values.
    RemoveAllFromArray(String, Vec<FieldValue>),
}

impl FieldTransform {
    /// The field path this transform targets — one accessor, one match arm
    /// per variant (ADR-052 § Decision 5c, "field_path() is a small
    /// accessor... trivial, not worth a separate ADR decision").
    pub fn field_path(&self) -> &str {
        match self {
            FieldTransform::ServerTimestamp(p)
            | FieldTransform::Increment(p, _)
            | FieldTransform::Maximum(p, _)
            | FieldTransform::Minimum(p, _)
            | FieldTransform::AppendMissingElements(p, _)
            | FieldTransform::RemoveAllFromArray(p, _) => p,
        }
    }

    /// Human-readable kind name for error messages (unimplemented-kind
    /// rejection in `apply_field_transform`, Slice 01).
    pub fn kind_name(&self) -> &'static str {
        match self {
            FieldTransform::ServerTimestamp(_) => "serverTimestamp",
            FieldTransform::Increment(..) => "increment",
            FieldTransform::Maximum(..) => "maximum",
            FieldTransform::Minimum(..) => "minimum",
            FieldTransform::AppendMissingElements(..) => "appendMissingElements",
            FieldTransform::RemoveAllFromArray(..) => "removeAllFromArray",
        }
    }
}

/// A single mutation in a transaction commit batch.
#[derive(Debug, Clone)]
pub enum Write {
    Update {
        path: DocumentPath,
        fields: BTreeMap<String, FieldValue>,
        /// If `Some(v)`, only succeeds when document version == v (OCC).
        version: Option<i64>,
        /// Optional precondition for the write (e.g. UpdateTime for OCC).
        precondition: Option<WritePrecondition>,
        /// Transforms to apply after the regular field update, read against
        /// the PRE-existing persisted value (ADR-052 § Decision 5c) — empty
        /// for every pre-existing/transform-free write.
        transforms: Vec<FieldTransform>,
    },
    Delete {
        path: DocumentPath,
        /// If `Some(v)`, only succeeds when document version == v (OCC).
        version: Option<i64>,
        /// Optional precondition for the write.
        precondition: Option<WritePrecondition>,
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
    /// `transaction_id`, when `Some`, registers this read in that
    /// transaction's own read set (firestore-transaction-read-consistency):
    /// `Commit` later re-validates the read document is unchanged, aborting
    /// otherwise. `None` (the vast majority of calls) is a plain,
    /// unregistered read — unchanged behavior.
    async fn get_document(
        &self,
        path: &DocumentPath,
        transaction_id: Option<&TransactionId>,
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
        precondition: Option<WritePrecondition>,
    ) -> Result<WriteResult, CoreError>;

    async fn delete_document(
        &self,
        path: &DocumentPath,
        precondition: Option<WritePrecondition>,
    ) -> Result<(), CoreError>;

    async fn run_query(
        &self,
        collection: &CollectionPath,
        query: &StructuredQuery,
        transaction_id: Option<&TransactionId>,
    ) -> Result<Vec<FirestoreDocument>, CoreError>;

    /// Runs an aggregation query (COUNT/SUM/AVG) against this backend.
    ///
    /// Default-provided body (ADR-041): implementors that have not yet added
    /// real aggregation support (`AgentBackendAdapter` until Slice 02)
    /// compile unmodified and simply reject at runtime — never a new
    /// `CoreError` variant, reusing the existing `FailedPrecondition`
    /// (`crates/embyr-agent`'s own exhaustive `core_error_to_status` match
    /// has no wildcard arm; adding a variant would force an edit there).
    async fn run_aggregation_query(
        &self,
        _collection: &CollectionPath,
        _query: &AggregationQuery,
        _transaction_id: Option<&TransactionId>,
    ) -> Result<AggregateValue, CoreError> {
        Err(CoreError::FailedPrecondition(
            "aggregation queries are not supported by this backend".into(),
        ))
    }

    /// Returns the distinct names of collections immediately under `parent`
    /// (one path segment deeper than `parent.collection_path`; empty
    /// `collection_path` = database root). Reuses `CollectionPath`'s
    /// existing (project_id, path-string) shape for a PARENT PREFIX, a
    /// distinct semantic from its other call sites (an exact collection
    /// name, or — with `all_descendants` — a collection-group name).
    ///
    /// Default-provided body (ADR-041/ADR-051 precedent): implementors
    /// without real support (`AgentBackendAdapter`, deferred) compile
    /// unmodified and reject at runtime via the existing `FailedPrecondition`
    /// variant — never a new `CoreError` variant.
    async fn list_collection_ids(
        &self,
        _parent: &CollectionPath,
        _limit: i32,
        _offset: i32,
    ) -> Result<Vec<String>, CoreError> {
        Err(CoreError::FailedPrecondition(
            "distinct collection listing is not supported by this backend".into(),
        ))
    }

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
