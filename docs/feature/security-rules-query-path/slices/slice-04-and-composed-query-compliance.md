# Slice 04: AND-Composed Query Compliance (Walking Skeleton)

**Story**: US-04 | **Release**: 1 | **Estimate**: 1.5 days

## Goal
A `RunQuery` against a collection whose rule combines an auth-presence
check and an ownership-equality check with `&&` (e.g.
`request.auth != null && request.auth.uid == resource.data.curator_id`) is
decided by independently requiring every conjunct to be satisfied — direct
structural mirror of `QueryFilter::Composite`'s own AND-only semantics.

## IN Scope
- Recursive decomposition of `Condition::And(left, right)` into requiring
  compliance of `left` AND `right` independently, reusing Slices 01/03's
  per-atom checks.
- `Condition::Literal(false)` anywhere in the AND tree short-circuiting to
  always-reject.
- New Trailmark domain example: `trip_photos` (curator-owned shared
  galleries).

## OUT Scope
- `Condition::Or`/`Condition::Not` composition (Slice 05 — rejected
  outright, not decomposed).
- Simulation of AND-composed rules (Slice 07, Release 2).

## Learning Hypothesis
**Disproves**: "An AND-composed rule cannot be decided by independently
requiring each conjunct, reusing `QueryFilter::Composite`'s own AND-only
semantics, without inventing new composition rules." Confirmed false if a
query satisfying both conjuncts is admitted, a query satisfying only one is
rejected, and each conjunct is checked independently (an anonymous session
is rejected on the auth conjunct even with a matching filter present).

## Acceptance Criteria
- AC-17-61: A query satisfying every AND'd conjunct is admitted.
- AC-17-62: A query satisfying only a subset of the AND'd conjuncts is
  rejected.
- AC-17-63: Each conjunct is checked independently.
- AC-17-64: AND-composed checking reuses the same per-conjunct rules as
  single-atom rules — no new per-conjunct semantics.

## Dependencies
- Slices 01 and 03 (the two atom-check families being composed).

## Reference Class
No direct 1:1 precedent in `security-rules`/`security-rules-write-path`
(both epics' rules were single-Compare shapes in their own domain
examples) — this slice is the one genuinely novel composition mechanism in
this feature, sequenced deliberately after Slice 05 (§ Prioritization) so
the highest-consequence reject-default is proven first.

## Pre-Slice SPIKE
**Flagged, not required to block start**: the inclusion of AND-decomposition
in v1's locked scope is a judgment call made without a live stakeholder
(feature-delta.md § Job Discovery Framing Resolution, confidence note; OQ-
SRQ-01). If the orchestrator defers OQ-SRQ-01 to Release 2, this slice moves
out of the Walking Skeleton.
