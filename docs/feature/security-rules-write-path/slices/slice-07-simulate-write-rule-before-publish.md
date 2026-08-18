# Slice 07: Alex Tests a Write Rule Against Concrete Old/New Examples Before Publishing It

**Story**: US-07 | **Release**: 2 | **Walking Skeleton**: No | **Estimate**: 1 day

## Goal
Extend `security-rules`' existing `simulate_access_rule` admin action so Alex can simulate a candidate write rule against synthetic old/new document payloads and an operation type, sharing the exact same evaluation routine real write enforcement uses.

## IN Scope
- Extend `SimulateAccessRuleBody` (or a sibling request type) to accept an operation type (`create`/`update`/`delete`) plus optional synthetic `resource` and `request.resource` payloads, matching each operation's natural shape (create: `request.resource` only; update: both; delete: `resource` only).
- Call the identical `parse_condition`/`evaluate` functions Slices 02–04's real enforcement uses — no second, independently-maintained evaluation copy.
- Confirm zero effect on live traffic (never calls the write-condition upsert path, never touches a live document).
- Confirm the anonymous case (no synthetic identity) matches Slice 05's real anonymous-write evaluation.

## OUT Scope
- Any change to real write enforcement itself (Slices 02–05, already shipped and unaffected by this slice).

## Learning Hypothesis
Disproves: a write-rule simulation action cannot share the exact same evaluation routine as real write enforcement without duplicating (and risking drift in) the extended evaluation logic.
Confirms (if it passes): `security-rules`' own ADR-029 "simulation shares the exact evaluation routine" guarantee extends cleanly to the write-path grammar addition — one function, three real call sites (create/update/delete enforcement) plus one simulation call site, still zero duplication.

## Acceptance Criteria
- AC-17-46, AC-17-47, AC-17-48.

## Dependencies
- Slices 02–05 (the evaluation routine and operand families this slice wraps must exist first).

## Production-Data Taste Test
Real candidate write rules (including an intentionally-backwards immutable-field condition, to prove bug-catching) and real synthetic old/new document payloads, checked against the real evaluation path — not a hand-rolled test double.

## Reference Class
Direct precedent: `security-rules`' own US-05/Slice 05 (`docs/feature/security-rules/slices/slice-05-simulate-rule-before-publish.md`), extended to the write shape.
