# Slice 05: GetDocument, Writes, and Non-Group Queries Remain Governed Exclusively by Existing Exact-Path Rules

**Story**: US-05 | **Release**: 1 | **Walking Skeleton**: Yes | **Estimate**: 1 day

## Goal
Prove — against the strongest possible contrast (a collection id carrying BOTH an exact-path rule and a group rule with DIFFERENT conditions) — that `GetDocument`, write-path, and non-group `RunQuery` are structurally unaffected by collection-group rules.

## IN Scope
- Test-only slice (per its own Technical Note): no new production logic expected beyond what Slices 01–04 already shipped.
- Real domain proof: `journal_entries` configured with an exact-path rule (`request.auth.uid == resource.data.owner_id`) AND a group rule with a deliberately weaker condition (`request.auth != null`) simultaneously.
- Prove GetDocument, non-group RunQuery, and writes on `journal_entries` and on `expeditions/trek-2026/journal_entries` (write-path) all behave exactly per their own existing exact-path rule lookups, unaffected by the group rule.

## OUT Scope
- Any production code change (this slice is a proof obligation, mirroring `security-rules-query-path`'s own US-06 discipline) — if the branch design from Slices 01–04 is structurally correct, no code should be needed here beyond test fixtures.

## Learning Hypothesis
Disproves: "GetDocument, writes, and non-group RunQuery cannot be proven structurally unaffected by a group rule's existence or content without actually exercising all three against a collection id carrying both rule types with different conditions."
Confirms (if it succeeds): the disjoint-table, disjoint-call-site-branch design (Resolution 1) is sufficient — independence is structural, not merely tested-and-hoped-for.

## Acceptance Criteria
- AC-17-93: GetDocument unmodified.
- AC-17-94: non-group RunQuery unmodified.
- AC-17-95: write-path unmodified.
- AC-17-96: group rule's existence/content has zero observable effect on any of the three, proven structurally.

## Dependencies
- Slices 01–04 (both rule types must be definable and enforceable to construct the contrast fixture).

## Effort Estimate
1 day. Reference class: `security-rules-query-path` Slice 06 / `security-rules-write-path` Slice 06 (both primarily regression-proof slices), same effort class.

## Pre-Slice SPIKE
None.
