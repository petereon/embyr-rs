# Slice 03: The Same Precedence-Aware Mechanism Gates Writes and Listen's Per-Event Re-Check

**Story**: US-03 | **Release**: 1 | **Walking Skeleton**: Yes | **Estimate**: 1.5 days

## Goal
Extend Slice 02's own precedence-aware composition to the 3 write handlers (`CreateDocument`/`UpdateDocument`/`DeleteDocument`) and `handle_add_target`'s 2 per-event (`Changed`/`Removed`) arms, mirroring 4a's/4b's own identical read+write+Listen parity precedent.

## IN Scope
- Thread the precedence-resolved pattern's own binding into all 3 write handlers.
- Thread the precedence-resolved pattern's own binding into Listen's 2 per-event call sites.
- A specific rule's (4a/4b) own write/live-update behavior is completely unaffected by a co-existing, structurally-overlapping recursive-wildcard catch-all.
- Zero observable side effect on a denied write or a denied live-update delivery.

## OUT Scope
- `RunQuery`'s non-group arm and Listen's own subscribe-time (initial-snapshot) compliance gate — unchanged scope boundary from 4b's own `OQ-PM-07`.
- Precedence-tie/overlap detection (Slice 04).

## Learning Hypothesis
Disproves: the identical precedence-aware routing mechanism cannot extend to the 3 write handlers and Listen's per-event re-check without a second, write/Listen-specific resolution path.
Confirms (if it succeeds): threading one additional resolution step through 6 already-wired call sites is mechanical, uniform, one-line-per-site — mirroring 4a's/4b's own identical propagation precedent.

## Acceptance Criteria
AC-17-245 through AC-17-249 (see `feature-delta.md` § User Stories, US-03).

## Dependencies
Slice 02 (the precedence-composition mechanism itself).

## Effort Estimate
1.5 days.

## Reference Class
Mirrors 4b's own Slice 03 (extending a proven read-path mechanism to write + Listen, zero new component) exactly.

## Pre-Slice SPIKE
Not required — a mechanical extension of an already-proven mechanism.
