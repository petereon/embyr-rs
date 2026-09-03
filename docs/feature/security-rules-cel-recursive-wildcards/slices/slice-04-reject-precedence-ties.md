# Slice 04: An Odd-Prefix Import, a Same-Specificity Tie, or an Unrelated-Shape Overlap Is Rejected

**Story**: US-04 | **Release**: 1 | **Estimate**: 1.5 days

## Goal
At import time, detect and reject (a) odd-prefix recursive-wildcard shapes (already rejected by Slice 01's own parser widening — this slice covers the taxonomy/naming), (b) two recursive-wildcard patterns at an identical fixed-prefix depth+skeleton (a genuine tie, no automatic precedence rule can resolve it), and (c) a recursive-wildcard pattern and a fixed-depth/exact-match pattern whose shapes are NOT in a structural containment relationship (an unrelated-shape overlap) — naming the offending pattern(s) in every case, reusing 4b's own Option C discipline at this narrower tie-breaking level.

## IN Scope
- Same-specificity tie detection: two recursive-wildcard patterns at identical prefix depth+skeleton, both intra-file and cross-import.
- Unrelated-shape overlap detection (patterns that could both apply to some concrete path but are not in a strict containment relationship) — reject, naming both.
- A pair NOT in a containment relationship (different leaf collection names, or otherwise non-overlapping reach) imports successfully — mirrors 4b's own AC-17-221.
- A rejected import leaves every existing pattern and rule completely unchanged.
- Distinguishable rejection reasons across this feature's own and 4a's/4b's own full taxonomy.

## OUT Scope
- Precedence RESOLUTION for a genuine containment relationship (that's Slice 02's own concern — this slice only handles cases where no automatic resolution rule applies).

## Learning Hypothesis
Disproves: a same-specificity tie or an unrelated-shape overlap cannot be rejected, naming the offending pattern(s), without either an undefined-precedence hazard or an unhelpfully generic rejection.

## Acceptance Criteria
AC-17-250 through AC-17-254 (see `feature-delta.md` § User Stories, US-04).

## Dependencies
Slice 01 (decomposition shape), Slice 02 (the shared `positions_compatible`-derived containment-check primitive).

## Effort Estimate
1.5 days.

## Reference Class
Mirrors 4b's own Slice 04 (`structurally_overlap`, reused for this feature's own narrower tie-detection case) — a single shared primitive, reused, not reinvented.

## Pre-Slice SPIKE
Not required.
