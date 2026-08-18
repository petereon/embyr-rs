# Slice 01: Ownership-Equality Query Compliance (Walking Skeleton)

**Story**: US-01 | **Release**: 1 | **Estimate**: 1.5 days

## Goal
A `RunQuery` call against a collection protected by a pure ownership-equality
read rule (`request.auth.uid == resource.data.owner_id`, either operand
order) is admitted and executes only when its filter tree includes a
matching equality constraint bound to the caller's own verified
`request.auth.uid` — proving static query-shape compliance is tractable
before any document is fetched.

## IN Scope
- New pure function in `embyr_core::access_control` (sibling to `evaluate()`)
  that pattern-matches `Condition::Compare(AuthUid, Eq, ResourceField(f))`
  (either order) against a `StructuredQuery`'s `QueryFilter` tree.
- Recursive walk through `QueryFilter::Composite`'s AND structure to find a
  `QueryFilter::Field(FieldFilter { field_path: f, op: Equal, value })`.
- Comparison of the filter's bound `value` against the caller's own
  `AuthContext.uid` — not merely field-name presence.
- Wiring into `grpc::handler::handle_run_query`, called BEFORE
  `adapter.run_query()`, consuming the same `get_access_rule` lookup and the
  same `VerifiedEndUserIdentity`/`None` value `handle_get_document` already
  reuses.

## OUT Scope
- Non-compliant-query rejection wording/status shape (Slice 02).
- Auth-presence-only / public rule shapes (Slice 03).
- AND-composed rules (Slice 04).
- Undecidable rule shapes (Slice 05).
- Regression proof (Slice 06).
- Simulation (Slice 07, Release 2).

## Learning Hypothesis
**Disproves**: "A query cannot be proven compliant with an ownership-equality
rule by statically comparing its filter tree against the rule's AST, without
either fetching a document first or requiring new query-execution
machinery." Confirmed false if the compliance function correctly admits a
matching-filter query and does so using only the already-translated
`StructuredQuery`, no adapter/SQL change.

## Acceptance Criteria
- AC-17-49: A `RunQuery` whose filter includes an equality constraint on the
  rule's referenced field, bound to the caller's own verified
  `request.auth.uid`, is admitted and executes.
- AC-17-50: Additional filters beyond the required one do not affect
  compliance.
- AC-17-51: A filter on the correct field but bound to a value other than
  the caller's own verified uid is rejected before execution.
- AC-17-52: Field-reference matching is exact-string, case-sensitive.

## Dependencies
- `security-rules` (FINALIZED) — `access_rules` table, `get_access_rule`,
  `parse_condition`, `Condition`/`Operand`/`AuthContext` types.
- `crates/embyr-core/src/domain/query.rs` — `StructuredQuery`/`QueryFilter`/
  `FieldFilter` (read, unmodified).

## Reference Class
Mirrors `security-rules`' own Slice 02 (US-02, `handle_get_document`
evaluation wiring) and `security-rules-write-path`'s Slice 02 (US-02,
`handle_create_document` wiring) — same "new pure logic + one new
pre-execution call site" shape, applied to `RunQuery` instead.

## Pre-Slice SPIKE
Not required — the AND-only `QueryFilter` shape and `evaluate()`'s
non-reusability were confirmed by direct code read during this DISCUSS (see
feature-delta.md § Job Discovery Framing Resolution).
