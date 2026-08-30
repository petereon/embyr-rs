# Slice 01: Alex Registers His Google OAuth Client ID For Trailmark

**Story**: US-01 | **Release**: 1 (Walking Skeleton) | **job_id**: JOB-19

## Goal

A project owner can register his own public Google OAuth Client ID for a project via an explicit admin action, activating Google sign-in for that project, with idempotent redefine on re-registration.

## IN Scope

- Admin action to register a Google OAuth Client ID for a project (exact endpoint shape DESIGN's call).
- Storage of the registered Client ID, structurally disjoint from `client_identity_credentials` (ADR-024/025) for the identical non-impersonation reason — hard constraint, see feature-delta.md § System Constraints.
- Idempotent redefine (a second registration for an already-registered project replaces the Client ID, not an error) — mirrors `AccessRule`'s own define/redefine lifecycle precedent (ADR-036's own cited reference).
- Standard admin-endpoint auth (missing/invalid Bearer → 401) and project-existence checks (404).

## OUT of Scope

- Deregistering Google sign-in (deferred, § Out of Scope).
- Any sign-in behavior (Slice 02).
- GitHub or any other provider (§ Job Discovery Framing Resolution, Resolution 1).
- Bounded-context placement and exact storage location — this DISCUSS's own evidenced lean is System DB extending BC-1 (Resolution 5), not locked, flagged for DESIGN.

## Learning Hypothesis

**Disproves**: a "register Google OAuth Client ID" admin action cannot store a project-scoped, public (non-confidential) Client ID using the existing admin-API conventions without inventing a new pattern, or cannot support idempotent redefine without a separate rotate-style endpoint.

**Confirms if it succeeds**: the existing admin-API conventions (session auth, project-ownership check, JSON error body) extend cleanly to a public, no-confidentiality-property credential class, and idempotent-upsert is a simpler lifecycle than the register-then-separately-rotate shape `client_identity_credentials` needed for its own (also-public, but customer-registered-once, embyr-verified-many-times) material.

## Acceptance Criteria

- [ ] AC-19-01: Valid first-time registration returns 201; Google sign-in becomes active for the project using the submitted Client ID.
- [ ] AC-19-02: Re-registration with a different Client ID for an already-registered project succeeds (200), and the new Client ID is the one used for subsequent `aud`-claim verification — no stale registration remains active.
- [ ] AC-19-03: Missing or invalid admin Bearer credential returns 401.
- [ ] AC-19-04: Registration for a non-existent or deleted project returns 404.

## Dependencies

None — this is the Walking Skeleton's first slice. Slice 02 depends on this one.

## Effort Estimate

1 day.

## Reference Class

Mirrors `client-auth`'s own US-01 (register verification credential) admin-action shape and `client-auth-hosted-identity`'s own US-01 enablement-action shape. **Correction (ADR-037 Decision 2, DESIGN-wave finding)**: this slice's own original framing ("no signing-key generation... embyr generates nothing here") was disproven by tracing `mint_client_identity_token()`'s signature — Slice 02 cannot mint without an embyr-owned signing key, so this slice DOES generate one (a new, disjoint `oauth_signing_keys` row, transactionally alongside the Client ID upsert), mirroring hosted identity's own US-01 in that respect after all. What remains genuinely simpler than both precedents: no rotation-window/dual-generation complexity for the Client ID itself (a single idempotent-upsert suffices, since the Client ID is not security-sensitive to overwrite), and no `api_key` field in the request body (the signing key is encrypted under `EMBYR_ENCRYPTION_KEY`, not ECIES/`api_key` — ADR-037 Decision 2's own positive consequence).

## Pre-Slice SPIKE

Not required — this slice reuses well-understood existing admin-API patterns (session auth, project-ownership check) with no new external-wire-format uncertainty. (The genuine wire-format uncertainty in this feature belongs to Slice 02, not this one.)
