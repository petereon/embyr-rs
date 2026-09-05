# Slice 01: `Equal`/`NotEqual` Queries Stop Crashing on Every Field Type (Walking Skeleton, the entire feature)

**Story**: US-01 | **Release**: 1 | **Estimate**: ≤0.25 day

## Goal
Route `Equal`/`NotEqual` to the already-existing `push_value_equality`, closing the crash for
`Timestamp`/`Bytes`/`Reference`/`Array`/`Map`-valued filter targets with zero new production logic.

## IN Scope
- `append_field_filter`'s own `Equal`/`NotEqual` match arms call `push_value_equality` instead of
  `push_scalar_comparison`.
- Doc comment updates on both helpers (accuracy only, no behavior change).
- Real end-to-end proof: `Equal` on a `Timestamp` field, previously panicking, now returns correct
  results.
- Unit coverage for all 5 previously-crashing types plus the Resolution 2 type-strictness
  tightening (regression guard).

## OUT Scope
- Range-operator (`<`/`<=`/`>`/`>=`) support for these same 5 types (Resolution 3, real, deferred,
  candidate id `firestore-range-operator-value-type-support`).

## Learning Hypothesis
Disproves: `Equal`/`NotEqual` can be made to work for every `FieldValue` type by reusing an
already-existing, already-tested function, with zero new production logic.

## Acceptance Criteria
AC-ENV-01 through AC-ENV-07 (see `feature-delta.md` § User Stories, US-01).

## Dependencies
`firestore-query-filter-operator-support` (FINALIZED 2026-09-05) — provides `push_value_equality`.

## Effort Estimate
≤0.25 day.

## Reference Class
Mirrors `firestore-query-filter-operator-support`'s own `In`/`NotIn` design exactly — this feature
is that design's own direct generalization to 2 more operators.

## Pre-Slice SPIKE
Not required — the exact mechanism was confirmed correct by direct code read during DESIGN.
