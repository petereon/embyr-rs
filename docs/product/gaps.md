# Gaps — single source of truth

Updated 2026-11-18 against master `af0300c`. This file replaces four earlier standalone trackers (the 8-item known-gaps list, the 2026-09-08 production-readiness audit, the 2026-11-13 go-live assessment and the 2026-11-13 ignored-test triage), all deleted; their identifiers are mapped in the [Closed index](#5-closed--id-index).

**How to maintain**
- One row per gap. Never create another gap-tracking file; a feature's own "Known gaps" section stays in its `feature-delta.md` and is linked from [section 6](#6-where-other-limits-are-documented).
- Close a gap by deleting its row and adding one line to the Closed index with the PR / ADR / evolution doc.
- Mark anything you did not verify against code as `(unverified)`. Do not guess.
- Effort: S = under a day, M = a few days, L = a week or more.

**Priority**
| P | Meaning |
|---|---|
| P0 | Security, data loss, or blocks any launch |
| P1 | Wrong results or operational risk for real clients / beta |
| P2 | Quality, observability, missing verification |
| P3 | Deferred / nice to have |

Verified-by-reading on 2026-11-18, not executed. No code-level P0 exists; the P0 rows are owner actions and the never-run deploy bundle.

---

## 1. Open gaps (code and docs)

Sorted by priority, then effort.

| ID | Gap | Area | Evidence / location | P | Effort | Who | Notes / next step |
|---|---|---|---|---|---|---|---|
| G-001 | Integer and double values are ordered / compared through `float8`, inexact above 2^53; ORDER BY has no implicit document-name tiebreak, so ties page non-deterministically | Query correctness | `crates/embyr-pg-storage/src/encoding/query.rs:236-246` (`ponytail:` comment), `backend_adapter.rs:977-986` (no `__name__` appended) | P1 | M | agent-buildable | Append `__name__` ASC as the final sort key (Firestore semantics) and a `numeric`/bigint path for large integers. Silent wrong order/duplicates across pages. |
| G-002 | Each REST hosted-identity request on a credential-cache hit builds a new `PostgresBackendAdapter` (new pool, handshake) to the customer DB | Reliability / DoS | `crates/embyr-server/src/adapters/project_auth.rs:92-100` (go-live X11, never listed as fixed) | P1 | M | agent-buildable | Cache the adapter next to the credential. `(unverified)` whether the pool connects eagerly; the construction per request is confirmed. |
| G-003 | Per-project `log_retention_days` is not enforced: the sweeper drops `query_logs_*` partitions older than the MAX retention across all live projects (default 90 d) | Privacy / compliance | `crates/embyr-server/src/adapters/query_log.rs:178-186`; [legal README](../legal/README.md) "Gaps to resolve" says no sweeper exists (it does, but not per project) | P1 | M | needs owner decision | Decide the real default, then delete per-project rows or make the policy text say "max of project settings". Legal text must match. |
| G-004 | TOTP has no replay guard (a used code is valid again in its window); failure counter atomicity `(unverified)` | Admin auth | go-live X13; no replay/last-used-step logic in `crates/embyr-server/src/admin/handlers/auth.rs` | P2 | S | agent-buildable | Store last accepted time-step per user; make the counter update a single SQL statement. |
| G-005 | Agent-version skew: an agent older than `DocChange.old_fields` withholds rule-gated deletes (fail closed); SUM/AVG against an older agent return `Unimplemented` | Agent mode (BYOC) | [agent-mode-listen](../feature/agent-mode-listen/feature-delta.md) Known gaps; `docs/SPEC.md:721`; [runbook](../operations/runbook.md) | P2 | S | owner action | Roll out the current agent release to BYOC customers; no code change. |
| G-006 | Hosted-identity ID tokens (1 h) are not revoked on password reset (stateless) | Security | [runbook](../operations/runbook.md) "Security notes for operators"; go-live X13 | P2 | M | agent-buildable | Per-user "valid after" timestamp checked on each request. |
| G-007 | Open Listen streams do not re-evaluate on rule change, identity deletion, or API-key rotation/revocation; they stay open until the client closes | Security / realtime | [api-key-rotation](../feature/api-key-rotation/feature-delta.md) "Out of scope" (AC-AKR-15); go-live X13 | P2 | M | agent-buildable | Close or re-authorize streams on rotate/revoke and rules import. Cross-replica invalidation is bounded by the credential-cache TTL. |
| G-008 | No image signing / SBOM; Trivy scan is report-only (`exit-code: "0"`) | Supply chain | `.github/workflows/ci.yml:312-317` | P2 | M | agent-buildable | Gate on HIGH/CRITICAL once the baseline is clean; add cosign + SBOM at release. |
| G-009 | `direct_pg` projects with only a legacy ECIES DSN (`backend_pg_dsn_enc IS NULL`) are skipped by the transaction sweeper and TTL sweeper; lazy convert-on-request never converts idle projects | Reliability | [ADR-055](architecture/adr-055-transaction-sweeper-backend-pg-dsn-enc-coverage-gap.md); [dsn-single-store](../feature/dsn-single-store/feature-delta.md) | P2 | M | agent-buildable | Check the conversion gauge; decide an owner-triggered conversion path or a forced re-save prompt. `(unverified)` how many rows remain in production. |
| G-010 | A valid-charset but unprovisioned `project_id` still costs 3 DB round trips pre-auth | Reliability | audit #14 residual / #49 (tracking-only); `OQ-PDA-02` in [preauth-db-amplification](../feature/preauth-db-amplification/discuss/feature-delta.md) | P2 | M | agent-buildable | Existence-check cache or bloom filter. Feature-sized, not a patch. |
| G-011 | GCP Secret Manager auth is a static bearer token, no in-process refresh (about 1 h lifetime) | Secrets | [secrets-manager-setup](../operations/secrets-manager-setup.md) section 4, ADR-018 A6, OQ-SM-4 | P2 | M | agent-buildable | Metadata-server / workload-identity token acquisition. |
| G-012 | BrowserChannel session state is in-process: needs sticky routing, no sharing across replicas, no resume after restart; the `SID` is not routable | Realtime / web SDK | [browser-channel](../feature/browser-channel/feature-delta.md) NOT supported; [network-topology](../operations/network-topology.md) options 1-4 | P2 | L | needs owner decision | Fine on one replica. Decide launch stance (see O-15); proper fix is a per-replica `SID` prefix routed by the LB. |
| G-013 | SLIs with no metric yet: Listen delivery latency + open streams, admin/HTTP request metrics (:9090, hosted-identity REST), `listener`/bounded `project_id` labels on `embyr_grpc_*`, tenant/listener pool gauges, `/healthz` as a metric, pre-handler failures (LB), `process_start_time_seconds` | Observability | [slo.md](../operations/slo.md) "Flagged: SLIs with no metric yet" | P2 | L | agent-buildable | SLI 3 cannot exist without the Listen histogram; do that first. |
| G-014 | SPA `update(Msg::DeleteServiceAccount)` does not cascade-remove the account's admin keys (server already revokes them) | Admin UI model | `tests/user_admin_ui/acceptance/slice_04_identity_admin_keys.rs:195` (`#[ignore = "REAL GAP..."]`); `crates/embyr-admin-ui/src/update.rs` | P3 | S | agent-buildable | Server is correct (`service_accounts.rs:208-237`); fix the model and un-ignore, or delete the dead test if the legacy simulation goes (G-022). |
| G-015 | Docker image is ~150 MB vs documented < 100 MB (now also includes the SPA) | Packaging | `tests/production_readiness/acceptance/pr02_dockerfile.rs:135`; `docs/feature/production-readiness/feature-delta.md` (AC-PR-02-03) | P3 | S | needs owner decision | Cheapest: change the documented limit and the test. Shrinking needs a distroless/slimmer runtime base. |
| G-016 | GCP secret fetcher base URL is a hard-coded constant: no env override, so the local-emulator test cannot run | Testability | `crates/embyr-server/src/config.rs:56` (`GCP_SECRET_MANAGER_BASE_URL`); `tests/secrets_management/acceptance/sm01_admin_key_secrets_manager.rs:102` (OQ-SM-4) | P3 | S | agent-buildable | Add an env override read at startup, then un-ignore the sm01 GCP test. |
| G-017 | No self-service account restore during the deletion grace window (operator clears `deleted_at` manually) | Account lifecycle | [account-deletion](../feature/account-deletion/feature-delta.md) schema section | P3 | S | needs owner decision | `POST /account/restore` valid until purge, password + TOTP. |
| G-018 | `GET /admin_keys` returns neither `service_account_id` nor `member_id` (no "bound to" column); hosted identity has no read endpoint (UI remembers enablement per session) | Admin API / UI | [admin-ui-api-wiring](../feature/admin-ui-api-wiring/feature-delta.md) section 10; `crates/embyr-server/src/admin/handlers/admin_keys.rs:40-47` | P3 | S | agent-buildable | Add fields to the list DTO and a status read for hosted identity. |
| G-019 | Hourly rollup does not feed the existing daily upsert from the same buffer | Metrics | [hourly-metrics-rollup](../feature/hourly-metrics-rollup/feature-delta.md) section 4 | P3 | S | agent-buildable | Optional cost reduction. |
| G-020 | Stale text: "All tests are #[ignore] (RED)" doc comments in `tests/card_payments/**`, `tests/admin_api_v2/**`, `tests/card_payments_backend/**`, `tests/user_admin_ui/**`; admin-ui-api-wiring section 10 delivery status and the agent-mode-listen "remove runbook limitation" note are out of date | Doc hygiene | `rg "All tests are #\[ignore\]" tests` | P3 | S | agent-buildable | Comment-only cleanup. |
| G-021 | Standing design note: `panic!()` guards in the hot query path (`append_field_filter`, `order_by_expr*`, cursor blocks) convert a future unvalidated caller bug into a task panic | Rust design | audit #57 (acknowledged, no change); `crates/embyr-pg-storage/src/encoding/query.rs` | P3 | S | needs owner decision | Reconsider only if a caller with unvalidated input appears. |
| G-022 | Legacy card-payments simulation still in the SPA model (`SetCard`, `SetPlan`, `SetInvoices`, `SetPaymentFailure`, `Card`, `Invoice`, `data::{FREE_CAPS, PRICING, ...}`) and its `tests/card_payments` suite | Dead code | `crates/embyr-admin-ui/src/msg.rs:177-179`, `data.rs`; `tests/card_payments/**` | P3 | M | needs owner decision | Retire the model code together with the whole suite, or keep as a reference. |
| G-023 | Pending contract migrations: drop `sdk_api_keys`, legacy ECIES signing-key column, `ecies_encrypted_dsn`, `agent_tls_bundle_enc` | Schema | [api-key-rotation](../feature/api-key-rotation/feature-delta.md) out of scope; [dsn-single-store](../feature/dsn-single-store/feature-delta.md) contract step; [migration-policy](../operations/migration-policy.md) | P3 | M | agent-buildable | Gated: only after the conversion gauge has been 0 for a release (see G-009). |
| G-024 | `read_time` on reads is ignored (no point-in-time reads) | Protocol | `crates/embyr-server/src/grpc/handler.rs:3689` | P3 | M | needs owner decision | Only if a client needs it. |
| G-025 | TTL policies: agent mode unsupported (API 409, sweeper skips), top-level fields only, deletion latency up to a sweep interval, deletions not metered | TTL | [ttl-policies](../feature/ttl-policies/feature-delta.md) Limits | P3 | M | agent-buildable | Agent mode needs `run_query` with `all_descendants` and `limit`. |
| G-026 | Agent mode Listen: clients see RESET after any agent-stream reconnect; `AgentNotifyBridge` has no reconnect of its own (each server reconnect opens a new agent-side PG connection) | Agent mode | [agent-mode-listen](../feature/agent-mode-listen/feature-delta.md) Known gaps; [runbook](../operations/runbook.md) | P3 | M | agent-buildable | Correct but noisy. |
| G-027 | BrowserChannel not supported: key/value map messages, test-channel phase 2, `TYPE=html`, `$req`/POST-over-GET, compression, legacy `Write.transform`, per-call gRPC metrics | Web SDK | [browser-channel](../feature/browser-channel/feature-delta.md) NOT supported | P3 | M | agent-buildable | Unverified against a real SDK first (V-05). |
| G-028 | Vector search has no index: exact in-memory kNN, 20,000-candidate scan cap, no pushdown in agent mode | Vector search | [vector-search](../feature/vector-search/feature-delta.md) Limits, Future upgrade path | P3 | L | agent-buildable | Optional pgvector index; agent protocol would need a `find_nearest` field. |
| G-029 | Server-side TLS only; no mTLS on the three listeners (deferred, no evidenced need) | Transport | KG-5 close note; [firestore-tls-support](../feature/firestore-tls-support/feature-delta.md) | P3 | M | needs owner decision | |
| G-030 | SSO follow-ups: no owner-facing audit view / read API, no back-channel logout, SSO-only members cannot self-link or delete the account (no fresh-login SSO reauth), no JIT provisioning, no SAML, one account per person | Admin SSO | [admin-oidc-sso](../feature/admin-oidc-sso/feature-delta.md) section 9 decisions O2-O7 | P3 | L | needs owner decision | Each is a deliberate V1 omission. |
| G-031 | `chrono` unmaintained advisory (RUSTSEC-2026-0543) ignored in `deny.toml`; other accepted advisories: pin-utils, rustls-pemfile, RUSTSEC-2026-0441, dev-only rsa Marvin (2023-0071) and tokio-tar (2025-0111) | Dependencies | `deny.toml` `[advisories] ignore` | P3 | L | agent-buildable | Migrating chrono to jiff/time is a large, deferred task. `cargo deny` is green on master (CI run 2026-11-18). |

---

## 2. Owner actions and decisions

| ID | Item | Why / source | P | Who | Next step |
|---|---|---|---|---|---|
| O-01 | Choose hosting platform, region, managed Postgres tier, LB, TLS termination, DNS names | [topology.md](../operations/topology.md) section 10 checklist (go-live O4) | P0 | needs owner decision | Everything in topology.md is platform-neutral until this is chosen. |
| O-02 | Provision the VPS and domain for the closed-beta single-box deploy | [single-vps-deploy](../operations/single-vps-deploy.md) section 0 | P0 | owner action | Blocks V-01. |
| O-03 | Create the first git tag and first CHANGELOG release | go-live O1: no tags exist (`git tag` empty), documented rollback cannot work without one | P0 | owner action | See [release-process](../operations/release-process.md). |
| O-04 | Lawyer review of `terms-of-service.md` / `privacy-policy.md`; fill every `[BRACKETED]` placeholder (`[COMPANY]`, `[ADDRESS]`, `[JURISDICTION]`, `[SUPPORT_EMAIL]`, `[PRIVACY_EMAIL]`, IČO / `[FULL NAME]`, `[REGION]`, `[LOG_RETENTION]`, `[SLA]`) | [legal README](../legal/README.md); Slovak sole-trader notes | P0 | owner action | Do not publish before review. `[LOG_RETENTION]` depends on G-003. |
| O-05 | Trade licence (živnosť) or company, VAT position, Stripe account in the right name; timing before charging anyone | [legal README](../legal/README.md) "Before charging anyone" | P1 | owner action | |
| O-06 | Support and privacy mailboxes that exist and are monitored; publish ToS, privacy and support pages | go-live S7 (no support contact) | P1 | owner action | |
| O-07 | Set the Stripe Pro product tax code (test and live) or disable Managed Payments by default | `tests/card_payments_backend/acceptance/cpb08_checkout_portal_sessions.rs:216` ignored: Stripe returns 400 "product tax code is missing" | P1 | owner action | Then run cpb08 with `--ignored`; also unblocks real Checkout. |
| O-08 | Verify ghcr package visibility for `embyr-server` / `embyr-agent` images | go-live O7 "unverified"; `gh api` could not confirm | P1 | owner action | |
| O-09 | Choose SMTP provider; set `EMBYR_PUBLIC_BASE_URL`, `EMBYR_REQUIRE_SMTP`, Stripe live keys | topology.md checklist 7 | P1 | owner action | |
| O-10 | Alert delivery target and on-call wiring (rules exist in `alert-rules.yml`) | go-live O6 remainder; topology.md checklist 9 | P1 | owner action | Test `EmbyrTargetDown` by stopping a replica. |
| O-11 | `EMBYR_ADMIN_KEY` must be at least 32 characters: the server refuses to start otherwise (deploy-time breaking change from PR #66) | `crates/embyr-server/src/config.rs:278` | P1 | owner action | Rotate any shorter key before deploying; see [runbook](../operations/runbook.md). |
| O-12 | Name the second IdP for SSO contract checks (recommended: Microsoft Entra, tenant-specific issuer); Google is the first | [admin-oidc-sso](../feature/admin-oidc-sso/feature-delta.md) O1 | P1 | needs owner decision | Feeds V-04. |
| O-13 | Ratify `slo.md` numbers after the first load test; remove "proposed" status | [slo.md](../operations/slo.md) | P2 | needs owner decision | After V-02. |
| O-14 | Free-plan project-count rule (code has a Free cap but no project-count cap); plan terms `[FREE_PLAN_LIMITS]`, `[PRO_PRICE]` | go-live "needs the owner"; legal README | P2 | needs owner decision | |
| O-15 | BrowserChannel at launch: support web-SDK realtime fallback or not; if yes, which stickiness option | [topology.md](../operations/topology.md) checklist 4; G-012 | P2 | needs owner decision | |
| O-16 | Decisions that gate code rows: default query-log retention (G-003), image size limit vs shrink (G-015), mTLS (G-029), retire card simulation (G-022) | this file | P2 | needs owner decision | |
| O-17 | BYOC customer guidance: DB `max_connections` headroom, TLS, egress IP allow-list; per-tier `EMBYR_RATE_LIMIT_RPS`, max projects per customer DB, `EMBYR_TRUSTED_PROXY_HOPS` | topology.md checklist 10-12 | P2 | needs owner decision | |
| O-18 | Secrets store (AWS vs GCP), who can read it, admin-key rotation owner | topology.md checklist 6 | P2 | needs owner decision | |
| O-19 | Trademark: check use of "Firestore" / "Google" in marketing (terms say API-compatible only) | [legal README](../legal/README.md) | P3 | owner action | |

---

## 3. Verification debt (implemented, never run against reality)

| ID | Item | Evidence | P | Who | Next step |
|---|---|---|---|---|---|
| V-01 | `deploy/single-vps/` bundle (compose, Caddyfile, scripts) was never executed; no Docker/Caddy/shellcheck at authoring time | [single-vps-deploy](../operations/single-vps-deploy.md) status banner | P0 | owner action | First real run on O-02; fix small things found, then drop the banner. |
| V-02 | Load test, SLO targets, pool/resource sizing, RTO/RPO, rolling deploy, replica/AZ kill, PITR restore drill all unvalidated | [loadtest/](../../loadtest/README.md), [topology.md](../operations/topology.md) "Do before go-live", [slo.md](../operations/slo.md), [backup-disaster-recovery](../operations/backup-disaster-recovery.md) | P1 | owner action | Run `ramp` + `soak` + bad-key scenario; record results in slo.md. |
| V-03 | Admin SPA screens never opened in a real browser: `web-sys` fetch transport, Leptos views, CSP and cookie behaviour unverified | [admin-ui-api-wiring](../feature/admin-ui-api-wiring/feature-delta.md) section 9 | P1 | owner action | Manual walkthrough of each screen on a deployed build. |
| V-04 | OIDC SSO never exercised against a real IdP (only a mock IdP in tests) | [admin-oidc-sso](../feature/admin-oidc-sso/feature-delta.md) O1 | P1 | owner action | Contract check against Google plus the O-12 IdP; record results in the slice 3 PR notes. |
| V-05 | BrowserChannel never verified against a real firebase-js-sdk client | [browser-channel](../feature/browser-channel/feature-delta.md) last NOT supported bullet | P1 | owner action | Run the web SDK with `experimentalForceLongPolling`. |
| V-06 | Vector `find_nearest` base-query restrictions were not cross-checked against live Firestore | [vector-search](../feature/vector-search/feature-delta.md) Limits | P3 | owner action | |
| V-07 | Many Docker/testcontainers-backed acceptance tests are only ever run in CI (8 GB laptop cannot run them); master CI was green on 2026-11-18 | `.github/workflows/ci.yml` | P3 | owner action | Accept as policy; local runs only for scoped targets. |
| V-08 | Remaining `#[ignore = ...]` attributes that are environment or structural, not product gaps | grouped, see list below | P3 | n/a | Keep ignored. |

V-08 detail: `cpb05_usage_metering` `a_single_projects_stripe_failure_does_not_abort_the_whole_run` (real Stripe fixture cannot fail 1 of 3 projects sharing one customer); `drp02_compose_lifecycle` (real docker compose, about 8 min, runs only with `--ignored`); `pr02_dockerfile` x5 (shared image tag race / needs pre-built image / warm layer cache / full `docker build`; the CI `docker` job already builds the image); 4 structural `full_*_regression_suite_passes_unmodified` markers in `security_rules`, `security_rules_cel_parity`, `security_rules_query_path`, `security_rules_write_path`. Not in this group: `cpb08` (O-07), `sm01` GCP (G-016), `pr02 docker_image_under_100mb` (G-015), `slice_04` (G-014).

---

## 4. Counts (at 2026-11-18)

| Table | P0 | P1 | P2 | P3 | Total |
|---|---|---|---|---|---|
| Open gaps (G) | 0 | 3 | 10 | 18 | 31 |
| Owner actions and decisions (O) | 4 | 8 | 6 | 1 | 19 |
| Verification debt (V; V-08 is a grouped note) | 1 | 4 | 0 | 3 | 8 |

---

## 5. Closed — ID index

Old identifiers still referenced from evolution docs, feature deltas, ADRs, Rust comments and `#[ignore]` reasons resolve here. `evo:` = `docs/evolution/`. `PR #n` = merged pull request on master. `verify` = closure could not be confirmed.

### known-gaps #1-#8 (originally 2026-09-06)

| ID | Closed by |
|---|---|
| KG-1 | evo:2026-09-06-firestore-transaction-read-consistency |
| KG-2 | evo:2026-09-07-firestore-end-cursor-support |
| KG-3 | evo:2026-09-07-firestore-or-filter-support |
| KG-4 | evo:2026-09-07-firestore-is-null-filter-support |
| KG-5 | evo:2026-09-08-firestore-tls-support (server-side TLS only; mTLS is G-029) |
| KG-6 | Stale finding: graceful shutdown already in `main.rs` (commit 6f58e89); later hardened by PR #21 and PR #68 |
| KG-7 | evo:2026-09-08-secrets-management-test-reliability |
| KG-8 | evo:2026-09-08-security-rules-cel-chaining-detection |

### production-readiness audit #1-#71 (2026-09-08 scan plus follow-up scans)

| ID | Closed by |
|---|---|
| #1 | evo:2026-09-08-stripe-webhook-secret-required |
| #2 | evo:2026-09-09-rate-limiter-project-id-validation (ADR-069) |
| #3 | evo:2026-09-09-stripe-webhook-body-limit (ADR-070) |
| #4 | evo:2026-09-09-realtime-listener-reconnect (ADR-071) |
| #5 | evo:2026-09-09-composite-index-real-creation (ADR-072) |
| #6 | evo:2026-09-09-soft-delete-purge-sweeper (ADR-073) |
| #7 | evo:2026-09-09-wire-secret-fetchers |
| #8 | evo:2026-09-09-embyr-agent-release-pipeline (ADR-074) |
| #9 | evo:2026-09-10-occ-precondition-validation |
| #10 | evo:2026-09-12-sanitize-backend-error-messages (ADR-075) |
| #11 | evo:2026-09-12-agent-field-path-validation |
| #12, #13 | evo:2026-09-13-admin-signin-hardening (ADR-076); #13 bounded, not eliminated |
| #14 | evo:2026-09-13-preauth-db-amplification; residual is G-010 |
| #15 | evo:2026-09-14-healthz-dependency-checks (ADR-078) |
| #16, #30 | evo:2026-09-14-pool-sizing-and-limits (ADR-079) |
| #17 | evo:2026-09-14-collection-group-query-index (ADR-080) |
| #18 | evo:2026-09-15-backup-disaster-recovery-docs |
| #19 | evo:2026-09-15-deployment-release-process (K8s/Helm/ECS deferred, see O-01) |
| #20 | evo:2026-09-15-rate-limiter-fail-open |
| #21 | evo:2026-09-15-cors-origin-policy |
| #22 | evo:2026-09-15-tls-startup-warning |
| #23 | evo:2026-09-15-scheduled-cargo-deny |
| #24 | evo:2026-09-19-request-body-size-limits (ADR-081) |
| #25 | evo:2026-09-19-structured-json-logging |
| #26 | evo:2026-09-19-field-path-defense-in-depth |
| #27 | evo:2026-09-19-dependency-version-dedup (thiserror/tower/testcontainers duplicates deferred) |
| #28 | evo:2026-09-19-env-var-documentation |
| #29 | evo:2026-09-19-observability-runbook |
| #31, #39 | commit fa3db3f (unused BLAKE3 suffix removed; event-type label bounded) |
| #32 | Already fixed in admin-signin-hardening (commit 5a54e22) |
| #33, #40 | commit d32366b; evo:2026-09-19-ecies-kdf-domain-separation |
| #34, #35, #38 | Verified no change needed (2026-09-19) |
| #36 | `#[allow(clippy::option_env_unwrap)]` with comment (2026-09-19) |
| #37 | Unit tests added, commit 74b43a1 |
| #41, #54, #55 | Clippy / `deny.toml` nits: commits b8bbb68, d03235f |
| #42, #43 | evo:2026-09-19-container-registry-publishing |
| #44 | evo:2026-09-19-timestamp-cursor-pagination |
| #45, #46 | evo:2026-09-19-deployment-operator-runbooks |
| #47 | commit e578098 (`embyr_sweeper_error_total`, alert) |
| #48 | commit 6491b73; `provision.rs:514` calls `backfill_collection_id` (verified in code 2026-11-18) |
| #49 | Tracking-only; residual is G-010 |
| #50, #51 | commit 2a98631 |
| #52, #53 | evo:2026-09-19-resource-sizing-network-topology |
| #56 | commit 720d7da; `signin_rate_limit.rs:102` counter present (verified in code 2026-11-18) |
| #57 | Acknowledged, no code change; standing note is G-021 |
| Bloat list ("delete:" / "yagni:" items) | commit 7e0d848 removed the dead code |
| #58, #59, #60 | PR #14; #58 superseded by ADR-086 SSO (PRs #84-#91) |
| #61 | PR #11 (process-wide Argon2 gate) |
| #62 | PR #26 (ADR-083, single server-key store) |
| #63 | PR #25 (account bootstrap and invitations) |
| #64 | PR #66 (`projects.suspended_by`, migration 0049) |
| #65 | PR #26 (DSN SSRF/TLS policy, `EMBYR_ALLOW_PRIVATE_DSN_HOSTS`) |
| #66 | PR #66 (`MAX_LOG_RETENTION_DAYS`, name length) |
| #67, #68 | PR #74 (TargetNotMember; Listen composite-index gate) |
| #69 | PR #66 (`describe_*!` registered; the obs01 `# HELP` test is no longer ignored) |
| #70 | OPEN as G-015 |
| #71 | Server side closed (`service_accounts.rs` revokes keys in the delete transaction); SPA model is G-014 |

### go-live assessment 2026-11-13 (P/S/O/X)

| ID | Closed by |
|---|---|
| P1 | PR #19 (parent-path scoping of RunQuery, aggregation, Listen) |
| P2 | PR #40 (multiplexed Listen targets, Documents targets, windows) |
| P3 | PR #57 (BrowserChannel); real-SDK check is V-05, limits G-012/G-027 |
| P4 | PR #37 (multiple aggregations, `up_to`); PR #77 (agent SUM/AVG) |
| P5 | PR #82 (vector), PR #83 (TTL), PR #76 (PartitionQuery); `read_time` is G-024 |
| P6, P7, P8 | PR #32 |
| P9 | `handler.rs:5059-5060` maps `OccConflict` / `TransactionAborted` to ABORTED; not traced against a real SDK retry (verify) |
| P10 | Fixed during the X1 work (PR #20) |
| P11 | PRs #43, #44; residual is G-001 |
| S1, S2 | PR #25; accept screen PR #59 |
| S3, S4 | PRs #33, #50, #51, #56, #59, #63 (SPA served from `/admin/`, Dockerfile UI stage) |
| S5 | PR #14, replaced by ADR-086 |
| S6 | PR #26 |
| S7 | PR #49, #53 (password reset); drafts PR #71, #80; support contact is O-06 |
| S8 | PR #38 (Checkout/Portal), PR #36 (account deletion), PR #39 (hourly metrics, verify the hardcoded-0 latency fix), PR #66 (#64) |
| O1 | PR #21 (immutable tags); first tag is O-03 |
| O2 | PR #54 (migration policy + lint), PR #55 and #61 (`ignore_missing`) |
| O3 | PR #21, PR #68 (`EMBYR_SHUTDOWN_DELAY_SECS`) |
| O4 | Docs only, PR #67; decision is O-01 |
| O5 | k6 load test + proposed SLO, PR #67; never run, see V-02 / O-13 |
| O6 | PR #21 (service-down alerts); delivery is O-10 |
| O7 | PR #54 (SHA-pinned actions, report-only Trivy); G-008, O-08 |
| O8 | PR #21 (`EMBYR_REQUIRE_SMTP`); GCP token is G-011 |
| X1 | PRs #20, #22 |
| X2 | PRs #27, #28, #30, #34 (ADR-085) |
| X3 | PR #18 |
| X4, X6 | PR #23 |
| X5, X8 | PR #31 |
| X7 | PR #14 (audit #58-60), PR #66 (#64), PR #26 (#65), PR #85 (JWKS timeout, per AC in feature-delta; verify) |
| X9, X12 | PR #35, PR #74 |
| X10 | PR #26 |
| X11 | OPEN as G-002 |
| X13 | Partly: role caps, `EMBYR_ADMIN_KEY` minimum, foreign-project 404 (PR #66); remaining lows are G-004, G-006, G-007 (the `?key=` query-string note is documented in the runbook) |
| CI sharding | PRs #48, #52 |

### ignored-test triage 2026-11-13

| Item | Resolution |
|---|---|
| 118 `#[ignore]` triage | PR #16 un-ignored 75, implemented 21, deleted 6; REAL GAPs: Listen index gate (closed, audit #68), `# HELP` (closed, #69), image size (G-015), SPA cascade (G-014); env/structural remainder is V-08 |

---

## 6. Where other limits are documented

Primary documents keep their own limits sections; they are not duplicated here.

| Topic | Document |
|---|---|
| Agent-mode Listen | [feature-delta](../feature/agent-mode-listen/feature-delta.md), [runbook](../operations/runbook.md) "Known limitations" |
| BrowserChannel | [feature-delta](../feature/browser-channel/feature-delta.md), [network-topology](../operations/network-topology.md) |
| TTL policies | [feature-delta](../feature/ttl-policies/feature-delta.md) Limits |
| Vector search | [feature-delta](../feature/vector-search/feature-delta.md) Limits |
| Admin SSO | [feature-delta](../feature/admin-oidc-sso/feature-delta.md), ADR-086 |
| API key rotation | [feature-delta](../feature/api-key-rotation/feature-delta.md) Out of scope, ADR-085 |
| DSN single store | [feature-delta](../feature/dsn-single-store/feature-delta.md), ADR-083, [ADR-055](architecture/adr-055-transaction-sweeper-backend-pg-dsn-enc-coverage-gap.md) |
| Admin UI wiring | [feature-delta](../feature/admin-ui-api-wiring/feature-delta.md) |
| Account deletion / bootstrap | [account-deletion](../feature/account-deletion/feature-delta.md), [account-bootstrap-and-invitations](../feature/account-bootstrap-and-invitations/feature-delta.md) |
| SLOs and missing SLIs | [slo.md](../operations/slo.md) |
| Topology, launch checklist | [topology.md](../operations/topology.md), [network-topology.md](../operations/network-topology.md), [resource-sizing.md](../operations/resource-sizing.md) |
| Single-VPS deploy | [single-vps-deploy.md](../operations/single-vps-deploy.md), [single-host-deployment.md](../operations/single-host-deployment.md) |
| Migrations, releases, backup | [migration-policy.md](../operations/migration-policy.md), [release-process.md](../operations/release-process.md), [backup-disaster-recovery.md](../operations/backup-disaster-recovery.md) |
| Secrets manager | [secrets-manager-setup.md](../operations/secrets-manager-setup.md) |
| Legal drafts | [legal/README.md](../legal/README.md) |
| Protocol behaviour and limits | [SPEC.md](../SPEC.md) |
| History of every closed item | `docs/evolution/` |
