# Slice 04: An Import (or a New Pattern) That Would Introduce Structural Overlap Is Rejected

**Story**: US-04 | **Release**: 1 | **Walking Skeleton**: No | **Estimate**: 1.5 days

## Goal
Detect, at import time, whether a new pattern would structurally overlap ANY other pattern — either within the same file, or against an already-stored pattern from a prior import — and reject the whole import if so, naming both colliding patterns.

## IN Scope
- Intra-file overlap detection (two patterns in the same import that could match the same concrete path shape).
- Cross-import overlap detection (a new pattern vs. every already-stored pattern for the project).
- Reuse the SAME structural-matching function Slice 02's routing mechanism uses (Resolution 1's own locked requirement — one shared implementation, never two).
- Distinguishable rejection reason (`PATTERN_OVERLAP` or DESIGN's own equivalent naming) naming both colliding patterns.
- Patterns with different leaf collection names never overlap, regardless of shared wildcard positions earlier in the path.
- Rejected import leaves every existing pattern/rule completely unchanged.

## OUT Scope
- Real-Firestore OR-composition semantics (Resolution 1, Option B — explicitly rejected as this feature's target).
- Recursive-wildcard-specific rejection (already handled, `RECURSIVE_WILDCARD`, unchanged taxonomy from Slice 01/4a).

## Learning Hypothesis
Disproves: an import (or a new pattern against already-stored patterns) containing structurally-overlapping patterns cannot be rejected, naming the specific colliding patterns, without either an undefined-precedence hazard or an unhelpfully generic rejection.
Confirms (if it succeeds): the same matching function that powers request-time routing (Slice 02) can be reused, unmodified, for import-time overlap detection — closing the "two independently-maintained matching implementations drift" risk named in § Journey's Shared Artifact table.

## Acceptance Criteria
AC-17-218 through AC-17-223 (see `feature-delta.md` § User Stories, US-04).

## Dependencies
Slice 01 (produces patterns to check), Slice 02 (the matching function this slice reuses).

## Effort Estimate
1.5 days.

## Reference Class
Mirrors 4a's own US-04 (all-or-nothing rejection, every offending block named) and `security-rules`'s own Resolution 2/3 "no blend of old and new" discipline, applied at the pattern-overlap layer.

## Pre-Slice SPIKE
Not required — depends on Slice 02's own matching function existing first; no independent technical uncertainty beyond that dependency.
