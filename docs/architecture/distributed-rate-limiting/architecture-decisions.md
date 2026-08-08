# Architecture Decisions — distributed-rate-limiting

> Extracted from `feature-delta.md` DESIGN wave sections. Permanent reference.
> ADR-015: `docs/product/architecture/adr-015-distributed-rate-limiter-postgres.md`
> Evolution: `docs/evolution/2026-08-08-distributed-rate-limiting.md`

---

## Component Decomposition

> Mode: Propose (autonomous analysis). All D1-D10 decisions locked. This section records new design-level decisions not pre-resolved in DISCUSS.

### File Change Table

| File | Change Type | What Changes |
|------|-------------|--------------|
| `crates/embyr-core/src/rate_limit.rs` | NEW | `RateLimitInfo { remaining: f64, limit: f64, reset_ms: u64 }` — pure domain type; zero IO imports |
| `crates/embyr-core/src/lib.rs` | MODIFY | Add `pub mod rate_limit;` |
| `crates/embyr-server/src/middleware/rate_limit.rs` | EXTEND | Add `pg_pool: Option<Arc<PgPool>>` and `timeout_counter` fields; add `check_pg()` internal method; change `check()` return type from `Result<(), ()>` to `Result<RateLimitInfo, RateLimitInfo>`; add `attach_rate_limit_headers()` free function; retain `TokenBucket` as fallback |
| `crates/embyr-server/src/grpc/handler.rs` | MODIFY | 9 call sites: match on `Result<RateLimitInfo, RateLimitInfo>`; attach trailing metadata to both success responses and rejection `Status` values |
| `crates/embyr-server/src/admin/handlers/provision.rs` | MODIFY | Wrap project INSERT in `sqlx::Transaction`; add `rate_buckets` INSERT in same transaction; `OperatorState` gains `rate_limit_capacity: f64` field |
| `crates/embyr-server/src/admin/state.rs` | MODIFY | `OperatorState` gains `rate_limit_capacity: f64` |
| `crates/embyr-server/src/lib.rs` | MODIFY | Read `EMBYR_RATE_LIMIT_RPS` from env (default `1000.0`); replace 3 hardcoded `RateLimiter::new(1000.0, 1000.0)` calls; update `RateLimiter::new()` to pass `pg_pool`; `start_test_server_with_rate_limit()` passes `None` for pg_pool (backward compatible) |
| `migrations/0018_rate_buckets.sql` | NEW | `CREATE TABLE rate_buckets (...)` with FK `ON DELETE CASCADE` |

Note: The DISCUSS handoff listed `migrations/system/0018_rate_buckets.sql`. The actual migrations directory for the system DB is `migrations/` (root), not `migrations/system/` — confirmed by inspection of existing migrations 0001–0017. The correct path is `migrations/0018_rate_buckets.sql`.

---

## Port Interface Decision

`RateLimiter` is NOT behind a port/trait interface. See ADR-015 for full rationale.

**Summary:** rate limiting has exactly one implementation (Postgres + in-process fallback) and no current requirement to swap it. A trait interface would add indirection with zero benefit for this feature. Test isolation is achieved via `RateLimiter::disabled()` and `RateLimiter::new(capacity, refill_rate, None)` (in-process only mode) — no test double is needed. ADR-015 documents this choice with the rejected alternative.

---

## `RateLimitInfo` Domain Type

**Location:** `crates/embyr-core/src/rate_limit.rs`

Pure struct — no IO, no async, no third-party dependencies beyond `std`. Derivable by both `embyr-server` (header construction) and any future `embyr-admin` metrics display.

```
RateLimitInfo {
    remaining: f64,  // tokens remaining after this request (floored at 0.0)
    limit: f64,      // configured capacity == EMBYR_RATE_LIMIT_RPS
    reset_ms: u64,   // epoch milliseconds when >= 1 token will be available
}
```

**`remaining` semantics:**
- `Ok(info)` path (allowed): `info.remaining` = tokens left after consuming 1. Always `>= 0.0`.
- `Err(info)` path (rejected): `info.remaining = 0.0` (floored). The bucket was empty.

**`reset_ms` computation:**

From the Postgres path (token count known from `RETURNING tokens`):
- Let `t = returned_tokens` (post-deduction value from RETURNING clause).
- If `t >= 1.0`: `reset_ms = now_epoch_ms()` (next token already available).
- Else: `reset_ms = now_epoch_ms() + ceil((1.0 - t) / refill_rate * 1000.0) as u64`.
- On rejection (0 rows, tokens effectively 0): `reset_ms = now_epoch_ms() + ceil(1.0 / refill_rate * 1000.0) as u64`.

From the in-process fallback path:
- Same formula using `bucket.tokens` (pre-consumption value after refill computation).

---

## `RateLimiter` API Contracts

### Constructor signatures

```
// Before (changed):
RateLimiter::new(capacity: f64, refill_rate: f64) -> Arc<Self>

// After:
RateLimiter::new(capacity: f64, refill_rate: f64, pg_pool: Option<Arc<PgPool>>) -> Arc<Self>

// Unchanged:
RateLimiter::disabled() -> Arc<Self>
```

When `pg_pool = None`: in-process fallback mode only (used by all existing test constructors — backward compatible with US-14 test suite).

When `pg_pool = Some(pool)`: attempts Postgres path first on every `check()` call.

### `check()` decision tree

```
async fn check(&self, project_id: &str) -> Result<RateLimitInfo, RateLimitInfo>
```

1. If `!self.enabled` → `Ok(RateLimitInfo { remaining: self.capacity, limit: self.capacity, reset_ms: 0 })`
2. If `self.pg_pool.is_some()`:
   a. `tokio::time::timeout(Duration::from_millis(20), self.check_pg(project_id)).await`
   b. On `Ok(result)` → return `result` (distributed path used)
   c. On `Err(_)` (timeout elapsed) → `metrics::counter!("rate_limit_pg_timeout_total").increment(1)` → fall through to step 3
   d. On `Ok(Err(pg_error))` (Postgres hard error, resolved within 20ms) → log warn → fall through to step 3
3. Acquire `self.buckets.lock()` → get or insert `TokenBucket::new(self.capacity, self.refill_rate)` for `project_id` → call `try_consume()` → return `Ok(info)` or `Err(info)` with `RateLimitInfo` computed from bucket state

Note: step 2d distinguishes timeout from hard error. Only timeout increments `rate_limit_pg_timeout_total`. Hard Postgres errors are logged at WARN and fall back silently.

### `check_pg()` internal method

```
async fn check_pg(&self, project_id: &str) -> Result<Result<RateLimitInfo, RateLimitInfo>, sqlx::Error>
```

Returns:
- `Ok(Ok(info))` — 1 row returned; request allowed
- `Ok(Err(info))` — 0 rows returned; request rate limited
- `Err(e)` — Postgres error; caller falls back to in-process bucket

---

## Atomic UPDATE SQL

```sql
-- Atomically consume 1 token from the rate bucket for the given project.
-- Returns the updated token count if allowed; 0 rows if rate limited.
UPDATE rate_buckets
SET tokens = LEAST($1::float8,
                   tokens + EXTRACT(EPOCH FROM (now() - last_refill)) * $2::float8
             ) - 1.0,
    last_refill = now()
WHERE project_id = $3
  AND tokens + EXTRACT(EPOCH FROM (now() - last_refill)) * $2::float8 >= 1.0
RETURNING tokens;
```

Parameters (bound in order):
- `$1` — `capacity` (f64): EMBYR_RATE_LIMIT_RPS; caps the refilled total via LEAST
- `$2` — `refill_rate` (f64): tokens per second (== capacity for a 1-second window)
- `$3` — `project_id` (VARCHAR)

Design notes:
- `LEAST(capacity, ...)` prevents unbounded token accumulation during idle periods.
- The `WHERE ... >= 1.0` predicate atomically gates deduction — no separate SELECT or advisory lock needed.
- Row lock scope: `UPDATE WHERE project_id = $3` acquires a row-level lock on the PK; no cross-project contention.
- `RETURNING tokens` carries the post-deduction value, used to populate `RateLimitInfo.remaining` and compute `reset_ms`.
- **Postgres UPDATE latency budget:** Expected latency under normal conditions: < 5ms. If the UPDATE consistently exceeds 10ms, investigate: long-running transactions holding row locks, connection pool exhaustion, Postgres autovacuum interference, or disk I/O saturation. The 20ms timeout is the hard safety bound.

---

## gRPC Trailing Metadata Contract

tonic version: **0.12** (root `Cargo.toml` line 16).

**tonic 0.12 metadata placement:**
- `Response<T>::metadata_mut()` for **unary** handlers → trailing metadata (HTTP/2 HEADERS frame with END_STREAM, after the response body).
- `Response<BoxStream>::metadata_mut()` for **streaming** handlers → initial metadata (first HEADERS frame).
- `Status::metadata_mut()` for **any rejection** → trailing metadata on the error HEADERS frame.

**`attach_rate_limit_headers(md: &mut MetadataMap, info: &RateLimitInfo)` — free function in `rate_limit.rs`:**

Sets on every response (success and rejection):
- `x-ratelimit-limit` → `info.limit as u64` formatted as decimal ASCII
- `x-ratelimit-remaining` → `info.remaining.max(0.0).floor() as u64` formatted as decimal ASCII
- `x-ratelimit-reset` → `info.reset_ms` formatted as decimal ASCII (epoch milliseconds)

Sets additionally on rejection only:
- `retry-after-ms` → `ceil(max(0.0, 1.0 - info.remaining) / refill_rate * 1000.0) as u64` formatted as decimal ASCII

**Success path (7 unary handlers):**
```
// After authenticate() succeeds and before handler body:
match self.rate_limiter.check(&project_id).await {
    Ok(info) => {
        let mut response = Response::new(payload);
        attach_rate_limit_headers(response.metadata_mut(), &info);
        Ok(response)
    }
    Err(info) => {
        let mut status = Status::resource_exhausted("rate limit exceeded");
        attach_rate_limit_headers(status.metadata_mut(), &info);
        attach_retry_after(status.metadata_mut(), &info, self.rate_limiter.refill_rate);
        return Err(status);
    }
}
```

**Success path (`run_query`, `listen` — streaming handlers):**
Rate-limit check occurs before the stream is created. On rejection, `Err(Status)` with metadata returned identically to unary handlers. On success, headers attached to `Response::metadata_mut()` (initial metadata of the streaming response).

**Implementation note for `listen`:** The `listen` handler checks rate limit before spawning the stream task. The long-lived bidirectional stream does not re-check rate limits on subsequent messages from the same stream (per D6 scope).

---

## `EMBYR_RATE_LIMIT_RPS` Environment Variable

Read once at startup in `crates/embyr-server/src/lib.rs`:

```
let capacity: f64 = std::env::var("EMBYR_RATE_LIMIT_RPS")
    .ok()
    .and_then(|s| s.parse::<f64>().ok())
    .filter(|v| v.is_finite() && *v > 0.0)
    .unwrap_or(1000.0);
let refill_rate = capacity;  // token bucket: refill rate == capacity for 1-RPS-second window
```

Replaces hardcoded `1000.0` at three call sites in `lib.rs` (lines 174, 231, 280).

`start_test_server_with_rate_limit()` (line 325) is preserved unchanged — it passes `pg_pool = None` to `RateLimiter::new()`, keeping the in-process-only behaviour required by the existing US-14 acceptance test suite.

A new test constructor `start_test_server_with_distributed_rate_limit(system_db, capacity, pg_pool)` was added for DRL acceptance tests.

---

## Provisioning Integration — Transaction Wrapping

**Context:** The current `provision.rs` executed each backend mode branch as separate autocommit `sqlx::query(...).execute(pool)` calls — no explicit transaction. Adding the `rate_buckets` INSERT atomically required wrapping both INSERTs in a `sqlx::Transaction`.

**Pattern for each backend mode branch:**
```
let mut tx = state.system_db.pool().begin().await?;
// ... INSERT INTO projects (...) on &mut tx ...
// INSERT INTO rate_buckets:
sqlx::query("INSERT INTO rate_buckets (project_id, tokens, last_refill) VALUES ($1, $2, now())")
    .bind(&req.project_id)
    .bind(state.rate_limit_capacity)
    .execute(&mut *tx)
    .await?;
tx.commit().await?;
```

If `rate_buckets` INSERT fails, the transaction rolls back entirely. No orphaned project row is possible.

**`OperatorState` change:** Added `rate_limit_capacity: f64`. Populated at composition root from the same `capacity` variable used to construct `RateLimiter`. No new env var read in the handler.

---

## Migration `0018_rate_buckets.sql`

```sql
CREATE TABLE rate_buckets (
    project_id  VARCHAR(63)      NOT NULL PRIMARY KEY
                                 REFERENCES projects(id) ON DELETE CASCADE,
    tokens      DOUBLE PRECISION NOT NULL,
    last_refill TIMESTAMPTZ      NOT NULL
);

-- Backfill rate_buckets for active projects that existed before this migration.
-- Uses 1000.0 as the default initial token count (matches the prior hardcoded default).
-- Operators running EMBYR_RATE_LIMIT_RPS != 1000 should update these rows:
--   UPDATE rate_buckets SET tokens = <new_capacity> WHERE true;
INSERT INTO rate_buckets (project_id, tokens, last_refill)
SELECT id, 1000.0, now()
FROM   projects
WHERE  status IN ('active', 'suspended')
ON CONFLICT DO NOTHING;
```

**Design notes:**
- `ON DELETE CASCADE` ensures project hard-deletion (sweeper after 168h) automatically removes the rate bucket row with no additional sweeper code.
- `DOUBLE PRECISION` matches the Rust `f64` type used in `TokenBucket.tokens` — no type conversion at the sqlx boundary.
- No `CHECK (tokens >= 0)` constraint: the atomic UPDATE enforces non-negativity through the `>= 1.0` WHERE predicate; a CHECK would add write overhead on every UPDATE.
- Backfill scope: `status IN ('active', 'suspended')` — `deleted` projects will be swept by the existing sweeper and need no rate bucket.
- **Backfill ambiguity resolved:** Without the backfill, a `check_pg()` call on an existing project would return 0 rows, which is indistinguishable from "rate limited." The backfill removes this ambiguity without requiring a second SELECT query on every check.

---

## Earned Trust — `RateLimiter` Postgres Dependency

`RateLimiter` is a concrete struct, not a driven adapter behind a port. Principle 12 (Earned Trust) still applies to its Postgres dependency.

**Startup probe:** The existing system DB startup probe (`db.Ping()` within 2s + `sqlx-migrate` schema migration run) already validates Postgres connectivity and confirms `rate_buckets` table presence before any listener opens.

**Three Earned Trust layers:**

| Layer | Mechanism | What it checks |
|-------|-----------|----------------|
| Compile-time | `sqlx::query!()` macro validates SQL against the live DB at compile time; type-checks `$1::float8`, `$3 VARCHAR` parameters | SQL correctness and parameter types |
| Structural (pre-commit) | Existing CI `cargo check` step covers `rate_limit.rs` changes; no new hook required for this module since it is not a driven adapter struct | Compilation succeeds |
| Behavioral (CI gold-test) | `tests/distributed_rate_limiting/acceptance/b13_fallback_on_pg_failure.rs` injects a Postgres delay, asserts the 20ms timeout fires, verifies fallback activates | Timeout detection and fallback path under real substrate pressure |

**Metric as Earned Trust signal:** `rate_limit_pg_timeout_total` provides operators real-time observability of Postgres pressure on the rate limiter path.

---

## Metrics

Uses the existing `metrics` crate (0.22.x) already wired to the Prometheus exporter on `GET /metrics` (admin port).

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `rate_limit_pg_timeout_total` | Counter | none | Increments each time the 20ms Postgres timeout fires and the fallback activates. Does NOT increment on hard Postgres errors (those are logged at WARN level, no counter). |

No new metric for hard errors at this time — the WARN log is sufficient for V1. A `rate_limit_pg_error_total` counter can be added in a future ADR if observability gaps are identified in production.

---

## Design Slice Map

| Slice | Stories | Design Notes |
|-------|---------|--------------|
| DRL-01 | US-DRL-04 (partial) | Add `migrations/0018_rate_buckets.sql`; add `pub mod rate_limit;` to `embyr-core/src/lib.rs`; define `RateLimitInfo` struct; read `EMBYR_RATE_LIMIT_RPS` in `lib.rs` |
| DRL-02 | US-DRL-01 | Extend `RateLimiter`: add `pg_pool`, `timeout_counter` fields; implement `check_pg()`; change `check()` return type; wire `system_db.pool()` in composition root; add `start_test_server_with_distributed_rate_limit()` |
| DRL-03 | US-DRL-03 | Add `tokio::time::timeout(20ms, ...)` wrapper in `check()`; increment `rate_limit_pg_timeout_total` on timeout; log WARN on hard errors |
| DRL-04 | US-DRL-02 | Add `attach_rate_limit_headers()` and `attach_retry_after()` free functions; update 9 call sites in `handler.rs` |
| DRL-05 | US-DRL-04 (complete) | Wrap provision INSERTs in `sqlx::Transaction`; add `rate_buckets` INSERT; add `rate_limit_capacity: f64` to `OperatorState` and wire in composition root |
