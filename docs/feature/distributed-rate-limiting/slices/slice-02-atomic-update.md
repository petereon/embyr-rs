# Slice DRL-02 — Atomic Postgres UPDATE (Distributed Enforcement Core)

**Feature:** distributed-rate-limiting
**Slice:** DRL-02 of DRL-05
**Estimate:** 1 day
**Stories:** US-DRL-01 — Distributed rate enforcement across cluster nodes
**Depends on:** DRL-01 (migration `0018_rate_buckets.sql` must exist; `EMBYR_RATE_LIMIT_RPS` env var must be wired)

---

## Goal

Replace the inner `try_consume()` logic with an atomic Postgres `UPDATE … RETURNING tokens` on the `rate_buckets` table. Change `RateLimiter::check()` return type to `Result<RateLimitInfo, RateLimitInfo>`. Keep all 4 existing US-14 tests green.

## Learning Hypothesis

Disproves: "The atomic UPDATE + RETURNING round-trip adds perceptible latency under the 500µs p99 budget required by AC-14c."
Confirms if succeeds: The distributed enforcement path satisfies the cluster-wide fairness guarantee while the AC-14c property test (10,000 samples, p99 < 500µs) continues to pass — because the fast local path (no Postgres contention, cached pool connection) is sub-millisecond.

## IN Scope

- Add `RateLimitInfo` struct to `embyr-core` (pure domain type, no IO imports):
  ```rust
  pub struct RateLimitInfo {
      pub remaining: f64,  // tokens remaining after this request (from RETURNING)
      pub limit: f64,      // capacity (= EMBYR_RATE_LIMIT_RPS)
      pub reset_ms: u64,   // epoch ms when next token will be available
  }
  ```
- Add `rate_buckets_check(pool, project_id, capacity, refill_rate) → Result<RateLimitInfo, RateLimitInfo>` method on `SystemDb` (or a new `RateBucketAdapter`):
  - Executes the atomic `UPDATE rate_buckets SET tokens = LEAST($1, tokens + EXTRACT(EPOCH FROM (now() - last_refill)) * $2) - 1, last_refill = now() WHERE project_id = $3 AND tokens + EXTRACT(EPOCH FROM (now() - last_refill)) * $2 >= 1 RETURNING tokens` (D2)
  - If 1 row returned: allowed → `Ok(RateLimitInfo { remaining: row.tokens, limit: capacity, reset_ms: ... })`
  - If 0 rows returned (tokens insufficient): rejected → `Err(RateLimitInfo { remaining: 0.0, limit: capacity, reset_ms: ... })`
- Update `RateLimiter::check()` signature: `pub async fn check(&self, project_id: &str) -> Result<RateLimitInfo, RateLimitInfo>`
- Wire `Arc<SystemDb>` into `RateLimiter` (pool is already available on `FirestoreService`)
- Update `start_test_server_with_rate_limit` in `lib.rs` to match new signature (tests must still build)
- Update all 9 call sites in `handler.rs` from:
  ```rust
  if self.rate_limiter.check(&project_id).await.is_err() { return rate_limited(); }
  ```
  to the `match` pattern (header attachment happens in DRL-04; for now, discard `RateLimitInfo` but match both arms):
  ```rust
  match self.rate_limiter.check(&project_id).await {
      Ok(_info) => { /* TODO DRL-04: attach headers */ }
      Err(_info) => return rate_limited_response(),
  }
  ```

## OUT Scope

- Response headers (DRL-04 — `_info` is intentionally discarded in this slice)
- 20ms fallback timeout (DRL-03)
- Provisioning rate_buckets row INSERT (DRL-05)
- `reset_ms` exact computation (DESIGN wave decision — the field must exist but computation can be a placeholder for now)

## Acceptance Criteria

- AC-DRL-01: cluster-wide enforcement — two `RateLimiter` instances sharing the same `SystemDb` pool (simulating two nodes) exhaust the aggregate limit correctly
- AC-DRL-01: US-14 existing 4 tests remain green (`burst_above_default_returns_resource_exhausted`, `rate_limiting_disabled_no_requests_rejected`, `rate_limiter_adds_less_than_half_ms_to_p99_latency`, `rate_limit_exhaustion_is_isolated_per_project`)
- AC-DRL-01: `RateLimiter::check()` return type is `Result<RateLimitInfo, RateLimitInfo>`
- `RateLimitInfo` is defined in `embyr-core` with zero IO imports (compile-time check via `deny.toml`)
- New acceptance test `us_drl_01_distributed_enforcement.rs`: two logical nodes sharing same system DB, 1500 requests against 1000-token limit → 1000 allowed ± 5%, 500 rejected ± 5%

## Dependencies

- DRL-01 complete: `rate_buckets` table exists; `EMBYR_RATE_LIMIT_RPS` wired
- `SystemDb` already holds a `sqlx::PgPool` — add one method, no restructuring required
- `FirestoreService` already holds `Arc<SystemDb>` — pass it to `RateLimiter::new()` alongside capacity and refill_rate

## Effort Estimate

1 day. Primary complexity: (1) atomic SQL UPDATE with `RETURNING` expression; (2) updating 9 call sites in handler.rs to match pattern without breaking compilation; (3) ensuring AC-14c latency test still passes. The `RateLimitInfo` type itself is trivial.
