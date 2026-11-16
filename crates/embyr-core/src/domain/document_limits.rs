//! Firestore document limits: total size, field-map nesting depth and field
//! path length. Pure, no IO — the single check every document-accepting write
//! entry point (CreateDocument, UpdateDocument, Commit/BatchWrite/Write
//! stream) runs before the document reaches a rule evaluator or a backend.

use std::collections::BTreeMap;

use crate::domain::document::DocumentPath;
use crate::domain::field_value::FieldValue;
use crate::error::CoreError;

pub const MAX_DOCUMENT_BYTES: usize = 1_048_576;
pub const MAX_NESTING_DEPTH: usize = 20;
pub const MAX_FIELD_PATH_BYTES: usize = 1500;

/// Per-document overhead and document-name constants from Firestore's
/// documented storage-size calculation.
const DOCUMENT_OVERHEAD: usize = 32;
const NAME_OVERHEAD: usize = 16;

/// Rejects a document that real Firestore would reject with INVALID_ARGUMENT:
/// larger than 1 MiB, nested deeper than 20 levels, or holding a field path
/// longer than 1500 bytes.
///
/// ponytail: size follows Firestore's documented rules approximately (name =
/// segment lengths + 16, field name = len+1, string len+1, bytes len, number/
/// timestamp 8, bool/null 1, reference len+16, +32 per document); exact byte
/// parity is not claimed. No geopoint arm: `FieldValue` has none.
pub fn validate_document_limits(
    path: &DocumentPath,
    fields: &BTreeMap<String, FieldValue>,
) -> Result<(), CoreError> {
    let name_size =
        path.collection_path.len() + 1 + path.document_id.len() + 1 + NAME_OVERHEAD;
    let size = name_size + DOCUMENT_OVERHEAD + map_size(fields, 1, None)?;
    if size > MAX_DOCUMENT_BYTES {
        return Err(CoreError::InvalidArgument(format!(
            "document size {size} bytes exceeds the {MAX_DOCUMENT_BYTES} byte limit"
        )));
    }
    Ok(())
}

/// `depth`: nesting level of the fields in `fields` (top-level fields = 1).
/// `parent_path_len`: byte length of the dotted path down to this map.
fn map_size(
    fields: &BTreeMap<String, FieldValue>,
    depth: usize,
    parent_path_len: Option<usize>,
) -> Result<usize, CoreError> {
    fields
        .iter()
        .map(|(name, value)| {
            let path_len = parent_path_len.map_or(name.len(), |p| p + 1 + name.len());
            if path_len > MAX_FIELD_PATH_BYTES {
                return Err(CoreError::InvalidArgument(format!(
                    "field path exceeds the {MAX_FIELD_PATH_BYTES} byte limit"
                )));
            }
            Ok(name.len() + 1 + value_size(value, depth, path_len)?)
        })
        .sum()
}

fn value_size(value: &FieldValue, depth: usize, path_len: usize) -> Result<usize, CoreError> {
    if depth > MAX_NESTING_DEPTH {
        return Err(CoreError::InvalidArgument(format!(
            "document nesting exceeds the maximum depth of {MAX_NESTING_DEPTH}"
        )));
    }
    match value {
        FieldValue::Null | FieldValue::Boolean(_) => Ok(1),
        FieldValue::Integer(_) | FieldValue::Double(_) | FieldValue::Timestamp(..) => Ok(8),
        FieldValue::String(s) => Ok(s.len() + 1),
        FieldValue::Bytes(b) => Ok(b.len()),
        FieldValue::Reference(r) => Ok(r.len() + NAME_OVERHEAD),
        FieldValue::Array(items) => items
            .iter()
            .map(|item| value_size(item, depth + 1, path_len))
            .sum(),
        FieldValue::Map(inner) => map_size(inner, depth + 1, Some(path_len)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::project::ProjectId;
    use proptest::prelude::*;

    fn path() -> DocumentPath {
        DocumentPath {
            project_id: ProjectId::new("p").unwrap(),
            collection_path: "c".into(),
            document_id: "d".into(),
        }
    }

    fn one(name: &str, value: FieldValue) -> BTreeMap<String, FieldValue> {
        BTreeMap::from([(name.to_string(), value)])
    }

    /// `levels` maps nested inside one another; the innermost holds a scalar.
    /// The scalar sits at depth `levels + 1`.
    fn nested(levels: usize) -> BTreeMap<String, FieldValue> {
        let mut value = FieldValue::Integer(1);
        for _ in 0..levels {
            value = FieldValue::Map(one("a", value));
        }
        one("a", value)
    }

    #[test]
    fn ordinary_document_is_accepted() {
        let fields = one("title", FieldValue::String("hello".into()));
        assert!(validate_document_limits(&path(), &fields).is_ok());
    }

    #[test]
    fn size_limit_is_enforced_at_the_exact_boundary() {
        // Fixed overhead for this document: name (1+1+1+1+16) + 32 + field
        // name "s" (1+1) + string terminator (1) = 55 bytes around the payload.
        let at_limit = one("s", FieldValue::String("x".repeat(MAX_DOCUMENT_BYTES - 55)));
        assert!(validate_document_limits(&path(), &at_limit).is_ok());
        let over = one("s", FieldValue::String("x".repeat(MAX_DOCUMENT_BYTES - 54)));
        let err = validate_document_limits(&path(), &over).unwrap_err();
        assert!(matches!(err, CoreError::InvalidArgument(m) if m.contains("size")));
    }

    #[test]
    fn size_is_summed_across_arrays_maps_bytes_and_references() {
        // 4 x 300 KiB values nested in array/map/bytes/reference: no single
        // value is over the limit, the document total is.
        let chunk = 300 * 1024;
        let fields = BTreeMap::from([
            ("arr".to_string(), FieldValue::Array(vec![FieldValue::String("x".repeat(chunk))])),
            ("map".to_string(), FieldValue::Map(one("k", FieldValue::String("x".repeat(chunk))))),
            ("bin".to_string(), FieldValue::Bytes(vec![0; chunk])),
            ("ref".to_string(), FieldValue::Reference("x".repeat(chunk))),
        ]);
        assert!(matches!(
            validate_document_limits(&path(), &fields),
            Err(CoreError::InvalidArgument(_))
        ));
    }

    proptest! {
        /// Spec oracle: 20 levels of nesting are allowed, 21 are not.
        #[test]
        fn nesting_up_to_twenty_levels_is_accepted_and_deeper_rejected(levels in 0usize..40) {
            // scalar depth = levels + 1
            let result = validate_document_limits(&path(), &nested(levels));
            prop_assert_eq!(result.is_ok(), levels + 1 <= 20);
        }

        /// Spec oracle: a field path of 1500 bytes is allowed, 1501 is not —
        /// whether the long name is top-level or the dotted path of nested names.
        #[test]
        fn field_path_over_1500_bytes_is_rejected(len in 1usize..3000) {
            let top = one(&"n".repeat(len), FieldValue::Null);
            prop_assert_eq!(validate_document_limits(&path(), &top).is_ok(), len <= 1500);
            // nested: "a." + name  => 2 + len bytes
            let nested_path = one("a", FieldValue::Map(one(&"n".repeat(len), FieldValue::Null)));
            prop_assert_eq!(validate_document_limits(&path(), &nested_path).is_ok(), len + 2 <= 1500);
        }
    }
}
