# Slice 07: Alex Can Pre-Check Whether a Candidate Collection-Group Query Would Be Accepted

**Story**: US-07 | **Release**: 2 | **Walking Skeleton**: No | **Estimate**: 1 day

## Goal
Extend `security-rules-query-path`'s own `simulate_query_compliance` admin action to accept a candidate collection-group rule, reusing `check_query_compliance()` identically to real enforcement.

## IN Scope
- `simulate_query_compliance`'s request body extended with an optional "this is a group-rule simulation" input (candidate group condition).
- Response reuses the existing `SimulateQueryComplianceResponse` shape (`compliant: bool`, `reasons: Vec<&'static str>`) — including the new `GROUP_RULE_NOT_DEFINED`-style reason code when no candidate group rule is supplied.
- Zero effect on live/published `RunQuery` traffic (mirrors AC-17-76's precedent).

## OUT Scope
- Any new endpoint (extends the existing one).
- Any second, independently-maintained compliance implementation.

## Learning Hypothesis
Disproves: "a group-query simulation cannot share the exact same `check_query_compliance()` function real enforcement uses without duplicating the decidable-shape logic a fourth time."
Confirms (if it succeeds): the existing simulation handler's extension pattern (already exercised 3 times across `security-rules`/`security-rules-write-path`/`security-rules-query-path`) generalizes cleanly to the group case.

## Acceptance Criteria
- AC-17-101: simulation returns the same admit/reject outcome real enforcement would produce.
- AC-17-102: "missing required filter" correctly reported.
- AC-17-103: "no collection-group rule defined" correctly reported when no candidate group rule is supplied.
- AC-17-104: zero effect on live traffic.

## Dependencies
- Slices 01–04 (the real compliance mechanism this slice wraps).
- `security-rules-query-path`'s own `simulate_query_compliance` handler (extended, not replaced).

## Effort Estimate
1 day. Reference class: `security-rules-query-path` Slice 07, same extension pattern, applied a second time.

## Pre-Slice SPIKE
None.
