# Slice B-04 — Project Patch, Metrics, Query Logs

**Feature:** admin-api-v2
**Slice:** B-04 of B-06
**Estimate:** 1 day
**Stories:** US-B04
**Depends on:** B-03 complete

---

## Goal

Runtime config changes (backend DSN, logging toggle) + metrics chart data + filterable query log table.

## Learning Hypothesis

Disproves: "Patching `backend_pg_dsn` at runtime causes live connections to fail — cache eviction doesn't propagate fast enough."  
Confirms if succeeds: After PATCH updates the DSN, a new SDK request within 1 cache TTL (5 min) uses the new DSN without requiring a server restart.

## IN Scope

- `PATCH /admin/v1/projects/:id` — partial update handler; ECIES re-encrypt DSN; credential cache eviction
- `GET /admin/v1/projects/:id/metrics` — sourced from `daily_project_metrics`; V1 non-hourly granularity
- `GET /admin/v1/projects/:id/query_logs` — filterable; cursor pagination; `query_logs` partitioned table
- `logging_enabled` flip: embyr's BC-2 write path checks this flag before inserting into `query_logs`
- Index: `CREATE INDEX ON query_logs(project_id, timestamp) PARTITION OF query_logs` (partition-level)

## OUT Scope

- Live metrics (SSE streaming) — V2
- Hourly metrics granularity — V2 (requires separate metrics aggregation job)
- Query log CSV export — V2

## Acceptance Criteria

- AC-B04-01 through AC-B04-06 (see feature-delta.md US-B04)

## Dependencies

- B-03 complete
- BC-2 write path can accept a `logging_enabled` flag check (embyr-core change or embyr-server config)

## Effort Estimate

1 day. PATCH handler is mechanical. Metrics query is a GROUP BY on existing table. Query log filter with cursor pagination is the most complex piece (~100 LOC handler).
