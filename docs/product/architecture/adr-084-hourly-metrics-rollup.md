# ADR-084: Hourly Per-Project Metrics Rollup with Latency Histogram

## Status

Accepted. Extends ADR-016 (Prometheus metrics are process-wide, unlabelled by project) and the daily metering design (`daily_project_metrics`).

## Context

The console needs real p50/p95/p99 and an hourly activity series per project. Existing data: `daily_project_metrics` (counts per day,
no latency); per-request Prometheus histograms (per gRPC method, process-local, no project label; project labels were bounded away by
ADR-069); `query_logs` (has `latency_ms`, but only when a project enables logging, and bounded by retention). None can answer the
question for every project.

## Decision

A new table `hourly_project_metrics` keyed `(project_id, hour, op)` holding `requests`, `units`, `errors` and a 13-bucket fixed latency
histogram (1-2-5 per decade, 1 ms to 5 s plus overflow), always on, independent of query logging. Writes go to an in-process buffer on
request completion and are flushed by one background task every 10 s as a multi-row additive upsert. Retention is 14 days by hourly
purge. Percentiles are computed on read by a pure function in `embyr-core` from the summed buckets. `daily_project_metrics` remains
the billing source of truth.

## Alternatives Considered

1. **Per-request upsert into the new table** (mirrors `MetricsAdapter`). Rejected: doubles hot-path system-DB writes; the daily upsert is
   already one write per request.
2. **Prometheus histogram with a `project_id` label.** Rejected: unbounded cardinality (ADR-069's reasoning), not persisted across
   restarts, not queryable by the admin API.
3. **Derive from `query_logs`.** Rejected: absent when logging is off, capped by retention and a 500-row page, and requires logging
   privacy trade-offs the user did not ask for.
4. **Store raw latency samples / t-digest per hour.** Rejected: raw samples grow with traffic; a t-digest needs a custom merge column
   type or serialisation. Fixed buckets add by plain integer addition, which is what an `ON CONFLICT` upsert across instances needs.
5. **`BIGINT[]` bucket column.** Rejected: no element-wise array addition in `DO UPDATE` without a custom function.

## Consequences

Positive: real latency and hourly activity for all projects; flush cost proportional to active (project, op) pairs, not requests;
multi-instance safe; storage bounded (~230 KB per project worst case).

Negative / accepted: percentile accuracy is bucket-bounded (not exact); up to 10 s of data lost on a crash; units in the hourly series
and in the daily table are maintained by two writers and can differ transiently; a 14-day horizon (longer history needs a coarser
daily rollup of latency, deferred); `listen` streams have no latency.

## Enforcement

Pure percentile and bucket functions with property tests (monotone, bounded by bucket, `Σ buckets = requests`); acceptance tests with
fault injection (DB down, concurrent flush, graceful shutdown); counters `embyr_rollup_flush_total`, `embyr_rollup_dropped_total`.
