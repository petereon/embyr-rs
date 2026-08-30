# Slice 02: Maria Gets a Real Identity Instantly With No Prior Credential

**Story**: US-02 | **Release**: 1 (Walking Skeleton) | **job_id**: JOB-20

## Goal

A first-time Trailmark visitor obtains a real, immediately-usable `VerifiedEndUserIdentity` with zero prior credential, subject to the exact same access-control guarantees any other identity mechanism already provides.

## IN Scope

- Driving-port handler for anonymous sign-in, recommended (Resolution 4, moderate confidence) to aim for `accounts:signUp`-compatible wire shape while remaining structurally separate from hosted-identity's own `sign_up.rs` handler and enablement gate.
- Mint a fresh `end_user_id` (UUID) and a token via the unchanged `mint_client_identity_token()`, using Slice 01's signing key — no persisted Customer DB row (Resolution 3, moderate-high confidence, stateless).
- Widen `attach_client_identity_if_present`'s existing credential-source fallback chain (`client_identity_credentials` → `hosted_identity_signing_keys` → `oauth_signing_keys`) with a fourth source, mirroring the identical pattern already applied twice.
- Reject sign-in when anonymous auth is not enabled for the project, distinguishably from an invalid-`api_key` rejection.
- Reject sign-in without a valid project `api_key` (recommended minimal abuse-mitigation/consistency bar, DESIGN's call whether structural or conventional).
- Two separate sign-in calls always mint two distinct, non-colliding identities.
- An anonymous identity is denied by an ownership rule exactly as any other non-owning identity would be — no implicit trust.

## OUT of Scope

- Any refresh-token mechanism or long-lived session survival beyond the minted token's own TTL — genuinely unresolved, feature-delta.md § Handoff Package Escalation 1. This slice's own AC-20-10 names the resulting v1 limitation honestly (re-sign-in after expiry mints a NEW identity) rather than silently building a workaround.
- Rate-limiting anonymous sign-in abuse beyond the existing generic per-project request-rate limit — feature-delta.md § System Constraints, explicitly DESIGN's call.
- Linking/upgrading the resulting identity to any other credential.
- Any `sign_in_provider`-style claim distinguishing an anonymous identity inside a security rule.

## Learning Hypothesis

**Disproves**: anonymous sign-in cannot mint a token verified through the EXISTING, unchanged `verify_client_identity_token()`/`attach_client_identity_if_present` fallback chain without colliding with an existing signing-key source or requiring changes to the pure verification function itself — OR `signInAnonymously()` cannot be pointed at a non-Google backend the way `signInWithCustomToken()`/`createUserWithEmailAndPassword()` already were.

**Confirms if it succeeds**: this codebase's own credential-source-fallback-chain pattern (ADR-026/036/037) generalizes cleanly to a fourth source with zero changes to the shared verification primitive, and a fully stateless (no Customer DB row) identity-minting flow is a legitimate, lower-cost instance of the same `VerifiedEndUserIdentity` contract every prior mechanism already honors.

## Acceptance Criteria

- [ ] AC-20-05: Valid anonymous sign-in (project has anonymous auth enabled, valid `api_key`) returns an immediate, real end-user identity; a subsequent Firestore call from that session succeeds carrying it.
- [ ] AC-20-06: Anonymous sign-in on a project without anonymous auth enabled is rejected, with a reason distinguishable from an invalid-`api_key` failure.
- [ ] AC-20-07: Anonymous sign-in without a valid project `api_key` is rejected, before any identity is minted.
- [ ] AC-20-08: Two separate anonymous sign-in calls on the same project always mint two distinct end-user identities — never a collision, never a shared identity.
- [ ] AC-20-09: An anonymous identity is subject to the exact same per-document access-control evaluation as any other verified identity — an ownership rule that denies a different uid denies an anonymous uid identically.
- [ ] AC-20-10: A new `signInAnonymously()` call made after a prior anonymous session's token has expired produces a new, different end-user identity — named v1 limitation, not silently glossed over.

## Dependencies

- Slice 01 (this feature) — the signing key this slice mints against.
- `attach_client_identity_if_present`, `verify_client_identity_token()`, `VerifiedEndUserIdentity` — shipped, unchanged.

## Effort Estimate

2 days (includes the SDK-wire-format spike, `OQ-AS-01`).

## Reference Class

Mirrors `client-auth-hosted-identity`'s own US-02 (Maria signs up) for the driving-port/response-shape convention, but structurally closer to `oauth-providers`' own sign-in handler for the storage profile — no Customer DB adapter resolution, no account row, mint-and-verify only.

## Pre-Slice SPIKE

Required, deferred to pre-DELIVER (mirrors `OQ-CA-01`/`OQ-CHI-01`'s own established precedent, not a blocker to this slice's own logical contract): confirm whether the real Firebase JS SDK's `signInAnonymously()`, when pointed at a non-Google backend, POSTs to a URL embyr controls the shape of, or a fixed Identity-Toolkit-specific path (`accounts:signUp` with `email`/`password` omitted) it must replicate exactly (`OQ-AS-01`).
