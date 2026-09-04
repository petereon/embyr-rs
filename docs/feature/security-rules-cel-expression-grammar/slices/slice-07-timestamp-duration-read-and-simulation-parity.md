# Slice 07: The Same Time-Window Grammar Gates Every Surface and Is Simulatable (LAST slice)

**Story**: US-07 | **Release**: 3 | **Estimate**: 0.5 day

## Goal
Extend Slice 06's timestamp/duration grammar to real read-path enforcement (`GetDocument`) and
`simulate_access_rule`, closing this feature's own last remaining parity gate.

## IN Scope
- `resource.data.<field> </<=/>/>= request.time` (and the reverse operand order) gates a real
  `GetDocument` call correctly.
- `simulate_access_rule` evaluates a timestamp/duration candidate condition via the identical
  `evaluate()` routine, accepting a caller-supplied synthetic "now" value (mirrors
  `simulate_access_rule`'s own existing synthetic-input discipline for every other operand family
  — e.g. `SimulatedAuth`).

## OUT Scope
- Anything beyond mechanical propagation of Slice 06's own grammar into 2 already-existing call
  sites.

## Learning Hypothesis
Disproves: timestamp/duration's own write-path + simulation parity needs anything beyond the
identical mechanical extension already proved twice (Slices 02/03, 05) — a confirmatory slice.

## Acceptance Criteria
AC-CEG-20, AC-CEG-21 (see `feature-delta.md` § User Stories, US-07).

## Dependencies
Slice 06.

## Effort Estimate
0.5 day.

## Reference Class
Mirrors Slices 02/03, 05.

## Pre-Slice SPIKE
Not required.
