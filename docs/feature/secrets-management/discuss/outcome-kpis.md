# Outcome KPIs — secrets-management

> Feature: secrets-management
> Wave: DISCUSS
> Updated: 2026-08-09

---

## Feature Objective

Enable Sam Chen to run embyr-server for security-policy-constrained customers with both
security-critical secrets (`EMBYR_ADMIN_KEY`, `EMBYR_ENCRYPTION_KEY`) sourced from the
customer's existing AWS/GCP Secrets Manager, and to rotate either secret in production
without a coordinated outage or permanently orphaned data — turning credential rotation
from a fire drill into a routine operation.

---

## Outcome KPIs

| # | Who | Does What | By How Much | Baseline | Measured By | Type |
|---|-----|-----------|-------------|----------|--------------|------|
| KPI-1 | Sam Chen (operating for secrets-manager-constrained customers) | Deploys embyr-server with zero literal `EMBYR_ADMIN_KEY`/`EMBYR_ENCRYPTION_KEY` values in the manifest | 100% of new secrets-manager-policy deployments avoid literal key values (0% possible today) | 0% — no secrets-manager sourcing exists for either key | Manifest audit at deploy review; presence of `*_AWS_SECRET_ARN`/`*_GCP_SECRET_NAME` vars, absence of plain key vars | Leading |
| KPI-2 | Sam Chen (rotating EMBYR_ENCRYPTION_KEY) | Completes an encryption-key rotation with zero rows permanently orphaned during the rotation window | 0 orphaned rows per rotation performed within the operator-managed window (vs. 100% orphaned today — every existing row becomes undecryptable) | 100% orphan rate today (single global key, no fallback) | Integration test: pre-rotation-encrypted TOTP secret decrypts successfully mid-window; post-rotation-window audit query for decrypt failures | Leading |
| KPI-3 | Sam Chen (rotating EMBYR_ADMIN_KEY) | Rotates the admin bearer token with zero simultaneous-client-breakage incidents | 0 unplanned operator-route/`/metrics` outages caused by admin-key rotation (vs. every historical rotation requiring a synchronized "flag day" today) | N/A — no rotation path exists today; every attempted rotation is a hard, simultaneous cutover | Ops incident tracker, tagged `admin-key-rotation`, pre/post feature; integration test proving dual-token acceptance | Leading |
| KPI-4 | Sam Chen (any deployment, secrets-manager-sourced or not) | Never has a secret value (admin key or encryption key) appear in logs, error messages, or the system DB | 0 occurrences, always (hard guardrail, not a target to improve toward) | N/A — no prior instrumentation existed to check this for these two keys | CI negative test: sentinel secret value, grep across log output + system DB dump | Guardrail |

---

## Metric Hierarchy

- **North Star**: KPI-2 — zero data permanently orphaned by an `EMBYR_ENCRYPTION_KEY`
  rotation. This is the single most severe, irreversible failure mode named in the problem
  statement (unlike an admin-key mistake, an orphaned row cannot be recovered after the
  fact).
- **Leading Indicators**: KPI-1 (secrets-manager adoption), KPI-3 (admin-key rotation
  safety)
- **Guardrail Metrics**:
  - KPI-4 — secret values never appear in logs/DB/error messages (hard invariant, not
    negotiable)
  - `embyr-core` IO-free invariant must NOT be violated by any story in this feature
  - Existing customer-DSN fetch behavior (`AwsSecretFetcher`/`GcpSecretFetcher` DSN-JSON
    path) must remain unchanged — this feature only ADDS a raw-string fetch path

---

## Measurement Plan

| KPI | Data Source | Collection Method | Frequency | Owner |
|-----|-------------|--------------------|-----------|-------|
| KPI-1 | Deployment manifest review | Manual audit at deploy time; scriptable grep for `EMBYR_ADMIN_KEY=` / `EMBYR_ENCRYPTION_KEY=` literal values vs. `*_ARN`/`*_SECRET_NAME` presence | Per deployment | Sam Chen / ops team |
| KPI-2 | Integration test suite | Pre-rotation-encrypted fixture decrypts successfully during simulated rotation window (US-SM-03 acceptance tests) | Every PR touching the decrypt-with-rotation helper | embyr-rs team |
| KPI-3 | Integration test suite + ops incident tracker | Dual-token acceptance test (US-SM-04); manual incident tag review post-release | Every PR (test); quarterly (incident review) | embyr-rs team / Sam Chen |
| KPI-4 | CI negative test | Sentinel secret value injected into test config; grep log output and system DB dump for zero matches | Every PR, every merge | embyr-rs team |

---

## Hypothesis

We believe that adding secrets-manager sourcing (reusing the existing `AwsSecretFetcher`/
`GcpSecretFetcher` pattern) plus dual-key/dual-token rotation windows (mirroring the
existing Argon2id dual-hash rotation shape) for `EMBYR_ADMIN_KEY` and
`EMBYR_ENCRYPTION_KEY` will let Sam Chen treat credential rotation as a routine operation
instead of a coordinated, risky event.

We will know this is true when:
- Sam deploys with zero literal secret values in the manifest for secrets-manager-policy
  customers (KPI-1)
- Sam rotates `EMBYR_ENCRYPTION_KEY` and every existing TOTP-enrolled user keeps signing in
  successfully throughout the window (KPI-2)
- Sam rotates `EMBYR_ADMIN_KEY` and neither his old-token clients nor his new-token clients
  experience an outage (KPI-3)
- No secret value, in any deployment configuration, ever appears in a log line, error
  message, or system DB row (KPI-4, guardrail)
