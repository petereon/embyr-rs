# Slice 01 — COUNT Aggregation, Postgres-Family Backend Modes

**Goal**: `RunAggregationQuery` with a COUNT aggregation works end-to-end for `backend_mode` = `direct_pg`/`aws_secret`/`gcp_secret`, with the identical access-rule enforcement `RunQuery` already has.

**Feature**: aggregation-queries
**Story**: US-01
**Estimated effort**: 1 day
**Sequence**: 1 of 4 (Walking Skeleton, part A)

---

## IN Scope

- Add `RunAggregationQuery` RPC + `RunAggregationQueryRequest`/`RunAggregationQueryResponse`/`StructuredAggregationQuery`/`Aggregation` messages to `proto/google/firestore/v1/firestore.proto`; regenerate `embyr-proto` stubs.
  - `Aggregation` message: `repeated` in the request (wire-future-proofed for multi-aggregation), server validates exactly 1 entry in v1.
  - `RunAggregationQueryResponse{result: {aggregate_fields: {alias -> Value}}, read_time}`, matching SPEC.md's own already-documented contract.
  - COUNT operator only functional in v1; SUM/AVG entries in the request return `Unimplemented` (Slices 03-04 add real support).
  - Wire method shape (unary vs. `stream`) per `OQ-AGG-01` — MUST be resolved before this slice's proto is finalized, not assumed.
- Add `run_aggregation_query` to `crates/embyr-core/src/storage/backend_adapter.rs::BackendAdapter`, with a default-provided body returning an "unimplemented" `CoreError` (so `AgentBackendAdapter` compiles unmodified until Slice 02).
- Implement `PostgresBackendAdapter::run_aggregation_query` (`crates/embyr-pg-storage/src/backend_adapter.rs`) — reuses `append_filter` and the existing collection-group WHERE-clause branch from `run_query` unchanged; new `SELECT COUNT(*)` clause.
- New `grpc/handler.rs::handle_run_aggregation_query` — mirrors `handle_run_query`'s own composition exactly: auth, rate-limit, suspension check, `attach_client_identity_if_present`, dual-arm access-rule lookup (`get_access_rule` / `get_group_access_rule` on `all_descendants`), `check_query_compliance()`, dispatch to `adapter.run_aggregation_query()`.
- Field-path validation reuses the existing `^[a-zA-Z_][a-zA-Z0-9_.]*$` invariant (not exercised meaningfully by COUNT itself, but the collection/filter path is).

## OUT Scope

- `backend_mode=agent` (Slice 02)
- SUM/AVG (Slices 03-04)
- Composite-index enforcement (not applicable — v1 aggregation requests carry no `order_by`)
- Plain-REST JSON gateway support (pre-existing gap, not this feature's scope)
- Multiple aggregations per request (wire shape allows it; server rejects it in v1)

---

## Learning Hypothesis

**Disproves**: "A new client-facing `RunAggregationQuery` RPC cannot reuse `check_query_compliance()` (ADR-031/032) unchanged for its own underlying filter tree without requiring a new evaluation function, OR the existing `append_filter`/collection-group WHERE-clause logic in `PostgresBackendAdapter::run_query` cannot be reused for a `COUNT(*)` SELECT without duplicating the query-building code."

**Confirms if successful**: BC-4's query-shape-compliance mechanism and BC-2's own filter-building code are both cleanly reusable, zero-new-design, for a structurally different response shape (a computed scalar instead of a document stream) — validating that aggregation was a natural, not merely theoretical, extension point of the existing architecture.

---

## Acceptance Criteria

- [ ] AC-01-01: A COUNT aggregation filtered to a caller's own documents (per an ownership-equality access rule) returns the exact count of matching documents, carrying no document field data in the response.
- [ ] AC-01-02: A COUNT aggregation attempting to count another end user's documents is rejected as a permission denial, using the identical `check_query_compliance()` mechanism and rejection-reason vocabulary `RunQuery` already produces for the same scenario.
- [ ] AC-01-03: A collection-group COUNT aggregation (`all_descendants=true`) is governed by `group_access_rules`, never by a same-named exact-path rule, mirroring `RunQuery`'s own dual-arm composition (ADR-032) unchanged.
- [ ] AC-01-04: A collection with no access rule defined runs the aggregation unrestricted — zero behavior change from pre-feature `RunQuery` on the same collection.
- [ ] AC-01-05: An access rule using an undecidable `Condition` shape rejects every aggregation against that collection, regardless of filter shape.
- [ ] AC-01-06: Zero matching documents returns `count: 0`, never an error.

---

## Dependencies

- ADR-031 (`check_query_compliance`), ADR-032 (`group_access_rules`): `required`, both already shipped.
- `OQ-AGG-01` (wire shape): `required` — must be resolved before locking the proto message shape (see feature-delta.md § Handoff Package).

---

## Note on scope boundary

This slice deliberately does NOT touch `embyr-agent`'s own internal proto or binary, and does NOT attempt SUM/AVG. See `feature-delta.md § Job Discovery Framing Resolution` Resolutions 2 and 3 for the reasoning.
