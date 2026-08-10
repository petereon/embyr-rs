# DISCUSS Decisions — secrets-management

## Key Decisions

- [D-SM-1] Secrets-manager backends in scope: AWS Secrets Manager + GCP Secret Manager only, reusing the existing `AwsSecretFetcher`/`GcpSecretFetcher` pattern. Rationale: both already exist and are proven for customer DSN fetching; no stakeholder ask or existing precedent for a third backend (e.g. HashiCorp Vault).
- [D-SM-2] Local/CI/dev environments keep working via plain env vars unchanged; secrets-manager sourcing is strictly additive/optional. Rationale: mirrors how `AwsSecretFetcher`/`GcpSecretFetcher` are already `Option<Arc<...>>` elsewhere; forcing every environment onto a cloud secrets manager would break existing CI and local dev flows for no benefit.
- [D-SM-3] Generalize `AwsSecretFetcher`/`GcpSecretFetcher` with a new raw-string fetch method distinct from the existing DSN-JSON `parse_dsn` path. Rationale: `EMBYR_ADMIN_KEY` and `EMBYR_ENCRYPTION_KEY` are not DSNs — forcing them into `{"dsn": "..."}` JSON would be an artificial and confusing shape.
- [D-SM-4] Secrets-manager fetch for these two keys happens once at startup (`ServerConfig::from_env()`), not per-request. Rationale: startup-time secrets don't need TTL caching (process re-reads on restart); differs intentionally from the existing per-request, TTL-cached customer-DSN fetch pattern.
- [D-SM-5] `EMBYR_ENCRYPTION_KEY` rotation uses a dual-key decrypt window (`EMBYR_ENCRYPTION_KEY_PREVIOUS`), not a synchronous batch re-encryption migration. Rationale: mirrors the existing Argon2id dual-hash rotation shape (`auth_key_hash`/`auth_key_hash_2`) already established in the codebase for project API-key auth; avoids an outage-risk batch migration at rotation time. Lazy re-encryption is explicitly deferred — the `auth_key_hash_2` precedent also has no self-healing rehash, only a static "check both if present" comparison.
- [D-SM-6] `EMBYR_ADMIN_KEY` rotation uses a dual-token accept window (`EMBYR_ADMIN_KEY_PREVIOUS`), distinct in shape from D-SM-5 because it is a bearer-token OR-comparison, not an AEAD decrypt-with-fallback. Rationale: problem statement explicitly distinguishes the two rotation semantics; `operator_auth_middleware`'s single `token == state.admin_key` check is the natural, minimal extension point.
- [D-SM-7] Rotation window duration is operator-managed; no automatic expiry timer is introduced. Rationale: consistency with the `auth_key_hash_2` precedent (operator-terminated); avoids adding new background-scheduling infrastructure to a feature that does not otherwise need one.

## Requirements Summary

- Primary job: JOB-14 (secrets-management) — new entry in `docs/product/jobs.yaml`
- Walking skeleton scope: US-SM-01 — admin key sourced from AWS/GCP Secrets Manager at startup
- Feature type: composition-root + admin-adapter extension (no new bounded context, no `embyr-core` changes)
- Stories: 4 (US-SM-01 through US-SM-04), estimated 3.5–4.5 days total

## Constraints Established

- Fetched secret values never appear in logs, error messages, or the system DB (mirrors `EMBYR_AGENT_DB_DSN` Invariant 13 from the embyr-agent feature)
- Exactly one sourcing mechanism (plain / AWS / GCP) may be configured per key; ambiguity is a startup config error
- `_PREVIOUS` values must differ from their current counterparts; equality is a startup config error
- `operator_auth_middleware` remains the single Bearer-comparison site; the rotation-aware decrypt logic must be a single shared function, not duplicated per call site
- Existing customer-DSN fetch behavior (DSN-JSON `parse_dsn`, per-request TTL cache) is unchanged — this feature only adds a parallel raw-string fetch path

## Scope Assessment: PASS

4 stories, 3 bounded contexts (startup/secrets sourcing; encryption-at-rest rotation; admin
bearer-token rotation), all within `embyr-server`. Estimated 3.5–4.5 days. Within right-sized
threshold. Walking skeleton (US-SM-01) estimated 1 day.

## Upstream Changes

- JOB-14 (`docs/product/jobs.yaml`): new entry. Additive — no change to any prior DISCOVER/DIVERGE assumption.
- No DIVERGE wave was run for this feature. The problem statement (a production-readiness audit finding) already specifies the solution shape: reuse the existing AWS/GCP fetcher pattern, and mirror the existing dual-hash rotation shape for the encryption key. Both have direct, working precedent already in the codebase (`aws_secret_fetcher.rs`, `gcp_secret_fetcher.rs`, `auth_key_hash`/`auth_key_hash_2` per architecture brief), which substantially reduces the marginal value of a separate divergent-options exploration. This is noted as a risk in `feature-delta.md` rather than silently skipped.
- No prior DISCOVER document exists specific to this feature; grounding is the production-readiness audit finding plus direct codebase analysis of `config.rs`, `aws_secret_fetcher.rs`, `gcp_secret_fetcher.rs`, `oidc_providers.rs`, `auth.rs`, `projects.rs`, `operator_auth.rs`, `admin/state.rs`, and ADR-017.
