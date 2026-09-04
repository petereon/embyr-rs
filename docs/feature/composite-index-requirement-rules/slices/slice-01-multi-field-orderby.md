# Slice 01: A Multi-Field Sort Correctly Requires a Composite Index (Walking Skeleton)

**Story**: US-01 | **Release**: 1 | **Estimate**: 0.5 day

## Goal
`requires_composite_index()` correctly detects 2+ `orderBy` fields as always needing a composite
index, regardless of filters — the single most severe currently-uncaught gap.

## IN Scope
- New rule in `requires_composite_index`: `query.order_by.len() >= 2` short-circuits to `true`,
  checked BEFORE any filter-field-membership logic (so it fires even with no filter, or when every
  `orderBy` field happens to already be a filtered field).
- Real end-to-end proof: a previously-silently-succeeding multi-`orderBy` `RunQuery` now correctly
  rejected `FAILED_PRECONDITION`, then succeeds once a matching composite index exists.
- Regression guard: the existing single-`orderBy`-field `category==`/`score` shape (AC-04f/AC-04g)
  is proven unchanged.

## OUT Scope
- `IN` + range-on-a-different-field, filter-only (Slice 02).
- Naming the missing index in the rejection message (Slice 03).

## Learning Hypothesis
Disproves: a real, previously-silently-succeeding multi-`orderBy` `RunQuery` can be correctly
rejected without breaking any existing single-`orderBy` regression.

## Acceptance Criteria
AC-CIR-01 through AC-CIR-03 (see `feature-delta.md` § User Stories, US-01).

## Dependencies
None — first slice.

## Effort Estimate
0.5 day.

## Reference Class
New logic, nearest reference class: this session's own established "add one new rule to a pure
gating function, verify old rules unchanged" discipline (mirrors 4c's `compare_relational`
timestamp-bug fix — a targeted rule addition, not a rewrite).

## Pre-Slice SPIKE
Not required — the exact rule (`order_by.len() >= 2` checked before the existing field-membership
logic) was confirmed correct via live Firestore verification during DISCUSS.
