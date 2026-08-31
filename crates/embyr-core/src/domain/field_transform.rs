//! firestore-field-transforms (Slice 01, ADR-052 § Decision 5a): the pure,
//! IO-free compute function every field-transform application (standalone
//! `Write::Transform` and combined `Write::Update.transforms`) runs through.
//!
//! Slice 01 implements ONLY `ServerTimestamp` for real — the other 5
//! `FieldTransform` variants exist (the enum has 6 variants per ADR-052 §
//! Decision 1) but are unreachable from translation this slice
//! (`translate_field_transforms` in `embyr-server` only ever constructs
//! `ServerTimestamp` today), so their arms fail closed with
//! `CoreError::InvalidArgument` rather than mutating `fields` — Slice 02/03
//! extend these arms with real arithmetic/structural-equality logic, not a
//! new function (ADR-052 § Component Decomposition).

use std::collections::BTreeMap;

use crate::{domain::field_value::FieldValue, error::CoreError, storage::backend_adapter::FieldTransform};

/// Applies one `FieldTransform` to `fields` in place. `now` is the commit
/// timestamp already computed once per `commit_transaction` call.
///
/// Returns `Some(value)` for value-producing kinds (the entry to push onto
/// `WriteResult.transform_results`, in call order) — this slice: only
/// `ServerTimestamp`. Returns `Err` for any transform kind Slice 01 does not
/// yet implement, without mutating `fields`.
pub fn apply_field_transform(
    fields: &mut BTreeMap<String, FieldValue>,
    transform: &FieldTransform,
    now: (i64, i32),
) -> Result<Option<FieldValue>, CoreError> {
    match transform {
        FieldTransform::ServerTimestamp(field_path) => {
            let value = FieldValue::Timestamp(now.0, now.1);
            fields.insert(field_path.clone(), value.clone());
            Ok(Some(value))
        }
        FieldTransform::Increment(..)
        | FieldTransform::Maximum(..)
        | FieldTransform::Minimum(..)
        | FieldTransform::AppendMissingElements(..)
        | FieldTransform::RemoveAllFromArray(..) => Err(CoreError::InvalidArgument(format!(
            "{} transform is not yet implemented",
            transform.kind_name()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AC-01-01/AC-01-03/AC-01-04: `ServerTimestamp` unconditionally sets the
    /// target field to `now` — creating it if absent, overwriting it if
    /// present — and reports the same value as its `transform_results` entry.
    #[test]
    fn server_timestamp_sets_field_to_now_creating_or_overwriting() {
        let now = (1_700_000_000_i64, 123_000_000_i32);
        let transform = FieldTransform::ServerTimestamp("updatedAt".to_string());

        let cases: Vec<BTreeMap<String, FieldValue>> = vec![
            BTreeMap::new(),
            BTreeMap::from([("updatedAt".to_string(), FieldValue::String("stale".to_string()))]),
        ];
        for mut fields in cases {
            let result = apply_field_transform(&mut fields, &transform, now)
                .expect("server timestamp transform never fails");
            assert_eq!(result, Some(FieldValue::Timestamp(now.0, now.1)));
            assert_eq!(
                fields.get("updatedAt"),
                Some(&FieldValue::Timestamp(now.0, now.1))
            );
        }
    }

    /// Slice-01 scope guard: a transform kind Slice 02/03 will implement
    /// later fails closed today — `InvalidArgument`, `fields` untouched —
    /// rather than silently no-op-ing or panicking.
    #[test]
    fn unimplemented_transform_kinds_fail_closed_without_mutating_fields() {
        let now = (1_700_000_000_i64, 0);
        let mut fields = BTreeMap::new();
        let transform = FieldTransform::Increment("viewCount".to_string(), FieldValue::Integer(1));

        let err = apply_field_transform(&mut fields, &transform, now)
            .expect_err("increment is not implemented in Slice 01");

        assert!(matches!(err, CoreError::InvalidArgument(_)));
        assert!(fields.is_empty());
    }
}
