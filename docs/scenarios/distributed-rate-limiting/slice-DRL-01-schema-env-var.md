# Slice DRL-01 — Schema + Env Var (Seed Slice)

**Feature:** distributed-rate-limiting
**Slice:** DRL-01 of DRL-05
**Estimate:** 0.5 days
**Stories:** US-DRL-04 (partial — migration only), US-DRL-01 (partial — env var wiring)
**Depends on:** nothing (foundation slice)

---

## Goal

Create the `rate_buckets` table and wire `EMBYR_RATE_LIMIT_RPS` env var to replace the hardcoded `1000.0` in the composition root. No behavioral change in rate limiting yet — this slice is purely additive infrastructure that unblocks all downstream slices.

## Learning Hypothesis

Disproves: "The `0018_rate_buckets.sql` migration conflicts with existing system DB migrations or creates locking issues during startup."
Confirms if succeeds: The migration applies cleanly in sequence after `0017_*`; the FK `rate_buckets.project_id → projects.id ON DELETE CASCADE` is valid; `EMBYR_RATE_LIMIT_RPS` env var reads correctly with default 1000.

## IN Scope

- Migration `migrations/system/0018_rate_buckets.sql`:
  ```sql
  CREATE TABLE rate_buckets (
      project_id  VARCHAR(63)       PRIMARY KEY
                                    REFERENCES projects(id) ON DELETE CASCADE,
      tokens      DOUBLE PRECISION  NOT NULL,
      last_refill TIMESTAMPTZ       NOT NULL DEFAULT now()
  );
  ```
- Read `EMBYR_RATE_LIMIT_RPS` env var at startup (composition root or config struct)
- Default value: 1000.0 if env var absent or unparseable (log a warning on parse failure)
- Replace all hardcoded `1000.0` capacity/refill_rate references in `rate_limit.rs` and composition root with the value read from env var
- No change to `RateLimiter::check()` signature or behavior yet

## OUT Scope

- Atomic Postgres UPDATE (DRL-02)
- Response headers (DRL-04)
- Graceful fallback (DRL-03)
- Provisioning `rate_buckets` row INSERT (DRL-05)
- Per-project granularity (explicitly out of scope per D4)

## Acceptance Criteria

- AC from feature-delta: US-DRL-04 AC item 1 (migration exists and FK is active)
- Migration applies cleanly against a fresh system DB and against a system DB with existing `projects` rows
- `EMBYR_RATE_LIMIT_RPS=500` sets capacity and refill_rate to 500.0; no restart side-effects
- When `EMBYR_RATE_LIMIT_RPS` is absent, default is 1000.0 and a `WARN`-level log message is emitted
- Existing US-14 tests (`start_test_server_with_rate_limit`) continue to pass (they use explicit capacity/refill args, not the env var)

## Dependencies

- System DB running (Postgres)
- Sequential migration numbering: `0018_rate_buckets.sql` is the next migration after `0017_*`; verify `migrations/system/` directory for current highest number before numbering

## Effort Estimate

0.5 days. Migration SQL is 6 lines. Env var wiring is ~15 LOC in config struct or `main.rs`. No logic changes to rate limiting behavior.
