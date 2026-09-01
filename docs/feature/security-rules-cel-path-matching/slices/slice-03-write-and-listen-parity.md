# Slice 03: The Same Routing Mechanism Gates Writes and Listen's Per-Event Re-Check

**Story**: US-03 | **Release**: 1 | **Walking Skeleton**: Yes | **Estimate**: 1 day

## Goal
Extend Slice 02's routing+binding mechanism to `CreateDocument`/`UpdateDocument`/`DeleteDocument` and to `handle_add_target`'s per-event `Changed`/`Removed` re-check — closing 4a's own deferred `OQ-CP-04` in the same pass, per Resolution 3.

## IN Scope
- Thread the routing mechanism's name-keyed binding into all 3 write handlers.
- Thread it into Listen's 2 per-event call sites (`Changed`/`Removed`), replacing 4a's own mechanical `None` with a real resolution.
- Create-vs-update-vs-delete binding parity (binding derived from the request's own target path, never from fetched content).
- Denied write/denied live-update-delivery zero-side-effect proof.

## OUT Scope
- The routing mechanism itself (built in Slice 02 — this slice only threads it to 3 more call sites).
- Listen's subscribe-time (initial-snapshot) compliance check re-verification (flagged for DESIGN, not this slice).

## Learning Hypothesis
Disproves: the identical routing+binding mechanism cannot extend to 3 write handlers and 2 Listen per-event call sites without a second, write/Listen-specific resolution path.
Confirms (if it succeeds): mirrors 4a's own Resolution 3 "mechanical, uniform, one-line-per-site" precedent, now at 6 call sites instead of 4.

## Acceptance Criteria
AC-17-213 through AC-17-217 (see `feature-delta.md` § User Stories, US-03).

## Dependencies
Slice 02 (the routing mechanism this slice threads to additional call sites).

## Effort Estimate
1 day.

## Reference Class
`custom-claims` (ADR-034) — mechanical, uniform propagation of one new resolved value across every existing call site, no new logic per site.

## Pre-Slice SPIKE
Not required — mechanical extension of an already-proven mechanism (Slice 02).
