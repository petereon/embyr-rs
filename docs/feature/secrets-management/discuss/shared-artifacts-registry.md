# Shared Artifacts Registry — secrets-management

> Feature: secrets-management
> Wave: DISCUSS
> Updated: 2026-08-09

---

## Registry

| Artifact | Source of Truth | Consumers | Owner | Integration Risk |
|----------|-----------------|-----------|-------|-------------------|
| `EMBYR_ADMIN_KEY` (plain) | Deployment manifest env var | `ServerConfig::from_env()` → `OperatorState.admin_key` | Sam Chen (operator) | HIGH — Bearer token for every operator route + `/metrics`; unchanged from today when no secret-ref var is set |
| `EMBYR_ADMIN_KEY_AWS_SECRET_ARN` | Deployment manifest env var | `ServerConfig::from_env()` (new; fetched once at startup via generalized `AwsSecretFetcher`) | Sam Chen (operator) | HIGH — mutually exclusive with plain `EMBYR_ADMIN_KEY` and with the GCP variant; ambiguity is a startup config error |
| `EMBYR_ADMIN_KEY_GCP_SECRET_NAME` | Deployment manifest env var | `ServerConfig::from_env()` (new; fetched once at startup via generalized `GcpSecretFetcher`) | Sam Chen (operator) | HIGH — same mutual-exclusivity rule as the AWS variant |
| `EMBYR_ADMIN_KEY_PREVIOUS` (+ its own optional `_AWS_SECRET_ARN` / `_GCP_SECRET_NAME` forms) | Deployment manifest env var | `operator_auth_middleware` (accepts current OR previous) | Sam Chen (operator) | HIGH — must never equal `EMBYR_ADMIN_KEY` (startup config error); absent by default (no window unless explicitly opened) |
| `EMBYR_ENCRYPTION_KEY` (plain) | Deployment manifest env var | `ServerConfig::from_env()` → `UserAdminState.encryption_key`; consumed by `oidc_providers.rs`, `auth.rs`, `projects.rs` | Sam Chen (operator) | HIGH — protects 3 AES-256-GCM sites; unchanged from today when no secret-ref var is set |
| `EMBYR_ENCRYPTION_KEY_AWS_SECRET_ARN` / `_GCP_SECRET_NAME` | Deployment manifest env var | `ServerConfig::from_env()` (new; fetched + hex-validated once at startup) | Sam Chen (operator) | HIGH — mutually exclusive with plain var and with the other backend's secret-ref var |
| `EMBYR_ENCRYPTION_KEY_PREVIOUS` (+ its own optional `_AWS_SECRET_ARN` / `_GCP_SECRET_NAME` forms) | Deployment manifest env var | Shared rotation-aware decrypt helper (currently wired into `auth.rs` TOTP verification; available for future `oidc_providers.rs`/`projects.rs` decrypt consumers) | Sam Chen (operator) | HIGH — must never equal `EMBYR_ENCRYPTION_KEY` (startup config error); absent by default |
| `fetched_secret_value` (admin key or encryption key, post-fetch) | In-process only — return value of `AwsSecretFetcher`/`GcpSecretFetcher` raw-fetch method | `ServerConfig` fields; NEVER logged, NEVER written to system DB, NEVER echoed in any error message | `ServerConfig::from_env()` | CRITICAL — same "never log a secret value" invariant already established for `EMBYR_AGENT_DB_DSN` (embyr-agent feature, Invariant 13). Negative test with a sentinel string is mandatory for both keys. |
| `admin_key` / `admin_key_previous` | `OperatorState` (constructed once at startup from `ServerConfig`) | `operator_auth_middleware` — the single Bearer-comparison site for all `/admin/v1/*` operator routes and `/metrics` | `crates/embyr-server/src/admin/middleware/operator_auth.rs` | HIGH — single source; if a second comparison site is ever added, it must reuse this same OR-candidate logic, not reimplement it |
| `encryption_key` / `encryption_key_previous` | `UserAdminState` (constructed once at startup from `ServerConfig`) | `oidc_providers.rs` (encrypt only, today), `auth.rs` (encrypt at enroll, decrypt-with-rotation at signin), `projects.rs` (encrypt only, today) | `crates/embyr-server/src/admin/state.rs` | HIGH — single source; the rotation-aware decrypt helper must be the only place that knows about `encryption_key_previous`, so all 3 sites stay consistent as decrypt consumers are added |
| `totp_secret_enc` | `users.totp_secret_enc` column (Postgres) | `auth.rs` signin (only current live decrypt consumer of any of the 3 AES-GCM sites) | `auth.rs` | HIGH — the only site this feature can test end-to-end for decrypt-with-rotation; see Domain Examples in US-SM-03 |
| `client_secret_enc` | `oidc_providers.client_secret_enc` column (Postgres) | Encrypt-only today (no decrypt consumer exists in the codebase) | `oidc_providers.rs` | MEDIUM — rotation-safety here is currently "encrypt under current key only"; decrypt-with-rotation is a documented future extension point, not independently testable today |
| `backend_pg_dsn_enc` | `projects.backend_pg_dsn_enc` column (Postgres) | Encrypt-only today (no decrypt/connect consumer exists in the codebase — distinct from the ECIES-based `backend_pg_creds_enc` agent-mode DSN, which is out of scope per the problem statement) | `projects.rs` | MEDIUM — same rotation-safety posture as `client_secret_enc` |

---

## Integration Validation Rules

1. **Secret-value log isolation**: the raw fetched value of `EMBYR_ADMIN_KEY` and
   `EMBYR_ENCRYPTION_KEY` (regardless of source — plain, AWS, or GCP) MUST NOT appear in any
   log line at any level, any HTTP response body, any error message, or any system DB row.
   Mandatory negative test with a sentinel string, mirroring the `EMBYR_AGENT_DB_DSN`
   precedent (embyr-agent feature, Invariant 13).

2. **Mutual exclusivity of sourcing**: for each of `EMBYR_ADMIN_KEY` and
   `EMBYR_ENCRYPTION_KEY`, at most one of {plain env var, AWS secret-ref var, GCP
   secret-ref var} may be set. Two or more set simultaneously is a startup config error
   (exit 1, no I/O attempted) — never a silent "first one wins" resolution.

3. **Rotation-window distinctness**: `EMBYR_ADMIN_KEY_PREVIOUS` must never equal
   `EMBYR_ADMIN_KEY`; `EMBYR_ENCRYPTION_KEY_PREVIOUS` must never equal
   `EMBYR_ENCRYPTION_KEY`. Both are startup config errors — a "rotation window" with
   identical current/previous values is definitionally not a rotation and likely a manifest
   typo.

4. **Single comparison/decrypt site**: `operator_auth_middleware` is the only Bearer-token
   comparison site (mirrors the existing ADR-009 auth-middleware-separation decision). The
   rotation-aware decrypt helper is the only site that knows about
   `encryption_key_previous`. If either check is duplicated ad hoc at a second call site,
   the two candidate-sets can drift — this is an integration failure this feature must
   prevent structurally, not just by convention.

5. **AEAD authentication, not a garbage-success risk**: AES-256-GCM's authentication tag
   guarantees that decrypting with the wrong key fails loudly (never silently produces
   plausible-looking garbage plaintext). Trying the current key then the previous key in
   sequence cannot mask real ciphertext corruption — a ciphertext that is genuinely
   corrupted fails under both keys and surfaces the same decrypt-failure response the
   system already returns today.
