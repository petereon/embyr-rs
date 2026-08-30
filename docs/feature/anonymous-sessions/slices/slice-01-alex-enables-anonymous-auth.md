# Slice 01: Alex Enables Anonymous Authentication For Trailmark

**Story**: US-01 | **Release**: 1 (Walking Skeleton) | **job_id**: JOB-20

## Goal

A project owner can opt a project into anonymous end-user authentication via an explicit admin action, generating embyr's own project-scoped signing key server-side, never exposing signing material.

## IN Scope

- Admin action to enable anonymous authentication for a project (exact endpoint shape DESIGN's call).
- Server-side generation of a project-scoped signing key, recommended (feature-delta.md Resolution 2, high confidence) as a NEW table encrypted with AES-256-GCM under `EMBYR_ENCRYPTION_KEY`, mirroring `oauth_signing_keys`'s own shape — structurally disjoint from `client_identity_credentials`, `hosted_identity_signing_keys`, AND `oauth_signing_keys` alike.
- Idempotent re-enablement (second call on an already-enabled project is a no-op success, not an error).
- Standard admin-endpoint auth (missing/invalid credential → 401) and project-existence checks (404).
- No signing material of any kind in any response body or log line.

## OUT of Scope

- Disabling anonymous authentication (deferred, feature-delta.md § Out of Scope).
- Any sign-in behavior (Slice 02).
- `backend_mode=agent` gating decision — genuinely open, feature-delta.md § Handoff Package Escalation 2.
- Reusing `hosted_identity_signing_keys` — explicitly evaluated and rejected (Resolution 2), mirrors `oauth-providers`' own identical rejection.

## Learning Hypothesis

**Disproves**: an "enable anonymous auth" admin action cannot auto-generate and safely custody its own project-scoped signing key, structurally disjoint from every other signing-key table, following the AES-256-GCM/`EMBYR_ENCRYPTION_KEY` pattern `oauth_signing_keys` already established, without inventing a new pattern.

**Confirms if it succeeds**: the `oauth_signing_keys` reference shape (ADR-037's own forward-looking recommendation) generalizes cleanly to a second, unrelated "embyr mints its own token with no natural `api_key`-bearing call site" mechanism — direct evidence that shape, not `hosted_identity_signing_keys`'s ECIES shape, is this codebase's actual reusable default for this class of problem.

## Acceptance Criteria

- [ ] AC-20-01: Valid enablement returns 201; anonymous authentication becomes active for the project; no signing material appears in the response.
- [ ] AC-20-02: A second enablement request for an already-enabled project succeeds idempotently — no error, no duplicate signing key.
- [ ] AC-20-03: Missing or invalid admin credential returns 401.
- [ ] AC-20-04: Enablement for a non-existent or deleted project returns 404.

## Dependencies

None — this is the Walking Skeleton's first slice. Slice 02 depends on this one.

## Effort Estimate

1 day.

## Reference Class

Mirrors `client-auth-hosted-identity`'s own US-01 (enable hosted identity) admin-action shape and idempotency contract, but the signing-key encryption mechanism follows `oauth-providers`' own `oauth_signing_keys` precedent (AES-256-GCM/`EMBYR_ENCRYPTION_KEY`), not hosted-identity's own ECIES/`api_key`-derived scheme — no natural `api_key`-bearing call site exists here to derive an ECIES pubkey from, the same condition that drove `oauth-providers` away from ECIES.

## Pre-Slice SPIKE

Not required — this slice reuses well-understood existing admin-API conventions and an already-proven encryption pattern (`oauth_signing_keys`) with no new external-wire-format uncertainty.
