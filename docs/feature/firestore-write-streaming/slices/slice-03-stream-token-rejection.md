# Slice 03: Stale or Mismatched stream_token Is Safely Rejected

## Goal
Ensure a `WriteRequest` presenting a `stream_token` other than the most recently issued one for that `stream_id` is rejected deterministically, before any write is attempted — closing the one genuinely open question (`feature-delta.md` Resolution 3) with an explicit, testable, DESIGN-confirmed behavior rather than leaving it undefined.

## IN Scope
- Compare the presented `stream_token` against the most-recently-issued one for that `stream_id` at the top of the per-message loop, before any write translation/apply begins.
- Reject on mismatch — the exact rejection shape (per-message rejection with stream staying open, vs. whole-stream termination requiring reconnect) is **DESIGN's own decision**, escalated in `feature-delta.md` § Handoff Package, Escalation 1. This slice's own AC describe only the observable outcome (rejected, no write applied), not the mechanism.
- Accept the correct, freshly-issued token normally (regression guard against over-rejecting).

## OUT Scope
- The underlying mismatch-detection/storage mechanism's own architecture (in-memory per-stream state vs. something else) — DESIGN's call, not prescribed here.
- Any change to `stream_id`/`stream_token` generation itself (Slice 01's own scope, unchanged).
- Agent-mode — deferred, `feature-delta.md` § Out of Scope.

## Learning Hypothesis
**Disproves if it fails**: a stream cannot safely reject a stale/mismatched `stream_token` without either (a) silently accepting a write against stale session state, or (b) requiring machinery beyond a simple compare-and-reject at the top of the per-message loop (e.g., cross-stream coordination, persistent storage).
**Confirms if it succeeds**: `stream_token` continuity is a purely in-session, in-memory concern — no new storage or cross-request coordination needed, keeping the RPC's own state fully scoped to its own spawned task's lifetime.

## Acceptance Criteria
- [ ] AC-03-01: A `WriteRequest` presenting a `stream_token` that does not match the most recently issued one for that `stream_id` is rejected before any write is attempted.
- [ ] AC-03-02: A `WriteRequest` presenting the correct, most-recently-issued `stream_token` is accepted and applied normally.
- [ ] AC-03-03: Rejection of a mismatched token never applies any part of that request's own write batch.

## Dependencies
- Slice 01 (this feature) — the receive-and-reply loop and `stream_token` rotation must exist first.
- **DESIGN's own resolution of the rejection mechanism** (`feature-delta.md` § Handoff Package, Escalation 1) — this slice is sequenced last (§ Prioritization) specifically so this answer is available before implementation.

## Effort Estimate
1 day, contingent on DESIGN's own mechanism decision landing first (no re-estimation expected regardless of which shape is chosen — both are small, localized changes to the per-message loop's own entry point).

## Pre-Slice SPIKE
Recommended once DESIGN's mechanism decision lands, only if the chosen shape introduces cross-request state this codebase has no existing precedent for; not needed if the decision confirms a simple in-task comparison.
