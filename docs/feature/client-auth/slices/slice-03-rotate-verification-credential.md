# Slice 03: Rotate a Project's Verification Credential Without Breaking Signed-In Users

**Story**: US-03 | **Release**: 2 | **Walking Skeleton**: No | **Estimate**: 1.25 days

## Goal
Alex rotates Trailmark's verification credential (routine schedule or suspected compromise) without invalidating already-signed-in Trailmark users mid-session.

## IN Scope
- Rotation action, admin-Bearer-authenticated (401 on missing/invalid).
- Dual-credential verification window: tokens minted under the immediately-previous credential continue to verify during the window.
- Tokens minted under a credential older than the rotation window are rejected as no longer valid.

## OUT Scope
- Unbounded rotation history — only current + immediately-previous credential are valid, mirroring the existing D5 dual-hash-window shape for `api_key`.
- Any change to Slice 01's registration action or Slice 02's core verification routine beyond adding the dual-credential check.

## Learning Hypothesis
Disproves: a credential cannot be rotated without either downtime for currently-signed-in users or an unbounded, ever-growing set of "still valid" old credentials.
Confirms (if it succeeds): the existing D5 dual-hash-window pattern (already proven for `api_key` rotation) generalizes cleanly to this new credential type.

## Acceptance Criteria
- [ ] AC-16-10: Valid rotation activates new credential; new tokens verify.
- [ ] AC-16-11: Tokens under immediately-previous credential still verify during the window.
- [ ] AC-16-12: Tokens under a credential older than the window are rejected.
- [ ] AC-16-13: Rotation without valid admin credentials rejected 401.

## Dependencies
Depends on Slice 01 (a credential must exist to rotate) and conceptually on Slice 02 (rotation is only observable through the verification path). Not a hard build-order block beyond Slice 01.

## Effort Estimate
1.25 days.

## Reference Class
DISCUSS#D5 — Argon2id dual-hash rotation window for `api_key` (`docs/feature/embyr-rs/discuss/feature-delta.md`).

## Pre-Slice Spike
Not needed — direct reuse of an already-proven pattern in this codebase.
