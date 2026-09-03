# Slice 06: Alex Simulates a Candidate Recursive-Wildcard Pattern's Precedence Outcome Before Importing It

**Story**: US-06 | **Release**: 2 | **Estimate**: 1 day

## Goal
Extend 4b's own routing-aware simulation action to also consider recursive-wildcard candidates in the precedence resolution, reporting not just allow/deny but WHICH pattern (the candidate, or an already-stored more specific one) actually won — reusing the SAME precedence/routing mechanism real enforcement uses.

## IN Scope
- Simulate a candidate recursive-wildcard pattern against a synthetic identity + synthetic concrete path.
- Report the winning pattern's own attribution (candidate vs. an already-stored more specific pattern) alongside allow/deny.
- A synthetic path that does not structurally match the candidate's own fixed prefix produces a distinguishable "no matching pattern" outcome.
- Zero effect on live/imported traffic.

## OUT Scope
- Any new capability beyond wrapping Slice 02's own mechanism for simulation purposes.

## Learning Hypothesis
Disproves: a simulation of a candidate recursive-wildcard pattern's own precedence outcome cannot share the exact same routing+precedence mechanism real enforcement uses without either duplicating logic or omitting a way to supply a synthetic path that exercises the precedence rule itself (not just the match/no-match outcome 4b's own US-06 sufficed for).

## Acceptance Criteria
AC-17-259 through AC-17-262 (see `feature-delta.md` § User Stories, US-06).

## Dependencies
Slice 02 (the precedence-composition mechanism), 4b's own `simulate_routed_access_rule` (or DESIGN's own equivalent evolution of it).

## Effort Estimate
1 day.

## Reference Class
Mirrors 4b's own Slice 06 (thin wrapper over the real routing mechanism, DDD-PM-9 precedent: never a second, independently-maintained implementation).

## Pre-Slice SPIKE
Not required.
