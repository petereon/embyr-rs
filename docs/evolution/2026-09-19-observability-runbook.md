# Evolution: observability-runbook

**Date:** 2026-09-19
**Closes:** Medium finding #29 (DevOps), `docs/product/production-readiness-audit-2026-09-08.md`
— the LAST Medium-severity finding. All Medium items (#20–#29, 10 findings) are now closed,
alongside the already-closed High-severity arc (#9–#19).

## Business Context

Prometheus metrics existed (from the `observability` feature, 2026-08-08) but no Grafana
dashboard, alert rules, or runbook were checked in — an operator would have had to build all
three from scratch, including for the Postgres-down-after-startup scenario named in finding
#15. This feature adds those three artifacts, re-deriving the current, real metric and
endpoint list from source rather than the original feature's now-9-features-old design doc.

## What was created

- `docs/operations/grafana-dashboard.json` — importable Grafana dashboard, 8 panels: gRPC
  request rate, gRPC error rate, gRPC p99 latency (2s SLA line), rate-limiter outcomes,
  rate-limiter Postgres fallback events (`pg_timeout`/`pg_error`), sign-in rate-limiter
  outcomes + timeouts, system-DB pool size/idle, and an optional blackbox_exporter-backed
  `/healthz` panel (clearly marked as requiring a separately-deployed exporter).
- `docs/operations/alert-rules.yml` — 5 Prometheus alert rules: system-DB pool exhaustion
  (the finding #15/#29 Postgres-down proxy signal), rate-limiter pg-error climb (finding
  #20), rate-limiter pg-timeout rate, gRPC error ratio > 5%, gRPC p99 > 2s SLA breach.
- `docs/operations/runbook.md` — incident notice/triage guide covering the same 3 failure
  modes as the alerts, explicitly including the named Postgres-down-after-startup scenario.
  Cross-references `docs/operations/backup-disaster-recovery.md` (finding #18) for actual
  recovery steps rather than duplicating them, and ADR-078 for `/healthz`/`/livez` semantics.

**Explicit non-claim:** none of this is deployed or wired into the server, CI, or an
`activate()`-equivalent bootstrap. These are checked-in artifacts an operator imports into
their own Grafana and applies to their own Prometheus — nothing in this repo auto-configures
a dashboard, auto-loads alert rules, or auto-sends alert notifications. Same framing as
`docs/operations/backup-disaster-recovery.md` uses for backup/DR being unautomated.

## Real metric/endpoint list re-derived from source (2026-09-19)

Read directly from `crates/embyr-server/src/observability.rs`,
`src/middleware/obs_helpers.rs`, `src/middleware/rate_limit.rs`,
`src/middleware/signin_rate_limit.rs`, `src/admin/handlers/prometheus_metrics.rs`,
`src/main.rs`, `src/lib.rs`, and `src/grpc/healthz.rs` — not from memory or the original
2026-08-08 evolution doc alone, since 9+ features have touched this area since:

- `embyr_grpc_requests_total{method, status}` (counter)
- `embyr_grpc_request_duration_seconds{method}` (histogram, 12 buckets incl. 2.0s Firestore SLA)
- `embyr_rate_limit_requests_total{project_id, outcome}` (counter; `project_id` bounded via
  `UNCONFIRMED_PROJECT_LABEL` sentinel, ADR-069)
- `embyr_rate_limit_pg_timeout_total` (counter)
- `embyr_rate_limit_pg_error_total` (counter — added by finding #20's fix, 2026-09-15)
- `embyr_signin_rate_limit_requests_total{outcome}` (counter)
- `embyr_signin_rate_limit_pg_timeout_total` (counter)
- `embyr_pg_pool_size{pool="system"}` / `embyr_pg_pool_idle{pool="system"}` (gauges)

Endpoints: `GET /metrics` on `:9090` (Bearer `EMBYR_ADMIN_KEY`); `GET /healthz` (readiness,
`SystemDb::probe()`, 3s timeout, ADR-078) and `GET /livez` (liveness, zero I/O) both on
`:8081`, unauthenticated — not `:9090` as finding #15's original location note said before
ADR-078 moved/redefined them.

## Key Files

- `docs/operations/grafana-dashboard.json` (new)
- `docs/operations/alert-rules.yml` (new)
- `docs/operations/runbook.md` (new)
- `docs/product/production-readiness-audit-2026-09-08.md` — row #29 updated to CLOSED

## Follow-Up

- Only Low-severity findings (#31–#41) and the separate bloat/over-engineering pass remain
  open in the audit; no Medium or High items are left.
- If an operator wants direct `/healthz`/`/livez` HTTP-status alerting rather than the
  pool-gauge proxy used here, deploy `blackbox_exporter` — not part of this feature's scope.
