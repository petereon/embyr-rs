# Slice 01: Register a Project's Verification Credential

**Story**: US-01 | **Release**: 1 | **Walking Skeleton**: Yes | **Estimate**: 1 day

## Goal
Alex registers Trailmark's verification credential with embyr, so Trailmark's backend can start minting per-end-user tokens embyr will actually check.

## IN Scope
- Admin-API action to register a project's verification credential (exact endpoint shape DESIGN's call).
- Rejection of malformed verification material with a specific reason.
- Rejection of a second registration attempt for an already-registered project (409, directs to rotation).
- Standard admin auth (401 on missing/invalid admin Bearer) and project-existence (404) checks, consistent with existing admin-API conventions.
- Non-disclosure guarantee: raw verification material is never echoed back in any response or log line.

## OUT Scope
- Rotation of an existing credential (US-03 / Slice 03).
- Verifying any end-user token against the registered credential (US-02 / Slice 02).
- Choice of verification-material shape (public key, shared secret, JWKS reference) — DESIGN's call.

## Learning Hypothesis
Disproves: a project-scoped verification credential cannot be registered and stored safely (never echoed back) using the existing admin-API auth/response conventions without inventing a new pattern.
Confirms (if it succeeds): the existing admin-API conventions (Bearer auth, project-existence checks, non-disclosure responses) generalize cleanly to a new credential type.

## Acceptance Criteria
- [ ] AC-16-01: Valid registration returns 201; credential stored and active; no raw material in response.
- [ ] AC-16-02: Malformed verification material returns 400, naming what's wrong.
- [ ] AC-16-03: Missing/invalid admin Bearer returns 401.
- [ ] AC-16-04: Registering when a credential already exists returns 409, directs to rotation.
- [ ] AC-16-05: Registering for a non-existent/deleted project returns 404.

## Dependencies
None — this is the first slice; nothing in this feature precedes it.

## Effort Estimate
1 day.

## Reference Class
`POST /admin/v1/projects` (existing provisioning endpoint) — same admin-Bearer-auth, project-existence-check, and non-disclosure-response conventions.

## Pre-Slice Spike
Not needed — mechanism-neutral admin-API pattern is well-established in this codebase (US-07/US-10/US-11 in `embyr-rs`).
