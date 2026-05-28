# Slice 05 — RunQuery (Composite, Ordering, Indexes, Collection Groups)

**Goal**: Complex queries with composite filters, ordering, cursors, and collection groups work — and require matching indexes.

## IN scope
- `orderBy` (single + multi-field, ASC/DESC)
- Composite filters (`AND` + `OR` nesting)
- Cursor-based pagination: `startAt`, `startAfter`, `endAt`, `endBefore`
- Collection group queries: `from` with `all_descendants: true`
- Index enforcement: queries requiring a composite index fail with `FAILED_PRECONDITION` if no matching READY index exists
- Admin API index CRUD (`POST /admin/v1/projects/{id}/indexes`, `GET`, `DELETE`) — BLOCKING: no complex query works until index is created
- Index build (async background scan; status transitions `CREATING` → `READY`)

## OUT scope
- Index build progress reporting (treat as fire-and-forget for this slice)

## Learning Hypothesis
Disproves: "Index enforcement can be deferred — queries without an index just run slower, not incorrectly."
Confirms if: a query requiring a composite index returns `FAILED_PRECONDITION` before the index is created, and succeeds after it becomes READY.

## Acceptance Criteria
- Multi-field orderBy returns documents in correct order
- `startAfter(lastDoc)` skips the cursor document
- `all_descendants: true` matches documents across nested sub-collections
- Query with composite filter and orderBy without an index returns `FAILED_PRECONDITION` with message mentioning "index"
- After creating the index and waiting for READY, the same query succeeds

## Dependencies
S04 (simple queries)

## Effort estimate
≤1 day
