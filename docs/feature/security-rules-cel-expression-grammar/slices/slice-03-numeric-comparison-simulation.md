# Slice 03: Alex Simulates a Numeric-Bound Candidate Rule Before Publishing It

**Story**: US-03 | **Release**: 1 | **Estimate**: 1 day

## Goal
`simulate_access_rule` evaluates a numeric-comparison candidate condition against a synthetic
resource, via the identical `evaluate()` routine real enforcement uses.

## IN Scope
- `simulate_access_rule` accepts and correctly evaluates a candidate condition using the new
  numeric literals + relational operators, against a caller-supplied synthetic `resource`.
- Zero new response shape — reuses `SimulateAccessRuleResponse` unchanged.

## OUT Scope
- Any new simulation route/handler.
- `in`, list, timestamp/duration simulation (later releases' own slices).

## Learning Hypothesis
Disproves: nothing new — a confirmatory slice proving simulation's existing "shares the exact
evaluation routine" architecture (ADR-029) needs zero change to accommodate a widened `Operand`
set, since it never special-cases operand shape.

## Acceptance Criteria
AC-CEG-08 (see `feature-delta.md` § User Stories, US-03).

## Dependencies
Slice 01.

## Effort Estimate
1 day (mostly acceptance-test authoring — implementation is expected to require zero production
code change, confirming the hypothesis above; if it DOES require a change, that is itself the
signal this slice exists to surface).

## Reference Class
Mirrors 4a's own US-05 (simulation-shares-evaluation precedent, reused unchanged by every
subsequent epic).

## Pre-Slice SPIKE
Not required.
