# Slice 02: Verify a Session's Custom Token and Reject Invalid Ones Specifically

**Story**: US-02 | **Release**: 1 | **Walking Skeleton**: Yes | **Estimate**: 1.5 days

## Goal
A Trailmark end-user session presenting a valid, current-project, unexpired token signs in successfully and carries a verified identity onto subsequent Firestore calls; one presenting no/malformed/expired/wrong-project token is rejected with a specific, distinguishable reason. A session that never signs in at all is unaffected.

## IN Scope
- Sign-in / identity-exchange action that verifies a token against the project's registered credential (Slice 01).
- Four distinguishable rejection reasons: missing, malformed, expired, wrong-project.
- Verified identity attached to the signed-in session's subsequent Firestore calls (exposure mechanism DESIGN's call).
- **Critical regression guardrail**: an ordinary Firestore data call from a session that never signed in continues to succeed exactly as before this feature shipped.

## OUT Scope
- Making end-user identity mandatory on any data-plane call (that is Security Rules' job — a separate follow-up epic).
- Token refresh mechanics beyond natural expiry.
- Rotation-window verification (US-03 / Slice 03) — this slice assumes exactly one active credential.

## Learning Hypothesis
Disproves: a signed-in session cannot carry a verified end-user identity onto subsequent Firestore calls without either (a) gating existing data-plane calls on identity (a regression) or (b) requiring a live Security Rules engine that doesn't exist yet.
Confirms (if it succeeds): identity verification can be added as a genuinely additive layer, with zero regression to the 72 existing `embyr-rs` acceptance scenarios.

## Acceptance Criteria
- [ ] AC-16-06: Valid token → sign-in succeeds, identity attached to subsequent calls.
- [ ] AC-16-07: Missing/malformed/expired/wrong-project token → rejected, each reason distinguishable from the other three.
- [ ] AC-16-08: Data call from a never-signed-in session succeeds unchanged (regression guardrail).
- [ ] AC-16-09: Verified identity available to embyr's request handling for the signed-in session's duration.

## Dependencies
Depends on Slice 01 (needs a registered credential to verify against). Hard build-order dependency — Slice 02 cannot be meaningfully tested without Slice 01 existing first (though it can be tested against a hand-seeded credential row if needed).

## Effort Estimate
1.5 days.

## Reference Class
`handler.rs`'s `extract_api_key`/`authenticate` — precedent for "authenticate before touching data," not reused directly (additive, not a replacement).

## Pre-Slice Spike
Not needed — DESIGN owns the token-verification mechanism; this slice locks observable behavior only, which is well-understood from the journey work in `feature-delta.md`.
