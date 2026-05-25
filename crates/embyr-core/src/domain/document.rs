use std::collections::BTreeMap;

use super::{field_value::FieldValue, project::ProjectId};

/// Full path to a specific document within a project.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DocumentPath {
    pub project_id: ProjectId,
    pub collection_path: String,
    pub document_id: String,
}

/// Path to a collection (or collection group) within a project.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CollectionPath {
    pub project_id: ProjectId,
    pub collection_path: String,
}

/// A Firestore document with its metadata.
#[derive(Debug, Clone)]
pub struct FirestoreDocument {
    pub path: DocumentPath,
    pub fields: BTreeMap<String, FieldValue>,
    /// (seconds, nanos) UTC wall-clock time when document was created.
    pub create_time: (i64, i32),
    /// (seconds, nanos) UTC wall-clock time of last write.
    pub update_time: (i64, i32),
    /// Monotonically increasing version counter for OCC.
    pub version: i64,
}

/// Result returned after a successful write operation.
#[derive(Debug, Clone)]
pub struct WriteResult {
    /// (seconds, nanos) commit timestamp of the write.
    pub update_time: (i64, i32),
    /// (seconds, nanos) creation timestamp — Some for create operations, None for updates.
    pub create_time: Option<(i64, i32)>,
}
