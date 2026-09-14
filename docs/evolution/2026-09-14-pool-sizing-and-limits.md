# Evolution: pool-sizing-and-limits

**Date:** 2026-09-14
**Feature:** All 3 embyr-server-resident Postgres pools (system, per-tenant, listener) get
env-var-overridable sizing plus a real `acquire_timeout` — a saturated pool now fails fast
with a clean error instead of a 30s tail-latency hang, without changing any existing
constructor signature.
**ADR:** `docs/product/architecture/adr-079-pool-sizing-and-limits.md`

## This closes finding #16 AND finding #30 from `docs/product/production-readiness-audit-2026-09-08.md`

Same root cause, confirmed by DESIGN's own reading of #30's cited lines against this
story's scope before claiming the closure.

## Business Context

Postgres pool sizing was hardcoded tiny (`max_connections(5)` or `(2)`) at every site,
with no env override, and 2 of the 3 pools had no `acquire_timeout` at all (sqlx's 30s
default applied). A saturated tenant's requests queued for up to 30 seconds before
failing, instead of failing fast with a clean error — and there was no way to right-size
any pool for a given deployment without a code change.

## Key Decisions

| Decision | Verdict |
|---|---|
| Scope | ONE story covering all 3 `embyr-server`-resident pools (system, per-tenant, listener) as mechanical repetition of one `PgPoolOptions` fix shape. `embyr-db-prep` (a 4th site cited by the audit) explicitly excluded — one-shot DBA tool, different persona/job, already has an adequate outer 10s timeout. |
| Env vars (OQ-PSL-01) | Per-pool-role, not one global pair: `EMBYR_SYSTEM_DB_MAX_CONNECTIONS`, `EMBYR_TENANT_DB_MAX_CONNECTIONS`/`_ACQUIRE_TIMEOUT_SECS`, `EMBYR_LISTENER_DB_MAX_CONNECTIONS`/`_ACQUIRE_TIMEOUT_SECS` — the system pool's own independent 5s acquire_timeout precedent and the listener's smaller natural size are real evidence the roles shouldn't move together. |
| Concurrency limiter (OQ-PSL-02) | NOT added — right-sized pools + short acquire_timeout directly satisfy the finding's own stated outcome (proven by the saturation test). A pre-emptive limiter is a materially larger, separate architectural commitment, deferred as a recommended follow-up, not silently dropped. |
| Constructor evolution | Additive only — `SystemDb::new` (60+ callers) and `PostgresBackendAdapter::new` (8+ callers) are byte-for-byte unchanged. New `with_pool_config(...)` variants added alongside, used only at the real production/per-tenant call sites. |
| Invalid env var value | Startup failure (process exit), never a silent fallback to default — `ConfigError::InvalidPoolConfig`. |
| ADR | New ADR-079 — OQ-PSL-01 and OQ-PSL-02 each carry genuine rejected alternatives needing a durable record, matching this session's own precedent for `ServerConfig`-extension features with a real decision inside. |
| DESIGN found 2 extra call sites | `project_auth.rs`'s `resolve_customer_db_adapter` (used by 4 REST handlers: sign_up, sign_in_with_password, reset_password x2) uses the same `CredentialCache`/pool mechanism as `authenticate()` but was never cited by the audit or DISCUSS's own initial grep — caught before DELIVER, not after. |

## Steps Completed

Full nWave subagent pipeline (DISCUSS=`nw-product-owner`, DESIGN=`nw-solution-architect` with peer review, DISTILL=`nw-acceptance-designer` with peer review, DELIVER=`nw-software-crafter`):

1. **DISCUSS**: read all 4 audit-cited sites directly, corrected the audit's own claim (the system pool already HAD an `acquire_timeout(5s)` — the gap was only no env override there). DoR 9/9.
2. **DESIGN**: resolved all 3 open questions, wrote ADR-079, found the 2 additional `project_auth.rs` call sites DISCUSS missed. Peer review: approved, 0 critical/high, 1 medium fixed inline (WS path-coverage clarity), 1 low non-blocking.
3. **DISTILL**: 12 scenarios (1 WS + 11 focused). Built a genuine saturation test using an external, uncommitted `SELECT ... FOR UPDATE` against the tenant's real Postgres to hold a pool connection busy (the SUT's own OCC `MustExist` check takes the identical lock) — 2 dead ends found and documented along the way (an empty `transaction: vec![]` isn't an implicit Commit; whole-table locking blocks at the wrong layer to distinguish pool-exhaustion from query-blocking). RED-verified: 8/12 failed for the right reason (`MISSING_FUNCTIONALITY`), 3 documented regression guards. Interrupted once by a session rate limit, cleanly resumed. Peer review: approved, 0 blockers, 9.3/10 average.
4. **DELIVER**: implemented exactly as designed (commit `547d475`). Found and fixed a genuine test-helper bug along the way: a shared `begin_transaction()` test helper unconditionally `.expect()`'d success on the (now-falsified) assumption that `BeginTransaction`'s own INSERT never contends with a saturated pool — once `max_connections` was genuinely wired small, that assumption broke legitimately. Converted to `Result` + `?`, no acceptance-test assertion touched. 11/11 target scenarios + 82/24 unit tests across both crates green.
5. **Independent verification**: fresh subagent re-confirmed all 8 code-level claims (especially that both existing constructors are byte-for-byte unchanged, and that the test-helper fix is legitimate, not a loosened assertion) and re-ran all scoped tests clean.
6. **QUALITY_GATE**: 26 mutants in `embyr-server`, 6 caught (all in the new `parse_positive_u32`/`ConfigError` decision logic — the feature's actual new logic), 20 unviable (100% compiler-rejected, verified per-mutant via compile logs, not assumed). `embyr-pg-storage`'s own equivalent mutant was not run as a separate pass — direct compiler evidence from the embyr-server run already proved it unviable, and its real logic is exercised live by the saturation test; documented as a deliberate, evidence-backed decision. Commit `567b43e`.

## Lessons Learned

1. **A mid-session machine-resource near-miss, caught and corrected in real time.** The QUALITY_GATE subagent's first attempt used `cargo mutants --workspace --test-workspace true` plus a raw `cargo test --workspace` — a direct violation of the standing "never run workspace-wide cargo" constraint, reached for because the diff spans 2 crates and `--workspace` looked like the obvious way to cover both in one pass. Caught via a direct `ps aux` check showing the literal flag, both processes killed immediately (`kill -9`), agent corrected and resumed. Root cause fixed on the second attempt: `-C`/`--cargo-arg` (which scopes cargo-mutants' OWN build step, unlike positional args after `--`) cut the per-mutant build from 276s to 9s by restricting to the one relevant test target. See [[feedback_machine_resource_constraints]] — the lesson generalized: "never run X --workspace" doesn't automatically register as covering every flag spelling that reintroduces workspace-wide scope; multi-crate diffs need this said explicitly.
2. **A 2-extra-call-site catch during DESIGN (not DELIVER, not QUALITY_GATE) is cheaper than catching it later** — `project_auth.rs`'s `resolve_customer_db_adapter` shares the exact same vulnerability class as `authenticate()` but wasn't in the audit's own citation list. Worth a standing habit for cross-cutting findings: always grep for OTHER usages of the same underlying pattern/type, not just the sites the audit happened to name.
3. **A test helper's own hardcoded assumption can become false once the fix it's testing actually works** — the `begin_transaction().expect()` bug only surfaced because pool saturation became genuinely reachable for the first time. A useful signal: if a test helper's doc comment states an assumption ("X never happens"), treat it as a hypothesis to re-verify once your own fix could plausibly falsify it, not a fact to trust blindly.

## Key Files

- `crates/embyr-server/src/config.rs` — `parse_positive_u32`, `ConfigError::InvalidPoolConfig`, 5 new `ServerConfig` fields.
- `crates/embyr-server/src/adapters/system_db.rs`, `crates/embyr-pg-storage/src/backend_adapter.rs` — `with_pool_config` additive constructors.
- `crates/embyr-server/src/grpc/handler.rs`, `crates/embyr-server/src/adapters/project_auth.rs` — the 6 real call sites.
- `crates/embyr-server/src/rest/{sign_up,sign_in_with_password,reset_password}.rs` — `HostedIdentityState` threading.
- `tests/production_readiness/acceptance/{pr11,pr12,pr13}_pool_sizing_*.rs` (new, 12 scenarios).
- `docs/product/architecture/adr-079-pool-sizing-and-limits.md`.
- `docs/feature/pool-sizing-and-limits/deliver/mutation/mutation-report.md`.

## Follow-Up Work

- Finding #17+ (High) remain — next in audit order.
- A pre-emptive concurrency limiter (shedding load before it queues) — explicitly deferred, not built, cross-referenced to the `distributed-rate-limiting` work as a natural extension point if this becomes a real need.
