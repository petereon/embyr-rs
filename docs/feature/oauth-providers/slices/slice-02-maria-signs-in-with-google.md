# Slice 02: Maria Signs In With Her Google Account

**Story**: US-02 | **Release**: 1 (Walking Skeleton) | **job_id**: JOB-19

## Goal

An end user who already has a Google account can sign in through embyr using her existing Google credential, with her session resolving to the identical `VerifiedEndUserIdentity` type every other identity mechanism already produces, and repeat sign-ins resolving to the same identity.

## IN Scope

- Verification of a Google-issued ID token: RS256 signature against Google's own live JWKS, `iss`/`aud`/`exp` checks, `aud` matched against the project's Slice-01-registered Client ID.
- Minting a `VerifiedEndUserIdentity` via the already-shipped `mint_client_identity_token()` (`client-auth-hosted-identity`), using an `end_user_id` derived deterministically from `(provider, sub)` — no Customer DB write (§ Job Discovery Framing Resolution, Resolution 3's own stateless lean).
- Distinguishable rejection reasons: not-enabled, audience mismatch, expired, malformed/invalid signature, JWKS-unreachable.
- Regression guardrail: an ordinary Firestore call from a session that never signed in via any path is unaffected.

## OUT of Scope

- GitHub or any other non-OIDC provider (§ Job Discovery Framing Resolution, Resolution 1).
- Persisted admin visibility into signed-in Google accounts (§ Out of Scope).
- `backend_mode=agent` gating — **RESOLVED (Resolution 2, confirmed by the orchestrator 2026-08-30, re-confirmed by DESIGN/ADR-037 § Context)**: no gating needed. Google sign-in is available to every `backend_mode`. No longer an open escalation.

## Learning Hypothesis

**Disproves**: a Google sign-in flow cannot verify a real Google-issued ID token against Google's own live JWKS and mint a `VerifiedEndUserIdentity` via the EXISTING `mint_client_identity_token()` unchanged, without requiring changes to either that function or the pure verification taxonomy; separately, `signInWithPopup()`/`GoogleAuthProvider` may not be pointable at a non-Google backend for the resulting token-presentation call the same opacity-permissive way `signInWithCustomToken()` was.

**Confirms if it succeeds**: this codebase's existing "verify a JWT, mint via an embyr-owned signing key" shape (already proven twice — `client_identity`'s own verify path, `client-auth-hosted-identity`'s own mint path) generalizes cleanly to a THIRD-PARTY-issued, RS256-signed, JWKS-published token, with zero changes to either existing primitive.

## Acceptance Criteria

- [ ] AC-19-05: A valid, unexpired Google ID token issued for the project's registered Client ID succeeds; the verified end-user identity attaches to subsequent Firestore calls.
- [ ] AC-19-06: The same Google account signing in on a later visit resolves to the identical `end_user_id` as its first sign-in.
- [ ] AC-19-07: An ID token whose audience does not match the registered Client ID is rejected, naming the mismatch, distinguishable from a not-enabled rejection.
- [ ] AC-19-08: Sign-in on a project with no Google Client ID registered is rejected, distinguishable from a token-validation failure.
- [ ] AC-19-09: An expired ID token is rejected, distinguishable from a signature or audience failure.
- [ ] AC-19-10: A Firestore data call from a session that never signed in via any path continues to succeed exactly as before this feature shipped.
- [ ] AC-19-11: Google's JWKS being temporarily unreachable causes sign-in to fail gracefully with a distinguishable, retryable-sounding reason — never a crash, never a silently-accepted unverified token.

## Dependencies

Depends on Slice 01 (needs a registered Client ID to validate `aud` against). Depends on `mint_client_identity_token()` (already shipped by `client-auth-hosted-identity`, DELIVER-complete this session).

## Effort Estimate

2 days (includes the SDK-wire-format spike, `OQ-OAP-01`).

## Reference Class

Mirrors `client-auth`'s own US-02 sign-in-action shape (stateless, token-in-body-is-the-credential) more closely than `client-auth-hosted-identity`'s own equivalent story — no `?key=` query param is structurally required here under Resolution 3's stateless design, since no Customer DB adapter needs resolving.

## Pre-Slice SPIKE

**Required**: `OQ-OAP-01` — confirm whether `signInWithPopup()`/`signInWithRedirect()` with `GoogleAuthProvider`, pointed at a non-Google backend, lets embyr control the shape of the token-presentation call or insists on a fixed Identity-Toolkit-specific `accounts:signInWithIdp` endpoint shape. Mirrors `OQ-CA-01`/`OQ-CHI-01`'s own precedent — not blocking DESIGN's logical contract, but required before DELIVER can finalize the transport.
