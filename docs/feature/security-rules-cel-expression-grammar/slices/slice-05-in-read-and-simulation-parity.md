# Slice 05: The Same Whitelist Grammar Gates Reads and Is Simulatable

**Story**: US-05 | **Release**: 2 | **Estimate**: 0.5 day

## Goal
Extend Slice 04's `in`/list-literal grammar to real read-path enforcement (`GetDocument`) and
`simulate_access_rule`.

## IN Scope
- `resource.data.<field> in [...]` gates a real `GetDocument` call correctly.
- `simulate_access_rule` evaluates an `in` candidate condition via the identical `evaluate()`
  routine.

## OUT Scope
- Anything beyond mechanical propagation of Slice 04's own grammar into 2 already-existing call
  sites (`GetDocument`, `simulate_access_rule`).

## Learning Hypothesis
Disproves: `in`'s own write-path + simulation parity needs anything beyond the identical mechanical
extension US-02/US-03 already proved for numeric comparisons — a confirmatory slice.

## Acceptance Criteria
AC-CEG-13, AC-CEG-14 (see `feature-delta.md` § User Stories, US-05).

## Dependencies
Slice 04.

## Effort Estimate
0.5 day.

## Reference Class
Mirrors Slices 02/03.

## Pre-Slice SPIKE
Not required.
