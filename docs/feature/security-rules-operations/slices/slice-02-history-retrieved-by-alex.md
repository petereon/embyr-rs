# Slice 02 (Walking Skeleton): Alex Sees Exactly What a Rule Used to Say, By Whom, and When

**Story**: US-02 | **Release**: 1 | **Estimate**: 0.75 day

## Goal
Alex can retrieve a collection's complete history — every captured entry, newest first, each correctly attributed — completing the Walking Skeleton.

## IN Scope
- A new, read-only admin action retrieving a collection's history from the table Slice 01 populates, ordered newest-first.
- Correct empty-list behavior for a collection that has never had a rule defined.
- Any-role (Viewer included) read access, mirroring the existing `simulate_*` any-role precedent.
- Standard admin-endpoint auth rejection (401) for a missing/invalid session.

## OUT Scope
- Restoring a rule (Slice 03).
- `write_access_rules`/`group_access_rules` history retrieval (Slices 04/05).
- Pagination, retention limits, or export (§ Out of Scope, feature-delta.md).

## Learning Hypothesis
**Disproves if it fails**: history captured in Slice 01 cannot be retrieved in correctly-ordered, correctly-attributed form without a new, real query surface.
**Confirms if it succeeds**: the Walking Skeleton is complete — Alex can see, end-to-end, exactly what a rule used to say and who changed it.

## Acceptance Criteria
- AC-17-160: History retrieval returns every entry, newest first, with condition/actor/timestamp.
- AC-17-161: A collection with no rule ever defined returns an empty list, not an error.
- AC-17-162: Any authenticated project member (Viewer included) can retrieve history.
- AC-17-163: A request without a valid admin session is rejected 401.

## Dependencies
Depends on Slice 01 (real captured history rows to retrieve).

## Effort Estimate
0.75 day.

## Reference Class
Mirrors `simulate_access_rule`'s existing any-role, read-only handler shape exactly — a new query, not a new authorization pattern.
