# Story Map — secrets-management

> Feature: secrets-management
> Wave: DISCUSS / Phase 2.5
> Persona: Sam Chen (P2 — Service Operator / Platform Engineer), JOB-14
> Updated: 2026-08-09

---

## Scope Assessment

**PASS** — 4 user stories, 3 bounded contexts within a single crate (`embyr-server`):
(1) startup configuration & secrets-manager sourcing, (2) encryption-at-rest key rotation,
(3) admin bearer-token rotation. No `embyr-core` changes (IO-free invariant untouched), no
new workspace crates, no proto changes. Estimated 3.5–4.5 days total. Each story is
independently shippable — none blocks release of the others except through the natural
build order (US-SM-01 generalizes the fetchers that US-SM-02 reuses).

Not oversized: fewer than 10 stories, fewer than 3 bounded contexts, no walking skeleton
requiring more than 5 integration points, estimated effort under 2 weeks.

---

## User: Sam Chen (P2 — Service Operator / Platform Engineer)
## Goal: Source EMBYR_ADMIN_KEY and EMBYR_ENCRYPTION_KEY from a secrets manager, and rotate
## either one in production without an outage or permanently orphaned data.

## Backbone

| Source Secrets at Startup | Rotate Encryption Key | Rotate Admin Key |
|----------------------------|------------------------|--------------------|
| Fetch EMBYR_ADMIN_KEY from AWS/GCP | Configure EMBYR_ENCRYPTION_KEY_PREVIOUS | Configure EMBYR_ADMIN_KEY_PREVIOUS |
| Fetch EMBYR_ENCRYPTION_KEY from AWS/GCP | Decrypt tries current key, falls back to previous | operator_auth_middleware accepts current OR previous |
| Fall back to plain env var (local/CI/dev unchanged) | New writes encrypt under current key only | /metrics honors the same dual-token window |
| Reject ambiguous/conflicting sourcing at startup | Reject identical current/previous keys at startup | Reject identical current/previous tokens at startup |
| | Close window: drop `_PREVIOUS`, restart | Close window: drop `_PREVIOUS`, restart |

---

## Walking Skeleton

**US-SM-01** — Admin key sourced from AWS/GCP Secrets Manager at startup.

Rationale: the thinnest new capability. No rotation complexity, no crypto-format
validation (unlike the encryption key), and it forces the one shared prerequisite every
other story depends on: generalizing `AwsSecretFetcher`/`GcpSecretFetcher` beyond their
current DSN-JSON-specific shape into a raw-string fetch path. Proves the whole
secrets-manager-sourcing mechanism end-to-end with the simplest possible payload.

```
Deployment manifest (ARN/resource ref) → ServerConfig::from_env() → raw-string fetch
  → admin_key populated → operator_auth_middleware unchanged → operator route succeeds
```

---

## Release 1: Secrets-Manager Sourcing (US-SM-01 + US-SM-02)

Outcome: Sam deploys embyr-server with zero literal `EMBYR_ADMIN_KEY` or
`EMBYR_ENCRYPTION_KEY` values in the deployment manifest, for customers whose security
policy already governs credentials through AWS/GCP Secrets Manager. Local/CI/dev
environments are unaffected — plain env vars keep working exactly as today.

| Story | Effort | Outcome KPI Targeted |
|-------|--------|------------------------|
| US-SM-01 (Walking Skeleton) — admin key from secrets manager | 1 day | KPI-1 |
| US-SM-02 — encryption key from secrets manager | 0.5–1 day | KPI-1 |

## Release 2: Encryption Key Rotation (US-SM-03)

Outcome: Sam rotates `EMBYR_ENCRYPTION_KEY` (planned or forced by a leak) without
permanently orphaning existing `oidc_providers.client_secret_enc`, `users.totp_secret_enc`,
or `projects.backend_pg_dsn_enc` rows, and without a synchronous re-encryption migration.

| Story | Effort | Outcome KPI Targeted |
|-------|--------|------------------------|
| US-SM-03 — dual-key decrypt window for EMBYR_ENCRYPTION_KEY | 1–1.5 days | KPI-2 |

## Release 3: Admin Key Rotation (US-SM-04)

Outcome: Sam rotates `EMBYR_ADMIN_KEY` without a coordinated flag-day cutover across every
operator client and `/metrics` scraper.

| Story | Effort | Outcome KPI Targeted |
|-------|--------|------------------------|
| US-SM-04 — dual-token accept window for EMBYR_ADMIN_KEY | 0.5–1 day | KPI-3 |

---

## Priority Rationale

1. **US-SM-01** (Walking Skeleton) — establishes the generalized raw-string secret-fetch
   mechanism that US-SM-02 directly reuses. Highest learning leverage: proves
   secrets-manager sourcing works end-to-end with the simplest payload (a bearer token,
   no format validation) before adding encryption-key hex validation.
2. **US-SM-02** — completes Release 1 (both secrets sourceable from a secrets manager).
   Directly extends US-SM-01's fetch mechanism; low incremental risk.
3. **US-SM-03** — encryption-key rotation is the highest-severity risk named in the
   problem statement (permanent, irreversible data loss on rotation today). Prioritized
   ahead of admin-key rotation because the failure mode (orphaned production data) is
   worse than the failure mode being solved by US-SM-04 (a coordinated but recoverable
   token cutover).
4. **US-SM-04** — admin-key rotation is lower risk (a bad rotation today is inconvenient,
   not data-destroying) and has no dependency on US-SM-01–03 beyond the shared
   "*_PREVIOUS, reject-if-identical" pattern established by US-SM-03. Ordered last.

**Riskiest assumption first**: US-SM-01 de-risks "can the existing DSN-specific secret
fetchers be generalized to raw-string secrets without disrupting the existing customer-DSN
fetch path?" — this is the technical assumption most likely to invalidate the whole
approach if wrong, so it is proven first via the walking skeleton.
