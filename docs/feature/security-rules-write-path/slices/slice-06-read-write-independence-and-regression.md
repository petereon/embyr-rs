# Slice 06: Untouched Collections and the Read/Write Independence Guardrail Hold

**Story**: US-06 | **Release**: 1 | **Walking Skeleton**: Yes | **Estimate**: 1.5 days

## Goal
Prove this feature's single highest-consequence claim: a collection's existing `security-rules` read rule has zero effect on its writes unless Alex explicitly defines a separate write rule for that same collection — plus the standard untouched-collection and full-regression-suite guarantees.

## IN Scope
- Structural proof (preferred) or, at minimum, comprehensive test proof that a collection with a read rule but no write rule (e.g. `trail_guides`) has fully unrestricted writes, identical to pre-feature behavior.
- Structural proof that a collection with neither rule (e.g. `app_config`) is completely unaffected by this feature.
- Structural proof that a write rule on one collection has zero observable effect on a sibling collection's writes.
- Re-run the full pre-existing regression suite (113 `embyr-rs`/`client-auth` + 20 `security-rules` acceptance = 133 scenarios) unmodified and confirm 0 regressions.

## OUT Scope
- Any new production logic — this slice is primarily a proof obligation over Slices 01–05's real behavior, mirroring `security-rules`' own US-04/Slice 04 discipline exactly.

## Learning Hypothesis
Disproves: a collection's existing read rule cannot be proven to have zero effect on that same collection's writes, and untouched collections/the full prior regression suite cannot be proven unaffected, without actually re-running all 133 pre-existing scenarios unmodified.
Confirms (if it passes): the write-condition lookup and read-condition lookup are genuinely independent code paths with no shared mutable state, and this feature is purely additive to every scenario that predates it.

## Acceptance Criteria
- AC-17-42, AC-17-43, AC-17-44, AC-17-45.

## Dependencies
- Slices 01–05 (this slice proves properties OVER their real, already-implemented behavior).

## Production-Data Taste Test
Real full regression suite (133 scenarios), real multi-collection `trailmark-prod` project state with mixed read-only/write-only/both/neither rule configurations.

## Reference Class
Direct precedent: `security-rules`' own US-04/Slice 04 (AC-17-14/15/16) — same "structural guardrail, not merely tested" discipline, extended to cover the NEW read/write independence dimension this feature introduces.
