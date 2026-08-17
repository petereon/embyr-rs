# Slice 04: Prove Untouched Collections and the Existing Suite Are Unaffected (Walking Skeleton)

**Story**: US-04 | **Release**: 1 | **Estimate**: 1 day | **job_id**: JOB-17

## Goal
A collection with no rule defined continues reading exactly as before this feature shipped, and a rule on one collection has zero observable effect on any other collection or the 113 pre-existing `embyr-rs`/`client-auth` regression scenarios.

## IN Scope
- Regression proof: full re-run of the 72 `embyr-rs` + 41 `client-auth` acceptance scenarios, unmodified, against a build containing this feature.
- Per-collection isolation proof: a project with a rule on one collection and none on a sibling collection behaves correctly for both.
- The "no rule defined = default allow" behavior itself (Resolution 2) — this slice is where that decision is proven, not just declared.

## OUT Scope
- Any new production logic beyond what Slices 01–03 already introduced — this is primarily a proof/guardrail slice, matching `client-auth`'s own AC-16-08 discipline.
- Structural (compile-time/type-level) unreachability guarantees are a nice-to-have DESIGN may pursue (mirroring `client-auth`'s AC-16-08 precedent) but are not mandated by this slice's DISCUSS-level scope.

## Learning Hypothesis
**Disproves if it fails**: a rule defined for one collection cannot be proven not to silently affect a different collection, or the existing regression suite, without actually re-running all 113 pre-existing scenarios unmodified — i.e., that the no-rule-defined default-allow path shares enough code with the rule-evaluation path to risk regression.
**Confirms if it succeeds**: the "no rule defined" path is a cheap, clearly separate branch (a rule-existence check) from the full evaluation path, adding no risk to unarmed collections.

## Acceptance Criteria
- AC-17-14: Collection with no rule defined behaves identically to pre-feature behavior.
- AC-17-15: A rule on one collection has zero effect on a sibling collection without its own rule.
- AC-17-16: Full 113-scenario pre-existing regression suite passes unmodified.

## Dependencies
- Slices 01–03 (this slice proves properties of their combined behavior).
- The existing 72-scenario `embyr-rs` and 41-scenario `client-auth` regression suites (both pre-existing, unmodified).

## Effort Estimate
1 day. Reference class: `client-auth`'s own AC-16-08(a) regression-gate re-run discipline (checkpoint re-run at two points in that feature's DELIVER wave).

## Pre-Slice SPIKE
Not required.
