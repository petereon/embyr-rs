# Service Level Indicators and Objectives (proposed)

Closes the SLO half of go-live finding O5
([`go-live-assessment-2026-11-13.md`](../product/go-live-assessment-2026-11-13.md)).

## Status: proposal, not a commitment

Every **SLI** below is derived from a metric that exists in `crates/embyr-server` today (or is
explicitly flagged as missing). Every **target number** (99.9 %, 250 ms, ...) is a *proposal
chosen before any load test has run*. The first real run of [`loadtest/`](../../loadtest/README.md)
against the production topology ([`topology.md`](./topology.md)) should either confirm them or
move them. Do not publish these to customers until that has happened.

Related: [`alert-rules.yml`](./alert-rules.yml) (existing alerts),
[`grafana-dashboard.json`](./grafana-dashboard.json), [`runbook.md`](./runbook.md).

## What the metrics can and cannot see

Source: `crates/embyr-server/src/observability.rs`, `middleware/obs_helpers.rs`,
`middleware/rate_limit.rs`, `main.rs` / `lib.rs` (pool gauges). Scraped from `GET /metrics` on
:9090 (bearer `EMBYR_ADMIN_KEY`).

- `embyr_grpc_requests_total{method,status}` and
  `embyr_grpc_request_duration_seconds{method}` are recorded by one wrapper around every
  Firestore RPC handler, so **native gRPC (:8080) and gRPC-Web (:8081) land in the same series**.
  There is no `listener` or `project_id` label.
- Histogram buckets (seconds): `0.001 0.005 0.01 0.025 0.05 0.1 0.25 0.5 1 2 5 10`. A latency SLI
  stated as "fraction of requests under X" is only exact when X is one of these edges; the
  targets below are chosen on those edges on purpose.
- `status` is the lowercased tonic code (`ok`, `not_found`, `unauthenticated`, `internal`,
  `unavailable`, `resource_exhausted`, ...). Backend failures (customer Postgres down, pool
  acquire timeout, unmapped core error) all collapse to `internal` via `sanitize_backend_error`.
- `Listen` and `Write` are streaming RPCs. The wrapper records when the **stream is established**,
  not message delivery, so their counter/histogram say nothing about real-time latency.
  Exclude them from latency SLIs.
- Only the **system** Postgres pool is exported (`embyr_pg_pool_size{pool="system"}`,
  `embyr_pg_pool_idle{pool="system"}`). Tenant and listener pools have no metric.
- Requests that never reach a handler (TLS failure, LB error, connection refused, body over the
  10 MiB gRPC / 2 MiB REST limit, malformed frame) are invisible. Server-side SLIs therefore
  **over-state** availability; pair them with an LB/blackbox view (see gaps).

## SLIs and SLOs

Window: rolling 30 days. Budget figures: 30 d = 43 200 min = 2 592 000 s.

| # | SLI | Proposed SLO | Error budget (30 d) |
|---|---|---|---|
| 1 | Data-plane availability (gRPC + gRPC-Web) | 99.9 % | 0.1 % of requests; 43.2 min of full outage |
| 2a | GetDocument latency | 99 % of requests < 250 ms | 1 % of requests may be slower |
| 2b | Commit latency | 99 % < 500 ms | 1 % |
| 2c | RunQuery latency (`limit` <= 100, indexed) | 99 % < 500 ms | 1 % |
| 3 | Listen delivery latency | p99 <= 2 s, commit -> delivered to subscriber | **no metric yet** |
| 4 | Admin console availability | 99.5 % | 0.5 % of probes; 216 min |

"p99 < X" and "99 % of requests under X" are the same statement; the second form is what the
bucket counters can compute exactly.

### SLI 1 - data-plane availability

*Good* = a request that did not fail because of the service. *Bad* = status in
`internal | unavailable | deadline_exceeded | data_loss | unknown`.

Deliberately **not** bad: `unauthenticated`, `permission_denied`, `not_found`, `invalid_argument`,
`already_exists`, `failed_precondition`, `aborted` (transaction contention), `cancelled`
(client gave up), and `resource_exhausted` (the per-project rate limit working as designed -
but see caveats: it also fires when a bad-key flood drains the failed-auth budget).

```promql
# SLI (ratio of good requests), 5m
1 - (
  sum(rate(embyr_grpc_requests_total{status=~"internal|unavailable|deadline_exceeded|data_loss|unknown"}[5m]))
  /
  sum(rate(embyr_grpc_requests_total[5m]))
)
```

Caveats:
- BYOC: a customer's own Postgres outage surfaces as `internal` and counts against *us*. With
  no `project_id` label it cannot be excluded. Decide whether to accept that or add a label.
- `embyr_rate_limit_requests_total{outcome="rejected"}` shows how much `resource_exhausted` is
  limiter-driven; it is per-project, so it cannot be subtracted exactly.
- Counts only handler-reaching requests (see above).

### SLI 2 - latency per method

```promql
# fraction of GetDocument requests faster than 250ms, 5m
  sum(rate(embyr_grpc_request_duration_seconds_bucket{method="GetDocument",le="0.25"}[5m]))
/ sum(rate(embyr_grpc_request_duration_seconds_count{method="GetDocument"}[5m]))

# Commit and RunQuery: same shape with le="0.5"
```

Caveats: includes the failures' duration (errors are usually fast, which flatters the number);
`Commit` latency includes the Postgres round trip(s) to the *customer* database, so network
distance between embyr-server and customer DBs is inside the SLI. `RunQuery` latency depends on
the customer's data and indexes; the target is only meaningful for bounded, indexed queries.

The existing `EmbyrGrpcLatencySLOBreach` alert (p99 > 2 s, all methods) is a much looser
ceiling than these targets. Keep it as the "something is badly wrong" guard.

### SLI 3 - Listen delivery latency  (GAP)

Not measurable today: no histogram of NOTIFY-received -> event-written-to-stream, and no gauge
of open Listen streams. Closest existing signals are health, not latency:
`embyr_pg_notify_listener_reconnecting{project_id}` (1 after ~15 s of failed reconnects) and
`embyr_pg_notify_listener_reconnect_attempts_total{project_id}`. Until a delivery histogram
exists, treat "Listen is healthy" as `max(embyr_pg_notify_listener_reconnecting) == 0`, and
measure delivery latency in a client (write via Commit, time until the Listen stream yields the
change). The k6 script in `loadtest/` does **not** drive Listen (see its README).

### SLI 4 - admin console availability  (GAP: probe only)

There are no HTTP metrics for :9090. Use `blackbox_exporter` (not bundled) against the admin
listener, reached from inside the private network:
- `GET http://<admin-host>:9090/healthz` -> 200 (readiness incl. system DB, 503 while draining), and
- `GET http://<admin-host>:9090/admin/` -> 200 (the SPA index, served when `EMBYR_ADMIN_UI_DIR`
  is set, which the Dockerfile does).

SLI = `avg_over_time(probe_success{job="embyr-admin-probe"}[5m])`, SLO = fraction of successful
probes. Admin is a low-QPS operator surface, so 99.5 % is deliberately weaker than the data plane.

## Alert to SLO mapping

| Alert (`alert-rules.yml`) | Relates to | Notes |
|---|---|---|
| `EmbyrGrpcErrorRateHigh` (>5 % non-OK, 5 m) | SLI 1 (loosely) | Counts *all* non-OK including client errors, so it fires on a bad-key flood and misses a real 1 % outage. Not an SLO alert. |
| `EmbyrGrpcLatencySLOBreach` (p99 > 2 s, 10 m) | SLI 2 (ceiling) | Looser than the proposed targets. |
| `EmbyrTargetDown`, `EmbyrMetricsAbsent`, `EmbyrRestartLoop` | SLI 1 (cause) | Process-level; also required for the SLI itself to exist. |
| `EmbyrSystemDbPoolExhausted` | SLI 1, 2 (leading) | Zero idle system connections: rate limiter and auth are on this pool. |
| `EmbyrRateLimiterPgErrorsClimbing`, `EmbyrRateLimiterPgTimeoutsHigh` | SLI 1, 2 (leading) | Limiter degraded to per-instance buckets; precedes latency regressions. |
| `EmbyrSweeperFailingRepeatedly` | none | Housekeeping; no SLO. |
| `EmbyrReadinessFailing` (commented out) | SLI 4 (and replicas) | Needs blackbox_exporter; enable it. |

**Not covered by any alert:** SLO burn rate (the thing SLO alerting is for), SLI 3, SLI 4.
Proposed multiwindow burn-rate rules for SLI 1, to add once the 99.9 % number is ratified
(`ERR_<w>` = the SLI 1 error ratio over window `w`, defined as recording rules):

```promql
# page: 2% of monthly budget in 1h (14.4x burn), confirmed on 5m
  (ERR_1h > 14.4 * 0.001) and (ERR_5m > 14.4 * 0.001)
# page: 5% in 6h (6x)
  (ERR_6h > 6 * 0.001) and (ERR_30m > 6 * 0.001)
# ticket: 10% in 3d (1x)
  (ERR_3d > 1 * 0.001) and (ERR_6h > 1 * 0.001)
```

For latency SLIs use the same factors with budget `0.01`.

## Flagged: SLIs with no metric yet

| Need | Why it matters | Smallest fix |
|---|---|---|
| Listen delivery latency + open-stream count | SLI 3 cannot exist | Histogram `embyr_listen_delivery_seconds` (change commit time -> stream write) and a gauge of active Listen streams, in the realtime registry. |
| Admin/HTTP request metrics (:9090, hosted-identity REST on :8081) | SLI 4 is probe-only; sign-up/sign-in latency and errors are blind | Axum middleware emitting `http_requests_total{route,status}` and a duration histogram. |
| `listener` and `project_id` labels on `embyr_grpc_*` | Cannot split :8080 vs :8081, or exclude a BYOC customer's own DB outage | Add a bounded `listener` label; `project_id` only behind the same "confirmed project" guard the rate limiter uses (ADR-069) to bound cardinality. |
| Tenant and listener pool gauges (size/idle/acquire timeouts) | Tenant pool exhaustion shows up only as `internal` errors | Export aggregate (per replica, not per project) gauges from the tenant adapters. |
| `/healthz` status as a metric | Readiness alert needs blackbox_exporter | Optional; blackbox is the standard answer. |
| Failures before the handler (TLS, LB, size limits) | SLI 1 is optimistic | Use the LB's own 5xx / target-health metrics as a second SLI source. |
| `process_start_time_seconds` | Restart detection is `changes(up)`, blind to fast loops | Standard process collector. |

## Calibrating after the first load test

1. Run the `ramp` then `soak` profiles ([`loadtest/README.md`](../../loadtest/README.md)) at the
   target RPS; read the server-side histograms above, not only k6's client view.
2. If the observed p99 is already near a target, the target is wrong, not the load test.
3. Re-derive error budgets (`budget_requests = target_rps * 2 592 000 * (1 - SLO)`), write the
   ratified numbers into this file, and delete the "proposal" status.
