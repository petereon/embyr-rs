# Slice 06: Untouched Collections and the Full Regression Baseline Are Unaffected

**Story**: US-06 | **Release**: 1 | **Walking Skeleton**: Yes | **Estimate**: 1 day

## Goal
Prove the 133-scenario FINALIZED regression baseline (`security-rules`) plus `security-rules-query-path`'s own delivered scenarios pass unmodified, and that group-query rejection is consistent across every ungoverned collection id in a project.

## IN Scope
- Re-run of the full pre-existing regression suite, unmodified, against a build including this feature.
- Proof that `app_config` (no rule of any kind) retains fully unrestricted GetDocument/write/non-group-query behavior — only its group-query behavior is newly gated (a new capability being gated, not a regression).
- Proof of consistency: multiple ungoverned collection ids in the same project all reject group queries identically, no exemptions.

## OUT Scope
- Any production code change — pure regression/consistency proof.

## Learning Hypothesis
Disproves: "this feature's new group-query enforcement cannot be proven additive to the FINALIZED baseline without actually re-running it unmodified."
Confirms (if it succeeds): the feature is purely additive — zero pre-existing test exercises `all_descendants = true`, so zero regression risk exists against the confirmed-existing baseline.

## Acceptance Criteria
- AC-17-97: 133-scenario FINALIZED baseline passes unmodified.
- AC-17-98: `security-rules-query-path`'s own delivered scenarios pass unmodified.
- AC-17-99: a collection with no rule of any kind retains unrestricted GetDocument/write/non-group-query behavior.
- AC-17-100: group-query rejection is consistent across every ungoverned collection id, no exemptions.

## Dependencies
- Slices 01–05 (all real behavior this slice proves over).

## Effort Estimate
1 day. Reference class: `security-rules-query-path` Slice 06 (AC-17-69..72), same discipline, same effort.

## Pre-Slice SPIKE
None.
