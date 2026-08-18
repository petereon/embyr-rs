# Slice 06: No-Rule and Sibling-Collection Regression Guardrail (Walking Skeleton)

**Story**: US-06 | **Release**: 1 | **Estimate**: 1 day

## Goal
A collection with no read rule defined, a sibling collection unaffected by
another collection's rule, and the full pre-existing 133-scenario
regression suite (113 prior + 20 `security-rules` acceptance) all keep
behaving exactly as before this feature shipped — proving query enforcement
is additive, not a silent behavior change elsewhere.

## IN Scope
- Structural guardrail: `get_access_rule` returning `None` short-circuits
  before the new compliance function or any query-shape inspection runs.
- Re-run of the full 133-scenario regression suite, unmodified.
- Confirmation `GetDocument`'s existing rule-enforcement behavior
  (`security-rules`) is byte-for-byte unmodified.

## OUT Scope
- New production logic — this slice is primarily a proof obligation over
  Slices 01–05's real behavior.

## Learning Hypothesis
**Disproves**: "Query enforcement on one collection cannot be proven to
leave untouched collections and the existing regression suite unaffected
without actually re-running them unmodified." Confirmed false if all 133
scenarios pass unmodified and a sibling collection's queries are
unaffected by another collection's rule.

## Acceptance Criteria
- AC-17-69: A `RunQuery` on a collection with no rule defined succeeds
  unfiltered, gated only by the existing `api_key`.
- AC-17-70: A rule on one collection has zero observable effect on
  `RunQuery` behavior for any other collection without its own rule.
- AC-17-71: The full 133-scenario pre-existing regression suite passes
  unmodified.
- AC-17-72: `GetDocument`'s existing rule-enforcement behavior is unmodified
  by this feature.

## Dependencies
- Slices 01–05 (the real behavior this slice proves is additive).

## Reference Class
Mirrors `security-rules`'s AC-17-14/15/16 and `security-rules-write-path`'s
AC-17-42/43/44/45 discipline exactly — the same regression-guardrail slice
shape, applied to `RunQuery`.

## Pre-Slice SPIKE
Not required.
