# Slice 04: `not-in` Queries Stop Crashing, Matching Real Firestore's Field-Must-Exist Rule (LAST slice)

**Story**: US-04 | **Release**: 1 | **Estimate**: 1 day

## Goal
A real, previously-panicking `NotIn` `RunQuery` executes correctly via an AND of scalar-inequality
checks, correctly excluding documents where the filtered field is entirely absent (live-verified
real-Firestore behavior), with zero explicit existence-check code for the non-empty-list case.

## IN Scope
- `NotIn` arm in `append_field_filter`: iterate the target `FieldValue::Array`'s own elements, call
  `push_scalar_comparison` once per element with `"!="`, join with `" AND "` inside parens.
- Empty-array fallback: push `fields->'{field_path}' IS NOT NULL` (Resolution 3 — the field-must
  -exist rule alone, since there's no per-element clause to derive it from when the list is empty).
- Real end-to-end proof: a document whose field exists and is not among the listed values is
  returned; a document whose field is ENTIRELY ABSENT is excluded; a document whose field is
  explicitly `null` (not absent) and `null` is not excluded is INCLUDED — proving the distinction
  between "absent" and "present but null."

## OUT Scope
- List-size limit enforcement (Resolution 4, out of scope).

## Learning Hypothesis
Disproves: `NotIn`'s own field-must-exist rule can be achieved via `NULL`-propagation alone (zero
explicit existence-check code) for the non-empty-list case, with an explicit fallback only for the
empty-list edge case.

## Acceptance Criteria
AC-QFO-08 through AC-QFO-10 (see `feature-delta.md` § User Stories, US-04).

## Dependencies
Slice 03 (`push_scalar_comparison`).

## Effort Estimate
1 day.

## Reference Class
Same reference class as Slice 03.

## Pre-Slice SPIKE
Not required — the `NULL`-propagation argument was confirmed by direct semantic reasoning during
DISCUSS, cross-checked against `!=`'s own existing (accidentally correct) behavior.
