# Slice 03: `in` Queries Stop Crashing

**Story**: US-03 | **Release**: 1 | **Estimate**: 1 day

## Goal
A real, previously-panicking `In` `RunQuery` executes correctly via an OR of scalar-equality
checks, one per target value, reusing the existing per-`FieldValue`-type cast dispatch.

## IN Scope
- Extract `push_scalar_comparison(qb, field_path, op, value)` from the existing `Equal`/`NotEqual`/
  range-operator match block (lines 54-72 today) — a pure refactor, zero behavior change for the 6
  pre-existing operators.
- `In` arm in `append_field_filter`: iterate the target `FieldValue::Array`'s own elements, call
  `push_scalar_comparison` once per element with `"="`, join with `" OR "` inside parens.
- Empty-array fallback: push `"FALSE"` (Resolution 3).
- Real end-to-end proof: a document matching ANY listed value is returned.
- Regression guard: `Equal`'s own pre-existing behavior is unchanged after the refactor.

## OUT Scope
- `NotIn` (Slice 04).
- Widening value-type coverage beyond `Integer`/`String`/`Double`/`Boolean` (pre-existing,
  orthogonal gap, explicitly out of scope per feature-delta.md § Out of Scope).

## Learning Hypothesis
Disproves: `In`'s own OR-of-scalar-equality composition can be built by extracting the existing
per-`FieldValue`-type dispatch into a shared helper without changing `Equal`'s own observable
behavior.

## Acceptance Criteria
AC-QFO-06, AC-QFO-07 (see `feature-delta.md` § User Stories, US-03).

## Dependencies
None — independent of Slices 01-02 (a different mechanism: scalar dispatch, not JSONB containment).

## Effort Estimate
1 day.

## Reference Class
Mirrors `composite-index-requirement-rules`'s own "targeted rule addition, not a rewrite"
discipline.

## Pre-Slice SPIKE
Not required.
