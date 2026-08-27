# Slice 02: Maria Signs Up For A Trailmark Account Directly With Embyr

**Story**: US-02 | **Release**: 1 (Walking Skeleton) | **job_id**: JOB-18

## Goal

A first-time end user can create an embyr-hosted account with email + password and immediately have a verified session, resolving to the identical `VerifiedEndUserIdentity` type the custom-token path already produces.

## IN Scope

- Signup handler: email + password → account created, Argon2id-hashed (identical parameters to `admin/handlers/auth.rs::signin`, `memory=65536KiB, iter=3, par=4`).
- Immediate session establishment on successful signup (mirrors real Firebase's own `createUserWithEmailAndPassword()` behavior).
- Token minted via Slice 01's disjoint, embyr-owned signing key, verified through the EXISTING `verify_client_identity_token()` unchanged.
- Rejection for: already-registered email (explicit reveal, matches real Firebase signup behavior), password below strength requirement, hosted identity not enabled for the project, missing fields.
- Empirical spike: confirm whether the Firebase JS SDK's `createUserWithEmailAndPassword()` can be pointed at a non-Google backend the same opacity-permissive way `signInWithCustomToken()` was (mirrors `OQ-CA-01`; see `OQ-CHI-01` in feature-delta.md § Handoff Package).

## OUT of Scope

- Email verification.
- Sign-in for returning users (Slice 03).
- Password reset (Slice 04, Release 2).

## Learning Hypothesis

**Disproves**: (a) a hosted signup flow cannot mint a token verified through the EXISTING `verify_client_identity_token()` unchanged without either reusing the customer's own registered credential (a security violation) or requiring changes to the pure verification function itself; (b) `createUserWithEmailAndPassword()` may not be pointable at a non-Google backend the same way `signInWithCustomToken()` was.

**Confirms if it succeeds**: the disjoint-credential-entity design (Resolution 3) achieves full reuse of the existing verification function and request-composition path with zero changes to either, and the SDK's Auth surface is as opaque/redirectable as its Firestore/`signInWithCustomToken` surface already proved to be.

## Acceptance Criteria

- [ ] AC-18-05: Valid signup creates the account, establishes a verified session immediately, subsequent Firestore call succeeds carrying identity.
- [ ] AC-18-06: Already-registered email rejected, reason named explicitly.
- [ ] AC-18-07: Password below strength requirement rejected, requirement named.
- [ ] AC-18-08: Signup on a project without hosted identity enabled rejected, distinguishable from credential-validation failure.
- [ ] AC-18-09: Raw plaintext password never appears in any log line or response body.

## Dependencies

Depends on Slice 01 (needs an enabled project + a signing key to mint against).

## Effort Estimate

2 days (includes the SDK-wire-format spike).

## Reference Class

Mirrors `client-auth`'s own US-02 (sign-in via custom token) for the "carry identity onto subsequent calls" guarantee; mirrors `admin/handlers/auth.rs::signin`'s Argon2id parameters directly.

## Pre-Slice SPIKE

**Required**: empirically confirm the Firebase JS SDK's `createUserWithEmailAndPassword()` wire behavior when pointed at a non-Google backend (host override), mirroring `OQ-CA-01`'s own class of uncertainty. Not blocking DESIGN's logical contract, but should run before DELIVER commits to a specific endpoint shape.
