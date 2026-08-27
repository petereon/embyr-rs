# Slice 03: Alex Restores a Rule to a Prior State, and the Restore Is Itself Remembered

**Story**: US-03 | **Release**: 1 | **Estimate**: 0.5 day

## Goal
Alex can redefine a rule using a prior history entry's exact condition text, restoring its previous evaluated behavior — and that restoration is itself captured as a new history entry, via the SAME mechanism Slice 01 already established.

## IN Scope
- Proof that "restore" requires zero new production mechanism: calling the existing define action with a previously-seen condition value both restores behavior AND (via Slice 01's own capture, unmodified) produces a new history entry.
- Correct behavior when the "restored" condition is identical to the currently-active one (still produces a new entry, no special-cased no-op).
- Correct rejection of an invalid restore condition via the EXISTING `SYNTAX_ERROR`/`UNSUPPORTED_CONSTRUCT` taxonomy — no new error class.

## OUT Scope
- A dedicated "restore by history-entry-id" convenience endpoint (DESIGN's call, not locked here).
- `write_access_rules`/`group_access_rules` (Slices 04/05).

## Learning Hypothesis
**Disproves if it fails**: restoring a rule to a prior state cannot reuse the exact existing define action plus Slice 01's own capture mechanism without a new, dedicated "revert" storage path or special case — Slice 01's mechanism has a gap.
**Confirms if it succeeds**: this feature's central simplicity claim holds — restore is a USE of the existing mechanism, not a new one.

## Acceptance Criteria
- AC-17-164: Redefining with a prior history entry's exact text restores identical evaluated behavior.
- AC-17-165: The restore is captured as a new history entry via the same mechanism, no special case.
- AC-17-166: Restoring an already-current condition still produces a new entry.
- AC-17-167: An invalid restore condition uses the existing validation taxonomy, unchanged.

## Dependencies
Depends on Slice 01 (capture mechanism) and Slice 02 (a way to retrieve the prior condition text to restore).

## Effort Estimate
0.5 day — primarily a proof obligation, not new logic.

## Reference Class
Mirrors `security-rules`' own US-04 ("proof, not new logic") and `custom-claims`' own US-03 (identical discipline, applied to storage/capture instead of grammar/evaluation).
