# Slice 03: Maria Signs In With Her Hosted Email/Password Credential

**Story**: US-03 | **Release**: 1 (Walking Skeleton) | **job_id**: JOB-18

## Goal

A returning end user can sign in with her hosted email + password credential and have her session carry her verified identity onto subsequent Firestore calls — the same guarantee signup already established — with zero account-enumeration leak on failure.

## IN Scope

- Signin handler: email + password → verified session, reusing Slice 02's account storage and Slice 01's signing key.
- Oracle-protected rejection: wrong password and unknown email return an IDENTICAL response shape, directly reusing `admin/handlers/auth.rs::invalid_credentials()`'s own already-accepted convention.
- Rejection for hosted identity not enabled on the project (distinguishable — this is project config, not a secret about a specific email).
- Regression guardrail: an ordinary Firestore call from a session that never signed in via ANY path (hosted or custom-token) continues to succeed unaffected — extends `client-auth`'s own AC-16-08.

## OUT of Scope

- Password reset (Slice 04).
- Account lockout / brute-force throttling policy (no evidence-based mechanism locked here; DESIGN may evaluate the admin console's own 3-failed-attempts lockout pattern as a candidate, not mandated).

## Learning Hypothesis

**Disproves**: hosted sign-in cannot share the identical oracle-protected rejection response shape the admin console's own password path already established, without either duplicating that logic or an awkward cross-module dependency.

**Confirms if it succeeds**: the oracle-protection pattern generalizes cleanly to a second, structurally similar password-checking call site via a shared response constructor (or equivalent), with zero drift risk between the two.

## Acceptance Criteria

- [ ] AC-18-10: Valid email+password sign-in succeeds; identity attaches to subsequent Firestore calls.
- [ ] AC-18-11: Wrong password and unknown email rejected with an identical response shape.
- [ ] AC-18-12: Sign-in on a project without hosted identity enabled rejected, distinguishable from credential mismatch.
- [ ] AC-18-13: A Firestore data call from a session that never signed in via any path continues to succeed unaffected (regression guardrail).

## Dependencies

Depends on Slice 02 (account must exist to sign in against).

## Effort Estimate

1 day.

## Reference Class

Mirrors `client-auth`'s own US-02 (sign-in via custom token) for the "carry identity forward" and "unsigned-in session unaffected" guardrail shape; mirrors `admin/handlers/auth.rs::invalid_credentials()` directly for the oracle-protection response.

## Pre-Slice SPIKE

Not required independently — covered by Slice 02's spike (`signInWithEmailAndPassword()` is the same wire-format-uncertainty class as `createUserWithEmailAndPassword()`, confirmed together).
