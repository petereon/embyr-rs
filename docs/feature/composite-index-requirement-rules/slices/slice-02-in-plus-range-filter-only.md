# Slice 02: A Filter-Only `IN` + Range Query Correctly Requires a Composite Index

**Story**: US-02 | **Release**: 1 | **Estimate**: 0.5 day

## Goal
`requires_composite_index()` correctly detects `IN` combined with a range filter (`<`/`<=`/`>`/`>=`)
on a DIFFERENT field, independent of `orderBy` presence — closing the top-level `order_by.is_empty()`
short-circuit's own blind spot.

## IN Scope
- `collect_filter_fields` (or a sibling helper) gains operator visibility (today: field paths only,
  no `FilterOp`) — needed to distinguish `In` from range operators from equality.
- New rule: `IN` on field A + a range operator on field B (A != B) → `true`, checked REGARDLESS of
  `order_by` (this rule must run even when `requires_composite_index`'s own top-level `order_by.
  is_empty()` early return would otherwise apply).
- Real end-to-end proof: a filter-only (no `orderBy`) `IN`+range `RunQuery` now correctly rejected.
- Regression guard (false-positive check): `IN` + a SEPARATE EQUALITY filter on a different field
  (compound-equality-only) still succeeds with no index — live-verified NOT to need composite.
- Regression guard: a lone `array-contains-any` or lone `not-in` still succeeds with no index.

## OUT Scope
- Two simultaneous `IN` filters (Resolution 4 — genuinely undetermined, deferred).
- Naming the missing index in the rejection message (Slice 03).

## Learning Hypothesis
Disproves: a filter-only `IN`+range query can be detected without the top-level `order_by.
is_empty()` short-circuit silently swallowing it, and without producing a false positive on the
already-correct compound-equality-only shape.

## Acceptance Criteria
AC-CIR-04 through AC-CIR-06 (see `feature-delta.md` § User Stories, US-02).

## Dependencies
None — independent of Slice 01 (touches a different branch of the same function).

## Effort Estimate
0.5 day.

## Reference Class
Same reference class as Slice 01.

## Pre-Slice SPIKE
Not required.
