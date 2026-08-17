# Slice 04: Standalone Pre-Production Token Verify/Debug Check

**Story**: US-04 | **Release**: 2 | **Walking Skeleton**: No | **Estimate**: 1 day

## Goal
Alex verifies, before shipping to production, that a token Trailmark's backend just minted resolves to the expected end-user identity — catching minting bugs during integration instead of in production.

## IN Scope
- Standalone verify/debug action that resolves a token's identity and expiry without creating a live signed-in session.
- Identical rejection-reason taxonomy to real sign-in (US-02): malformed / expired / wrong-project.

## OUT Scope
- Any new verification logic — this slice should wrap Slice 02's existing verification routine, not duplicate it (see `feature-delta.md` § System Constraints shared-artifact risk).
- Rate limiting or abuse protections on the debug check beyond what admin endpoints already have — not locked here.

## Learning Hypothesis
Disproves: a standalone pre-production verify/debug check cannot share the exact same rejection-reason taxonomy as real sign-in without duplicating (and risking drift in) the verification logic.
Confirms (if it succeeds): Slice 02's verification routine is cleanly reusable as a library call, not entangled with sign-in-session side effects.

## Acceptance Criteria
- [ ] AC-16-14: Valid token → resolved identity + expiry returned; no live session created.
- [ ] AC-16-15: Rejection reasons match US-02's taxonomy exactly (malformed / expired / wrong-project).

## Dependencies
Depends conceptually on Slice 02's verification routine existing to wrap. Independently testable against a stubbed verification routine if sequencing requires it, per `feature-delta.md` § Prioritization.

## Effort Estimate
1 day.

## Reference Class
Firebase Admin SDK's `admin.auth().verifyIdToken()` — a standalone verify capability distinct from a live sign-in flow; real-world precedent for this story's shape.

## Pre-Slice Spike
Not needed.
