//! Common test infrastructure — firestore-field-transforms acceptance tests
//! (Slice 01, US-01, ADR-052/053).
//!
//! Reuses `SecurityRulesFullContext`/`create_document`/`begin_transaction`/
//! `commit_writes`/`string_field` via a path import (mirroring
//! `firestore_batch_write`'s own identical precedent — this feature is a new
//! consumer of already-shipped fixtures, not a new context class). Adds only
//! the field-transform-specific `Write`/`FieldTransform` builders this
//! feature's own scenarios need.

#![allow(dead_code, unused_imports)]

#[path = "../../security_rules_write_path/common/mod.rs"]
mod security_rules_write_path_common;
pub use security_rules_write_path_common::{
    begin_transaction, commit_writes, create_document, string_field, SecurityRulesFullContext,
};

use embyr_proto::firestore::{
    document_transform::{field_transform::TransformType, FieldTransform, ServerValue},
    write::Operation,
    DocumentTransform, Write,
};

/// A `set_to_server_value: REQUEST_TIME` field transform targeting
/// `field_path` — the ONE real transform kind Slice 01 implements.
pub fn server_timestamp_transform(field_path: &str) -> FieldTransform {
    FieldTransform {
        field_path: field_path.to_string(),
        transform_type: Some(TransformType::SetToServerValue(ServerValue::RequestTime as i32)),
    }
}

/// A `set_to_server_value` field transform carrying an unsupported
/// `ServerValue` (AC-01-05) — `SERVER_VALUE_UNSPECIFIED`, the only other
/// value the enum defines.
pub fn unsupported_server_value_transform(field_path: &str) -> FieldTransform {
    FieldTransform {
        field_path: field_path.to_string(),
        transform_type: Some(TransformType::SetToServerValue(ServerValue::Unspecified as i32)),
    }
}

/// A standalone `Write { operation: transform(...) }` — the first of the two
/// wire shapes this slice proves (AC-01-01/03/04/05).
pub fn transform_write(document_name: &str, field_transforms: Vec<FieldTransform>) -> Write {
    Write {
        update_mask: None,
        update_transforms: vec![],
        current_document: None,
        operation: Some(Operation::Transform(DocumentTransform {
            document: document_name.to_string(),
            field_transforms,
        })),
    }
}

/// A `Write { operation: update(...), update_transforms: [...] }` — the
/// second wire shape this slice proves (AC-01-02): a regular field update
/// AND an attached transform on the SAME write.
pub fn update_write_with_transforms(
    resource_name: &str,
    fields: std::collections::HashMap<String, embyr_proto::firestore::Value>,
    update_transforms: Vec<FieldTransform>,
) -> Write {
    Write {
        update_mask: None,
        update_transforms,
        current_document: None,
        operation: Some(Operation::Update(embyr_proto::firestore::Document {
            name: resource_name.to_string(),
            fields,
            ..Default::default()
        })),
    }
}
