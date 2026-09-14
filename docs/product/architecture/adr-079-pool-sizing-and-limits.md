# ADR-079: Pool Sizing and Acquire-Timeout Configurability — Per-Role Env Vars, Additive Constructors, No New Concurrency Limiter (Yet)

## Status

Accepted

## Context

Audit finding #16 (High, Reliability) and #30 (Medium, Database — same root cause, confirmed by
direct read of `docs/product/production-readiness-audit-2026-09-08.md` line 40): `embyr-server`'s
3 long-running Postgres pools (`system_db.rs`, `backend_adapter.rs`, `handler.rs`'s `handle_listen`)
hardcode `max_connections`, and 2 of the 3 have no `acquire_timeout` at all (sqlx default 30s).
A saturated tenant pool queues silently for up to 30s before erroring — indistinguishable from a
full outage. DISCUSS (`docs/feature/pool-sizing-and-limits/feature-delta.md`) locked scope to these
3 `embyr-server`-resident sites, reusing `ServerConfig`'s established optional-numeric-env-var
convention (`EMBYR_CAP_CHECK_INTERVAL_SECS` and siblings), and left 3 questions open for DESIGN:
naming (per-role vs global env vars), whether a separate concurrency limiter is also required, and
whether `embyr-db-prep` should be pulled into scope.

**DESIGN's own reading found what DISCUSS's did not**: `PostgresBackendAdapter::new(&dsn)` is not
called only from `handler.rs`'s 3 `authenticate()` branches. `crates/embyr-server/src/adapters/
project_auth.rs::resolve_customer_db_adapter` (client-auth-hosted-identity, ADR-036 Decision 7) is a
4th and 5th call site (lines 89, 156) using the SAME `CredentialCache`, same cache-key shape, same
"cache miss builds a fresh pool" mechanism — a genuine 2 additional instances of the identical
root cause DISCUSS scoped, reached via hosted-identity's REST signup/signin/reset path rather than
the gRPC Firestore path. Grepping every caller of `PostgresBackendAdapter::new` (10+ sites) and
`SystemDb::new` (60+ sites, nearly every integration test file plus `main.rs`) also confirmed a
signature change to either constructor is not viable — see Decision 2.

## Decision

### 1. Per-role environment variables (resolves OQ-PSL-01)

Five new `EMBYR_*`-prefixed, optional, numeric env vars, parsed via one shared helper
(`parse_positive_u32`, mirroring `parse_port`'s existing shape in `crates/embyr-server/src/
config.rs`) reused 5 times — non-numeric or non-positive is `ConfigError::InvalidPoolConfig`,
never a silent fallback (AC-PSL-05):

| Env var | Default | Site |
|---|---|---|
| `EMBYR_SYSTEM_DB_MAX_CONNECTIONS` | 5 | `system_db.rs` (`acquire_timeout` stays hardcoded 5s — DISCUSS's handoff scopes this site to `max_connections` only) |
| `EMBYR_TENANT_DB_MAX_CONNECTIONS` | 5 | `backend_adapter.rs` construction, all 5 per-tenant call sites |
| `EMBYR_TENANT_DB_ACQUIRE_TIMEOUT_SECS` | 5 | same 5 call sites |
| `EMBYR_LISTENER_DB_MAX_CONNECTIONS` | 2 | `handler.rs`'s `handle_listen` ad hoc pool |
| `EMBYR_LISTENER_DB_ACQUIRE_TIMEOUT_SECS` | 5 | same site |

All defaults preserve today's exact behavior byte-for-byte when unset (AC-PSL-01).

### 2. Additive constructors — `SystemDb::new`/`PostgresBackendAdapter::new` are never touched

`SystemDb::new(&str)` has 60+ callers (nearly every integration test file, `main.rs`,
`system_db.rs`'s own unit tests). `PostgresBackendAdapter::new(&str)` has 10+ callers outside this
feature's scope (`transaction_sweeper.rs`, `composite_index_builder.rs`, `admin/handlers/
composite_indexes.rs`, `embyr-db-prep/main.rs`, 5+ acceptance-test files) — all structurally
different, short-lived, one-off, non-tenant-contended pool constructions this feature does not
touch. Changing either constructor's signature is a breaking change with no offsetting benefit.

Both types gain a new, additive `with_pool_config(dsn, max_connections, acquire_timeout)`
constructor. `new()` keeps its exact current hardcoded behavior for every existing caller. Only the
5 genuine per-tenant call sites (`handler.rs` × 3, `project_auth.rs` × 2) and the 1 system-pool
production-startup call site switch to the new constructors.

### 3. No new concurrency limiter this story (resolves OQ-PSL-02)

Right-sized pools + a short, env-overridable `acquire_timeout` are sufficient to satisfy the
finding's own stated outcome — "fast clean error instead of 30s tail latency" — and are directly
proven by AC-PSL-04's real saturated-pool integration test. A pre-emptive concurrency limiter
(semaphore/tower admission gate shedding load before pool acquisition) is a materially larger,
separate architectural commitment: it requires deciding scope (per-tenant vs per-process), error
semantics, and interaction with `distributed-rate-limiting`'s existing pre-`authenticate()`
`RESOURCE_EXHAUSTED` rejection path — none of which DISCUSS scoped or has real production
concurrency telemetry to justify today. Bundling it into this story would retroactively invalidate
DISCUSS's own Elephant Carpaccio PASS verdict (mechanical repetition of one fix shape). Deferred as
a recommended follow-up finding, not silently dropped.

### 4. `embyr-db-prep` stays out of scope (confirms OQ-PSL-03)

Read `crates/embyr-db-prep/src/main.rs` directly: its pool-open call is already wrapped in an outer
`tokio::time::timeout(Duration::from_secs(10), ...)`, and its `adapter.probe()` call is independently
wrapped the same way. One-shot, single-caller, no concurrent-tenant contention possible. DISCUSS's
exclusion stands — no contrary evidence found.

### 5. Scope correction — `project_auth.rs` added

`resolve_customer_db_adapter`'s 2 `PostgresBackendAdapter::new(&dsn)` call sites (lines 89, 156) are
added to this story's build target, alongside `handler.rs`'s 3. Same `CredentialCache`, same root
cause, same fix shape — a widening of site count within the already-locked scope, not a new concern.

## Alternatives Considered

### OQ-PSL-01: one global pair of vars (rejected)

A single `EMBYR_DB_POOL_MAX_CONNECTIONS`/`EMBYR_DB_POOL_ACQUIRE_TIMEOUT_SECS` pair applied uniformly
is simpler for the operator (one knob) but the system pool has already independently chosen a
different concurrency profile (5 connections, 5s timeout, documented rationale) from the listener
pool (2 connections) — forcing them to move together contradicts the system pool's own existing,
load-bearing precedent and would make a legitimate resize of one role silently resize an unrelated
role.

### OQ-PSL-02: add a concurrency limiter now (rejected)

See Decision 3. Also considered: rely solely on `distributed-rate-limiting`'s existing RPS limiter
without any pool change (rejected — a different failure boundary entirely: pre-`authenticate()`
cluster-wide request-rate rejection, not post-auth per-tenant connection-pool exhaustion; does not
bound how long an already-admitted request queues for a connection).

### Constructor evolution: change `new()` signatures directly (rejected)

Machine-verified blast radius (60+ and 10+ callers respectively) makes this a breaking change with
no offsetting benefit over an additive constructor. Also considered a `PoolConfig` builder struct
passed to one unified constructor (rejected — two `u32`/`Duration` parameters per site do not
warrant a new type; simplest-solution-first).

## Consequences

### Positive
- Every `embyr-server`-resident pool a saturated tenant could queue against fails fast, is
  operator-sizeable without a rebuild, and preserves today's behavior byte-for-byte when unset.
- Zero breaking changes: `SystemDb::new`/`PostgresBackendAdapter::new` and every existing caller
  (60+ and 10+ respectively) are untouched.
- Closes findings #16 and #30 together — same root cause, same fix.

### Negative / Trade-offs
- 5 new env vars is more operator-facing surface than one global knob (accepted — see OQ-PSL-01
  alternatives).
- A saturated tenant's requests still burn a queued task for up to `acquire_timeout` before failing
  (bounded, not eliminated) — a pre-emptive concurrency limiter would shed load earlier. Deferred;
  recommend filing a follow-up finding once real production concurrency telemetry exists, scoped
  against `distributed-rate-limiting`'s existing `RESOURCE_EXHAUSTED` semantics for consistency.
- `FirestoreService` (handler.rs) and its ~9 struct-literal construction sites in `lib.rs`/`main.rs`
  each gain 4 new fields; test-server constructors initialize them to today's hardcoded defaults
  (5/5/2/5), a mechanical, behavior-preserving addition.

## Enforcement

- Unit tests on `parse_positive_u32` (non-numeric, zero, negative-as-string, absent-uses-default),
  mirroring `config.rs`'s existing `parse_port`/`parse_rate_limit_rps` test style.
- AC-PSL-04/AC-PSL-06: real testcontainers-Postgres saturated-pool integration test — the
  fault-injection proof this feature's fast-fail contract rests on (Earned Trust: a saturated pool
  is the substrate lie this feature must survive with a bounded, typed error, not a silent 30s hang).
- No `dependency-cruiser`/layering enforcement needed — this feature does not change any
  hexagonal/port boundary, only pool-construction parameters and their sourcing.
