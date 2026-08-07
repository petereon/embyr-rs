# Slice B-06 — OIDC Providers + Billing

**Feature:** admin-api-v2
**Slice:** B-06 of B-06
**Estimate:** 1 day
**Stories:** US-B06
**Depends on:** B-05 complete

---

## Goal

SSO configuration for the account + usage breakdown for billing. Last slice — all user-admin-ui mock data replaced.

## Learning Hypothesis

Disproves: "OIDC `id_token` validation fails at runtime because the issuer's JWKS endpoint is unreachable in the test environment."  
Confirms if succeeds: OIDC callback validates a real (test) `id_token` in the integration test using a local mock JWKS endpoint.

## IN Scope

**OIDC Providers (5 routes):**
- `GET /admin/v1/oidc_providers`
- `POST /admin/v1/oidc_providers`
- `PATCH /admin/v1/oidc_providers/:id`
- `DELETE /admin/v1/oidc_providers/:id`
- `GET /admin/v1/auth/oidc/callback`

**Billing (1 route):**
- `GET /admin/v1/billing?range=<7d|30d|month|last_month>`

- `client_secret` AES-256-GCM encryption/decryption via `EMBYR_ENCRYPTION_KEY`
- OIDC `id_token` validation: `openidconnect` crate or equivalent; verify issuer, audience, expiry, signature (JWKS fetch with 60s cache)
- OIDC state parameter: CSRF protection (store nonce in session before redirect; verify on callback)
- Billing aggregation: SQL `GROUP BY project_id, date BETWEEN $start AND $end` on `daily_project_metrics`

## OUT Scope

- Ownership transfer (Danger Zone — future slice)
- Account delete (Danger Zone — future slice)
- Pricing rate display
- Invoice generation

## Acceptance Criteria

- AC-B06-01 through AC-B06-08 (see feature-delta.md US-B06)

## Dependencies

- B-05 complete
- `EMBYR_ENCRYPTION_KEY` in place (B-01 dependency, already resolved)
- Integration test: mock OIDC provider (wiremock or local HTTP server) serving JWKS endpoint

## Effort Estimate

1 day. OIDC callback is the most complex piece (JWKS fetch + id_token validation). Billing query is a straightforward GROUP BY. AES-256-GCM encryption reuses the same pattern as TOTP secret encryption from B-01.
