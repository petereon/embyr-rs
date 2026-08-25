# Slice 02: A Compliant Collection-Group Query Is Admitted, Spanning Every Nesting Depth

**Story**: US-02 | **Release**: 1 | **Walking Skeleton**: Yes | **Estimate**: 1.5 days

## Goal
A `RunQuery` with `all_descendants = true` whose filter satisfies a defined group rule is admitted and returns matching rows from every nesting depth the collection id occurs at.

## IN Scope
- `handle_run_query` branch: when `all_descendants == true`, look up `group_access_rules` (Slice 01's table) instead of `access_rules`.
- When a group rule exists, call `check_query_compliance()` (ADR-031, unchanged) with the group condition.
- `Admitted` outcome proceeds to the existing, unmodified `adapter.run_query()` call (already collection-group-capable).
- Real domain proof: Maria's entries exist in BOTH top-level `journal_entries` and nested `expeditions/trek-2026/journal_entries`; her compliant group query returns both.

## OUT Scope
- Rejection paths (Slice 03: non-compliant; Slice 04: ungoverned).
- Any new decidable shape or evaluator branch — `check_query_compliance()` is reused completely unmodified.
- Any change to `backend_adapter::run_query`'s SQL.

## Learning Hypothesis
Disproves: "a collection-group query cannot be proven compliant with a group rule, and cannot correctly return matching rows from every actual nesting depth, without either enumerating nested paths first or requiring new query-execution machinery."
Confirms (if it succeeds): reusing `check_query_compliance()` unchanged, against the correct (group) table, is sufficient — the pre-existing `all_descendants` SQL branch does the real row-narrowing work.

## Acceptance Criteria
- AC-17-81: compliant group query admitted, returns rows from every nesting depth.
- AC-17-82: extra filters don't break compliance.
- AC-17-83: `check_query_compliance()`/`QueryComplianceOutcome`/`UnsatisfiedConjunct` reused completely unmodified.
- AC-17-84: row-level narrowing across nesting depths is proven via the pre-existing SQL branch, not new logic.

## Dependencies
- Slice 01 (group rule must exist to test against).
- `check_query_compliance()`, `filter_binds_field_to_uid()` (ADR-031) — read-only reference.

## Effort Estimate
1.5 days. Reference class: `security-rules-query-path` Slice 01 (the WS that introduced `check_query_compliance()`'s own call site), same complexity class, plus real multi-location test fixture setup.

## Pre-Slice SPIKE
None — mechanism confirmed reusable by direct code read (ADR-031), no unknowns.
