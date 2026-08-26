# Slice 01: A Custom Claim Survives From Mint to Verified Identity

**Story**: US-01 | **Release**: 1 | **Walking Skeleton**: Yes | **Estimate**: 1 day

## Goal
A claim Trailmark's own backend embeds in the token at mint time (e.g. `is_moderator: true`) is present, unmodified, on the resolved `VerifiedEndUserIdentity` after `verify_client_identity_token()` succeeds.

## IN Scope
- Extend `ClientIdentityClaims` (currently `sub`/`aud`/`exp` only, `crates/embyr-core/src/client_identity/mod.rs`) to capture arbitrary extra JSON claims present in the token payload.
- Extend `VerifiedEndUserIdentity` with a `claims` field (exact representation — flatten vs. typed map vs. `serde_json::Value` — DESIGN's call).
- Preserve exact JSON type on round-trip (no coercion, e.g. a string claim value stays a string).
- A token minted with zero extra claims verifies exactly as before this feature shipped (empty claims, zero regression).

## OUT Scope
- Any rule referencing a claim (US-02) — this slice only makes claims *available*, not *usable*.
- Any change to signature verification, algorithm pinning, or rejection taxonomy (ADR-024's cryptographic path is untouched).
- Any wire-format change on Trailmark's own minting side (claims are already silently accepted today, per Finding 1 — this slice only stops discarding them).

## Learning Hypothesis
**Disproves if it fails**: that a custom claim can survive from mint to verified identity without a wire-format change or a new admin API. If this fails (e.g. `serde`'s decode step rejects unknown fields, or the JWT library truncates the payload), Resolution 1's entire "mint-time-embedded" conclusion is undermined and Option A (request-time lookup) must be reconsidered.

**Confirms if it succeeds**: the zero-wire-format-change, zero-new-I/O claim central to this feature's own Resolution 1.

## Acceptance Criteria
- AC-17-137: A custom claim embedded in the token at mint time is present on the verified identity after verification, exactly as minted.
- AC-17-138: A token minted with no claims at all verifies exactly as before this feature shipped.
- AC-17-139: The claims map is available to embyr's own request handling for at least the duration of the verified-request lifecycle.
- AC-17-140: The full pre-existing regression suite (136+ scenarios, all 6 prior epics) passes unmodified.

## Dependencies
None — foundational story, sequenced first.

## Production-Data Taste Test
Real Ed25519-signed token minted with a real extra claim (using the project's own `mint_token` test-helper pattern, `client_identity/mod.rs::tests`), real `verify_client_identity_token()` call, real assertion the claim round-trips. No synthetic exception.

## Effort Estimate
1 day. Reference class: `client-auth`'s own US-01 (credential registration) — a pure type/parsing extension with no new adapter.

## Pre-Slice SPIKE
Not needed — the exact `serde` mechanism (flatten attribute vs. explicit map) is a well-understood, low-uncertainty implementation choice, deferred to DESIGN's own call per the feature-delta's Technical Notes.
