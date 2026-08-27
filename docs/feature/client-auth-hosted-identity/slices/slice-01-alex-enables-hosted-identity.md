# Slice 01: Alex Enables Hosted Email/Password Identity For Trailmark

**Story**: US-01 | **Release**: 1 (Walking Skeleton) | **job_id**: JOB-18

## Goal

A project owner can opt a project into embyr-hosted email/password identity via an explicit admin action, generating embyr's own project-scoped signing key server-side, never exposing signing material.

## IN Scope

- Admin action to enable hosted identity for a project (exact endpoint shape DESIGN's call).
- Server-side generation of a project-scoped signing key structurally disjoint from `client_identity_credentials` (ADR-024/025) — hard constraint, see feature-delta.md § Job Discovery Framing Resolution, Resolution 3.
- Idempotent re-enablement (second call on an already-enabled project is a no-op success, not an error).
- Standard admin-endpoint auth (missing/invalid Bearer → 401) and project-existence checks (404).
- No signing material of any kind in any response body or log line.

## OUT of Scope

- Disabling hosted identity (deferred, § Out of Scope).
- Any signup/signin behavior (Slices 02/03).
- Storage location decision (System DB vs. Customer DB) — UNRESOLVED, flagged for DESIGN (Resolution 2).

## Learning Hypothesis

**Disproves**: an "enable hosted identity" admin action cannot auto-generate and safely custody its own project-scoped signing key, structurally disjoint from `client_identity_credentials`, using the existing admin-API conventions (`SessionContext` extractor, no-raw-material-in-response) without inventing a new pattern.

**Confirms if it succeeds**: the existing admin-API conventions extend cleanly to a self-service-generated (not customer-submitted) credential, and the disjoint-entity constraint can be enforced structurally (a distinct table/PK), not just by convention.

## Acceptance Criteria

- [ ] AC-18-01: Valid enablement returns 201; hosted identity becomes active; no signing material in the response.
- [ ] AC-18-02: A second enablement for an already-enabled project succeeds idempotently — no error, no duplicate key.
- [ ] AC-18-03: Missing/invalid admin Bearer credential returns 401.
- [ ] AC-18-04: Enablement for a non-existent/deleted project returns 404.

## Dependencies

None — this is the Walking Skeleton's first slice. Slices 02/03 depend on this one.

## Effort Estimate

1 day.

## Reference Class

Mirrors `client-auth`'s own US-01 (register verification credential) admin-action shape, and `sdk_keys.rs`'s project-ownership-check + no-raw-material-in-response conventions — but simpler, since embyr generates the key itself rather than accepting customer-submitted material (no malformed-input validation branch needed).

## Pre-Slice SPIKE

Not required — this slice reuses well-understood existing admin-API patterns (`SessionContext`, project-ownership check) with no new external-wire-format uncertainty.
