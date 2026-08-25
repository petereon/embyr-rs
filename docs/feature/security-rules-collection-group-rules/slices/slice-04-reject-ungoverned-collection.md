# Slice 04: A Collection-Group Query Against an Ungoverned Collection ID Is Rejected Outright

**Story**: US-04 | **Release**: 1 | **Walking Skeleton**: Yes | **Estimate**: 1.5 days

## Goal
A `RunQuery` with `all_descendants = true` against a collection id with NO group rule defined is rejected outright, before execution — regardless of whether a same-named exact-path rule exists anywhere for that collection id. This is the feature's single highest-consequence design risk.

## IN Scope
- `get_group_access_rule` returning `None` → reject outright (new default, distinct from `security-rules`' own "no rule ⇒ unrestricted" default for the exact-path/non-group case).
- Distinguishable rejection reason (e.g. `GROUP_RULE_NOT_DEFINED`) from Slice 03's "unsatisfied conjunct"/"unsupported rule shape" rejections.
- Structural proof: the rejection decision is reached via a single indexed lookup against `group_access_rules` alone — no query against `access_rules` is issued.
- Domain proof against BOTH: a collection id with a same-named exact-path rule but no group rule (`journal_entries`), and a collection id with no rule of any kind (`app_config`).

## OUT Scope
- Any fallback to the exact-path rule (explicitly rejected, Resolution 1/2).
- Any conditional logic checking whether an exact-path rule exists before deciding to reject (would require a scan, explicitly rejected on both correctness and cost grounds).

## Learning Hypothesis
Disproves: "a collection-group query against an ungoverned collection id cannot be safely rejected outright, independent of any same-named exact-path rule, without either an unsafe fallback or an expensive scan."
Confirms (if it succeeds): a single indexed lookup against the new table alone is sufficient and correct — no cross-table check needed.

## Acceptance Criteria
- AC-17-89: ungoverned collection id → rejected outright, before Postgres, regardless of exact-path rule existence.
- AC-17-90: confirmed distinct from — does not reopen — `security-rules`'s Resolution 2.
- AC-17-91: decided via single indexed lookup only, no `access_rules` scan.
- AC-17-92: rejection distinguishable from Slice 03's rejections.

## Dependencies
- Slice 01 (table must exist to return `None` from).

## Effort Estimate
1.5 days — the single highest-consequence slice; extra time budgeted for mutation-testing coverage of the reject-default arm (designated per-feature mutation surface, CLAUDE.md).

## Pre-Slice SPIKE
None — mechanism and default are fully locked in DISCUSS (Resolution 2); no open design question remains for DELIVER to resolve independently, only implementation and test-writing.
