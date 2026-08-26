# Slice 07: Alex Simulates a Claims-Based Rule Before Publishing

**Story**: US-07 | **Release**: 2 | **Walking Skeleton**: No | **Estimate**: 1 day

## Goal
Alex calls the existing rule-simulation admin action with a candidate claims-referencing condition plus a synthetic identity carrying a synthetic claims map, and sees the resolved allow/deny outcome — with zero effect on live traffic.

## IN Scope
- Extend the existing `simulate_access_rule` admin handler's request body with an optional synthetic claims map on the synthetic identity.
- Reuse the identical `evaluate()` routine real enforcement uses (no duplicate evaluation logic).
- Support for the missing-claim case (empty synthetic claims map) matching US-04's real fail-closed behavior.
- Support for string-literal comparisons (Slice 06) in the simulated condition.

## OUT Scope
- Any new admin endpoint — reuses the existing one.
- Any change to real enforcement's own call sites.

## Learning Hypothesis
**Disproves if it fails**: that a claims-based rule can be simulated via the exact same evaluation routine real enforcement uses, without duplicating (and risking drift in) the evaluation logic a sixth time (after `client-auth`'s US-04, `security-rules`'s US-05, and each subsequent epic's own reuse).

**Confirms if it succeeds**: the shared-evaluation-routine discipline holds even for the newest operand family.

## Acceptance Criteria
- AC-17-154: Simulating a candidate claims-referencing rule against a synthetic identity carrying a synthetic claims map returns the same allow/deny outcome real evaluation would produce.
- AC-17-155: Simulation supports the missing-claim case identically to real fail-closed evaluation.

## Dependencies
Depends on Slice 02 (evaluation mechanism) and Slice 06 (string-literal support, for the most valuable domain example — a simulated role check).

## Production-Data Taste Test
Real candidate claims-based rule + real synthetic identity carrying a synthetic claims map, checked against the real `evaluate()` routine (not a hand-rolled test double).

## Effort Estimate
1 day. Thin wrapper over already-existing simulation infrastructure.

## Pre-Slice SPIKE
Not needed.
