# ADR-041: Agent-Mode Aggregation Scope — Proxy-Only, and a Correction to Slice 01's Own `CoreError` Assumption

## Status

Accepted

## Context

DISCUSS's Resolution 3 already decided, with a documented options table: `backend_mode=agent` COUNT (Slice 02) proxies `embyr-agent`'s own already-shipped `RunAggregationQuery` RPC (unary, COUNT-only, `docs.len()`-based) unchanged; SUM/AVG-for-agent and the agent's own COUNT-implementation inefficiency are explicitly deferred as named follow-ups. This ADR confirms that call with DESIGN-level evidence and locks the exact mechanism — and corrects one DISCUSS-level implementation assumption that DESIGN's own reading of `crates/embyr-agent/src/server.rs` in full (required reading item 8) shows does not hold as stated.

### The correction

Slice 01's own Technical Notes state the `BackendAdapter::run_aggregation_query` trait method should have "a default-provided body returning `CoreError::Unimplemented` (**or equivalent**)." Reading `crates/embyr-agent/src/server.rs:84-97` in full shows `core_error_to_status` there is an **exhaustive match over every `CoreError` variant, with no wildcard arm**:

```rust
fn core_error_to_status(e: CoreError) -> Status {
    match e {
        CoreError::DocumentNotFound(msg) => Status::not_found(msg),
        // ... 9 more explicit arms, covering every variant that exists today ...
        CoreError::ResourceExhausted(msg) => Status::resource_exhausted(msg),
    }   // no `_ =>` arm
}
```

`CoreError` is a single enum defined once in `embyr-core` and used verbatim by both `embyr-server` and `embyr-agent`. Adding a NEW variant (e.g., `CoreError::Unimplemented`) to close Slice 01's own literal wording would force a compile-fix edit inside `crates/embyr-agent/src/server.rs` — a source-level change to the agent binary, directly contradicting Resolution 3's own explicit "zero changes to... the agent binary." This holds regardless of whether the new variant is ever actually PRODUCED on the agent's own code path; the exhaustive match alone forces the edit.

## Decision Drivers

1. Resolution 3's "zero changes to the agent binary" is honored **literally** — not just functionally (no new agent-visible behavior) but structurally (not one line of `crates/embyr-agent/` source is touched by this feature, including compile-fix-only edits).
2. `docs/SPEC.md §RunAggregationQuery` documents `Unimplemented` as the client-facing gRPC status for "unsupported aggregation operator" — the CLIENT-FACING contract, not the internal `CoreError` plumbing, is what must carry that status code correctly.
3. Reuse over invention (standing session practice) — `CoreError::FailedPrecondition` already exists, already maps correctly in both existing `core_error_to_status` implementations, and needs no new arm anywhere.

## Decision

1. **No new `CoreError` variant.** The `BackendAdapter::run_aggregation_query` default-provided body (in practice unreachable once both concrete adapters override it by the end of Slice 02) and `AgentBackendAdapter`'s own SUM/AVG-rejection branch (Slice 02, v1) both return the EXISTING `CoreError::FailedPrecondition(String)`. Zero shared-enum change; zero forced edit, of any kind, inside `crates/embyr-agent/`.

2. **A new, small, LOCAL error-mapping function** — `aggregation_error_to_status`, added in `crates/embyr-server/src/grpc/handler.rs` alongside (not replacing) the existing shared `core_error_to_status` — is used ONLY by `handle_run_aggregation_query`:
   ```rust
   fn aggregation_error_to_status(e: CoreError) -> Status {
       match e {
           CoreError::FailedPrecondition(msg) => Status::unimplemented(msg),
           other => core_error_to_status(other),
       }
   }
   ```
   This delivers SPEC.md's own documented `Unimplemented` client-facing contract for "this backend/operator combination isn't implemented," scoped to exactly one RPC handler — `FailedPrecondition`'s OTHER existing meaning elsewhere in the codebase (e.g., `RunQuery`'s composite-index-required rejection, a structurally unrelated call site using the shared `core_error_to_status` unchanged) is unaffected.

3. **`AgentBackendAdapter::run_aggregation_query`** (`crates/embyr-server/src/adapters/agent_backend.rs`, Slice 02): matches on the domain `AggregationKind`.
   - `Count` → builds the agent's own `RunAggregationQueryRequest` (`parent` + translated `StructuredQuery`, filter forwarded per ADR-039 § Decision 3) → calls `embyr-agent`'s existing, unchanged `RunAggregationQuery` RPC → maps `count: i64` into `AggregateValue::Count`. Error translation reuses the SAME `grpc_err` convention every other `AgentBackendAdapter` method already uses (`CoreError::BackendUnavailable(format!("agent gRPC error: {e}"))`), satisfying AC-01-09 (an unreachable agent produces a status distinguishable from both a permission denial and a successful zero-count result — `core_error_to_status`'s existing `_ => Status::internal(...)` fallback already delivers this for `BackendUnavailable`, unchanged).
   - `Sum` | `Avg` → `Err(CoreError::FailedPrecondition("SUM/AVG aggregation is not supported for backend_mode=agent in v1".into()))`.

4. **Zero changes to `storage_agent.proto` or any file under `crates/embyr-agent/`** — genuinely zero, per Decision 1's own correction, not merely "zero behavioral changes." `embyr-agent`'s own COUNT-implementation inefficiency (`docs.len()` fetch-then-count) is reused exactly as-is, named as a follow-up (unchanged from DISCUSS's own Resolution 3).

## Alternatives Considered

1. **Add `CoreError::Unimplemented` as Slice 01's Technical Notes literally suggested** — Rejected per this ADR's own Decision 1 reasoning: forces an (admittedly trivial, one-line) compile-fix edit inside the agent binary, contradicting Resolution 3.
2. **Have the shared `core_error_to_status` itself special-case `FailedPrecondition` → `Status::unimplemented`** — Rejected: `FailedPrecondition` already carries a distinct, correct meaning for OTHER call sites (`handle_run_query`'s composite-index rejection maps `Status::failed_precondition` there deliberately); changing the SHARED function's mapping would silently alter that unrelated behavior. A handler-local override (Decision 2) achieves the same client-facing result with zero blast radius elsewhere.
3. **Detect "SUM/AVG requested for `backend_mode=agent`" in the handler itself, before calling the adapter at all** — Rejected: would require the handler to know backend-mode-specific operator support (a leaky abstraction across the `BackendAdapter` port boundary); letting the adapter itself decide what it supports, uniformly for both concrete adapters via the same trait method, is the existing, correct port/adapter discipline (ADR-002/ADR-029) and requires no handler-level backend-mode branching.

## Consequences

**Positive**: Resolution 3's "zero agent-binary changes" holds literally, not just functionally — verifiable by `git diff crates/embyr-agent/` showing nothing. SPEC.md's own documented error taxonomy (`Unimplemented` for unsupported operators) is honored precisely, scoped correctly. Zero new shared-enum surface for other future features to accidentally rely on or collide with.

**Negative**: `handle_run_aggregation_query` uses a DIFFERENT error-mapping function than every other handler in the file (a deliberate, documented exception, not an oversight) — flagged here explicitly so a future reader of `grpc/handler.rs` does not "fix" it into uniformity without reading this ADR first.
