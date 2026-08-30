# Slice 02 — COUNT Aggregation, Agent-Mode Backend

**Goal**: `RunAggregationQuery` with a COUNT aggregation works identically for `backend_mode=agent` projects, by proxying `embyr-agent`'s own already-shipped `RunAggregationQuery` RPC unchanged.

**Feature**: aggregation-queries
**Story**: US-02
**Estimated effort**: 0.5 day
**Sequence**: 2 of 4 (Walking Skeleton, part B; requires Slice 01)

---

## IN Scope

- Implement `AgentBackendAdapter::run_aggregation_query` (`crates/embyr-server/src/adapters/agent_backend.rs`) — calls the agent's existing `RunAggregationQuery` RPC (`crates/embyr-agent/src/server.rs::run_aggregation_query`, already shipped, COUNT-only) unchanged, translates its `count: i64` response into this feature's own `aggregate_fields` response shape under the caller's requested (or synthesized) alias.
- Error translation for an unreachable/erroring agent — reuse the same error-mapping convention every other `AgentBackendAdapter` method already uses for its own agent RPC calls.

## OUT Scope

- Any change to `proto/embyr/agent/v1/storage_agent.proto` or `crates/embyr-agent/src/server.rs` — the agent's own COUNT implementation (including its known fetch-then-`len()` inefficiency) is reused exactly as-is (feature-delta.md § Job Discovery Framing Resolution, Resolution 3, Option B).
- `backend_mode=agent` SUM/AVG — named follow-up, requires extending the agent's own proto (Resolution 3, Option A/C, rejected for this feature).

---

## Learning Hypothesis

**Disproves**: "The client-facing aggregation contract (proto shape + handler-level compliance check) cannot be satisfied for `backend_mode=agent` by proxying to `embyr-agent`'s own already-shipped `RunAggregationQuery` RPC without requiring changes to the agent's own proto or binary."

**Confirms if successful**: the access-rule compliance check, being handler-level and backend-independent (evaluated once, before ANY adapter dispatch), means agent-mode support is a pure adapter-proxy exercise — no duplicated authorization logic, no agent-side changes needed for COUNT correctness.

---

## Acceptance Criteria

- [ ] AC-01-07: A COUNT aggregation against a `backend_mode=agent` project returns the same response shape and correct count as the Postgres-family path (Slice 01), proxied through `embyr-agent`'s own existing `RunAggregationQuery` RPC unchanged.
- [ ] AC-01-08: Access-rule compliance is evaluated in `embyr-server`, before the agent is contacted — a caller never entitled to run the aggregation never causes a request to reach the customer's own agent process.
- [ ] AC-01-09: An unreachable agent produces a distinguishable, retryable-sounding error — never a silently-wrong count, never a crash.
- [ ] AC-01-10: Existing agent-mode `RunQuery`/`GetDocument` behavior is unaffected by this feature shipping — zero regression.

---

## Dependencies

- Slice 01: `required` (establishes the `BackendAdapter::run_aggregation_query` trait method and the handler-level compliance-check composition this slice's adapter implementation plugs into).
- `embyr-agent`'s own `RunAggregationQuery` RPC: `required`, already shipped.
