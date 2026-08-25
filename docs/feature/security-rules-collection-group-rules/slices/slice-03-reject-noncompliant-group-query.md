# Slice 03: A Non-Compliant Collection-Group Query Is Rejected Before Execution

**Story**: US-03 | **Release**: 1 | **Walking Skeleton**: Yes | **Estimate**: 1 day

## Goal
A `RunQuery` with `all_descendants = true` whose filter does NOT satisfy a defined group rule is rejected before touching Postgres, naming the missing constraint.

## IN Scope
- The `Rejected { unsatisfied_conjuncts }` branch of `check_query_compliance()`'s outcome, applied to the group-rule call site added in Slice 02.
- Reuse of `security-rules-query-path`'s own rejection-formatting convention (`query_compliance_rejection`-style, `Status::permission_denied`, `[REASON_CODE]` message convention) — extended to be reachable from the group-rule call site.
- Distinguishability: this rejection must read differently from Slice 04's "no group rule defined" rejection.

## OUT Scope
- The "no group rule defined" default (Slice 04).
- Any new `UnsatisfiedConjunct` variant.

## Learning Hypothesis
Disproves: "a non-compliant collection-group query cannot be rejected before touching Postgres, reusing `check_query_compliance()` completely unmodified."
Confirms (if it succeeds): the identical rejection mechanism `security-rules-query-path` already built works unmodified for the group case — only the input condition source changed.

## Acceptance Criteria
- AC-17-85: no filter → rejected before fetch.
- AC-17-86: unrelated filters, missing required conjunct → rejected.
- AC-17-87: wrong operator → rejected.
- AC-17-88: rejection distinguishable from Slice 04's "no group rule defined" rejection.

## Dependencies
- Slice 02 (the admit-path call site this slice adds the reject-path branch to).

## Effort Estimate
1 day. Reference class: `security-rules-query-path` Slice 02 (`AC-17-53..56`), same mechanism, same effort, applied to a second call site.

## Pre-Slice SPIKE
None.
