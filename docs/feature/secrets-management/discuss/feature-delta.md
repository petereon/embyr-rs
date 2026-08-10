<!-- markdownlint-disable MD024 -->
# Feature Delta — secrets-management

> Feature ID: secrets-management
> Updated: 2026-08-09
> Status: DESIGN wave complete — Ready for DISTILL wave

---

## Wave: DISCUSS
## Date: 2026-08-09
## Status: Complete — Ready for DESIGN wave

---

## Job Traceability

### JOB-14 (NEW — see `docs/product/jobs.yaml`)

**Persona:** Sam Chen (P2 — Service Operator / Platform Engineer)
**Job Story:** When I operate embyr-server for a customer whose security policy already
governs credentials through AWS or GCP Secrets Manager, and when I need to rotate the admin
bearer token or the shared encryption key, I want embyr to source both secrets from the
secrets manager and support in-place rotation without an outage or orphaned data, so I can
meet my security policy without operating a separate credential store or accepting downtime
risk during rotation.
**Opportunity score:** 13
**Priority:** high

**Mapping:**
- US-SM-01 → JOB-14 (admin key sourced from secrets manager — walking skeleton)
- US-SM-02 → JOB-14 (encryption key sourced from secrets manager)
- US-SM-03 → JOB-14 (encryption-key rotation without orphaning data)
- US-SM-04 → JOB-14 (admin-key rotation without a hard cutover outage)

---

## Problem Statement

embyr-server's two most security-critical secrets are sourced as raw environment variables
with no secrets-manager integration and no rotation path, identified from a
production-readiness audit:

1. **`EMBYR_ADMIN_KEY`** — the Bearer token guarding every operator route
   (provision/delete/suspend projects) and the `/metrics` endpoint. A leaked env var
   (misconfigured deployment manifest, process listing, accidental log line) grants full
   operator access.

2. **`EMBYR_ENCRYPTION_KEY`** — a single global AES-256-GCM key (32 bytes, loaded via
   `ServerConfig::from_env()` in `crates/embyr-server/src/config.rs`) that protects THREE
   separate secret categories in the system DB, all via direct
   `Aes256Gcm::new_from_slice(&state.encryption_key)`:
   - `oidc_providers.client_secret_enc` (`crates/embyr-server/src/admin/handlers/oidc_providers.rs:130`)
   - `users.totp_secret_enc` (`crates/embyr-server/src/admin/handlers/auth.rs:255`)
   - `projects.backend_pg_dsn_enc` for admin-api-v2 `direct_pg` backend mode
     (`crates/embyr-server/src/admin/handlers/projects.rs:153`)

   If this key is ever rotated (planned, or forced by a leak), every existing encrypted
   value becomes permanently undecryptable — there is no versioned-key or dual-key
   decrypt-old/encrypt-new migration path, unlike the existing Argon2id "dual-hash rotation
   window" pattern already used elsewhere in this codebase for project API-key auth
   (`auth_key_hash` / `auth_key_hash_2`, per `docs/product/architecture/brief.md` §
   Auth interceptor design, step 4).

   Note: this is DIFFERENT from the operator-provisioned project DSN encryption, which uses
   ECIES with a private key re-derived from each project's own API key
   (`crates/embyr-core/src/auth/ecies.rs`) — that scheme has no single global key and is NOT
   in scope here. Only the three AES-256-GCM sites above, keyed by the single global
   `EMBYR_ENCRYPTION_KEY`, are in scope.

**Codebase verification (2026-08-09):** of the three AES-256-GCM sites, only `auth.rs` TOTP
signin currently has a live decrypt call site. `oidc_providers.client_secret_enc` and
`projects.backend_pg_dsn_enc` are write-only today — no decrypt/connect consumer exists yet
in the codebase for either. This scopes US-SM-03's testable rotation-safety to the one live
decrypt path, while still requiring the shared decrypt mechanism to be provably correct
against all three ciphertext shapes (see US-SM-03 UAT scenario 6).

---

## Locked Decisions

All decisions are pre-decided from codebase analysis and the problem statement's explicit
guidance. Do not re-derive in DESIGN wave.

| ID | Decision | Source |
|----|----------|--------|
| D-SM-1 | Secrets-manager backends in scope: AWS Secrets Manager + GCP Secret Manager only, reusing the existing `AwsSecretFetcher`/`GcpSecretFetcher` pattern. No HashiCorp Vault or other backend — no existing precedent in the codebase and no stakeholder ask. | Problem statement explicit guidance; `crates/embyr-server/src/adapters/{aws,gcp}_secret_fetcher.rs` already exist |
| D-SM-2 | Local/CI/dev environments keep working via plain env vars unchanged. Secrets-manager sourcing is strictly additive and optional — new env vars are optional; when absent, `ServerConfig::from_env()` falls back to the existing plain `EMBYR_ADMIN_KEY`/`EMBYR_ENCRYPTION_KEY` behavior byte-for-byte. | Problem statement explicit guidance; mirrors how `AwsSecretFetcher`/`GcpSecretFetcher` are already `Option<Arc<...>>` elsewhere in the codebase |
| D-SM-3 | Existing secret fetchers are DSN-JSON-specific (`parse_dsn` expects `{"dsn": "..."}`). Extend both fetchers with a new generic raw-string fetch method — do not force `EMBYR_ADMIN_KEY`/`EMBYR_ENCRYPTION_KEY` into the DSN JSON shape. The secret value for these two keys is the raw secret string (admin key) or raw hex string (encryption key), not `{"dsn": ...}` JSON. | `crates/embyr-server/src/adapters/aws_secret_fetcher.rs` `parse_dsn()`; same in `gcp_secret_fetcher.rs` |
| D-SM-4 | Fetching the admin/encryption key from a secrets manager happens ONCE at startup (`ServerConfig::from_env()`, step 1 of the ADR-017 14-step startup sequence), not per-request. This differs from the customer-DSN fetcher usage pattern (per-request, TTL-cached) — startup-time secrets do not need TTL caching since the process re-reads on restart. | ADR-017 startup sequence; existing per-request TTL-cache pattern in `AwsSecretFetcher::get_dsn` is customer-DSN-specific and unaffected |
| D-SM-5 | `EMBYR_ENCRYPTION_KEY` rotation must not require simultaneous re-encryption of all existing rows (no outage-risk batch migration). Adopt a **dual-key decrypt window**: optional `EMBYR_ENCRYPTION_KEY_PREVIOUS` holds the retiring key. Decrypt tries current key first, falls back to previous key on AEAD failure — mirroring the existing dual-hash (`auth_key_hash` / `auth_key_hash_2`) rotation shape. New writes always encrypt under the current key only (no dual-write). Lazy re-encryption / background migration is explicitly DEFERRED — this slice ships the dual-key decrypt window only, matching the `auth_key_hash_2` precedent which also has no self-healing rehash. | Problem statement explicit guidance; `docs/product/architecture/brief.md` line 914 (`auth_key_hash_2` "if present" check, no rehash-on-success found in codebase) |
| D-SM-6 | `EMBYR_ADMIN_KEY` rotation semantics differ from encryption-key rotation — it is a bearer-token comparison, not ciphertext. Adopt a **dual-token accept window**: optional `EMBYR_ADMIN_KEY_PREVIOUS`. `operator_auth_middleware` accepts either `admin_key` or `admin_key_previous` (when set) during the cutover window. No outage — operators update client config on their own schedule within the window, then drop `EMBYR_ADMIN_KEY_PREVIOUS` once cutover completes. | Problem statement explicit guidance; `crates/embyr-server/src/admin/middleware/operator_auth.rs` current single-token comparison |
| D-SM-7 | Rotation window duration/expiry is operator-managed — no automatic expiry timer in-process for either `_PREVIOUS` variable. Mirrors the `auth_key_hash_2` precedent (operator-terminated, not system-scheduled). No new expiry-scheduling infrastructure is introduced by this feature. | Consistency with existing dual-hash precedent; avoids adding a new background scheduler for a feature that does not otherwise need one |

---

## System Constraints

These cross-cutting constraints apply to all stories in this feature.

- `embyr-core` must remain IO-free. `deny.toml` enforces this; no story in this feature adds IO crates to `embyr-core` — all work is in `embyr-server` (composition root + admin adapters/handlers).
- No new workspace crates are introduced.
- No new TCP listener ports, no new gRPC/HTTP routes — this feature changes startup config resolution and two existing auth/decrypt call sites only.
- Fetched secret values (`EMBYR_ADMIN_KEY`, `EMBYR_ENCRYPTION_KEY`, regardless of source) MUST NOT appear in any log line at any level, any HTTP response body, any error message, or any system DB row. Mandatory negative test with a sentinel string, mirroring the `EMBYR_AGENT_DB_DSN` precedent (embyr-agent feature, Invariant 13).
- For each of `EMBYR_ADMIN_KEY` and `EMBYR_ENCRYPTION_KEY`, at most one of {plain env var, AWS secret-ref var, GCP secret-ref var} may be set. Ambiguity is a startup config error, never a silent "first one wins."
- `EMBYR_ADMIN_KEY_PREVIOUS` must never equal `EMBYR_ADMIN_KEY`; `EMBYR_ENCRYPTION_KEY_PREVIOUS` must never equal `EMBYR_ENCRYPTION_KEY`. Both are startup config errors.
- `operator_auth_middleware` remains the single Bearer-comparison site (ADR-009 auth-middleware-separation is not reopened by this feature). The rotation-aware decrypt logic must be a single shared function, not duplicated per call site.
- Existing customer-DSN fetch behavior (`AwsSecretFetcher::get_dsn`/`GcpSecretFetcher::get_dsn`, the DSN-JSON `parse_dsn` path, per-request TTL caching) is unchanged by this feature — this feature only ADDS a raw-string fetch path alongside it.
- No lazy re-encryption / background re-encrypt-on-decrypt job is in scope for this feature (D-SM-5) — flagged as a residual operational risk for DESIGN wave awareness, not a blocking gap.

---

## Scope Assessment

**PASS** — 4 user stories, 3 bounded contexts within a single crate (`embyr-server`):
(1) startup configuration & secrets-manager sourcing, (2) encryption-at-rest key rotation,
(3) admin bearer-token rotation. No `embyr-core` changes, no new workspace crates, no proto
changes. Estimated 3.5–4.5 days total, well under the 2-week oversized threshold. Full
detail in `story-map.md`.

Not oversized: fewer than 10 stories, fewer than 3 bounded contexts by count, no walking
skeleton requiring more than 5 integration points, estimated effort under 2 weeks.

**Walking Skeleton:** US-SM-01 — admin key sourced from AWS/GCP Secrets Manager at startup.
Rationale and detail in `story-map.md`.

---

## Story Map Summary

See `story-map.md` for the full backbone, walking skeleton rationale, and release slicing.

| Release | Stories | Outcome |
|---------|---------|---------|
| Release 1 — Secrets-Manager Sourcing | US-SM-01 (WS), US-SM-02 | Both secrets sourceable from AWS/GCP Secrets Manager; local/CI/dev unaffected |
| Release 2 — Encryption Key Rotation | US-SM-03 | `EMBYR_ENCRYPTION_KEY` rotates without permanently orphaning existing data |
| Release 3 — Admin Key Rotation | US-SM-04 | `EMBYR_ADMIN_KEY` rotates without a coordinated flag-day outage |

### Priority Rationale (summary — full detail in story-map.md)

1. US-SM-01 (Walking Skeleton) — establishes the generalized raw-string secret-fetch mechanism US-SM-02 reuses.
2. US-SM-02 — completes Release 1; directly extends US-SM-01.
3. US-SM-03 — highest-severity risk (irreversible data loss on rotation today); prioritized ahead of admin-key rotation.
4. US-SM-04 — lower risk (a bad rotation is inconvenient, not data-destroying); no hard dependency on US-SM-01–03.

---

## Definition of Ready

See `dor-validation.md` for the full per-story and feature-level 9-item checklist.

**Overall DoR: PASS — all 4 stories ready for DESIGN wave.**

---

## Outcome KPIs (Feature Level)

See `outcome-kpis.md` for the full KPI table, metric hierarchy, and measurement plan.

**North Star:** zero data permanently orphaned by an `EMBYR_ENCRYPTION_KEY` rotation.

---

## Out of Scope

- HashiCorp Vault or any secrets-manager backend beyond AWS Secrets Manager and GCP Secret Manager (D-SM-1)
- Lazy re-encryption / background re-encrypt-on-decrypt migration job (D-SM-5)
- Automatic rotation-window expiry / scheduled key retirement (D-SM-7)
- ECIES-based operator-provisioned project DSN encryption (`crates/embyr-core/src/auth/ecies.rs`) — explicitly out of scope per the problem statement; unrelated key material
- Adding a decrypt/connect consumer for `oidc_providers.client_secret_enc` or `projects.backend_pg_dsn_enc` where none exists today — this feature only guarantees the shared decrypt mechanism is correct when such a consumer is eventually added
- Any change to `embyr-agent`'s `EMBYR_AGENT_DB_DSN` handling (separate feature, separate secret)

---

## Wave Decisions Log

**2026-08-09** — D-SM-1 through D-SM-7 locked from the problem statement's explicit
guidance and codebase analysis (`config.rs`, `aws_secret_fetcher.rs`, `gcp_secret_fetcher.rs`,
`oidc_providers.rs`, `auth.rs`, `projects.rs`, `operator_auth.rs`, `admin/state.rs`,
ADR-017). No DIVERGE wave run for this feature — the problem statement already specifies the
solution shape (reuse existing fetcher pattern; mirror the existing dual-hash rotation
shape). DIVERGE risk: noted, low — both the sourcing mechanism and the rotation shape have
direct, working precedent already in the codebase, reducing the value of a separate
divergent-options exploration.

**Scope Assessment:** PASS — 4 user stories, 3 bounded contexts, estimated 3.5–4.5 days
total. No scope split required beyond the natural Release 1/2/3 slicing already reflected in
the story map.

Verified during codebase analysis (not assumed from the problem statement): only `auth.rs`
TOTP signin has a live decrypt call site among the three AES-GCM locations; the other two
(`oidc_providers.client_secret_enc`, `projects.backend_pg_dsn_enc`) are write-only today.
This is reflected honestly in US-SM-03's Problem/Technical Notes rather than claiming
end-to-end rotation testing for decrypt paths that do not yet exist.

**2026-08-09** — Peer review (`nw-product-owner-reviewer`), iteration 1: **APPROVED**, zero
critical/high issues. Elevator Pitch test (Dimension 0) passed on all 4 stories. DoR 9-item
checklist independently re-verified as accurate (not just trusted from
`dor-validation.md`'s self-report). JTBD traceability confirmed against `jobs.yaml` (JOB-14
well-formed). Slice composition confirmed — no infrastructure-only release (every release
contains at least one user-visible-value story). All 8 LeanUX anti-patterns confirmed
avoided. Three non-blocking open design questions (OQ-1 decrypt-helper module location,
OQ-2 whether `_PREVIOUS` needs its own AWS/GCP secret-ref variants, OQ-3 optional
window-age warning log) flagged for DESIGN wave — see `dor-validation.md`. No second review
iteration required.

## Status: Ready for DESIGN wave handoff

---

## Wave: DESIGN
## Date: 2026-08-09
## Status: Complete — Ready for DISTILL wave

---

### Wave: DESIGN / [REF] Architecture Summary

This feature makes no changes to the domain model or bounded context boundaries. It extends
the composition root (`config.rs`, `main.rs`), two existing driven adapters
(`AwsSecretFetcher`, `GcpSecretFetcher`), and two existing Bearer-comparison/decrypt call
sites in `embyr-server`. Full detail: `docs/product/architecture/brief.md` §
"Application Architecture — secrets-management" and
`docs/product/architecture/adr-018-secrets-management.md`.

**Bounded contexts touched:** `embyr-server` composition root, `embyr-server::adapters`,
`embyr-server::admin` (state, middleware, router, auth handler).

**Bounded contexts NOT touched:** `embyr-core` (System Constraint honored — zero new
dependencies, zero new IO); `oidc_providers.rs`/`projects.rs` write-only encrypt sites;
`FirestoreService`'s per-project secret-fetcher wiring; ECIES per-project DSN encryption.

---

### Wave: DESIGN / [REF] ADR Written

| ADR | File | Decision |
|-----|------|----------|
| ADR-018 | `docs/product/architecture/adr-018-secrets-management.md` | `ServerConfig::from_env()` becomes async and resolves 4 logical secrets (admin_key, admin_key_previous, encryption_key, encryption_key_previous) from up to 3 sources each; `AwsSecretFetcher`/`GcpSecretFetcher` gain `get_raw_secret()`; new `adapters::encryption::decrypt_with_rotation()`; dual-token bearer comparison in `operator_auth_middleware` and `dual_auth_middleware`. |

---

### Wave: DESIGN / [REF] DoR Open Questions — Resolved

| ID | Question | Resolution | Where |
|----|----------|------------|-------|
| OQ-1 | Decrypt-helper module location | New `crates/embyr-server/src/adapters/encryption.rs` — not `embyr-core` (feature-scope constraint honored even though the function is IO-free), not an associated function on `ServerConfig`/`UserAdminState` (wrong responsibility). | ADR-018 §5, A2 |
| OQ-2 | Does `_PREVIOUS` need its own AWS/GCP secret-ref variants? | Yes — the already-approved US-SM-03/US-SM-04 ACs require it. Implemented via one shared resolver function called 4× to keep code cost low despite the env-var-count growth (6 new vars). | ADR-018 §3 |
| OQ-3 | Optional window-age warning log | Presence-based `tracing::warn!` at every startup when a `_PREVIOUS` var is configured — not age-based, no new scheduler, preserving D-SM-7's "no automatic expiry infra" constraint. | ADR-018 §8 |

**New DESIGN-identified open question (not in original DoR):** OQ-SM-4 — when should
`AwsSecretFetcher`/`GcpSecretFetcher` be upgraded to implement the brief's already-documented
`SecretFetcher` trait with real `probe()` (closing the gap this feature works around with an
interim static `EMBYR_GCP_ACCESS_TOKEN`)? Not blocking — see ADR-018 Alternatives Considered
A6 and brief.md § Open Questions.

---

### Wave: DESIGN / [REF] Component Decomposition (summary — full table in brief.md)

| File | Change Type |
|------|-------------|
| `crates/embyr-server/src/config.rs` (`ServerConfig`, `ConfigError`) | EXTEND |
| `crates/embyr-server/src/adapters/aws_secret_fetcher.rs` | EXTEND |
| `crates/embyr-server/src/adapters/gcp_secret_fetcher.rs` | EXTEND |
| `crates/embyr-server/src/adapters/encryption.rs` | NEW |
| `crates/embyr-server/src/admin/state.rs` (`OperatorState`, `UserAdminState`) | EXTEND |
| `crates/embyr-server/src/admin/middleware/operator_auth.rs` | EXTEND |
| `crates/embyr-server/src/admin/middleware/dual_auth.rs` | EXTEND (DESIGN-added consistency fix) |
| `crates/embyr-server/src/admin/handlers/auth.rs` | EXTEND |
| `crates/embyr-server/src/admin/router.rs` (`build_admin_router`) | EXTEND |
| `crates/embyr-server/src/main.rs` | EXTEND |
| `crates/embyr-server/src/admin/handlers/oidc_providers.rs`, `projects.rs` | UNCHANGED |

---

### Wave: DESIGN / [REF] Reuse Analysis (summary — full table + rationale in brief.md/ADR-018)

Every touched or new component is classified EXTEND or CREATE NEW, default EXTEND per
nw-design rules. Only one CREATE NEW: the rotation-aware decrypt helper
(`adapters/encryption.rs`) — no existing generalized AES-GCM decrypt-with-fallback function
exists anywhere in the codebase. Every other component (fetchers, config, state, middleware,
router) reuses and extends an existing type. The Argon2id dual-hash precedent
(`auth_key_hash`/`auth_key_hash_2`) is REFERENCE ONLY — its rotation *shape* is mirrored, its
code is untouched (different bounded context).

---

### Wave: DESIGN / [REF] External Integrations Requiring Contract Tests

- AWS Secrets Manager (`GetSecretValue` API) — recommended: consumer-driven contract via Pact
  in the CI acceptance stage, alongside existing LocalStack-backed integration tests.
- GCP Secret Manager (`AccessSecretVersion` REST API) — recommended: consumer-driven contract
  via Pact in the CI acceptance stage, alongside existing mock-HTTP-server-backed tests.

Both integrations are pre-existing (US-10/US-11); this feature adds a new call shape
(`get_raw_secret`) against the same two already-integrated services.

---

### Wave: DESIGN / [REF] Quality Gates — Passed

- [x] Requirements (D-SM-1 through D-SM-7, US-SM-01 through US-SM-04) traced to components
- [x] Component boundaries with clear responsibilities
- [x] Technology choices in ADR-018 with 6 rejected alternatives (A1–A6)
- [x] Quality attributes addressed: security, reliability, maintainability, portability
- [x] Dependency-inversion compliance: `embyr-core` untouched
- [x] No new C4 diagram required: no new container or system-context boundary
- [x] Integration patterns unchanged: existing signatures extended, not redesigned
- [x] OSS preference validated: zero new crate dependencies
- [x] AC behavioral (not implementation-coupled)
- [x] External integrations annotated with contract-test recommendation
- [x] Architecture enforcement: `cargo deny check` continues to gate `embyr-core`
- [x] Reuse Analysis complete — zero unjustified CREATE NEW

---

### Wave: DESIGN / [REF] Peer Review

**Reviewer:** `nw-solution-architect-reviewer` — iteration 1: **APPROVED**, zero critical/high
issues. Two medium-severity items identified (test-suite breaking-change blast radius not
quantified; enforcement mechanism for the "secret value never appears in logs/DB/responses"
negative test not specified) — both closed same-iteration by extending ADR-018's Enforcement
section (quantified: 2 existing unit tests need a mechanical one-field update; specified: a
DISTILL-wave integration test mirroring the `EMBYR_AGENT_DB_DSN`/Invariant-13 precedent, no
new CI job). Reviewer independently verified: the `embyr-core` IO-free scope constraint is
real (not a rationalization); the interim `EMBYR_GCP_ACCESS_TOKEN` design is an honest,
appropriately-flagged stopgap (not under-scoped — safe because the token is used exactly once
at startup and discarded, per D-SM-4); the "fetch IS the probe" Earned Trust argument is sound
(stronger than a generic credential probe, not a rationalization to skip probe work); all
three DoR open-question resolutions (OQ-1, OQ-2, OQ-3) are directly responsive, not
restatements; the `dual_auth_middleware` DESIGN-added consistency fix is justified and
transparently flagged rather than silently expanding scope. Priority validation: Q1 YES
(documented production-readiness-audit blockers, not speculative), Q2 ADEQUATE (6 rejected
alternatives), Q3 CORRECT (data-loss risk prioritized ahead of availability risk, matching
DISCUSS-wave story-map ordering), Q4 JUSTIFIED (north-star KPI + zero-new-infrastructure
reuse). No second review iteration required.

---

## Wave: DISTILL
## Date: 2026-08-09
## Status: Complete — Ready for DELIVER wave

---

### Wave: DISTILL / [REF] Prior-wave reading confirmation

+ `docs/feature/secrets-management/discuss/feature-delta.md` (DISCUSS + DESIGN sections)
+ `docs/product/architecture/adr-018-secrets-management.md`
+ `docs/product/architecture/brief.md` § Application Architecture — secrets-management
+ `docs/feature/secrets-management/discuss/user-stories.md`
+ `crates/embyr-server/src/config.rs`
+ `crates/embyr-server/src/adapters/aws_secret_fetcher.rs`
+ `crates/embyr-server/src/adapters/gcp_secret_fetcher.rs`
+ `crates/embyr-server/src/admin/handlers/auth.rs` (TOTP decrypt call site, ~line 255)
+ `crates/embyr-server/src/admin/middleware/operator_auth.rs`
+ `crates/embyr-server/src/admin/middleware/dual_auth.rs`
+ `tests/production_readiness/` (`ServerProcess` harness pattern, reused independently)
+ `tests/admin_api_v2/common/mod.rs` (`AdminTestContext` seeding pattern, reused independently)
+ `docs/feature/secrets-management/discuss/wave-decisions.md`
- `docs/feature/secrets-management/design/wave-decisions.md` (not found — DESIGN content lives inline in this file's `## Wave: DESIGN` section instead of a separate `design/` directory; not a gap, matches this feature's established DISCUSS/DESIGN co-location)
- `docs/feature/secrets-management/devops/` (not found — no DEVOPS wave ran for this feature; graceful degradation applied, see Reconciliation below)
- `auth_key_hash_2` dedicated test file (grepped `tests/` and `crates/` — no dedicated test found; the Argon2id dual-hash precedent is referenced only in `brief.md` prose, not in an isolated test DISTILL could mirror scenario-shape from 1:1. Scenario shape instead mirrors `tests/production_readiness/acceptance/pr01_config_from_env.rs`, the closest existing precedent for subprocess-driven `ServerConfig` acceptance tests.)

### Wave: DISTILL / [REF] Wave-Decision Reconciliation (HARD GATE)

Read `discuss/wave-decisions.md` (present, D-SM-1 through D-SM-7). `design/wave-decisions.md`
and `devops/wave-decisions.md` are absent as separate files — this feature's DESIGN content
is appended directly into this same `feature-delta.md` (the `## Wave: DESIGN` section above),
which was checked line-by-line against every DISCUSS decision: zero contradictions found (the
DESIGN section explicitly implements D-SM-1 through D-SM-7 without altering any of them — see
"Wave: DESIGN / [REF] Reuse Analysis" cross-referencing each). No DEVOPS wave ran for this
feature (directory absent) — per the Graceful Degradation Matrix this is a WARN, not a block;
default environment matrix (clean | with-pre-commit | with-stale-config) is assumed for the
subprocess-based scenarios below, consistent with `tests/production_readiness/`'s existing
precedent.

**Reconciliation passed — 0 contradictions.**

### Wave: DISTILL / [REF] Language + Infrastructure Policy + Port Bootstrap

- `[lang-mode] rust` — detected via workspace root `Cargo.toml` (5-crate Cargo workspace).
- `[policy-mode] inherit` — `docs/architecture/atdd-infrastructure-policy.md` already exists
  and already covers every port this feature needs (AWS Secrets Manager via LocalStack +
  `embyr-server` binary subprocess rows, both added by prior features). No new rows required;
  no `--policy=fresh` requested.
- `[port-mode] inherit` — `tests/common/state_delta.rs` already bootstrapped (feature
  `embyr-rs`, 2026-05-24). Re-exported into `tests/secrets_management/common/mod.rs` via the
  same `#[path = "../../common/state_delta.rs"]` pattern already used by
  `tests/production_readiness/common/mod.rs` and `tests/admin_api_v2/common/mod.rs`.

### Wave: DISTILL / [REF] Walking Skeleton Strategy

Per the Architecture of Reference (driving port = real adapter; driven-internal = real
adapter via project policy; driven-external/non-deterministic = fake with output capture):
the admin key's AWS Secrets Manager source is a **driven-internal-shaped** dependency for
this feature's purpose (embyr's own startup-critical credential resolution, not a
customer-facing non-deterministic external the system merely calls through) — the Project
Infrastructure Policy already resolves this to LocalStack (`@real-io`) for `AwsSecretFetcher`,
consistent with the existing `us_10_aws_secrets.rs` precedent. GCP Secret Manager remains
**driven-external-shaped and currently untestable via subprocess** (see Adapter Coverage
below) — treated as `@requires_external`, contract-smoke deferred pending OQ-SM-4.

**Walking skeleton**: `server_starts_with_admin_key_from_aws_secrets_manager`
(`tests/secrets_management/acceptance/sm01_admin_key_secrets_manager.rs`) — real Postgres
container + real LocalStack container + real `embyr-server` binary subprocess + real HTTP
request bearing the fetched secret value. NOT `#[ignore]`. Litmus test: "Sam sources the
admin key from AWS Secrets Manager; the server starts and the fetched key works as the
operator Bearer token, with zero literal admin-key value in the process environment" — a
non-technical stakeholder can confirm this is what Sam needs (Dimension 5 compliant).

Verified RED (not BROKEN): ran the walking skeleton against real LocalStack + Postgres
containers. It fails at `wait_for_healthy` (30s timeout) because `ServerConfig::from_env()`
does not yet recognise `EMBYR_ADMIN_KEY_AWS_SECRET_ARN` — confirmed via direct binary
invocation that the failure is `missing required environment variable: EMBYR_ADMIN_KEY`
(MISSING_FUNCTIONALITY), not a test-infrastructure defect.

### Wave: DISTILL / [REF] Scenario list with tags

**US-SM-01 — `sm01_admin_key_secrets_manager.rs`** (6 scenarios, 1 WS)

| Scenario | Tags |
|---|---|
| `server_starts_with_admin_key_from_aws_secrets_manager` | `@walking_skeleton @driving_port @real-io @US-SM-01 @AC-SM-01-01` (NOT `#[ignore]`) |
| `server_starts_with_admin_key_from_gcp_secret_manager` | `@requires_external @US-SM-01 @AC-SM-01-02` (`#[ignore]`, blocked on OQ-SM-4) |
| `plain_env_var_still_works_when_no_arn_configured` | `@US-SM-01 @AC-SM-01-03 @backward-compat` |
| `startup_refuses_ambiguous_admin_key_sourcing` | `@error @US-SM-01 @AC-SM-01-04` |
| `exits_1_when_admin_key_secret_fetch_fails` | `@error @real-io @US-SM-01 @AC-SM-01-05` |
| `admin_key_secret_value_never_appears_in_logs_or_db` | `@error @real-io @US-SM-01 @security` |

**US-SM-02 — `sm02_encryption_key_secrets_manager.rs`** (6 scenarios)

| Scenario | Tags |
|---|---|
| `server_starts_with_encryption_key_from_aws_secrets_manager` | `@real-io @US-SM-02 @AC-SM-02-01` |
| `oidc_client_secret_encryption_succeeds_with_secrets_manager_sourced_key` | `@real-io @US-SM-02 @AC-SM-02-02 @AC-SM-02-06` |
| `plain_env_var_still_works_for_encryption_key` | `@US-SM-02 @AC-SM-02-03 @backward-compat` |
| `fetched_encryption_key_of_invalid_length_is_rejected_at_startup` | `@error @real-io @US-SM-02 @AC-SM-02-04` |
| `startup_refuses_ambiguous_encryption_key_sourcing` | `@error @US-SM-02 @AC-SM-02-05` |
| `exits_1_when_encryption_key_secret_fetch_fails` | `@error @real-io @US-SM-02 @AC-SM-02-05` |

**US-SM-03 — `sm03_encryption_key_rotation.rs`** (7 scenarios)

| Scenario | Tags |
|---|---|
| `totp_signin_decrypts_with_current_key` | `@real-io @US-SM-03 @AC-SM-03-04` |
| `totp_signin_decrypts_with_previous_key_during_rotation_window` | `@real-io @US-SM-03 @AC-SM-03-02` |
| `totp_signin_fails_when_neither_current_nor_previous_key_decrypts` | `@error @real-io @US-SM-03 @AC-SM-03-05` |
| `totp_signin_malformed_ciphertext_below_nonce_minimum_fails_cleanly` | `@error @real-io @US-SM-03 @AC-SM-03-05` |
| `startup_rejects_identical_current_and_previous_encryption_key` | `@error @US-SM-03 @AC-SM-03-06` |
| `decrypt_with_rotation_tries_current_before_previous` | `@in-memory @US-SM-03 @AC-SM-03-03` |
| `decrypt_with_rotation_works_against_oidc_and_dsn_shaped_ciphertext` | `@in-memory @US-SM-03 @AC-SM-03-03` |

**US-SM-04 — `sm04_admin_key_rotation.rs`** (8 scenarios)

| Scenario | Tags |
|---|---|
| `operator_route_accepts_current_admin_key` | `@real-io @US-SM-04 @AC-SM-04-04` |
| `operator_route_accepts_previous_admin_key_during_rotation_window` | `@real-io @US-SM-04 @AC-SM-04-02` |
| `metrics_endpoint_honors_dual_token_window` | `@real-io @US-SM-04 @AC-SM-04-03` |
| `operator_route_rejects_token_matching_neither` | `@error @real-io @US-SM-04 @AC-SM-04-06` |
| `hard_cutover_with_no_grace_period_remains_available` | `@error @real-io @US-SM-04 @AC-SM-04-04` |
| `startup_rejects_identical_current_and_previous_admin_key` | `@error @US-SM-04 @AC-SM-04-05` |
| `dual_auth_middleware_accepts_previous_admin_key_too` | `@real-io @US-SM-04 @AC-SM-04-02 @consistency-fix` |
| `retired_admin_token_rejected_once_rotation_window_closed` | `@error @real-io @US-SM-04 @AC-SM-04-07` |

**Total: 27 scenarios** (1 walking skeleton, 26 `#[ignore]`). Error/edge scenarios: 13/27 ≈
48% (target ≥40% met). Story traceability: all 4 stories (US-SM-01 through US-SM-04) have
scenarios tagged; all locked ACs (AC-SM-01-01 through AC-SM-04-07) have at least one tagged
scenario.

### Wave: DISTILL / [REF] Adapter coverage table

| Adapter | `@real-io` scenario | Covered by |
|---|---|---|
| `AwsSecretFetcher::get_raw_secret` (new method) | YES | Walking skeleton + `exits_1_when_admin_key_secret_fetch_fails` + `admin_key_secret_value_never_appears_in_logs_or_db` (sm01); `server_starts_with_encryption_key_from_aws_secrets_manager` + `fetched_encryption_key_of_invalid_length_is_rejected_at_startup` + `exits_1_when_encryption_key_secret_fetch_fails` (sm02) — all real LocalStack |
| `GcpSecretFetcher::get_raw_secret` (new method) | NO — blocked, documented | `server_starts_with_admin_key_from_gcp_secret_manager` scaffolded `#[ignore = "blocked on OQ-SM-4..."]` — `GcpSecretFetcher::new(base_url, ...)` has no env-var override reachable from `ServerConfig`, so no subprocess-level real I/O is possible without a DELIVER-wave (or follow-up feature) design addition. Per Mandate 6 exception for costly/blocked externals: enumerated, not skipped, `@requires_external`. |
| `adapters::encryption::decrypt_with_rotation` (new module) | N/A (`@in-memory` — pure function, no I/O) | `decrypt_with_rotation_tries_current_before_previous` + `decrypt_with_rotation_works_against_oidc_and_dsn_shaped_ciphertext` (direct calls — the function signature IS the driving port for a pure domain-shaped helper) |
| System Postgres (`SystemDb`) | YES | Every scenario in all 4 files uses a real `testcontainers-rs` Postgres 15-alpine container |
| `operator_auth_middleware` (extended) | YES | All of sm04's HTTP scenarios, real subprocess + real HTTP |
| `dual_auth_middleware` (extended, DESIGN-added) | YES | `dual_auth_middleware_accepts_previous_admin_key_too` |

Zero "NO — MISSING" rows remain; the one blocked row (GCP) is explicitly enumerated with a
named blocker (OQ-SM-4) per the DISTILL constraint to document rather than silently omit.

### Wave: DISTILL / [REF] Scaffolds (Mandate 7)

| File | Type | Marker |
|---|---|---|
| `crates/embyr-server/src/adapters/encryption.rs` | NEW module | `// SCAFFOLD: true`; `decrypt_with_rotation` panics with `"...not yet implemented -- RED scaffold..."`; `RotationDecryptError` enum fully defined (both variants used by tests) |
| `crates/embyr-server/src/adapters/mod.rs` | EXTEND (1 line) | `pub mod encryption;` added (alphabetical position, between `email` and `gcp_secret_fetcher`) |

No other `crates/` files were modified — `config.rs`, `admin/state.rs`, `admin/router.rs`,
`admin/middleware/operator_auth.rs`, `admin/middleware/dual_auth.rs`, `admin/handlers/auth.rs`,
`main.rs` are all EXTEND-classified in DESIGN but deliberately left untouched: every scenario
in this feature is subprocess-driven (env vars in, HTTP/exit-code/stderr out) or a direct call
to the one NEW module, so no other Rust-level import is required for the suite to compile.
This keeps DISTILL's footprint to "scaffold only" per the task's explicit constraint — DELIVER
implements the EXTEND changes during GREEN for each story.

Verified: `cargo test --no-run -p embyr-server --test secrets_management` compiles cleanly;
`cargo clippy -p embyr-server --test secrets_management -- -D warnings` passes with zero
warnings.

### Wave: DISTILL / [REF] Test placement

`tests/secrets_management/{mod.rs, common/mod.rs, acceptance/sm0{1,2,3,4}_*.rs}` — mirrors the
`tests/production_readiness/` layout exactly (single `mod.rs` root, `common/` harness,
`acceptance/` scenario files, one `[[test]] name = "secrets_management"` entry in
`crates/embyr-server/Cargo.toml`, placed immediately after the `production_readiness` entry).
The harness (`ServerProcess`, `find_free_port`, `start_postgres_container`,
`embyr_server_binary`) is an independent copy of `tests/production_readiness/common/mod.rs`'s
shape, not a cross-import — matches this workspace's established per-suite-harness precedent
(`tests/admin_api_v2/common/mod.rs` likewise does not import another suite's harness). New
harness additions specific to this feature: `start_localstack`/`make_sm_client`/
`create_raw_secret` (LocalStack, mirrors `tests/acceptance/us_10_aws_secrets.rs`),
`seed_totp_user`/`encrypt_totp_secret`/`corrupt_totp_secret`/`totp_code_now` (TOTP seeding,
mirrors `tests/admin_api_v2/common/mod.rs`'s `AdminTestContext` seeding).

### Wave: DISTILL / [REF] Driving Adapter coverage

The only driving adapter DESIGN specifies for this feature is the pre-existing
`embyr-server` binary's environment-variable-driven config resolution (no new CLI flags, no
new HTTP endpoints, no new hooks — System Constraint: "no new TCP listener ports, no new
gRPC/HTTP routes"). Every scenario in all 4 files exercises this path via real subprocess
invocation (`ServerProcess::start*`), never via direct Rust-level calls to
`ServerConfig::from_env()` — this is a deliberate DISTILL choice (see Scaffolds section)
that also satisfies the Driving Adapter Verification mandate: exit code, stdout/stderr
content, and env-var handling are all verified end-to-end through the actual binary a
Sam-shaped operator would run (`cargo run -p embyr-server`).

### Wave: DISTILL / [REF] Pre-requisites for DELIVER

1. `cargo build --bin embyr-server` before running any test in this suite (binary path
   resolution mirrors `tests/production_readiness/common/mod.rs`).
2. Docker daemon available — every scenario needs at least a Postgres testcontainer; most
   also need LocalStack.
3. DELIVER implements, per story, in this order (mirrors `story-map.md` release slicing):
   Release 1 (US-SM-01 → US-SM-02) → Release 2 (US-SM-03) → Release 3 (US-SM-04).
4. `oidc_client_secret_encryption_succeeds_with_secrets_manager_sourced_key` (sm02) needs a
   session-auth seeding helper (account/user/session row + cookie) before it can be unskipped
   — noted inline in the test as a DELIVER implementation note, mirroring
   `tests/admin_api_v2/common/mod.rs`'s `AdminTestContext` pattern.
5. `server_starts_with_admin_key_from_gcp_secret_manager` (sm01) is blocked on OQ-SM-4 — a
   test-only base-URL override for `GcpSecretFetcher` construction does not exist in
   `ServerConfig` today. DELIVER may either (a) add a minimal test-only override env var as
   part of this feature, or (b) leave the scenario `#[ignore]`d and file the follow-up
   feature the brief already recommends (§ Open Questions, OQ-SM-4).
6. `TEST_ENCRYPTION_KEY_PREVIOUS` (harness constant) is a fixed, arbitrary 64-hex-char value
   distinct from `TEST_ENCRYPTION_KEY` — verified via `hex::decode` round-trip during
   scaffolding (an off-by-one in an earlier draft, 62 vs 64 chars, was caught by running
   the in-memory `decrypt_with_rotation` scenarios and fixed before handoff).

### Wave: DISTILL / [REF] Pre-DELIVER fail-for-the-right-reason gate

Ran a representative sample against real infrastructure (Docker available in this
environment):

| Scenario | Classification | Evidence |
|---|---|---|
| `server_starts_with_admin_key_from_aws_secrets_manager` (WS) | MISSING_FUNCTIONALITY | Fails at `wait_for_healthy` (30s); direct binary invocation confirms `missing required environment variable: EMBYR_ADMIN_KEY` — `EMBYR_ADMIN_KEY_AWS_SECRET_ARN` not yet recognised |
| `plain_env_var_still_works_when_no_arn_configured` | PASSES already (not RED) | Backward-compat path (D-SM-2) is unmodified existing behavior — correctly green with zero DELIVER work; still `#[ignore]`d per one-at-a-time convention, trivially enabled first |
| `decrypt_with_rotation_tries_current_before_previous` | MISSING_FUNCTIONALITY | Panics with the scaffold's exact message: `"adapters::encryption::decrypt_with_rotation not yet implemented -- RED scaffold..."` |
| `decrypt_with_rotation_works_against_oidc_and_dsn_shaped_ciphertext` | MISSING_FUNCTIONALITY | Same scaffold panic |
| `startup_rejects_identical_current_and_previous_encryption_key` | MISSING_FUNCTIONALITY | Exit code assertion passes (process does exit 1, coincidentally via DB-connect failure); the specific `DuplicateRotationKey`-shaped stderr-message assertion correctly fails, since that validation does not exist yet |
| `startup_rejects_identical_current_and_previous_admin_key` | MISSING_FUNCTIONALITY | Same shape as above |

Zero scenarios in the sample failed for a test-infrastructure reason (no `IMPORT_ERROR`,
`FIXTURE_BROKEN`, or `SETUP_FAILURE`). Full-suite classification (all 27 scenarios) is
DELIVER's per-story RED-phase entry gate per ADR-025 — this sample establishes the harness
itself is sound across every code path it exercises (LocalStack secret creation, Postgres
seeding, subprocess spawn/health-check/HTTP, direct in-process calls).

### Wave: DISTILL / [REF] Self-Review Checklist

- [x] 1. WS strategy declared above (Architecture of Reference: driving = real adapter, AWS
      Secrets Manager = LocalStack per Project Infrastructure Policy)
- [x] 2. WS scenario tagged `@real-io`; all InMemory-shaped scenarios tagged `@in-memory`
- [x] 3. `AwsSecretFetcher` has `@real-io` coverage; `GcpSecretFetcher` documented as blocked
      (not silently missing); `decrypt_with_rotation` is `@in-memory` (pure function)
- [x] 4. N/A — no InMemory double used for a driven-internal port in this feature (System
      Postgres is real in every scenario)
- [x] 5. Container preference documented (testcontainers-rs Postgres + LocalStack, matching
      existing project policy)
- [x] 6. `adapters::encryption` scaffold created for the one new production module imported
- [x] 7. Driving Adapter: the only driving adapter (env-var-driven binary) has WS coverage
      via real subprocess invocation
- [x] 7 (Mandate 7 marker). `// SCAFFOLD: true` present in `encryption.rs`
- [x] 8. Scaffold method (`decrypt_with_rotation`) raises `panic!`, not `NotImplementedError`-shaped
- [x] 9. Verified RED (not BROKEN) — `cargo test --no-run` compiles; spot-run scenarios panic
      with the scaffold message or fail assertions, never `ImportError`-shaped
- [x] 11 (F-001). `@real-io @adapter-integration`-shaped coverage present for `AwsSecretFetcher`
- [x] 12 (F-002). N/A — no `capsys`-equivalent pattern in this Rust suite
- [x] 13 (F-005). N/A — Rust suite; no `des.adapters.driven.*` import boundary applies. The
      Rust-equivalent boundary (driving port only, never bypass to unimplemented internals) is
      honored: every scenario enters via the subprocess/HTTP surface or the one pure-function
      driving port
- [x] 14 (F-004). Timing budgets: `wait_for_healthy` 30s, `wait_for_exit` 10-15s — consistent
      with `tests/production_readiness/`'s established budgets for this exact test shape
- [x] 15 (F-003). N/A — no `sys.path` manipulation in Rust

### Wave: DISTILL / [REF] Mandate compliance evidence (CM-A/B/C/D/E/F/G/H)

- **CM-A** (Mandate 1, driving ports only): every scenario invokes either the `embyr-server`
  binary subprocess (env vars in, HTTP/exit-code/stderr out) or
  `adapters::encryption::decrypt_with_rotation` directly (the function signature IS the
  driving port for this pure helper, per `nw-tdd-methodology`'s domain-function exception).
  Zero imports of internal validators/parsers/middleware internals.
- **CM-B** (Mandate 2, business language): scenario/function names use domain terms
  (`admin_key`, `rotation_window`, `signin`, `operator_route`) — technical terms (HTTP status
  codes, "AEAD", "AES-GCM") appear only inside doc comments and step BODIES, never in Gherkin
  (no `.feature` files in this Rust suite — the polyglot matrix's Rust idiom is
  `<feature>_scenarios.rs` function names carrying the business narrative, matching the
  existing `pr01_config_from_env.rs`/`b01_auth_migrations.rs` precedent in this workspace).
- **CM-C** (Mandate 3, complete journeys): every scenario has a documented Given/When/Then in
  its doc comment tracing to a UAT scenario in `user-stories.md`; the walking skeleton
  demonstrates full user value (Sam's admin key works end-to-end with zero literal value in
  the manifest).
- **CM-D** (Mandate 4, pure function extraction): `decrypt_with_rotation` IS the pure-function
  extraction this feature's DESIGN specifies (ADR-018 §5) — tested directly with zero fixture
  ceremony (`decrypt_with_rotation_tries_current_before_previous`). Impure code (AWS/GCP
  fetch, subprocess spawn) is isolated behind the `ServerProcess`/LocalStack harness.
- **CM-E** (Mandate 8, Universe-bound state-delta): applied to the HTTP-response-mutating
  scenarios in sm03/sm04 (signin outcome, operator-route status) via `assert_state_delta`
  with port-exposed Universe entries (`response.signin.status_code`,
  `response.operator_route.status_code`). The exit-code/stderr config-validation scenarios in
  sm01/sm02 use plain `assert_eq!`/`assert!`, matching the established
  `pr01_config_from_env.rs` precedent for this exact test shape (subprocess exit-code checks
  have no meaningful before/after Universe to declare beyond the single exit-code slot).
- **CM-F** (Mandate 9, layer-dependent PBT): no `@given`/`RuleBasedStateMachine` used anywhere
  in this suite — every scenario is layer 3+ (subprocess/FS acceptance or `@in-memory`
  pure-function direct call), correctly example-only per Mandate 9.
- **CM-G** (Mandate 10, two-tier acceptance): Tier B (state-machine PBT) is NOT applicable —
  this feature is config-shaped (startup resolution + two auth/decrypt call sites), matching
  the explicit "Skip Tier B" criterion ("the only observable is exit-code/HTTP-status, no
  rich domain-input state machine to model"). Tier A only, as declared.
- **CM-H** (Mandate 11, example-based sad paths): all 13 error-tagged scenarios are named,
  explicit examples (`exits_1_when_...`, `startup_rejects_...`, `..._fails_cleanly`) — zero
  PBT machinery imported.

---
