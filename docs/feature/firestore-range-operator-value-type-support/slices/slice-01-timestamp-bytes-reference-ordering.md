# Slice 01: Range Queries Stop Crashing on Timestamp/Bytes/Reference Fields (Walking Skeleton)

**Story**: US-01 | **Release**: 1 | **Estimate**: 0.5 day

## Goal
`push_scalar_comparison` gains 3 new match arms implementing correct real-Firestore-accurate
ordering for `Timestamp` (Postgres `ROW(...)` comparison), `Bytes` (decode base64 to `bytea`), and
`Reference` (flat string comparison).

## IN Scope
- `FieldValue::Timestamp(s, n)` arm: `ROW((fields->'{f}'->'s')::bigint, (fields->'{f}'->'n')::int)
  {op} ROW($1, $2)`.
- `FieldValue::Bytes(b)` arm: `decode(fields->'{f}'->>'v', 'base64') {op} decode($1, 'base64')`,
  reusing `STANDARD` (the same base64 engine `field_value_to_json` already uses).
- `FieldValue::Reference(r)` arm: `fields->'{f}'->>'v' {op} $1`.
- Real end-to-end proof: `GreaterThan` on `Timestamp`, `LessThan` on `Bytes` (with a hand-picked
  pair whose base64-text order disagrees with byte-value order), `GreaterThanOrEqual` on
  `Reference`.
- Regression guard: the 4 pre-existing types unchanged.

## OUT Scope
- `Array`/`Map` rejection (Slice 02).

## Learning Hypothesis
Disproves: `Timestamp`/`Bytes`/`Reference` range comparisons can be made to work correctly via 3
new, type-specific match arms, without a new domain type or a new function.

## Acceptance Criteria
AC-RNG-01 through AC-RNG-04 (see `feature-delta.md` § User Stories, US-01).

## Dependencies
None — first slice.

## Effort Estimate
0.5 day.

## Reference Class
`push_scalar_comparison`'s own existing 4-arm shape, widened by 3 more arms following the same
pattern.

## Pre-Slice SPIKE
Not required — every ordering mechanism was confirmed correct via live Firestore verification and
direct code read during DISCUSS/DESIGN.
