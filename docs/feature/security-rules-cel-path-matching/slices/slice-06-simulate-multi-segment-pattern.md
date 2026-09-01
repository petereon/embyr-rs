# Slice 06: Alex Simulates a Multi-Segment Pattern Before Importing It

**Story**: US-06 | **Release**: 2 | **Walking Skeleton**: No | **Estimate**: 1 day

## Goal
Extend 4a's own simulation action to accept a synthetic CONCRETE PATH (not just a synthetic document ID) so Alex can exercise routing itself, not merely evaluation, before importing a candidate multi-segment pattern.

## IN Scope
- Simulation request accepts a candidate multi-segment pattern, a synthetic identity, and a synthetic concrete path.
- Response reports the resolved allow/deny outcome AND the routing's own bound variable values.
- A synthetic path that does not structurally match the candidate pattern produces a distinguishable "no matching pattern" outcome (never a false "deny").
- Zero effect on live/imported traffic.

## OUT Scope
- Any change to real enforcement — this slice is a thin wrapper over Slice 02's real routing+evaluation path.

## Learning Hypothesis
Disproves: a simulation of a multi-segment candidate pattern cannot share the exact same routing mechanism real enforcement uses without either duplicating routing logic or omitting a way to supply a synthetic concrete path.
Confirms (if it succeeds): mirrors 4a's own US-06 precedent, now proven for routing, not merely single-variable evaluation.

## Acceptance Criteria
AC-17-228 through AC-17-231 (see `feature-delta.md` § User Stories, US-06).

## Dependencies
Slice 02 (the real routing+evaluation path this slice wraps).

## Effort Estimate
1 day.

## Reference Class
4a's own Slice 06 (`simulate_access_rule` extension, additive field, response contract unchanged) — DESIGN must confirm whether the concrete-path input crosses ADR-032's own "genuinely different contract → new handler" threshold or remains an additive extension.

## Pre-Slice SPIKE
Not required.
