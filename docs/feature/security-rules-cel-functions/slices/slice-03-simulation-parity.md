# Slice 03: Alex Simulates a Function-Call-Bearing Candidate Rule

**Story**: US-03 | **Release**: 1 | **Estimate**: 0.5 day (confirmatory)

## Goal
Prove `simulate_access_rule` works correctly against the ALREADY-STORED, already-expanded
condition a function-authored import produced — zero new production code.

## IN Scope
- Acceptance test only: import a function-call-bearing rule (Slice 01), then call
  `simulate_access_rule` with synthetic `resource`/`auth` inputs against the stored (expanded)
  condition, assert correct allow/deny.

## OUT Scope
- Any production code change (same guardrail as Slice 02).
- Simulating a candidate condition that ITSELF contains raw, unexpanded `name()` syntax (outside
  any `.rules`-file import context) — `simulate_access_rule` takes a bare condition string, with no
  file-level function-definition context; a bare `name()` there is, correctly, still rejected by
  the existing, unmodified `CUSTOM_FUNCTION` mechanism. Out of this feature's own scope by design
  (ADR-067's mechanism is import-time only) — not a gap, a structural boundary.

## Learning Hypothesis
Disproves: `simulate_access_rule` needs ANY production code change beyond Slice 01's own
import-time expansion.

## Acceptance Criteria
AC-CF-06 (see `feature-delta.md` § User Stories, US-03).

## Dependencies
Slice 01.

## Effort Estimate
0.5 day.

## Reference Class
Same reference class as Slice 02.

## Pre-Slice SPIKE
Not required.
