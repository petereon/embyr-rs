# Slice 01: OR-Query Execution (Union Semantics, Walking Skeleton)

**Goal**: `RunQuery` with a top-level `Filter.or(a, b)` returns the union of documents matching
either branch, correctly parenthesized when nested inside AND context.

## IN scope
- New `QueryFilter::CompositeOr(Vec<QueryFilter>)` domain variant
  (`crates/embyr-core/src/domain/query.rs`), strictly additive.
- `translate_filter` (`crates/embyr-server/src/grpc/handler.rs`): accept `CompositeOp::Or`.
- `append_filter` (`crates/embyr-pg-storage/src/encoding/query.rs`): new `CompositeOr` arm —
  OR-join children, wrap the whole expression in parens.
- `requires_composite_index`/`missing_index_fields`: new `CompositeOr` arm reusing the existing
  field-collection recursion.
- `domain_filter_to_agent_filter` (`crates/embyr-server/src/adapters/agent_backend.rs`): new
  `CompositeOr` arm returning `CoreError::FailedPrecondition` — agent-mode explicitly deferred.

## OUT scope
- Security-rule compliance checking for OR (`filter_binds_field_to_uid`) — Slice 02.
- `backend_mode=agent` OR-filter execution — separately deferred (§ Out of Scope in
  feature-delta.md).

## Learning hypothesis
**Disproves** (if it fails): OR-query execution is a small, additive extension needing no changes
to any of the 15 existing AND-only `Composite` call sites. If it turns out the additive-variant
approach requires touching those call sites anyway, the "smaller, safer diff" premise was wrong.
**Confirms** (if it succeeds): a strictly additive domain-type extension, combined with the
compiler's own exhaustiveness checking, is enough to find and update every REQUIRED touch point
without missing one.

## Acceptance criteria
AC-OR-01, AC-OR-02, AC-OR-03, AC-OR-07 (feature-delta.md § US-01, plus the agent-mode rejection
piece of § US-02).

## Dependencies
None — first slice.

## Effort estimate
≤1 day.

## Reference class
This session's own repeated "additive variant over breaking change" pattern (e.g.
`firestore-malformed-filter-shape-validation`'s own additive rejection checks in `translate_filter`).

## Production-data acceptance criterion
Real `RunQuery` calls against a real running server + real Postgres backend: seed documents with
distinct field values, run a real `Filter.or(a, b)` query and confirm the union is returned; run a
nested `AND(x==1, OR(y==2, y==3))` query and confirm correct precedence (not the wrong,
unparenthesized result).
