# Slice 05: Untouched Patterns, 4a's and 4b's Own Imports, and the Full Regression Baseline Are Unaffected

**Story**: US-05 | **Release**: 1 | **Estimate**: 1 day

## Goal
Prove, via the full existing regression suite plus targeted new precedence-guardrail scenarios, that importing a recursive-wildcard catch-all has zero effect on any 4a exact-match row, any 4b fixed-depth pattern, or any collection with no rule and no reaching recursive-wildcard pattern.

## IN Scope
- Re-run the full pre-existing regression baseline (4a's + 4b's own delivered scenarios) unmodified.
- New guardrail scenarios: a specific rule's own behavior is unaffected even after a structurally-overlapping catch-all becomes active.
- A collection with no rule and no reaching pattern of any kind remains fully unrestricted.

## OUT Scope
- New capability of any kind — this is purely a proof obligation over Slices 01–04's own real behavior.

## Learning Hypothesis
Disproves: importing a recursive-wildcard pattern cannot be proven not to silently affect 4a's own rows, 4b's own patterns, or a collection already governed by a more specific rule, without actually re-running the full existing regression suite plus targeted new precedence-guardrail scenarios.

## Acceptance Criteria
AC-17-255 through AC-17-258 (see `feature-delta.md` § User Stories, US-05).

## Dependencies
Slices 01–04 (this feature's own real, shipped behavior to prove non-interference over).

## Effort Estimate
1 day.

## Reference Class
Mirrors 4a's and 4b's own identical US-05 discipline, extended to re-prove non-interference under this feature's own new precedence composition specifically.

## Pre-Slice SPIKE
Not required.
