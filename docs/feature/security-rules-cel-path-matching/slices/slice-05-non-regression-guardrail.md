# Slice 05: Untouched Patterns, 4a's Own Imports, and the Full Regression Baseline Are Unaffected

**Story**: US-05 | **Release**: 1 | **Walking Skeleton**: No | **Estimate**: 1 day

## Goal
Prove — not assert — that this feature's own routing/overlap-detection mechanism has zero observable effect on 4a's own single-wildcard imports, the original zero-wildcard rules, any collection with no rule/pattern at all, and the full pre-existing regression suite.

## IN Scope
- Re-run the full pre-existing regression suite (133+ `security-rules`-family scenarios plus 4a's own delivered scenarios) unmodified.
- Targeted new scenarios proving 4a's own `profiles/{userId}`-shaped pattern is unaffected by this feature's routing mechanism coexisting alongside it.
- Targeted new scenario proving a collection with no rule/pattern of any kind remains fully unrestricted.

## OUT Scope
- Any new capability — this slice is a pure proof obligation over Slices 01–04's real behavior.

## Learning Hypothesis
Disproves: this feature's new routing mechanism cannot be proven additive to the pre-existing baseline plus 4a's own delivered scenarios without actually re-running them unmodified.
Confirms (if it succeeds): zero regression, mirroring every prior JOB-17 epic's own guardrail-last discipline.

## Acceptance Criteria
AC-17-224 through AC-17-227 (see `feature-delta.md` § User Stories, US-05).

## Dependencies
Slices 01–04 (this slice proves their combined real behavior).

## Effort Estimate
1 day.

## Reference Class
Mirrors 4a's own Slice 05 and every prior JOB-17 epic's own "regression-suite-as-guardrail" precedent.

## Pre-Slice SPIKE
Not required.
