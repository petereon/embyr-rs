# Slice OBS-05 — Postgres Connection Pool Metrics

**Feature:** observability
**Slice:** OBS-05 of OBS-05
**Estimate:** 0.5 day
**Stories:** US-OBS-05
**Depends on:** OBS-01 complete (Prometheus recorder installed)
**Status:** IMPLEMENTED (commit 87ffced, 2026-08-08)

---

## Goal

Expose `embyr_pg_pool_size{pool}` and `embyr_pg_pool_idle{pool}` gauges updated
on every Prometheus scrape (or on a 15-second background task). Sam can now
alert when idle connections approach zero, signaling pool exhaustion before
request timeouts cascade.

---

## Learning Hypothesis

Disproves: "sqlx PgPool does not expose a stable API for pool statistics."

Confirms if succeeds: `pool.size()` and `pool.num_idle()` are available on
`sqlx::PgPool` as non-async methods; the values update dynamically as connections
are acquired and released; gauge values in /metrics track the actual pool state.

**Result:** CONFIRMED. `pool.size() -> u32` and `pool.num_idle() -> usize` are
stable public methods on `sqlx::PgPool` 0.7. Both obs05 acceptance tests pass.
SQLite/AnyPool conditional branch implemented for dev mode.

---

## IN Scope

- Two gauges per pool:
  - `embyr_pg_pool_size{pool}` — total connections (idle + in-use), from `pool.size() as f64`
  - `embyr_pg_pool_idle{pool}` — idle connections, from `pool.num_idle() as f64`
- `pool` label values:
  - `"system"` — the system DB pool (`PgPool` in production)
  - NOTE: Customer DB pools are on-demand per-project; they are not tracked in V1
    (unbounded cardinality). Only the system DB pool is instrumented.
- Update strategy: a Tokio background task spawned in `lib.rs` alongside the
  existing sweeper tasks; polls pool stats and updates gauges every 15 seconds:
  ```
  tokio::spawn(async move {
      let mut interval = tokio::time::interval(Duration::from_secs(15));
      loop {
          interval.tick().await;
          metrics::gauge!("embyr_pg_pool_size", "pool" => "system").set(pool.size() as f64);
          metrics::gauge!("embyr_pg_pool_idle", "pool" => "system").set(pool.num_idle() as f64);
      }
  });
  ```
- Gauge also updated on scrape request via the `/metrics` handler calling the
  same update before `handle.render()` (belt-and-suspenders for freshness, D-OBS-8)

## OUT Scope

- Per-customer-project pool stats (high cardinality, V2 concern)
- `embyr_pg_pool_wait_count` (not exposed by sqlx 0.7 PgPool API)

---

## Acceptance Criteria

See `feature-delta.md` US-OBS-05 for AC-OBS-05-01 through AC-OBS-05-04.

Key gate: acceptance test scrapes /metrics after startup and asserts
`embyr_pg_pool_size{pool="system"}` ≥ 1 and
`embyr_pg_pool_idle{pool="system"}` ≥ 0 and
`embyr_pg_pool_idle{pool="system"}` ≤ `embyr_pg_pool_size{pool="system"}`.

---

## Dependencies

- OBS-01 complete
- `sqlx::PgPool` API: `pool.size()` → `u32` and `pool.num_idle()` → `usize`
  confirmed stable public methods in sqlx 0.7
- Access to the system DB `PgPool` in the background task spawn site in `lib.rs`

---

## Effort Estimate

0.5 day. ~20 LOC for the background task spawn + handler update. SQLite dev mode
uses a conditional branch — gauges are omitted when the system DB uses `AnyPool`.
