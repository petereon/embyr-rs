//! firestore-field-transforms (Slice 01/02, ADR-052 § Decision 5a): the
//! pure, IO-free compute function every field-transform application
//! (standalone `Write::Transform` and combined `Write::Update.transforms`)
//! runs through.
//!
//! Slice 02 adds real `Increment`/`Maximum`/`Minimum` arithmetic
//! (type-preserving per `docs/SPEC.md`'s promotion rule, `checked_add` ->
//! `InvalidArgument` on `i64` overflow per ADR-053 Escalation 1). Slice 03
//! still owns `AppendMissingElements`/`RemoveAllFromArray`, which continue
//! to fail closed with `CoreError::InvalidArgument` rather than mutating
//! `fields` — unreachable from translation until that slice extends
//! `translate_field_transforms`.

use std::collections::BTreeMap;

use crate::{domain::field_value::FieldValue, error::CoreError, storage::backend_adapter::FieldTransform};

/// Applies one `FieldTransform` to `fields` in place. `now` is the commit
/// timestamp already computed once per `commit_transaction` call.
///
/// Returns `Some(value)` for value-producing kinds (the entry to push onto
/// `WriteResult.transform_results`, in call order) — `ServerTimestamp`/
/// `Increment`/`Maximum`/`Minimum`. Returns `Err` without mutating `fields`
/// for a non-numeric operand/target, `increment` overflow, or a transform
/// kind Slice 03 does not yet implement.
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
        FieldTransform::Increment(field_path, delta) => {
            let value = compute_increment(fields.get(field_path), delta)?;
            fields.insert(field_path.clone(), value.clone());
            Ok(Some(value))
        }
        FieldTransform::Maximum(field_path, given) => {
            let value = compute_extremum(fields.get(field_path), given, Extremum::Maximum)?;
            fields.insert(field_path.clone(), value.clone());
            Ok(Some(value))
        }
        FieldTransform::Minimum(field_path, given) => {
            let value = compute_extremum(fields.get(field_path), given, Extremum::Minimum)?;
            fields.insert(field_path.clone(), value.clone());
            Ok(Some(value))
        }
        FieldTransform::AppendMissingElements(..) | FieldTransform::RemoveAllFromArray(..) => {
            Err(CoreError::InvalidArgument(format!(
                "{} transform is not yet implemented",
                transform.kind_name()
            )))
        }
    }
}

fn is_numeric(value: &FieldValue) -> bool {
    matches!(value, FieldValue::Integer(_) | FieldValue::Double(_))
}

fn as_f64(value: &FieldValue) -> f64 {
    match value {
        FieldValue::Integer(i) => *i as f64,
        FieldValue::Double(d) => *d,
        _ => unreachable!("caller guarantees a numeric FieldValue"),
    }
}

/// `increment` (ADR-052 § Decision 5a, ADR-053 Escalation 1): missing field
/// treated as `Integer(0)`/`Double(0.0)` matching the delta's own type;
/// existing non-numeric field -> `InvalidArgument`; `i64` overflow via
/// `checked_add` -> `InvalidArgument` (never wraps or saturates).
fn compute_increment(current: Option<&FieldValue>, delta: &FieldValue) -> Result<FieldValue, CoreError> {
    if !is_numeric(delta) {
        return Err(CoreError::InvalidArgument("increment delta must be numeric".into()));
    }
    let base = match current {
        None => zero_like(delta),
        Some(v) if is_numeric(v) => v.clone(),
        Some(_) => return Err(CoreError::InvalidArgument("increment target is not numeric".into())),
    };
    add_numeric(&base, delta)
}

fn zero_like(delta: &FieldValue) -> FieldValue {
    match delta {
        FieldValue::Double(_) => FieldValue::Double(0.0),
        _ => FieldValue::Integer(0),
    }
}

fn add_numeric(current: &FieldValue, delta: &FieldValue) -> Result<FieldValue, CoreError> {
    match (current, delta) {
        (FieldValue::Integer(c), FieldValue::Integer(d)) => c
            .checked_add(*d)
            .map(FieldValue::Integer)
            .ok_or_else(|| CoreError::InvalidArgument("increment overflow: result exceeds i64 range".into())),
        _ => Ok(FieldValue::Double(as_f64(current) + as_f64(delta))),
    }
}

#[derive(Clone, Copy)]
enum Extremum {
    Maximum,
    Minimum,
}

impl Extremum {
    fn kind_name(self) -> &'static str {
        match self {
            Extremum::Maximum => "maximum",
            Extremum::Minimum => "minimum",
        }
    }
}

/// `maximum`/`minimum` (ADR-052 § Decision 5a, ADR-053 Escalation 2's own
/// `docs/SPEC.md` addition): missing field set DIRECTLY to `given` — the
/// OPPOSITE rule from `increment`'s 0/0.0 baseline; existing non-numeric
/// field -> `InvalidArgument`; otherwise a type-preserving comparison,
/// keeping the winning side's own original `FieldValue` variant. Ties keep
/// the EXISTING value's own type (no needless promotion when the value does
/// not actually change) — an implementation choice, not SPEC-mandated.
fn compute_extremum(
    current: Option<&FieldValue>,
    given: &FieldValue,
    which: Extremum,
) -> Result<FieldValue, CoreError> {
    if !is_numeric(given) {
        return Err(CoreError::InvalidArgument(format!(
            "{} comparand must be numeric",
            which.kind_name()
        )));
    }
    match current {
        None => Ok(given.clone()),
        Some(v) if is_numeric(v) => Ok(pick_extremum(v, given, which)),
        Some(_) => Err(CoreError::InvalidArgument(format!(
            "{} target is not numeric",
            which.kind_name()
        ))),
    }
}

fn pick_extremum(current: &FieldValue, given: &FieldValue, which: Extremum) -> FieldValue {
    let current_wins = match which {
        Extremum::Maximum => as_f64(current) >= as_f64(given),
        Extremum::Minimum => as_f64(current) <= as_f64(given),
    };
    if current_wins {
        current.clone()
    } else {
        given.clone()
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

    /// Slice-02 scope guard: an array-kind transform Slice 03 will implement
    /// later fails closed today — `InvalidArgument`, `fields` untouched —
    /// rather than silently no-op-ing or panicking. (`Increment` moved to
    /// Slice 02's own real-implementation tests below — it is no longer an
    /// unimplemented kind.)
    #[test]
    fn unimplemented_array_transform_kinds_fail_closed_without_mutating_fields() {
        let now = (1_700_000_000_i64, 0);
        let mut fields = BTreeMap::new();
        let transform =
            FieldTransform::AppendMissingElements("sharedWithUserIds".to_string(), vec![FieldValue::String("u-diego".to_string())]);

        let err = apply_field_transform(&mut fields, &transform, now)
            .expect_err("appendMissingElements is not implemented until Slice 03");

        assert!(matches!(err, CoreError::InvalidArgument(_)));
        assert!(fields.is_empty());
    }

    // Slice 02 (US-02, ADR-052 § Decision 5a, ADR-053 Escalation 1) —
    // `increment`/`maximum`/`minimum`. Test Budget: 7 behaviors x 2 = 14
    // unit tests max; 7 written (one per behavior; `maximum`/`minimum`
    // variations of the SAME behavior are parametrized within one test
    // each, per Mandate 5).
    use proptest::prelude::*;

    proptest! {
        /// Behavior 1: `increment` on an existing numeric field is
        /// type-preserving — int+int stays int; any double operand (delta
        /// or existing) promotes the result to double. Reference model is a
        /// plain f64 sum plus an independent "double if either side is
        /// double" rule — not the implementation's own match arms.
        #[test]
        fn increment_on_existing_numeric_field_is_type_preserving(
            existing_int in -1_000_000_000_i64..1_000_000_000_i64,
            delta_int in -1_000_000_000_i64..1_000_000_000_i64,
            existing_double in -1e9_f64..1e9_f64,
            delta_double in -1e9_f64..1e9_f64,
            use_double_existing in any::<bool>(),
            use_double_delta in any::<bool>(),
        ) {
            let (existing, existing_f) = if use_double_existing {
                (FieldValue::Double(existing_double), existing_double)
            } else {
                (FieldValue::Integer(existing_int), existing_int as f64)
            };
            let (delta, delta_f) = if use_double_delta {
                (FieldValue::Double(delta_double), delta_double)
            } else {
                (FieldValue::Integer(delta_int), delta_int as f64)
            };

            let mut fields = BTreeMap::from([("viewCount".to_string(), existing)]);
            let transform = FieldTransform::Increment("viewCount".to_string(), delta);
            let result = apply_field_transform(&mut fields, &transform, (0, 0))
                .expect("increment on an in-range numeric field never fails");

            if use_double_existing || use_double_delta {
                match &result {
                    Some(FieldValue::Double(d)) => prop_assert!((d - (existing_f + delta_f)).abs() < 1e-6),
                    other => prop_assert!(false, "expected Double, got {other:?}"),
                }
            } else {
                prop_assert_eq!(result.clone(), Some(FieldValue::Integer(existing_int + delta_int)));
            }
            prop_assert_eq!(fields.get("viewCount"), result.as_ref());
        }

        /// Behavior 2: `increment` against a MISSING field treats it as
        /// `Integer(0)`/`Double(0.0)` per the delta's own type — the result
        /// is exactly the delta (0 + delta == delta), a simplification of
        /// the "missing == zero" rule used as the reference oracle.
        #[test]
        fn increment_against_missing_field_treated_as_zero_of_delta_type(
            delta_int in -1_000_000_000_i64..1_000_000_000_i64,
            delta_double in -1e9_f64..1e9_f64,
            use_double in any::<bool>(),
        ) {
            let delta = if use_double { FieldValue::Double(delta_double) } else { FieldValue::Integer(delta_int) };
            let mut fields: BTreeMap<String, FieldValue> = BTreeMap::new();
            let transform = FieldTransform::Increment("viewCount".to_string(), delta.clone());
            let result = apply_field_transform(&mut fields, &transform, (0, 0))
                .expect("increment against a missing field never fails");

            prop_assert_eq!(result.clone(), Some(delta));
            prop_assert_eq!(fields.get("viewCount"), result.as_ref());
        }

        /// Behavior 3 (ADR-053 Escalation 1): `i64` overflow on `increment`
        /// returns `InvalidArgument` via `checked_add` — never wraps or
        /// saturates — and leaves `fields` unmutated. `delta` is constructed
        /// to guarantee genuine overflow (no `prop_assume` rejection).
        #[test]
        fn increment_overflow_returns_invalid_argument_without_mutating_fields(
            headroom in 0_i64..10_i64,
            extra in 0_i64..10_i64,
        ) {
            let existing = i64::MAX - headroom;
            let delta = headroom + extra + 1;
            let mut fields = BTreeMap::from([("viewCount".to_string(), FieldValue::Integer(existing))]);
            let transform = FieldTransform::Increment("viewCount".to_string(), FieldValue::Integer(delta));

            let err = apply_field_transform(&mut fields, &transform, (0, 0))
                .expect_err("i64 overflow must be rejected, never wrap or saturate");

            prop_assert!(matches!(err, CoreError::InvalidArgument(_)));
            prop_assert_eq!(fields.get("viewCount"), Some(&FieldValue::Integer(existing)));
        }

        /// Behavior 4: `increment` against an existing NON-numeric field is
        /// `InvalidArgument`, and the field is left completely unchanged.
        #[test]
        fn increment_against_non_numeric_existing_value_is_invalid_argument_and_unchanged(
            delta in -1000_i64..1000_i64,
            s in "[a-z]{0,10}",
        ) {
            let existing = FieldValue::String(s);
            let mut fields = BTreeMap::from([("viewCount".to_string(), existing.clone())]);
            let transform = FieldTransform::Increment("viewCount".to_string(), FieldValue::Integer(delta));

            let err = apply_field_transform(&mut fields, &transform, (0, 0))
                .expect_err("increment against a non-numeric target must be rejected");

            prop_assert!(matches!(err, CoreError::InvalidArgument(_)));
            prop_assert_eq!(fields.get("viewCount"), Some(&existing));
        }

        /// Behavior 5: `maximum`/`minimum` against an EXISTING numeric field
        /// compare correctly and preserve the winning side's own type — the
        /// OPPOSITE-operator variations of this one behavior are exercised
        /// together (Mandate 5), not as two separate tests.
        #[test]
        fn maximum_minimum_compare_against_existing_and_preserve_winning_type(
            existing_int in -1_000_000_i64..1_000_000_i64,
            given_int in -1_000_000_i64..1_000_000_i64,
            use_double_existing in any::<bool>(),
            use_double_given in any::<bool>(),
        ) {
            let existing = if use_double_existing {
                FieldValue::Double(existing_int as f64)
            } else {
                FieldValue::Integer(existing_int)
            };
            let given = if use_double_given {
                FieldValue::Double(given_int as f64)
            } else {
                FieldValue::Integer(given_int)
            };
            let existing_f = existing_int as f64;
            let given_f = given_int as f64;

            let mut max_fields = BTreeMap::from([("promoBoostCount".to_string(), existing.clone())]);
            let max_transform = FieldTransform::Maximum("promoBoostCount".to_string(), given.clone());
            let max_result = apply_field_transform(&mut max_fields, &max_transform, (0, 0))
                .expect("maximum on numeric fields never fails");
            let expected_max = if existing_f >= given_f { existing.clone() } else { given.clone() };
            prop_assert_eq!(max_result, Some(expected_max));

            let mut min_fields = BTreeMap::from([("promoBoostCount".to_string(), existing.clone())]);
            let min_transform = FieldTransform::Minimum("promoBoostCount".to_string(), given.clone());
            let min_result = apply_field_transform(&mut min_fields, &min_transform, (0, 0))
                .expect("minimum on numeric fields never fails");
            let expected_min = if existing_f <= given_f { existing } else { given };
            prop_assert_eq!(min_result, Some(expected_min));
        }

        /// Behavior 6: `maximum`/`minimum` against a MISSING field set it
        /// DIRECTLY to the given value — the OPPOSITE rule from
        /// `increment`'s own 0/0.0 baseline (Behavior 2 above).
        #[test]
        fn maximum_minimum_against_missing_field_set_directly(
            given_int in -1_000_000_i64..1_000_000_i64,
            given_double in -1e6_f64..1e6_f64,
            use_double in any::<bool>(),
        ) {
            let given = if use_double { FieldValue::Double(given_double) } else { FieldValue::Integer(given_int) };

            let mut max_fields: BTreeMap<String, FieldValue> = BTreeMap::new();
            let max_transform = FieldTransform::Maximum("promoBoostCount".to_string(), given.clone());
            let max_result = apply_field_transform(&mut max_fields, &max_transform, (0, 0))
                .expect("maximum against a missing field never fails");
            prop_assert_eq!(max_result.clone(), Some(given.clone()));
            prop_assert_eq!(max_fields.get("promoBoostCount"), max_result.as_ref());

            let mut min_fields: BTreeMap<String, FieldValue> = BTreeMap::new();
            let min_transform = FieldTransform::Minimum("promoBoostCount".to_string(), given.clone());
            let min_result = apply_field_transform(&mut min_fields, &min_transform, (0, 0))
                .expect("minimum against a missing field never fails");
            prop_assert_eq!(min_result, Some(given));
        }

        /// Behavior 7: `maximum`/`minimum` against an existing NON-numeric
        /// field is `InvalidArgument`, field left unchanged.
        #[test]
        fn maximum_minimum_against_non_numeric_existing_value_is_invalid_argument_and_unchanged(
            given in -1000_i64..1000_i64,
            s in "[a-z]{0,10}",
        ) {
            let existing = FieldValue::String(s);

            let mut max_fields = BTreeMap::from([("promoBoostCount".to_string(), existing.clone())]);
            let max_transform = FieldTransform::Maximum("promoBoostCount".to_string(), FieldValue::Integer(given));
            let max_err = apply_field_transform(&mut max_fields, &max_transform, (0, 0))
                .expect_err("maximum against a non-numeric target must be rejected");
            prop_assert!(matches!(max_err, CoreError::InvalidArgument(_)));
            prop_assert_eq!(max_fields.get("promoBoostCount"), Some(&existing));

            let mut min_fields = BTreeMap::from([("promoBoostCount".to_string(), existing.clone())]);
            let min_transform = FieldTransform::Minimum("promoBoostCount".to_string(), FieldValue::Integer(given));
            let min_err = apply_field_transform(&mut min_fields, &min_transform, (0, 0))
                .expect_err("minimum against a non-numeric target must be rejected");
            prop_assert!(matches!(min_err, CoreError::InvalidArgument(_)));
            prop_assert_eq!(min_fields.get("promoBoostCount"), Some(&existing));
        }
    }
}
