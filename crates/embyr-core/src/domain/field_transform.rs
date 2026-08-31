//! firestore-field-transforms (Slice 01/02/03, ADR-052 § Decision 5a): the
//! pure, IO-free compute function every field-transform application
//! (standalone `Write::Transform` and combined `Write::Update.transforms`)
//! runs through.
//!
//! Slice 02 adds real `Increment`/`Maximum`/`Minimum` arithmetic
//! (type-preserving per `docs/SPEC.md`'s promotion rule, `checked_add` ->
//! `InvalidArgument` on `i64` overflow per ADR-053 Escalation 1). Slice 03
//! adds real `AppendMissingElements`/`RemoveAllFromArray` (structural
//! equality via `FieldValue::PartialEq`, zero new equality logic) — both
//! array-kind transforms always return `Ok(None)` on success, NEVER
//! contributing a `WriteResult.transform_results` entry (ADR-053 Escalation
//! 2 Resolution), distinct from every other kind in this module.

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
        FieldTransform::AppendMissingElements(field_path, values) => {
            append_missing_elements(fields, field_path, values)?;
            Ok(None)
        }
        FieldTransform::RemoveAllFromArray(field_path, values) => {
            remove_all_from_array(fields, field_path, values)?;
            Ok(None)
        }
    }
}

/// `appendMissingElements` (ADR-052 § Decision 5a): missing field ->
/// created as `values` given, in order; existing `Array` -> append each of
/// `values` not already present (`FieldValue::PartialEq`, already derived —
/// zero new equality logic), preserving `values`'s own order for the
/// appended tail and deduping duplicates within `values` itself; existing
/// non-array -> `InvalidArgument` (residual, by analogy to the numeric
/// non-numeric-target rule). Never contributes a `transform_results` entry.
fn append_missing_elements(
    fields: &mut BTreeMap<String, FieldValue>,
    field_path: &str,
    values: &[FieldValue],
) -> Result<(), CoreError> {
    let mut merged = match fields.get(field_path) {
        None => Vec::new(),
        Some(FieldValue::Array(existing)) => existing.clone(),
        Some(_) => {
            return Err(CoreError::InvalidArgument(
                "appendMissingElements target is not an array".into(),
            ))
        }
    };
    for value in values {
        if !merged.contains(value) {
            merged.push(value.clone());
        }
    }
    fields.insert(field_path.to_string(), FieldValue::Array(merged));
    Ok(())
}

/// `removeAllFromArray` (ADR-052 § Decision 5a): missing field -> true
/// no-op, field NOT created; existing `Array` -> retain only elements not
/// structurally equal to any of `values`, removing ALL matching occurrences;
/// existing non-array -> `InvalidArgument`. Never contributes a
/// `transform_results` entry.
fn remove_all_from_array(
    fields: &mut BTreeMap<String, FieldValue>,
    field_path: &str,
    values: &[FieldValue],
) -> Result<(), CoreError> {
    match fields.get(field_path) {
        None => Ok(()),
        Some(FieldValue::Array(existing)) => {
            let retained: Vec<FieldValue> =
                existing.iter().filter(|item| !values.contains(item)).cloned().collect();
            fields.insert(field_path.to_string(), FieldValue::Array(retained));
            Ok(())
        }
        Some(_) => Err(CoreError::InvalidArgument(
            "removeAllFromArray target is not an array".into(),
        )),
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

        // Slice 03 (US-03, ADR-052 § Decision 5a, ADR-053 Escalation 2) —
        // `appendMissingElements`/`removeAllFromArray`. Test Budget: 6
        // behaviors x 2 = 12 unit tests max; 6 written (one per behavior).
        // Every behavior below also asserts `result == None` inline — array
        // transforms never populate `transform_results` — rather than a
        // separate 7th test duplicating that single fact.

        /// Behavior 1 (AC-03-01/AC-03-02): `appendMissingElements` against an
        /// existing array appends only the genuinely-new incoming values, in
        /// order, deduping both against the existing array AND within the
        /// incoming values themselves. Reference oracle: an independently
        /// implemented `HashSet`-tracked fold, not the production `Vec::contains`
        /// scan.
        #[test]
        fn append_missing_elements_appends_only_genuinely_new_values_in_order(
            existing in prop::collection::vec("[a-c]", 0..5),
            incoming in prop::collection::vec("[a-c]", 0..5),
        ) {
            let existing_values: Vec<FieldValue> = existing.iter().cloned().map(FieldValue::String).collect();
            let incoming_values: Vec<FieldValue> = incoming.iter().cloned().map(FieldValue::String).collect();

            let mut fields = BTreeMap::from([(
                "sharedWithUserIds".to_string(),
                FieldValue::Array(existing_values.clone()),
            )]);
            let transform =
                FieldTransform::AppendMissingElements("sharedWithUserIds".to_string(), incoming_values.clone());
            let result = apply_field_transform(&mut fields, &transform, (0, 0))
                .expect("appendMissingElements on an array field never fails");
            prop_assert_eq!(result, None, "array transforms never populate transform_results");

            let mut seen: std::collections::HashSet<String> = existing.iter().cloned().collect();
            let mut expected = existing_values;
            for (raw, value) in incoming.iter().zip(incoming_values.iter()) {
                if seen.insert(raw.clone()) {
                    expected.push(value.clone());
                }
            }
            prop_assert_eq!(fields.get("sharedWithUserIds"), Some(&FieldValue::Array(expected)));
        }

        /// Behavior 2 (AC-03-04): `appendMissingElements` against a MISSING
        /// field creates it as the incoming values (still deduped/ordered per
        /// Behavior 1's own rule, applied from an empty base).
        #[test]
        fn append_missing_elements_against_missing_field_creates_it_as_given(
            incoming in prop::collection::vec("[a-c]", 1..5),
        ) {
            let incoming_values: Vec<FieldValue> = incoming.iter().cloned().map(FieldValue::String).collect();
            let mut fields: BTreeMap<String, FieldValue> = BTreeMap::new();
            let transform =
                FieldTransform::AppendMissingElements("sharedWithUserIds".to_string(), incoming_values.clone());
            let result = apply_field_transform(&mut fields, &transform, (0, 0))
                .expect("appendMissingElements against a missing field never fails");
            prop_assert_eq!(result, None);

            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut expected = Vec::new();
            for (raw, value) in incoming.iter().zip(incoming_values.iter()) {
                if seen.insert(raw.clone()) {
                    expected.push(value.clone());
                }
            }
            prop_assert_eq!(fields.get("sharedWithUserIds"), Some(&FieldValue::Array(expected)));
        }

        /// Behavior 3 (residual, ADR-052 § Consequences): `appendMissingElements`
        /// against an existing NON-array field is `InvalidArgument`, field
        /// left completely unchanged.
        #[test]
        fn append_missing_elements_against_non_array_existing_value_is_invalid_argument_and_unchanged(
            s in "[a-z]{0,10}",
        ) {
            let existing = FieldValue::String(s);
            let mut fields = BTreeMap::from([("sharedWithUserIds".to_string(), existing.clone())]);
            let transform = FieldTransform::AppendMissingElements(
                "sharedWithUserIds".to_string(),
                vec![FieldValue::String("u-diego".to_string())],
            );

            let err = apply_field_transform(&mut fields, &transform, (0, 0))
                .expect_err("appendMissingElements against a non-array target must be rejected");

            prop_assert!(matches!(err, CoreError::InvalidArgument(_)));
            prop_assert_eq!(fields.get("sharedWithUserIds"), Some(&existing));
        }

        /// Behavior 4 (AC-03-03): `removeAllFromArray` removes ALL matching
        /// occurrences of every given value, not just the first.
        #[test]
        fn remove_all_from_array_removes_all_matching_occurrences(
            existing in prop::collection::vec("[a-c]", 0..8),
            to_remove in prop::collection::vec("[a-c]", 0..4),
        ) {
            let existing_values: Vec<FieldValue> = existing.iter().cloned().map(FieldValue::String).collect();
            let remove_values: Vec<FieldValue> = to_remove.iter().cloned().map(FieldValue::String).collect();
            let remove_set: std::collections::HashSet<&String> = to_remove.iter().collect();

            let mut fields =
                BTreeMap::from([("sharedWithUserIds".to_string(), FieldValue::Array(existing_values))]);
            let transform = FieldTransform::RemoveAllFromArray("sharedWithUserIds".to_string(), remove_values);
            let result = apply_field_transform(&mut fields, &transform, (0, 0))
                .expect("removeAllFromArray on an array field never fails");
            prop_assert_eq!(result, None);

            let expected: Vec<FieldValue> = existing
                .iter()
                .filter(|v| !remove_set.contains(v))
                .cloned()
                .map(FieldValue::String)
                .collect();
            prop_assert_eq!(fields.get("sharedWithUserIds"), Some(&FieldValue::Array(expected)));
        }

        /// Behavior 5 (AC-03-05): `removeAllFromArray` against a MISSING field
        /// is a true no-op — the field is NOT created.
        #[test]
        fn remove_all_from_array_against_missing_field_is_a_no_op_and_does_not_create_it(
            to_remove in prop::collection::vec("[a-c]", 0..4),
        ) {
            let remove_values: Vec<FieldValue> = to_remove.iter().cloned().map(FieldValue::String).collect();
            let mut fields: BTreeMap<String, FieldValue> = BTreeMap::new();
            let transform = FieldTransform::RemoveAllFromArray("sharedWithUserIds".to_string(), remove_values);
            let result = apply_field_transform(&mut fields, &transform, (0, 0))
                .expect("removeAllFromArray against a missing field never fails");
            prop_assert_eq!(result, None);
            prop_assert!(fields.is_empty(), "a no-op must not create the field");
        }

        /// Behavior 6 (residual, ADR-052 § Consequences): `removeAllFromArray`
        /// against an existing NON-array field is `InvalidArgument`, field
        /// left completely unchanged.
        #[test]
        fn remove_all_from_array_against_non_array_existing_value_is_invalid_argument_and_unchanged(
            s in "[a-z]{0,10}",
        ) {
            let existing = FieldValue::String(s);
            let mut fields = BTreeMap::from([("sharedWithUserIds".to_string(), existing.clone())]);
            let transform = FieldTransform::RemoveAllFromArray(
                "sharedWithUserIds".to_string(),
                vec![FieldValue::String("u-diego".to_string())],
            );

            let err = apply_field_transform(&mut fields, &transform, (0, 0))
                .expect_err("removeAllFromArray against a non-array target must be rejected");

            prop_assert!(matches!(err, CoreError::InvalidArgument(_)));
            prop_assert_eq!(fields.get("sharedWithUserIds"), Some(&existing));
        }
    }
}
