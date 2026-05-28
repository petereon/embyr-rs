# Slice 03A — Query Operations: RunQuery, ListDocuments, BatchGet, Aggregation, ListCollectionIds

**Goal**: All read-only query RPCs work through the agent identically to the Postgres adapter, including collection group queries.

**Feature**: embyr-agent
**Estimated effort**: ≤1 day
**Sequence**: 5 of 6 (after S01A and S02A; S02A needed for test data)

---

## IN Scope

- `StorageAgent::run_query` (via `RunQueryRequest` with `StructuredQuery`)
  - Filter operators: `==`, `!=`, `<`, `<=`, `>`, `>=`, `in`, `not-in`, `array-contains`, `array-contains-any`
  - Unary filters: `IS_NULL`, `IS_NOT_NULL`, `IS_NAN`, `IS_NOT_NAN`
  - AND / OR composite filters
  - OrderBy clauses (ASC/DESC, numeric type promotion for int64/double comparison)
  - Cursor pagination: `startAt`, `startAfter`, `endAt`, `endBefore`
  - `limit` and `offset`
  - Collection group query: `all_descendants=true` (matches any collection named `{collection_id}` under parent, at any depth)
  - Streaming response: one `RunQueryResponse` per matching document + final `{done: true}` response
  - Field path validation: `^[a-zA-Z_][a-zA-Z0-9_.]*$` before SQL interpolation (SPEC.md Invariant 6)
  - `from` clause with >1 selector: returns `Unimplemented`
- `StorageAgent::list_documents`
  - Pagination (default page size 100, look-ahead `pageSize + 1`)
  - Collection filter (optional)
- `StorageAgent::batch_get_documents` (note: not in current proto — proto extension needed or implement as multiple GetDocument calls)
  - `new_transaction` option: creates read-only transaction, returns transaction bytes as first response
  - One response per requested path (found or missing), order may differ from request
- `RunAggregationQuery`: COUNT (no field), SUM (numeric field), AVG (numeric field); alias required
- `ListCollectionIds`: distinct collection IDs of immediate child collections under parent

## OUT Scope

- Composite index enforcement (complex queries currently succeed without explicit index; enforcement is a later concern)
- `transaction` / `read_time` consistency selectors in RunQuery (stub for now — just implement base case)

---

## Learning Hypothesis

**Disproves**: "SQL translation of all Firestore filter operators (including IS_NAN, not-in, array-contains) can be completed in a single agent implementation day."

**Confirms if successful**: The Postgres adapter's SQL query builder can be reused directly in the agent, since the agent is just a different deployment of the same storage logic.

---

## Acceptance Criteria

- [ ] `RunQuery` with `==` filter returns only matching documents (SPEC.md §Query System §Filter Operators)
- [ ] `RunQuery` with `!=` filter excludes documents missing the field (SPEC.md §Query System: "A document missing the filtered field is excluded from all filter predicates")
- [ ] `RunQuery` with `IS_NAN` matches only double NaN values (SPEC.md §Query System §Unary filters)
- [ ] `RunQuery` with `all_descendants=true` matches documents in nested collections (SPEC.md §Collection Group Queries)
- [ ] `RunQuery` with `from` clause containing >1 selector returns `Unimplemented` (SPEC.md §RunQuery)
- [ ] `RunQuery` streaming response ends with `{done: true}` item (SPEC.md §gRPC Service §RunQuery)
- [ ] Field paths are validated against `^[a-zA-Z_][a-zA-Z0-9_.]*$`; invalid paths return `InvalidArgument` (SPEC.md Invariant 6)
- [ ] `RunQuery` with cursor + `limit`: returns correct page of results (SPEC.md §Query System §Cursors)
- [ ] `ListDocuments` default page size is 100; `next_page_token` absent on last page (SPEC.md §Pagination)
- [ ] `RunAggregationQuery` COUNT returns correct count of matching documents (SPEC.md §RunAggregationQuery)
- [ ] `RunAggregationQuery` SUM returns correct sum of specified numeric field (SPEC.md §RunAggregationQuery)
- [ ] `RunAggregationQuery` AVG returns correct average (SPEC.md §RunAggregationQuery)
- [ ] `RunAggregationQuery` with unsupported operator returns `Unimplemented` (SPEC.md §RunAggregationQuery Errors)
- [ ] `ListCollectionIds` returns distinct immediate child collection IDs (SPEC.md §ListCollectionIds)

---

## Spec Traceability

| AC | SPEC.md Reference |
|----|------------------|
| Missing-field exclusion for filter operators | §Query System: "A document missing the filtered field is excluded from all filter predicates, including !=, not-in, IS_NOT_NULL, IS_NOT_NAN" |
| IS_NAN behavior | §Query System §Unary filters |
| Collection group all_descendants | §Query System §Collection Group Queries |
| done:true final response | §gRPC Service §RunQuery response format |
| Field path validation | §Invariants Invariant 6 |
| Cursor boundary logic table | §Query System §Cursors |
| ListDocuments default page size | §Pagination §Default page sizes |
| Aggregation operators | §RunAggregationQuery §Supported aggregation operators |

---

## Dependencies

- S01A (agent wiring): `required`
- S02A (write operations for test data): `required`

---

## Note on BatchGetDocuments

The current `storage_agent.proto` does not declare a `BatchGetDocuments` RPC. This slice includes implementing it as part of the agent protocol extension OR implementing it as N parallel `GetDocument` calls in the `AgentAdapter` on the SaaS side. Decision deferred to DESIGN wave; flag as open design question.
