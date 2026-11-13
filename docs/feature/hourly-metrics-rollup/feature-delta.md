# hourly-metrics-rollup — Feature Delta (combined DISCUSS + DESIGN)

ADR: `docs/product/architecture/adr-084-hourly-metrics-rollup.md`. Prerequisite for the admin-UI latency tiles and activity charts
(`admin-ui-api-wiring` slice 06).

## 1. Problem

`GET /projects/:id/metrics` returns `p95_read_ms: 0.0`, `p95_write_ms: 0.0` and a `sparkline` that is `reads_today / 24` repeated 24 times:
the system stores only `daily_project_metrics` (read/write/delete counts per day; written per request by `MetricsAdapter` as a
best-effort upsert, `record_read`/`record_write` from `grpc/handler.rs`). There is no latency data and no sub-day resolution, and
query logs (which carry `latency_ms`) are written only when a project enables logging. The console needs real p50/p95/p99 and a real
hourly series regardless of logging. **Decision (user, final):** a new hourly per-project rollup, counts per op plus latency
histogram buckets, always on.

## 2. Acceptance criteria

- AC-HMR-01: Given requests of each op class against a project with query logging **off**, Then rollup rows exist for the current hour with matching `requests` and a histogram whose bucket total equals `requests`.
- AC-HMR-02: Given known latencies `{3, 3, 8, 40, 900} ms`, Then `p50` falls in the 2–5 ms bucket range and `p95` in the 500–1000 ms range, each within its bucket bounds (error ≤ one bucket width), and `samples = 5`.
- AC-HMR-03: Given no samples in the window, Then percentiles are `null` and `samples` is 0 (never `0.0`).
- AC-HMR-04: Given activity in three different hours, Then `series` has one entry per hour of the window (24 for `24h`, 168 for `7d`), zero-filled, ascending, ending at the current hour.
- AC-HMR-05: Given several server instances writing the same project-hour, Then counts add (no lost update, no duplicate key error).
- AC-HMR-06: Given the process is shut down gracefully, Then the in-memory buffer is flushed; given a crash, Then at most one flush interval of data is lost and requests were never affected.
- AC-HMR-07: Given the system database is unavailable, Then requests succeed, the buffer stays bounded, and `embyr_rollup_dropped_total` increments when the bound is hit.
- AC-HMR-08: Given rows older than the retention window, Then the sweeper deletes them and keeps newer ones; given a deleted project, Then its rows are removed with it.
- AC-HMR-09: Given a session for another account, Then 404; given `window` other than `24h|7d`, Then 422.
- AC-HMR-10: Given `GET /admin/v1/metrics/summary`, Then one call returns per-project `reads_today`, `writes_today`, `deletes_today`, `p95_read_ms`, `p95_write_ms` for every non-deleted project of the caller's account.
- AC-HMR-11: Existing counters (`reads_today`, `writes_today`, `deletes_today`) and billing keep their current values and source (`daily_project_metrics`).
- AC-HMR-12: `listen` streams are not latency-sampled; errors are counted per hour.

## 3. Schema and bucket layout

```
CREATE TABLE hourly_project_metrics (
    project_id  VARCHAR(63)  NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    hour        TIMESTAMPTZ  NOT NULL,                 -- date_trunc('hour', now()) in UTC
    op          SMALLINT     NOT NULL,                 -- 0 read, 1 write, 2 delete, 3 run_query
    requests    BIGINT       NOT NULL DEFAULT 0,
    units       BIGINT       NOT NULL DEFAULT 0,       -- documents touched; same unit as daily_project_metrics
    errors      BIGINT       NOT NULL DEFAULT 0,
    b00 BIGINT NOT NULL DEFAULT 0, b01 …, b12 BIGINT NOT NULL DEFAULT 0,   -- latency histogram, 13 buckets
    PRIMARY KEY (project_id, hour, op)
);
```
Bucket upper bounds (ms, inclusive): `1, 2, 5, 10, 20, 50, 100, 200, 500, 1000, 2000, 5000, +inf` (b00…b12). Roughly 1-2-5 per decade across
the range a Firestore-compatible backend plausibly operates in; relative error of an interpolated percentile is ≤ the bucket ratio
(≤ 2.5x at the top of a bucket, typically far less). Fixed columns instead of an array because Postgres has no element-wise array `+` for
`ON CONFLICT DO UPDATE`, and 13 `BIGINT`s keep the upsert a plain column-wise addition. `requests = Σ b` by construction (asserted in tests).
One row per (project, hour, op): at most 4 rows/hour/project.

Pure function in `embyr-core` (no IO): `percentile(buckets: &[u64; 13], q: f64) -> Option<f64>` — cumulative count, linear interpolation
inside the containing bucket, `+inf` bucket reported as its lower bound (5000 ms, flagged `">5000"` in the response when hit).
Index: the primary key serves every query (`project_id = $1 AND hour >= $2`).

## 4. Write path and cost

Today each recorded request already costs one system-DB upsert. A second per-request upsert would double hot-path writes and was rejected.
Design: a process-wide `RollupBuffer` (`Mutex<HashMap<(project, hour, op), Acc>>`, `Acc` = counters + 13 buckets) updated in
nanoseconds on request completion, **not** awaited and not touching the DB. A background task flushes every 10 s (configurable,
`EMBYR_ROLLUP_FLUSH_SECS`) with one multi-row `INSERT … ON CONFLICT (project_id, hour, op) DO UPDATE SET requests = t.requests + EXCLUDED.requests, …`
(chunks of ≤ 500 keys). Cost: ≤ active-projects × ops rows per flush instead of one write per request; with 1,000 busy projects that is
≤ 4,000 upserted rows every 10 s (about 400 rows/s worst case, typically far fewer). Flush also on SIGTERM. The buffer is bounded
(`EMBYR_ROLLUP_MAX_KEYS`, default 50,000); beyond it new keys are dropped and counted. Latency is measured by the handler around the
backend call using the same clock as `record_grpc_duration`; the project id is known after `authenticate`. The existing daily upsert
is left alone (a follow-up may feed it from the same buffer).

Retention: `EMBYR_ROLLUP_RETENTION_DAYS`, default 14 (336 hourly points per op; 7-day window plus slack). A purge cycle in the existing
sweeper module style runs hourly: `DELETE FROM hourly_project_metrics WHERE hour < now() - $1`. Storage worst case ≈ 4 × 24 × 14 = 1,344
rows × ~170 B ≈ 230 KB per project.

## 5. Endpoints (session auth, any role, account-scoped)

`GET /admin/v1/projects/:id/metrics?window=24h|7d` (default `24h`) → 200
```
{ "reads_today": 0, "writes_today": 0, "deletes_today": 0,          // unchanged, from daily_project_metrics
  "latency": { "read":  { "p50_ms": 4.1, "p95_ms": 11.0, "p99_ms": 28.0, "samples": 1240 },
               "write": { "p50_ms": null, "p95_ms": null, "p99_ms": null, "samples": 0 } },
  "series": [ { "hour": "2026-11-13T09:00:00Z", "reads": 3200, "writes": 410, "deletes": 12, "errors": 3 }, … ],
  "p95_read_ms": 11.0, "p95_write_ms": 0.0,                          // kept one release, now real (0.0 when no samples)
  "sparkline": [ … ] }                                                // deprecated: derived from series, removed with the UI migration
```
`series` units are `units` (documents), consistent with `reads_today`. `GET /admin/v1/metrics/summary` → `[{project_id, name, reads_today, writes_today, deletes_today, p95_read_ms|null, p95_write_ms|null}]`
(fixes the one-call-per-database cost in the console, gap G-19).

## 6. Security considerations

Account scoping as today (404 for foreign projects, never 403, to avoid existence leaks). No user-controlled SQL (window validated to an enum).
Metrics contain no document content or paths. Label cardinality is not a concern (rows, not Prometheus series). The write path adds no new
network surface. Always-on collection must not depend on `logging_enabled`, so it also records for projects that disabled logging for
privacy; only counts and latency are stored.

## 7. Slice plan

1. Migration + pure `percentile`/bucket-index functions in `embyr-core`. Tests: AC-HMR-02, 03 (unit/property: monotone in q, within bucket).
2. `RollupBuffer`, flush task, handler instrumentation, graceful flush. Tests: AC-HMR-01, 05, 06, 07, 12.
3. Retention sweeper. Test: AC-HMR-08.
4. Endpoint changes + `/metrics/summary`. Tests: AC-HMR-04, 09, 10, 11.

## 8. Test strategy

Pure: bucket index, percentile interpolation, buffer merge/cap (no IO). Acceptance against the real router and gRPC handler with
testcontainers Postgres: drive requests through the gRPC adapter with a controllable clock/injected latency, force a flush, assert rows and
endpoint JSON; fault injection: DB down during flush, two buffers flushing the same key, project deleted mid-flush. CI only.
