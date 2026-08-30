# Slice 03 — SUM Aggregation, Postgres-Family Backend Modes

**Goal**: `RunAggregationQuery` with a SUM aggregation on a numeric field works end-to-end for `backend_mode` = `direct_pg`/`aws_secret`/`gcp_secret`.

**Feature**: aggregation-queries
**Story**: US-03
**Estimated effort**: 1 day
**Sequence**: 3 of 4 (Release 2; requires Slice 01)

---

## IN Scope

- Extend `PostgresBackendAdapter::run_aggregation_query` to support `SUM(field)` — numeric type coercion mirroring the existing cursor-comparison pattern (`(fields->>'field')::float8`), excluding documents missing the field or holding a non-numeric value.
- Extend the wire-level `Aggregation` enum handling in `grpc/handler.rs::handle_run_aggregation_query` to accept `SUM` (still single-aggregation-per-request in v1).
- Field-path validation for the summed field reuses the existing `^[a-zA-Z_][a-zA-Z0-9_.]*$` invariant.

## OUT Scope

- `backend_mode=agent` SUM — named follow-up (feature-delta.md § Job Discovery Framing Resolution, Resolution 3); `AgentBackendAdapter::run_aggregation_query` continues to support COUNT only and returns `Unimplemented` for a SUM request in v1.
- AVG (Slice 04).
- Multiple aggregations per request.

---

## Learning Hypothesis

**Disproves**: "Numeric-field SUM cannot reuse the SAME WHERE-clause-building/filter/collection-group/compliance-check machinery Slice 01 established, requiring instead a materially different query path."

**Confirms if successful**: only the `SELECT`/aggregate clause differs between COUNT and SUM — the entire access-control and filter-building layer Slice 01 built is a stable, reusable foundation for every subsequent aggregation type.

---

## Acceptance Criteria

- [ ] AC-01-11: A SUM aggregation on a numeric field, filtered to a caller's own documents, returns the correct total across all documents holding a valid numeric value for that field.
- [ ] AC-01-12: A document missing the summed field, or holding a non-numeric value for it, is silently excluded from the sum — never causes an error.
- [ ] AC-01-13: Summing across zero matching documents returns `sum: 0`, never an error.
- [ ] AC-01-14: SUM aggregation is governed by the identical access-rule compliance mechanism as COUNT (Slice 01) — same rejection behavior for the same unauthorized scenario.
- [ ] AC-01-15: A field path failing the existing field-path validation invariant is rejected as invalid before query execution.
- [ ] AC-01-16: Existing COUNT aggregations (Slices 01-02) are unaffected by SUM's addition — zero regression.

---

## Dependencies

- Slice 01: `required`.
