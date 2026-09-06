# Slice 02: Array/Map Range Queries Get a Clean Rejection Instead of a Crash (LAST slice)

**Story**: US-02 | **Release**: 1 | **Estimate**: 0.5 day

## Goal
`translate_filter` rejects a range-operator filter against an `Array`- or `Map`-valued target with
a clean `INVALID_ARGUMENT`, reusing its own existing `Result<_, String>` error mechanism —
matching real Firestore's own actual behavior (Array range queries are confirmed unsupported;
Map's own support is unconfirmed, treated conservatively).

## IN Scope
- One `if matches!(...)` check in `translate_filter`'s own `FieldFilter` arm, checked after `op`/
  `value` are computed, before constructing `QueryFilter::Field`.
- Real end-to-end proof: a range-operator query against an `Array`-valued field returns a clean
  `INVALID_ARGUMENT`; same for `Map`.

## OUT Scope
- Building any real ordering semantic for `Array`/`Map` (Resolution 2, explicitly out of scope).
- A general filter-operator-vs-value-type validation framework (feature-delta.md § Out of Scope).

## Learning Hypothesis
Disproves: `Array`/`Map` + range-operator combinations can be rejected cleanly by reusing
`translate_filter`'s own existing error mechanism, without any new validation infrastructure.

## Acceptance Criteria
AC-RNG-05, AC-RNG-06 (see `feature-delta.md` § User Stories, US-02).

## Dependencies
None — independent of Slice 01 (a different file, a different mechanism).

## Effort Estimate
0.5 day.

## Reference Class
Mirrors `CompositeOp::Unspecified`'s own identical "unsupported construct → clean `Err(String)`"
precedent in the SAME function.

## Pre-Slice SPIKE
Not required.
