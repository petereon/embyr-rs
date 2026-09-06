# Slice 01: Malformed Filter Shapes Get a Clean Rejection Instead of a Panic (Walking Skeleton, entire feature)

**Story**: US-01 | **Release**: 1 | **Estimate**: ≤0.25 day

## Goal
Extend `translate_filter`'s own existing Array/Map-rejection check with 2 more `if` conditions,
closing the last 4 confirmed-harmless-to-other-tenants panics with clean, named
`INVALID_ARGUMENT` errors.

## IN Scope
- `In`/`NotIn`/`ArrayContainsAny` given a non-`Array` value → named rejection.
- A range operator (`LessThan`/`LessThanOrEqual`/`GreaterThan`/`GreaterThanOrEqual`) given a `Null`
  value → named rejection.
- Real end-to-end proof: 2 real, previously-panicking malformed `RunQuery` calls now return a clean
  `INVALID_ARGUMENT`.
- Regression guard: every well-formed filter shape fixed by the 3 prior features still works.

## OUT Scope
- Any further malformed-input hardening beyond these 2 specific trigger shapes.
- Reframing as a security/DoS-mitigation feature (explicitly rejected — confirmed self-contained
  blast radius).

## Learning Hypothesis
Disproves: the 2 remaining malformed-filter-shape panics can be closed by extending
`translate_filter`'s own already-proven Array/Map-rejection check, without any new mechanism.

## Acceptance Criteria
AC-MFS-01 through AC-MFS-04 (see `feature-delta.md` § User Stories, US-01).

## Dependencies
`firestore-range-operator-value-type-support` (FINALIZED 2026-09-06) — provides the exact
`translate_filter` check this feature extends.

## Effort Estimate
≤0.25 day.

## Reference Class
Mirrors `firestore-range-operator-value-type-support`'s own `translate_filter` check almost
verbatim — this feature is that pattern's own direct extension.

## Pre-Slice SPIKE
Not required — the exact mechanism and insertion point were confirmed by direct code read during
DESIGN.
