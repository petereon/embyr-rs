# Slice 02: `array-contains-any` Queries Stop Crashing

**Story**: US-02 | **Release**: 1 | **Estimate**: 0.5 day

## Goal
A real, previously-panicking `ArrayContainsAny` `RunQuery` executes correctly via an OR of `push_
array_contains` checks, one per target value.

## IN Scope
- `ArrayContainsAny` arm in `append_field_filter`: iterate the target `FieldValue::Array`'s own
  elements, call `push_array_contains` once per element, join with `" OR "` inside parens.
- Empty-array fallback: push `"FALSE"` (Resolution 3).
- Real end-to-end proof: a document matching ANY listed value is returned; one matching none is
  excluded.

## OUT Scope
- `In`/`NotIn` (Slices 03-04).
- List-size limit enforcement (Resolution 4, out of scope).

## Learning Hypothesis
Disproves: `ArrayContainsAny`'s own OR-of-containment-checks composition needs anything beyond N
copies of Slice 01's own single-check shape.

## Acceptance Criteria
AC-QFO-04, AC-QFO-05 (see `feature-delta.md` § User Stories, US-02).

## Dependencies
Slice 01 (`push_array_contains`).

## Effort Estimate
0.5 day.

## Reference Class
Same reference class as Slice 01.

## Pre-Slice SPIKE
Not required.
