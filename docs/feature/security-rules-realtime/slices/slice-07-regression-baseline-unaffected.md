# Slice 07: Untouched Collections and the Full Regression Baseline Are Unaffected

**Story**: US-07 | **Release**: 1 | **Walking Skeleton**: Yes | **Estimate**: 1.5 days

## Goal
Prove this feature's own new mechanism is additive to the 133-scenario FINALIZED baseline plus the 3 non-finalized-but-DESIGN-stable priors' own delivered scenarios, and to existing Listen-adjacent test fixtures (resume-token delivery, keepalive, RESET) — none of which is rewritten by this feature.

## IN Scope
- Full re-run of the 133-scenario FINALIZED baseline against a build including this feature.
- Full re-run of `security-rules-write-path`/`security-rules-query-path`/`security-rules-collection-group-rules`'s own delivered acceptance scenarios.
- Re-run of existing resume-token/keepalive/RESET Listen mechanics fixtures.
- A project with no rules anywhere: every Listen subscription's content stays fully unrestricted, every subscription's scope stays correctly limited (Slice 01), consistently.

## OUT Scope
- Any new production code — this slice is a proof obligation, mirroring every prior epic's own regression-baseline slice.

## Learning Hypothesis
**Disproves if it fails**: This feature's own new mechanism cannot be proven additive to the existing regression baseline without actually re-running it unmodified.
**Confirms if it succeeds**: Zero existing scenario exercises multi-collection Listen fan-out or Listen-with-a-defined-rule, so none is affected by this feature's own new code.

## Acceptance Criteria
- AC-17-130: The 133-scenario FINALIZED regression baseline passes unmodified.
- AC-17-131: The 3 non-finalized-but-DESIGN-stable priors' own delivered acceptance scenarios pass unmodified.
- AC-17-132: A project with no rules anywhere retains fully unrestricted Listen content while gaining correctly-scoped delivery, consistently.
- AC-17-133: Existing resume-token/keepalive/RESET Listen mechanics are unaffected.

## Production-Data Taste Test
Real full regression suite, real multi-collection project state including collections with no rule and no active Listen subscription.

## Dependencies
Slices 01–06 (proves their combined real behavior is additive, not disruptive).

## Reference Class
Mirrors every prior epic's own final Walking-Skeleton regression-proof slice — the single highest-consequence regression risk, sequenced last as a proof over all prior slices' real behavior.
