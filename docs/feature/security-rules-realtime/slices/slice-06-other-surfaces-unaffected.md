# Slice 06: GetDocument, Writes, and RunQuery Remain Unaffected, and an Unruled Collection's Content Stays Unrestricted

**Story**: US-06 | **Release**: 1 | **Walking Skeleton**: Yes | **Estimate**: 1.5 days

## Goal
Prove this feature's own new BC-3-internal mechanism and BC-4 call sites are structurally isolated from `GetDocument`/writes/`RunQuery` (group and non-group), and prove an unruled collection's Listen CONTENT remains fully unrestricted — only its cross-collection SCOPE (Slice 01) newly correct, never its content newly gated.

## IN Scope
- Direct proof, against a collection carrying an active rule AND an active Listen subscription simultaneously, that `GetDocument`/writes/`RunQuery` (group and non-group) behave exactly as the 4 prior epics left them.
- Direct proof that an unruled collection's Listen subscription delivers every document, initial snapshot and every live event, exactly as before this feature shipped — content-wise.

## OUT Scope
- Any new production code — this slice is a proof obligation over Slices 01–05's real behavior, not new logic.

## Learning Hypothesis
**Disproves if it fails**: This feature's own enforcement mechanism cannot be proven structurally independent of GetDocument/writes/RunQuery, and cannot be proven to leave an unruled collection's CONTENT fully unrestricted, without exercising a collection carrying both an active rule and an active Listen subscription simultaneously.
**Confirms if it succeeds**: Zero code changes to any of the five other handlers; the collection-scoping fix (Slice 01) is genuinely orthogonal to content restriction.

## Acceptance Criteria
- AC-17-126: `GetDocument`'s rule-lookup behavior is byte-for-byte unmodified.
- AC-17-127: Write-path's rule-lookup behavior is byte-for-byte unmodified.
- AC-17-128: `RunQuery`'s rule-lookup behavior (both `access_rules` and `group_access_rules`) is byte-for-byte unmodified.
- AC-17-129: An unruled collection's Listen subscription content remains fully unrestricted.

## Production-Data Taste Test
Real `journal_entries` with an active rule, real simultaneous GetDocument/write/RunQuery/Listen traffic against it.

## Dependencies
Slices 01–05 (proves their combined real behavior).

## Reference Class
Mirrors every prior epic's own AC-17-9x-style independence discipline, now proven against a collection carrying an active rule AND an active Listen subscription simultaneously — the strongest possible contrast.
