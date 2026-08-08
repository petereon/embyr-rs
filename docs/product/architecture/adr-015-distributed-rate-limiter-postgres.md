# ADR-015: Distributed Rate Limiter — Postgres Token Bucket with In-Process Fallback

## Status

Accepted

## Context

embyr-rs enforces per-project rate limits to prevent any single tenant from starving others.
The existing implementation (`crates/embyr-server/src/middleware/rate_limit.rs`) uses an
in-process `HashMap<ProjectId, TokenBucket>` — one per embyr instance.

This architecture has a specific horizontal scaling defect: with N embyr instances behind a
load balancer, a project configured for 1000 RPS can actually fire N × 1000 RPS cluster-wide
before any single node rejects a request. Sam Chen (service operator) cannot offer tenants a
meaningful rate-limit SLA in a multi-node deployment.

The DISCUSS wave locked decision D1: the coordination backend is the existing Postgres system
DB — no new coordination plane (Redis, Kafka, distributed cache) is introduced. The `rate_buckets`
table in the system DB becomes the shared enforcement point.

Related locked decisions:
- D2: Algorithm — atomic UPDATE with token replenishment via `EXTRACT(EPOCH FROM ...)`
- D3: Failure mode — fail open with in-process fallback, 20ms timeout hard-coded
- D4: Configuration via `EMBYR_RATE_LIMIT_RPS` env var (default 1000), operator-wide
- D7: Return type `Result<RateLimitInfo, RateLimitInfo>` — both arms carry rate-limit info
- D9: Admin port `:9090` is NOT rate-limited
- D10: `rate_buckets` schema — `DOUBLE PRECISION` to match existing `f64`

## Decision

**Extend `RateLimiter` with a Postgres path; retain `TokenBucket` as the in-process fallback.
No port/trait interface is added.**

### Concrete struct, not a port interface

`RateLimiter` remains a concrete struct. A trait interface was considered and rejected (see
Alternatives). The rationale: rate limiting has exactly one implementation — Postgres-backed
distributed enforcement with in-process fallback. There is no test-double swap required; tests
use `RateLimiter::disabled()` or `RateLimiter::new(capacity, refill_rate, None)` (in-process-only
mode). Adding a trait interface would introduce indirection with no concrete benefit at this time.

### `RateLimitInfo` domain type in `embyr-core`

A new module `embyr-core::rate_limit` carries the `RateLimitInfo` struct:

```
pub struct RateLimitInfo {
    pub remaining: f64,   // tokens remaining after this request; floor at 0.0
    pub limit: f64,       // configured capacity (EMBYR_RATE_LIMIT_RPS)
    pub reset_ms: u64,    // epoch ms when >= 1 token will be available
}
```

This is a pure value type with zero IO imports. It lives in `embyr-core` (not `embyr-server`)
so it remains importable by any future crate (e.g., admin metrics display) without pulling in IO
dependencies. `embyr-core`'s existing `deny.toml` constraint (`no tokio, sqlx, tonic, axum`)
is preserved.

### `RateLimiter::check()` signature change

```
// Before: Result<(), ()>
// After:  Result<RateLimitInfo, RateLimitInfo>
async fn check(&self, project_id: &str) -> Result<RateLimitInfo, RateLimitInfo>
```

`Ok(info)` — request allowed; `info.remaining` = tokens left after consuming 1.
`Err(info)` — request rejected; `info.remaining = 0.0`; `info.reset_ms` = when to retry.

Both arms carry `RateLimitInfo` so that response headers can be attached unconditionally at
every call site regardless of outcome (D7).

### `RateLimiter::new()` parameter change

```
// Before:
fn new(capacity: f64, refill_rate: f64) -> Arc<Self>

// After:
fn new(capacity: f64, refill_rate: f64, pg_pool: Option<Arc<PgPool>>) -> Arc<Self>
```

When `pg_pool = None` (used by all existing test constructors and `RateLimiter::disabled()`):
in-process fallback only. When `pg_pool = Some(pool)`: attempts the Postgres path first.
Existing `start_test_server_with_rate_limit()` passes `None` — backward compatible with US-14.

### Decision path on each `check()` call

1. If `!self.enabled` → `Ok(RateLimitInfo { remaining: capacity, limit: capacity, reset_ms: 0 })`
2. If `pg_pool.is_some()`:
   a. `tokio::time::timeout(Duration::from_millis(20), check_pg(project_id)).await`
   b. `Ok(Ok(info))` within 20ms → return `info` (distributed Postgres path)
   c. `Err(_)` (20ms elapsed) → `metrics::counter!("rate_limit_pg_timeout_total").increment(1)` → step 3
   d. `Ok(Err(sqlx_error))` (hard DB error within 20ms) → log WARN → step 3
3. In-process bucket: `HashMap<String, TokenBucket>` — get or insert bucket for `project_id`; call `try_consume()`; return `Ok(info)` or `Err(info)`.

The 20ms timeout is **hard-coded** — it is not a configuration parameter (D3).

### `check_pg()` internal method — return type explanation

```
async fn check_pg(&self, project_id: &str) -> Result<Result<RateLimitInfo, RateLimitInfo>, sqlx::Error>
```

The nested `Result` carries three semantically distinct outcomes. DELIVER must handle all three arms:

```rust
match self.check_pg(project_id).await {
    // 1 row returned — Postgres allowed the request
    Ok(Ok(info)) => return Ok(info),
    // 0 rows returned — Postgres rejected (bucket empty)
    Ok(Err(info)) => return Err(info),
    // Postgres error (connection failure, query error) — fall through to in-process fallback
    Err(e) => {
        tracing::warn!("rate_buckets UPDATE error: {e}; using in-process fallback");
        // continue to in-process bucket (step 3 in check() decision path)
    }
}
```

**Why nested `Result` over a three-variant enum:** the outer `Result<_, sqlx::Error>` mirrors the
existing sqlx calling convention throughout the codebase (every other sqlx call site uses the same
shape). The inner `Result<RateLimitInfo, RateLimitInfo>` mirrors the public `check()` return type,
making the allowed/rejected semantics symmetric. A custom enum (`RateBucketOutcome::Allowed | Rejected | Error`)
was considered but rejected: it introduces a new private type with no meaning beyond this method,
adding ceremony without clarity. If the crafter finds the nesting confusing during implementation,
they may introduce a private enum — this is a DELIVER-internal refactoring decision.

**Important:** the `Ok(Err(sqlx_error))` arm in the decision path (step 2d in `check()`) is for
hard errors. The timeout arm (`Err(_)` from `tokio::time::timeout`) is a different signal: it fires
when the Future did not complete within 20ms — this is the `Err(tokio::time::Elapsed)` from
`timeout()` itself, wrapping the inner future. The crafter must not conflate `Err(Elapsed)` from
the outer timeout with `Err(sqlx::Error)` from the inner future.

### Atomic UPDATE SQL

```sql
UPDATE rate_buckets
SET tokens = LEAST($1::float8,
                   tokens + EXTRACT(EPOCH FROM (now() - last_refill)) * $2::float8
             ) - 1.0,
    last_refill = now()
WHERE project_id = $3
  AND tokens + EXTRACT(EPOCH FROM (now() - last_refill)) * $2::float8 >= 1.0
RETURNING tokens;
```

- `$1` = capacity (f64), `$2` = refill_rate (f64), `$3` = project_id (VARCHAR).
- Single round-trip; no SELECT + UPDATE race condition.
- Row-level lock on `project_id` PK only — no cross-project contention.
- `RETURNING tokens` provides post-deduction count for `RateLimitInfo.remaining`.
- 0 rows returned ↔ rate limited. 1 row returned ↔ allowed.

### `rate_buckets` table (`migrations/0018_rate_buckets.sql`)

```sql
CREATE TABLE rate_buckets (
    project_id  VARCHAR(63)      NOT NULL PRIMARY KEY
                                 REFERENCES projects(id) ON DELETE CASCADE,
    tokens      DOUBLE PRECISION NOT NULL,
    last_refill TIMESTAMPTZ      NOT NULL
);

INSERT INTO rate_buckets (project_id, tokens, last_refill)
SELECT id, 1000.0, now()
FROM   projects
WHERE  status IN ('active', 'suspended')
ON CONFLICT DO NOTHING;
```

The backfill with `1000.0` (the prior hardcoded default) ensures existing projects have a row
before the updated `check_pg()` path is active, removing the ambiguity between "rate limited"
(row exists, no tokens) and "row missing" (would also return 0 rows). Operators running
`EMBYR_RATE_LIMIT_RPS != 1000` should update backfilled rows with
`UPDATE rate_buckets SET tokens = <new_capacity> WHERE true;` after migration.

The `ON DELETE CASCADE` FK ensures project hard-deletion (sweeper at 168h) automatically
removes the rate bucket row — no sweeper code change required.

### Provisioning integration

The `provision()` handler currently uses autocommit INSERT for each backend mode. With D8 
(rate bucket created atomically with project), both INSERTs must share a `sqlx::Transaction`.
Failure of the `rate_buckets` INSERT rolls back the `projects` INSERT — no orphaned rows.

`OperatorState` gains `rate_limit_capacity: f64` so the provision handler knows the initial
token count without re-reading `EMBYR_RATE_LIMIT_RPS` at request time. Populated at the
composition root from the same `capacity` value used to construct `RateLimiter`.

### gRPC trailing metadata — tonic 0.12

tonic 0.12 provides:
- `Response<T>::metadata_mut()` → trailing metadata for unary handlers (HTTP/2 HEADERS after data frame).
- `Response<BoxStream>::metadata_mut()` → initial metadata for streaming handlers (first HEADERS frame).
- `Status::metadata_mut()` → trailing metadata on error HEADERS frame (both unary and streaming).

A free function `attach_rate_limit_headers(md: &mut MetadataMap, info: &RateLimitInfo)` sets
`x-ratelimit-limit`, `x-ratelimit-remaining`, `x-ratelimit-reset` on every response. A second
helper `attach_retry_after(md, info, refill_rate)` adds `retry-after-ms` on rejection only.
Both live in `crates/embyr-server/src/middleware/rate_limit.rs`. They are called inline at
each of the 9 call sites in `handler.rs` (D6) — not in tower middleware.

### Metric

`rate_limit_pg_timeout_total` (counter) increments on each 20ms timeout. Uses the existing
`metrics` crate (0.22.x), exported via the Prometheus endpoint on the admin port. Hard Postgres
errors do not increment this counter — they are logged at WARN level.

### `EMBYR_RATE_LIMIT_RPS` env var

Read once at startup; default `1000.0`. Replaces three hardcoded `RateLimiter::new(1000.0, 1000.0)`
calls at lines 174, 231, and 280 of `crates/embyr-server/src/lib.rs`. Changing the value requires
a process restart (D4).

## Alternatives Considered

### Alternative 1: Redis as distributed coordination backend (rejected)

Redis pub/sub or a Redis atomic counter (`INCR` + `EXPIRE`) would provide distributed rate limiting
without a SQL round-trip. Typical Redis latency: 0.1–0.5ms vs. Postgres 1–5ms.

**Rejected because:**
- Introduces a new infrastructure dependency. embyr-rs's Operational Simplicity quality attribute
  (rank 6) is explicitly "no coordination plane (no Redis, no Kafka, no Zookeeper)." Adding Redis
  solely for rate limiting contradicts this design principle.
- The system DB (Postgres) is already a required dependency; adding Redis creates a second point of
  failure with no corresponding benefit.
- The 20ms timeout + in-process fallback means Postgres latency on the hot path is capped at 20ms
  regardless of Postgres performance. Redis's latency advantage over Postgres does not justify the
  operational cost.
- The existing `rate_buckets` UPDATE stays under 5ms in normal operation. p99 < 500µs is the guardrail
  (AC-14c); this is achievable with Postgres for row-level UPDATE operations on a small table.

### Alternative 2: Gossip / eventually-consistent counter across nodes (rejected)

Nodes could gossip per-project request counts using a CRDT (G-Counter) and merge periodically.
Each node enforces limits locally using the merged view.

**Rejected because:**
- Requires implementing or importing a gossip protocol and CRDT library — significant complexity
  and a new operational surface.
- "Eventually consistent" limits are a different guarantee from the "cluster-wide enforcement"
  that D1 requires. Gossip introduces a convergence delay window where limits are exceeded.
- No existing Rust gossip library is mature enough for production use without significant evaluation.
- Overkill: the same guarantee is achievable with a single Postgres UPDATE round-trip.

### Alternative 3: Fixed-window counter (rejected)

Use a simple `rate_buckets(project_id, window_start TIMESTAMPTZ, count BIGINT)` table. Increment
`count` on each request; reject when `count > limit`.

**Rejected because:**
- Fixed-window counting allows a 2× burst at window boundaries: a project can fire N requests at
  the end of window K and N more at the start of window K+1, for 2N requests in a short period.
- Token bucket (D2) does not have this property — the refill is continuous and smooth.
- Fixed window would require a `DELETE` sweep for expired rows; token bucket avoids this via
  the `last_refill` timestamp in each row.

### Alternative 4: Port/trait interface for `RateLimiter` (rejected)

Define `trait RateLimitPort` in `embyr-core::rate_limit` and implement `struct PgRateLimiter`
in `embyr-server`.

**Rejected because:**
- Rate limiting is not a domain concept that requires swappable implementations. The current and
  foreseeable future is one implementation: Postgres primary + in-process fallback.
- Test isolation is already achieved via `RateLimiter::disabled()` and `pg_pool = None`. A trait
  adds indirection (dynamic dispatch on the hot path) with no testability benefit.
- If a future requirement genuinely demands a swappable implementation (e.g., Redis backend for a
  specific deployment tier), a trait can be extracted at that time with a targeted ADR. YAGNI applies.

## Consequences

### Positive

- Cluster-wide rate enforcement: all embyr instances share `rate_buckets` rows, enforcing limits
  regardless of node count. Sam's 3-node cluster enforces 1000 RPS total, not 3000 RPS.
- Graceful degradation: 20ms timeout + in-process fallback means rate limiting never becomes a
  blocking failure mode. Postgres unavailability downgrades to per-instance enforcement (1× cap,
  not ∞).
- No new infrastructure dependency: Postgres is already required. `rate_buckets` is a single
  small table with one row per active project.
- Backfill migration ensures zero gap for existing projects between migration deploy and
  distributed enforcement.
- FK `ON DELETE CASCADE` keeps `rate_buckets` consistent with project lifecycle automatically.
- `RateLimitInfo` in `embyr-core` is importable by any future crate without IO coupling.
- Machine-readable headers (`x-ratelimit-*`, `retry-after-ms`) enable SDK-side adaptive backoff
  (Alex's use case) without any protocol changes.

### Negative / Trade-offs

- Postgres round-trip on every gRPC call: ~1–5ms added to the hot path when Postgres is healthy.
  The 20ms timeout is the hard absolute bound on the Postgres path. **AC-14c scope change:**
  - **AC-14c (existing, preserved for fallback path):** p99 of `RateLimiter::check()` < 500µs when Postgres
    is unavailable and the in-process `TokenBucket` is used. This guarantees degraded mode has no
    latency cost beyond the failed timeout wait (which is bounded at 20ms itself, not part of the
    in-process bucket path — the timeout fires asynchronously). The property test assertion
    `p99 < 500µs` applies ONLY to the in-process fallback code path.
  - **AC-14c-new (Postgres path):** No separate p99 SLA is defined for the Postgres UPDATE path.
    The hard guarantee is: `check()` NEVER exceeds 20ms due to the `tokio::time::timeout` gate,
    regardless of Postgres latency. Expected latency under normal load: < 5ms. If the
    `rate_limit_pg_timeout_total` counter increments frequently (> 1% of requests), the operator
    should investigate Postgres performance (long-running queries, lock contention, connection pool
    exhaustion). The 20ms timeout is the only contractual bound.
  - **Property test responsibility:** The existing US-14 property test (AC-14c) must be updated to
    run in in-process mode only (`pg_pool = None`). A new latency test for the Postgres path should
    assert `check()` completes within 20ms under simulated load (not a p99 assertion — a worst-case bound).
- Backfill uses hardcoded `1000.0`. Operators with a non-default `EMBYR_RATE_LIMIT_RPS` must
  manually update backfilled rows. This is a one-time operational step documented in the migration.
- `provision.rs` now uses explicit transactions. This is a small increase in complexity but is
  the correct pattern for atomicity.
- `OperatorState` gains a new field. All call sites constructing `OperatorState` must be updated
  at the composition root.
- tonic 0.12 metadata semantics differ between unary and streaming handlers (trailing vs. initial
  metadata). For streaming handlers, `x-ratelimit-*` headers land in the initial metadata frame,
  not the trailing frame. This is a V1 acceptable asymmetry; the Firebase SDK reads both.

## Enforcement

- `embyr-core::rate_limit` boundary: `cargo-deny` + existing `deny.toml` enforces zero IO imports.
- `rate_buckets` migration runs at startup as part of `sqlx-migrate` before any listener opens —
  the migration is the structural proof that the table exists before `check_pg()` is called.
- Behavioral: `tests/acceptance/us_drl_03_graceful_fallback.rs` (CI) injects a 500ms Postgres delay
  and asserts the 20ms timeout fires with fallback activation and counter increment. This is the
  Earned Trust behavioral layer for the Postgres dependency.
