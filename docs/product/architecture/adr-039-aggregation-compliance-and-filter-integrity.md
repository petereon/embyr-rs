# ADR-039: Aggregation Compliance Reuse and Filter Integrity — Including a Severity-Flagged Pre-Existing Finding

## Status

Accepted

## Context

DISCUSS's own reading confirmed `check_query_compliance()` (ADR-031) is a pure function — `fn(condition: &Condition, filter: Option<&QueryFilter>, auth: Option<&AuthContext>) -> QueryComplianceOutcome` — with no coupling to `RunQuery`'s own response shape, directly reusable for an aggregation query's own embedded `StructuredQuery` filter tree. This ADR confirms that reuse and locks the exact composition, but its primary purpose is to document a **severity-flagged, DISCUSS-unanticipated finding** surfaced only by reading `crates/embyr-server/src/adapters/agent_backend.rs::AgentBackendAdapter::run_query` in full during this DESIGN pass (required reading item 7 in this task's own brief).

### The finding

`AgentBackendAdapter::run_query` (already shipped, client-facing, `backend_mode=agent`'s own `RunQuery` proxy) has this signature and body:

```rust
async fn run_query(
    &self,
    collection: &CollectionPath,
    _query: &StructuredQuery,          // <- deliberately unused (underscore-prefixed)
    _transaction_id: Option<&TransactionId>,
) -> Result<Vec<FirestoreDocument>, CoreError> {
    ...
    let sq = AgentStructuredQuery {
        from: vec![CollectionSelector { collection_id: collection.collection_path.clone(), all_descendants: false }],
        filter: None,          // <- the caller's own filter is never forwarded
    };
    ...
}
```

`handle_run_query` computes `domain_query.filter` from the incoming proto, passes it to `check_query_compliance()` for the admission decision, and — for `backend_mode=agent` — then calls `adapter.run_query(&collection, &domain_query, None)`, which **silently discards `domain_query.filter` entirely** before contacting the agent. The agent's own Postgres query (`crates/embyr-agent/src/server.rs::run_query` via `self.storage.run_query`) executes **unfiltered**, returning every document in the collection.

**Concrete consequence**: for any `backend_mode=agent` project with an ownership-equality access rule (e.g., `request.auth.uid == resource.data.owner_id`), a signed-in caller filtering to `owner_id == "their-own-uid"` is correctly ADMITTED by `check_query_compliance()` (the filter shape satisfies the rule), but the documents actually RETURNED are every document in the collection — including every OTHER end user's documents. This is a genuine, previously-undocumented, already-in-production cross-tenant/cross-user data exposure for every `backend_mode=agent` deployment's `RunQuery` calls. Postgres-family (`direct_pg`/`aws_secret`/`gcp_secret`) is unaffected — its own `run_query` correctly applies `append_filter`.

This is **not this feature's bug to fix** — `RunQuery` is outside this feature's own slice scope, and fixing it is a materially separate change to an already-shipped, unrelated RPC. It is flagged here, prominently, at HIGH severity, as the top-priority item in this DESIGN's own Handoff, because it directly shapes Slice 02's design: the new aggregation proxy must not repeat this pattern, or `RunAggregationQuery`'s own COUNT under `backend_mode=agent` would not just be numerically wrong, but would leak an unauthorized-scope count (revealing another user's row-count/existence) via the same mechanism.

## Decision Drivers

1. **Never repeat a known-dangerous pattern in new code** — "consistency with a pre-existing bug" is not a valid design goal, even though it would technically be the path of least resistance (copy `run_query`'s own shape).
2. `check_query_compliance()` itself must remain untouched (zero code changes) — the fix is entirely about what happens to the filter AFTER admission, not the admission decision itself.
3. BC-2/BC-4 separation (ADR-002, ADR-029) — adapters (`PostgresBackendAdapter`, `AgentBackendAdapter`) are IO-focused, not authorization-focused; the authorization decision belongs at the handler layer, exactly once.

## Decision

1. **`check_query_compliance()` is reused byte-for-byte unchanged**, invoked on the aggregation's own embedded `StructuredQuery`'s filter tree, at the identical composition point `handle_run_query` already uses (dual-arm `get_access_rule`/`get_group_access_rule` on `all_descendants`, strictly before any backend dispatch).

2. **Filter-identity invariant**: the exact same in-memory `QueryFilter` value passed to `check_query_compliance()` is the same value later passed to `adapter.run_aggregation_query(...)` — never reconstructed, never re-derived, by construction (one local `domain_query`/`AggregationQuery` binding in `handle_run_aggregation_query`, shared by reference into both calls). This mirrors the property `RunQuery`'s own Postgres-family path already has (and extends it, correctly, to the aggregation path for both backend families).

3. For `backend_mode=agent` COUNT (Slice 02): a **new** `domain_filter_to_agent_filter` function is added to `crates/embyr-server/src/adapters/agent_backend.rs`, forwarding the compliance-checked `QueryFilter` into the agent's own `RunAggregationQueryRequest.structured_query.filter` (`embyr.agent.v1.Filter`, which the agent's already-shipped `run_aggregation_query` handler already reads and applies via its own `proto_filter_to_domain` — confirmed by reading `crates/embyr-agent/src/server.rs:469-511`). This mirrors the conversion-function pattern already established in the same file (`field_value_to_agent_value`). Unmappable filter shapes — the agent's own `FieldFilterOp` enum (`storage_agent.proto`) has no `IS_NAN`/`IS_NOT_NAN` variants, unlike the client-facing domain `FilterOp` — fail closed (`CoreError::InvalidArgument`, surfaced as `Status::invalid_argument`), never silently dropped.

4. `AgentBackendAdapter::run_query`'s own pre-existing `filter: None` bug is named here explicitly and NOT fixed by this feature (out of slice scope) — flagged at **HIGH severity** in this DESIGN's Handoff as its own candidate urgent bugfix, independent of this feature's delivery. Recommended remediation (for that follow-up, not this feature): reuse the same `domain_filter_to_agent_filter` this ADR introduces.

## Alternatives Considered

1. **Silently mirror `run_query`'s own existing pattern (`filter: None`) for "consistency"** — Rejected. Would introduce a confidentiality bug into brand-new code with full knowledge of the danger; consistency with a known-broken precedent is not a design virtue.
2. **Re-run `check_query_compliance()` a second time inside the adapter layer (defense in depth)** — Rejected for v1. Duplicates the check's execution across two layers (handler + adapter) and blurs the BC-2/BC-4 separation this codebase's own ADR-002/ADR-029 establish; the filter-identity invariant (Decision 2) is judged sufficient given it is a structural (type-level sharing), not conventional, guarantee. Named as a candidate hardening follow-up if a future audit wants defense-in-depth on this specific boundary.

## Consequences

**Positive**: aggregation's own access-control guarantee is provably identical to `RunQuery`'s Postgres-family guarantee for both backend families — the walking skeleton's own Learning Hypothesis (Slice 01) is confirmed structurally, not just tested empirically. The severe pre-existing `RunQuery`/agent-mode gap is surfaced for the first time, with a concrete, low-cost remediation already designed and ready to reuse.

**Negative**: `crates/embyr-server/src/adapters/agent_backend.rs` gains a second filter-translation function (`domain_filter_to_agent_filter`, alongside the untouched, still-broken `run_query`) — a temporary asymmetry within the same file, until the flagged follow-up lands. This is judged an acceptable, explicitly-documented interim state, not a silent inconsistency.

## Changed Assumptions

None to `adr-031`/`adr-032` — `check_query_compliance()`/`group_access_rules` composition is reused with zero modification, confirming DISCUSS's own reading rather than revising it. The finding in this ADR's Context is new information about an UNRELATED, already-shipped RPC (`RunQuery`), not a correction to any prior ADR's own decision.
