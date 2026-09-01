# Slice 05: Importing Protects Only What's Named; Re-Import Is Idempotent

**Story**: US-05 | **Release**: 1 | **Estimate**: 1 day

## Goal
Prove — not just assert — that importing a file has zero observable effect on any collection not named in that file (including rules defined via the existing JSON API), and that re-importing an unchanged file is a no-op in effect.

## IN Scope
- Non-interference proof: a hand-defined `trail_guides` rule (via existing JSON API) is unaffected by an import naming other collections.
- Idempotent re-import: identical file imported twice produces one history entry, not two, and confirms unchanged state.
- Full pre-existing regression suite (133+ scenarios across all 7 prior JOB-17 epics) re-run unmodified.

## OUT Scope
- New production logic — this slice is primarily a proof obligation over Slices 01–04's real behavior, mirroring `security-rules`'s own US-04/AC-17-14/15/16 discipline.

## Learning Hypothesis
Disproves: "Importing cannot be proven not to silently affect a rule for a collection NOT named in the imported file, or to duplicate state on re-import, without actually re-running the full existing regression suite plus a same-file-twice check."

## Acceptance Criteria
AC-17-194, AC-17-195, AC-17-196, AC-17-197 (see feature-delta.md § User Stories, US-05).

## Dependencies
- Slices 01–04 (this slice proves properties of their combined real behavior).

## Effort Estimate
1 day.

## Pre-Slice SPIKE
Not required.
