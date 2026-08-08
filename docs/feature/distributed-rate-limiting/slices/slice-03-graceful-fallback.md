# Slice DRL-03 — Graceful Per-Instance Fallback

**Feature:** distributed-rate-limiting
**Slice:** DRL-03 of DRL-05
**Estimate:** 1 day
**Stories:** US-DRL-03 — Graceful per-instance fallback when system DB is unavailable
**Depends on:** DRL-02 (distributed `check()` must exist; per-instance `TokenBucket` co-exists as fallback)

---

## Goal

Wrap the Postgres atomic UPDATE in a 20ms `tokio::time::timeout`. On timeout: fall back to the per-instance `TokenBucket`, capped at exactly 1× `EMBYR_RATE_LIMIT_RPS`. Increment `rate_limit_pg_timeout_total` counter. Automatic recovery when Postgres comes back (no restart required).

## Learning Hypothesis

Disproves: "Adding a 20ms timeout wrapping the sqlx UPDATE causes unexpected behavior on the success path (extra overhead, task cancellation side-effects)."
Confirms if succeeds: On the non-timeout path, p99 latency of `check()` stays below 500µs (AC-14c remains green). On the timeout path, the per-instance `TokenBucket` activates within 21ms and enforces the 1× cap correctly.

## IN Scope

- Retain the existing per-instance `TokenBucket` inside `RateLimiter` as the fallback map (already present in the current `HashMap<String, TokenBucket>`)
- Wrap the `SystemDb::rate_buckets_check()` call in `tokio::time::timeout(Duration::from_millis(20), ...)`:
  - On `Ok(Ok(info))` → allowed, return `Ok(info)` (distributed path, normal case)
  - On `Ok(Err(info))` → rejected, return `Err(info)` (distributed path, bucket empty)
  - On `Err(_timeout)` → fallback: call per-instance `bucket.try_consume()`:
    - If allowed → return `Ok(RateLimitInfo { remaining: bucket.tokens, limit: capacity, reset_ms: 0 })`
    - If rejected → return `Err(RateLimitInfo { remaining: 0.0, limit: capacity, reset_ms: 0 })`
    - Increment `rate_limit_pg_timeout_total` counter
- `rate_limit_pg_timeout_total` counter: `AtomicU64` on `RateLimiter`; exposed via `pub fn timeout_count(&self) -> u64` for test assertions and metrics
- The per-instance `TokenBucket` capacity must be set to the same `EMBYR_RATE_LIMIT_RPS` value (D3 invariant: 1× not 2×)
- `RateLimiter::disabled()` path bypasses both Postgres AND per-instance fallback (existing behavior preserved)

## OUT Scope

- Full metrics integration (Prometheus/OpenTelemetry export) — the counter exists but export wiring is deferred to DESIGN/DEVOPS wave
- Configurable timeout (20ms is hard-coded per D3; no `EMBYR_RATE_LIMIT_PG_TIMEOUT_MS` env var)
- Configurable fallback cap (always 1× per D3; no knob)
- Alerting on `rate_limit_pg_timeout_total` threshold (DEVOPS wave)

## Acceptance Criteria

- AC-DRL-03: when Postgres UPDATE times out (>20ms), per-instance `TokenBucket` activates for that request — no RESOURCE_EXHAUSTED due to timeout itself
- AC-DRL-03: per-instance fallback cap is exactly 1× `EMBYR_RATE_LIMIT_RPS` — sending capacity+1 requests while fallback is active results in ≥1 RESOURCE_EXHAUSTED
- AC-DRL-03: when Postgres recovers (mock timeout removed), next request uses distributed path — `RateLimiter.timeout_count()` stops incrementing
- AC-14c property test (p99 < 500µs) remains green on the non-timeout path
- New acceptance test `us_drl_03_graceful_fallback.rs`: inject Postgres timeout via mock/test double; verify fallback activates; verify `timeout_count()` increments; remove injection; verify distributed path resumes

## Dependencies

- DRL-02 complete: `RateLimiter::check()` with `Result<RateLimitInfo, RateLimitInfo>` return type must exist
- The per-instance `TokenBucket` struct can be kept as-is from the original `rate_limit.rs`; it is not removed in DRL-02
- Test strategy for timeout injection: pass a `SystemDb` wrapped in a test double that adds a 50ms delay, OR use `tokio::time::pause()` with `advance()` if the sqlx pool can be mocked — DESIGN wave decides the test double approach

## Effort Estimate

1 day. Primary complexity: (1) `tokio::time::timeout` wrapping a fallible async call with two success arms; (2) ensuring the `AtomicU64` counter is visible without deadlock; (3) writing the chaos/timeout injection test. The fallback `TokenBucket` logic is unchanged from the original implementation.
