# vector-search — Feature Delta

**Closes**: go-live assessment P5, vector half (TTL is separate and still open).
**Shape**: small, single-pass feature; no new ADR (no new port, no schema change).

## What shipped

`RunQuery` honours `StructuredQuery.find_nearest` (vendored proto gained `FindNearest`, field 9, tags as in the
public `google/firestore/v1/query.proto`).

- **Algorithm**: exact brute-force kNN. The query's filter runs through the normal gated path
  (`prepare_gated_query`: security rules, composite-index gate, rate limiting, auth), the matching documents are
  fetched (`adapter.run_query`, works for `direct_pg`, secret-manager modes and `agent`), and a pure function in
  `embyr-core` (`domain/vector.rs`) ranks them. No pgvector, no new column, no migration.
- **Vector representation**: Firestore's wire form, a map `{"__type__": "__vector__", "value": [doubles]}`. It is
  an ordinary `FieldValue::Map` in storage (tagged `{t, v}` JSON), so it round-trips unchanged; doubles are stored
  as JSON numbers (shortest round-trip form) and read back exactly.
- **Distances**: EUCLIDEAN (L2), COSINE (distance `1 - cos`, clamped to `[0, 2]`), DOT_PRODUCT (the dot product,
  larger is nearer). Ties break by `(collection_path, document_id)`. Threshold: `<=` for EUCLIDEAN/COSINE, `>=`
  for DOT_PRODUCT. `distance_result_field` is written into the returned documents.
- **Excluded documents**: vector field missing, not a vector, different dimension, non-finite component,
  zero-norm vector under COSINE, or an undefined (overflowing) distance.

## Limits

| Limit | Value | Behaviour beyond |
|---|---|---|
| `limit` | 1..=1000 | `InvalidArgument` |
| dimension (query and stored) | 1..=2048 | `InvalidArgument` (query); write of a vector > 2048 rejected by `validate_document_limits` |
| candidate documents scanned | `MAX_VECTOR_SCAN_DOCS` = 20,000 | `FailedPrecondition` ("narrow it with a filter") |

Cost is O(candidates x dimension) CPU plus holding up to 20,000 documents in memory per query. Rate limiting is the
existing per-project limiter (one request = one token), so a vector query is cheap to issue and not cheap to run:
the cap, not the limiter, is what bounds it.

Rejected with `InvalidArgument`: `find_nearest` combined with `order_by`, `limit`, `offset`, `start_at`, `end_at`;
`find_nearest` on `RunAggregationQuery`, `PartitionQuery`, `Listen` (including the BrowserChannel JSON parser).
The base-query restrictions are deliberately conservative (the ranking and `FindNearest.limit` replace them) and were
not cross-checked against the live Firestore backend.

No vector index is needed or consulted, so unlike real Firestore a missing "vector index" never fails a query.

## Security

A vector query is a list: it goes through `prepare_gated_query`, the same function `RunQuery` and `PartitionQuery`
use, so rule compliance (exact, group, pattern), the composite-index check and the project/suspension checks
are identical. `find_nearest` can never reach the SQL builder (it is ranked in memory); `vector_field` and
`distance_result_field` are still validated with `validate_field_path`. Rules that are not expressible as a
query-shape check are not evaluated per document, exactly as for `RunQuery`.

## Future upgrade path (optional pgvector index)

If a customer has the `vector` extension and wants more than 20,000 candidates per query: detect the extension in
the customer-DB probe, add an opt-in per-collection expression index (`((fields->...)::vector(N))` with HNSW or
IVFFlat) created through the existing composite-index machinery, and push `ORDER BY <-> / <=> / <#>` + `LIMIT`
into `run_query` when a ready index matches; keep the brute-force path as the fallback. The pure distance
function stays the reference oracle for testing the SQL path. The agent protocol would need a `find_nearest`
field to push the same down in `backend_mode=agent`.

## Test coverage

- Unit (local): `embyr-core` `domain::vector` (known values per measure, exclusion rules, ties, threshold
  semantics, nested fields, request validation, property checks: bounded and sorted, exactness vs threshold);
  `document_limits` (2048-dimension write cap); `embyr-server` handler request-validation tests
  (`find_nearest_tests`).
- Docker acceptance, compile-only in this PR (`tests/acceptance/vector_search.rs`, target `vector_search`): nearest-k
  ordering for each measure, result field, thresholds, filter + vector, SDK-written vector round trip, invalid
  requests and >2048-dimension write, rule denial, over-cap. Run by CI.
