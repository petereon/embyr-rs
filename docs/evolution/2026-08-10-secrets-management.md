# Evolution: secrets-management

**Date:** 2026-08-10
**Feature:** AWS/GCP Secrets Manager sourcing + dual-key/dual-token rotation for `EMBYR_ADMIN_KEY` / `EMBYR_ENCRYPTION_KEY`
**Job:** JOB-14 (secrets-lifecycle)
**ADR:** ADR-018

## Business Context

`EMBYR_ADMIN_KEY` and `EMBYR_ENCRYPTION_KEY` were raw environment variables with no secrets-manager integration and no rotation path. `EMBYR_ENCRYPTION_KEY` is a single global AES-256-GCM key protecting three secret categories (OIDC client secrets, TOTP secrets, admin-api-v2 backend DSNs) — rotating it with no migration path would permanently orphan existing encrypted data. Discovered during a production-readiness audit. Persona: Sam Chen (P2 — Service Operator / Platform Engineer).

## What Shipped (4 user stories)

| Story | Description |
|-------|-------------|
| US-SM-01 (Walking Skeleton) | `EMBYR_ADMIN_KEY` sourceable from AWS/GCP Secrets Manager at startup; plain-env-var fallback preserved |
| US-SM-02 | `EMBYR_ENCRYPTION_KEY` sourced the same way, reusing identical validation |
| US-SM-03 | `EMBYR_ENCRYPTION_KEY_PREVIOUS` opens a dual-key decrypt-rotation window (`decrypt_with_rotation()` in `adapters/encryption.rs`), wired into the live TOTP signin path |
| US-SM-04 | `EMBYR_ADMIN_KEY_PREVIOUS` opens a dual-token bearer-accept window in both `operator_auth_middleware` and `dual_auth_middleware` (the second a DESIGN-added consistency fix), using constant-time comparison via the `subtle` crate |

## Key Decisions (ADR-018)

- `ServerConfig::from_env()` became `async` — real breaking-change blast radius; all call sites updated.
- New `AwsSecretFetcher::get_raw_secret()` / `GcpSecretFetcher::get_raw_secret()` extend the existing DSN-specific fetchers (EXTEND, not new adapters).
- Shared `resolve_secret_source` precedence helper: exactly one of {plain, AWS ARN, GCP name} must be set, else `AmbiguousSecretSource`.
- GCP path uses an interim `EMBYR_GCP_ACCESS_TOKEN` stopgap (no full Workload Identity chain) — deliberately scoped, tracked as OQ-SM-4.
- Rotation is dual-key/dual-token acceptance, not eager re-encryption — mirrors the existing Argon2id dual-hash rotation precedent already established in this codebase.

## Steps Completed

| Step | Name | Status |
|------|------|--------|
| 01-01 | `decrypt_with_rotation` GREEN (walking skeleton) | PASS |
| 01-02 | Async config + AWS admin-key walking skeleton | PASS |
| 02-01 | Reject ambiguous admin-key sourcing / handle fetch failure | PASS |
| 03-01 | Source encryption key from AWS Secrets Manager | PASS |
| 03-02 | Reject ambiguous encryption-key sourcing / handle fetch failure | PASS |
| 03-03 | Session-auth seeding harness + OIDC encryption scenario (test-only) | PASS |
| 04-01 | Wire `decrypt_with_rotation` into TOTP signin | PASS |
| 05-01 | Dual-token `operator_auth_middleware` | PASS (1c060de; fixed a DISTILL scaffold URL defect en route) |
| 05-02 | `dual_auth_middleware` accepts previous admin key too | PASS |
| 06-01 | `GcpSecretFetcher` raw secret + resolver GCP dispatch | PASS |

All 10 steps show complete `PREPARE → RED_ACCEPTANCE → GREEN → COMMIT` DES traces in `execution-log.json` (independently verified against `git log`, not merely asserted).

## Real Bugs Found and Fixed During DELIVER

1. **Silently uncommitted module registration.** A missing `pub mod encryption;` in `adapters/mod.rs` was never actually committed across 6+ commits — every crafter's local build succeeded because their own uncommitted working tree had the line, but the committed history never built from a clean checkout. Caught via an isolated-worktree verification build. Fixed in `5f5bed3`.
2. **Stale local master.** Local `master` had silently diverged from `origin/master` since before this feature started — 6 Dependabot security PR merges (including a `jsonwebtoken` auth-bypass fix) landed on the GitHub remote via `gh pr merge` earlier but were never fetched locally, so the entire feature was built on a stale, more-vulnerable base. Also caught via isolated-worktree verification. Reconciled via `git merge origin/master` in `e7b54b3` with one resolved `Cargo.toml` conflict. Lesson captured: always `git fetch`/`pull` after `gh pr merge` before continuing local work on the same branch.
3. **DISTILL-scaffold test bug.** `operator_route_accepts_previous_admin_key_during_rotation_window` targeted `GET /admin/v1/projects` (session-auth-guarded) instead of `GET /metrics` (operator-auth-guarded) — the scenario's own docstring already said the right route. Fixed as a test-only URL correction.

## Quality Gates

- **Adversarial security review:** APPROVE, no blockers. Secret values never logged verbatim; constant-time dual-token comparison confirmed via `subtle::ConstantTimeEq`; async `from_env()` migration fully covered; `decrypt_with_rotation()` verified against all 3 ciphertext shapes (TOTP / OIDC / DSN).
- **Mutation testing:** `encryption.rs` 90% kill rate (9/10, healthy; 1 benign boundary-equivalent survivor: `ciphertext.len() < 12` vs `<= 12`, unreachable in practice due to AES-GCM's 16-byte tag). `config.rs`'s 71-mutant run is **inconclusive** — initially scoped to `--lib` only (excluding the acceptance suite where the secret-resolution/precedence functions are actually exercised end-to-end), and a manual re-validation attempt was blocked by a reproducible environmental TLS issue (`aws-smithy-http-client` native-roots cert-loading failure, unrelated to the code under test). Logged as follow-up rather than blocking, given 28/29 real acceptance scenarios plus the approved security review.
- 28/29 acceptance scenarios pass (1 correctly `#[ignore]`d — GCP local-emulator testing has no override, tracked as OQ-SM-4).
- `cargo build --workspace` / `cargo clippy --workspace -- -D warnings` clean throughout.

## Follow-Up Work (Backlog)

- Re-run `config.rs` mutation testing with correct scope (`--test secrets_management`, not `--lib`) on infrastructure without the TLS cert-loading instability.
- Investigate whether the TLS native-roots flakiness could recur for real users hitting AWS Secrets Manager in production (found during testing, not confirmed as a production risk).
- GCP full production credential path (Workload Identity or equivalent) — `EMBYR_GCP_ACCESS_TOKEN` is an interim stopgap.

## Key Files

- `crates/embyr-server/src/adapters/encryption.rs` — `decrypt_with_rotation()`
- `crates/embyr-server/src/config.rs` — async `ServerConfig::from_env()`, `resolve_secret_source` precedence logic
- `crates/embyr-server/src/adapters/aws_secret_fetcher.rs`, `gcp_secret_fetcher.rs` — `get_raw_secret()`
- `crates/embyr-server/src/admin/middleware/operator_auth.rs`, `dual_auth.rs` — dual-token constant-time bearer comparison
- `docs/product/architecture/adr-018-secrets-management.md`
- `tests/secrets_management/` — 28/29 acceptance scenarios
