# Timestamp Cursor Pagination Fix

**Date**: 2026-09-19
**Closes**: Finding #44 (High, Database/Correctness), `docs/product/production-readiness-audit-2026-09-08.md` Follow-Up Scan
**Commit**: `ba7da5a`

## Business Context

`startAt`/`startAfter`/`endAt`/`endBefore` cursor pagination silently
dropped the WHERE-clause boundary for any orderBy field type other than
Integer/String/Double. Timestamp — the single most common real-world
`orderBy`+cursor pairing (e.g. paginating by `created_at`) — got no boundary
predicate at all. A client paginating by a Timestamp field saw every "page"
restart from the beginning: silent wrong data delivered with a 200-equivalent
success response, not a crash. No upstream validation rejected the
combination either, so the gap was invisible until read against real data.

## Key Decision

**Types now covered by cursor bounds**: Integer, String, Double, Boolean,
Timestamp, Bytes, Reference — the exact same set `push_scalar_comparison`
already established for range-operator filters (`<`/`<=`/`>`/`>=`) via the
earlier `firestore-range-operator-value-type-support` feature. A cursor
bound IS a range comparison under the hood, so the fix reuses that function
verbatim (made `pub(crate)`) instead of re-deriving type-aware SQL casting a
second time in `backend_adapter.rs`.

**Types rejected, not silently ignored**: Array, Map, Null. Real Firestore
has no meaningful ordering for a composite type (SPEC.md's own OrderBy
section only documents numeric/string/timestamp ordering), and this matches
`translate_filter`'s existing rejection of the same types for range-operator
filters. Added `validate_cursor_value_types` in `handler.rs`, invoked for
both `start_at` and `end_at`, returning a clean `invalid_argument` gRPC
status instead of letting a malformed cursor reach the SQL builder's
defensive panic.

## Key Files

- `crates/embyr-pg-storage/src/encoding/query.rs` — `push_scalar_comparison` made `pub(crate)`, doc updated to note cursor reuse.
- `crates/embyr-pg-storage/src/backend_adapter.rs` — both cursor blocks (startAt/startAfter, endAt/endBefore) call `push_scalar_comparison` instead of a narrower inline match.
- `crates/embyr-server/src/grpc/handler.rs` — `validate_cursor_value_types` + call sites; unit test `cursor_value_type_validation_tests`.
- `tests/acceptance/us_04_query_collection.rs` — `end_cursor_bounds_results_for_timestamp_field_type` (RED-verified against pre-fix code: `endAt(300)` returned all 4 seeded docs instead of bounding to 3).

## Follow-Up

None outstanding for this finding. Array/Map/Null are explicitly rejected
(not silently dropped) — no deferred gap. Cursor validation now has the same
type-safety posture as filter validation.
