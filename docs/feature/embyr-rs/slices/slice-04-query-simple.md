# Slice 04 — RunQuery (Simple Filter)

**Goal**: SDK can query a collection with a single field filter; server translates to SQL correctly.

## IN scope
- `RunQuery` RPC with `StructuredQuery`
- Filter operators: `==`, `!=`, `<`, `<=`, `>`, `>=`, `IS_NAN`, `IS_NOT_NAN`, `ARRAY_CONTAINS`
- Single `from` clause (collection, not collection group)
- `limit` and `offset`
- `select` field mask (projection)
- `IN`, `NOT_IN`, `ARRAY_CONTAINS_ANY` array operators

## OUT scope
- `orderBy` + multi-field ordering (slice 05)
- Composite index enforcement (slice 05)
- Collection group queries (`all_descendants: true`) (slice 05)
- Cursor-based pagination (`startAt`, `endAt`) (slice 05)

## Learning Hypothesis
Disproves: "SQL translation of Firestore filter operators against JSONB columns produces incorrect results for edge cases (null, NaN, boolean type coercion)."
Confirms if: filter results match Google Firestore behavior for all operator types, including IS_NAN on null and non-number values.

## Acceptance Criteria
- `where("age", ">=", 18)` returns only documents with age ≥ 18
- `where("score", "==", NaN)` is equivalent to `IS_NAN` filter
- `where("items", "array-contains", "x")` returns documents where array contains "x"
- `limit(2)` returns exactly 2 documents
- `select(["name"])` returns only the `name` field

## Dependencies
S01 (gRPC server, DB adapter)

## Effort estimate
≤1 day
