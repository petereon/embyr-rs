# Slice 01: `array-contains` Queries Stop Crashing (Walking Skeleton)

**Story**: US-01 | **Release**: 1 | **Estimate**: 0.5 day

## Goal
A real, previously-panicking `ArrayContains` `RunQuery` executes correctly via a JSONB `@>`
containment check.

## IN Scope
- `push_array_contains(qb, field_path, target)` helper: `fields->'{field}'->'v' @> $1::jsonb`,
  `$1` bound as a one-element JSON array wrapping `field_value_to_json(target)` (reused unchanged).
- `ArrayContains` arm in `append_field_filter` calls it once.
- Real end-to-end proof: a document whose array field contains the target value is returned; one
  whose array does not is excluded; no panic either way.

## OUT Scope
- `ArrayContainsAny`/`In`/`NotIn` (Slices 02-04).

## Learning Hypothesis
Disproves: a real, previously-panicking `ArrayContains` query can be made to execute correctly
using the same `serde_json::Value` binding mechanism already proven elsewhere in this crate,
without a new sqlx feature or dependency.

## Acceptance Criteria
AC-QFO-01 through AC-QFO-03 (see `feature-delta.md` § User Stories, US-01).

## Dependencies
None — first slice.

## Effort Estimate
0.5 day.

## Reference Class
Mirrors this crate's own existing `fields_to_json(...)`/`.bind(&fields_json)` precedent, applied
to a smaller JSON shape.

## Pre-Slice SPIKE
Not required — the exact JSONB storage shape and binding mechanism were both confirmed by direct
code read during DESIGN.
