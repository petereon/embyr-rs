# Architecture Decisions — observability

> Extracted from `feature-delta.md` DESIGN wave sections. Permanent reference.
> ADR-016: `docs/product/architecture/adr-016-prometheus-metrics.md`
> Evolution: `docs/evolution/2026-08-08-observability.md`

---

## Component Decomposition

All changes are confined to `embyr-server`. `embyr-core` is not modified.

### File Change Table

| File | Change Type | What Changed |
|------|-------------|-------------|
| `Cargo.toml` (workspace root) | MODIFY | Added `metrics = "0.23"` and `metrics-exporter-prometheus = "0.15"` to `[workspace.dependencies]` |
| `crates/embyr-server/Cargo.toml` | MODIFY | Added `metrics.workspace = true` and `metrics-exporter-prometheus.workspace = true` to `[dependencies]` |
| `crates/embyr-server/src/observability.rs` | NEW FILE | `get_or_install_prometheus_handle() -> PrometheusHandle` backed by `std::sync::OnceLock`; histogram bucket configuration for `embyr_grpc_request_duration_seconds`; called by all server constructors before any listener opens |
| `crates/embyr-server/src/lib.rs` | MODIFY | Import `pub mod observability`; call `get_or_install_prometheus_handle()` in all five `start_test_server_*` constructors before listener spawn; spawn pool metrics background task (OBS-05); pass `PrometheusHandle` to `build_admin_router` |
| `crates/embyr-server/src/admin/state.rs` | MODIFY | Added `prometheus_handle: metrics_exporter_prometheus::PrometheusHandle` field to `OperatorState` |
| `crates/embyr-server/src/admin/router.rs` | MODIFY | Added `prometheus_handle: PrometheusHandle` parameter to `build_admin_router`; added `GET /metrics` route to `operator_router`; four backward-compatible wrappers (`build`, `build_with_aws`, `build_with_gcp`, `build_with_secret_fetchers`) call `get_or_install_prometheus_handle()` internally — no caller changes required |
| `crates/embyr-server/src/admin/handlers/prometheus_metrics.rs` | NEW FILE | `get_prometheus_metrics` handler: updates pool gauges then calls `s.prometheus_handle.render()`; returns `Content-Type: text/plain; charset=utf-8`; named to avoid collision with existing `metrics.rs` (project-level metrics handler) |
| `crates/embyr-server/src/admin/handlers/mod.rs` | MODIFY | Added `pub mod prometheus_metrics` |
| `crates/embyr-server/src/middleware/obs_helpers.rs` | NEW FILE | `grpc_status_label(code: tonic::Code) -> &'static str` mapping all tonic status codes to lowercase strings; `&'static str` constants for all 10 gRPC method names |
| `crates/embyr-server/src/middleware/mod.rs` | MODIFY | Added `pub mod obs_helpers` |
| `crates/embyr-server/src/middleware/rate_limit.rs` | MODIFY | Added `metrics::counter!("embyr_rate_limit_pg_timeout_total").increment(1)` alongside existing `tracing::warn!`; added `embyr_rate_limit_requests_total{project_id, outcome}` counter at all `Ok` and `Err` return paths of `check()` |
| `crates/embyr-server/src/grpc/handler.rs` | MODIFY | Added `metrics::counter!` + `metrics::histogram!` calls at all 10 `impl Firestore` handler methods |

Note: The DISTILL slice brief (OBS-02) named the helper file `crates/embyr-server/src/middleware/metrics.rs`. The DESIGN wave resolved this as `obs_helpers.rs` to avoid ambiguity with the existing project-metrics handler at `admin/handlers/metrics.rs`. The DELIVER implementation used `obs_helpers.rs`.

---

## Metric Contract

All metrics use the `embyr_` prefix. Prometheus conventions: `_total` suffix for counters, `_seconds` suffix for duration histograms, no suffix for gauges.

### `embyr_grpc_requests_total`

Type: counter. Labels: `method`, `status`.

Incremented after every gRPC handler return (both OK and error paths), using the result code determined before return. Covers all 10 currently implemented Firestore handlers.

`method` values (static `&'static str` constants in `obs_helpers.rs`):
`GetDocument`, `CreateDocument`, `UpdateDocument`, `DeleteDocument`, `BatchGetDocuments`,
`BeginTransaction`, `Commit`, `Rollback`, `RunQuery`, `Listen`.

Note: AC-OBS-02-02 (DISTILL) listed 9 methods in draft state — excluded `CreateDocument` and `Listen`, included the unimplemented `RunAggregationQuery`. The DELIVER implementation instruments all 10 existing handlers. `RunAggregationQuery` is added when implemented.

`status` values (mapped by `grpc_status_label()` from `tonic::Code`):
`ok`, `not_found`, `unauthenticated`, `permission_denied`, `resource_exhausted`,
`internal`, `unavailable`, `aborted`, `already_exists`, `invalid_argument`,
`failed_precondition`, `unimplemented`.

### `embyr_grpc_request_duration_seconds`

Type: histogram. Labels: `method` (same values as above).

Records wall-clock elapsed seconds from handler entry to response dispatch on both OK and error paths.

Bucket boundaries: `[0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0]` seconds.

The `2.0` bucket aligns to the Firestore SLA threshold (p99 ≤ 2 s, AC-05c), enabling `histogram_quantile(0.99, ...) > 2.0` as the SLO alert expression.

`listen` handler: records stream setup latency only (from first message receipt to stream response send). Per-message latency deferred to V2.

### `embyr_rate_limit_requests_total`

Type: counter. Labels: `project_id`, `outcome` (`allowed` or `rejected`).

Incremented at every `RateLimiter::check()` return. The `project_id` parameter is `&str`; `.to_owned()` is required for the metrics macro label value.

HIGH CARDINALITY (D-OBS-7): one time series per project per outcome. Acceptable up to ~10,000 projects. Above that threshold, operators use recording rules to aggregate. Documented in operator runbook.

### `embyr_rate_limit_pg_timeout_total`

Type: counter. Labels: none.

Incremented at the Postgres 20ms timeout branch in `RateLimiter::check()`, alongside the existing `tracing::warn!` which is retained. Hard Postgres errors (within-timeout failures) do NOT increment this counter; they remain log-only per ADR-015.

### `embyr_pg_pool_size`

Type: gauge. Labels: `pool = "system"`. Value: `pool.size() as f64` (total connections).

### `embyr_pg_pool_idle`

Type: gauge. Labels: `pool = "system"`. Value: `pool.num_idle() as f64` (idle connections).

Customer DB pools are per-project on-demand; not instrumented in V1 (cardinality unbounded).

---

## PrometheusHandle Lifecycle

### Installation

`crates/embyr-server/src/observability.rs` exposes `get_or_install_prometheus_handle()`, backed by `static PROMETHEUS_HANDLE: std::sync::OnceLock<PrometheusHandle>`. The OnceLock initializer:
1. Configures `PrometheusBuilder` with the histogram bucket boundaries.
2. Calls `.install_recorder()` exactly once.
3. Returns a clone of the installed handle on every subsequent call.

`PrometheusHandle` is `Clone`. Cloning shares the underlying recorder arc — it does not create a new recorder.

### Decision: OperatorState over thin MetricsState

DISCUSS handed off the question: extend `OperatorState` or create a thin `MetricsState`?

**Decision: extend `OperatorState`.** Rationale: `GET /metrics` is an operator-only route, already in the `operator_router` guarded by `operator_auth_middleware`. The handler requires only the `PrometheusHandle` and the system DB pool (for pool gauge update before render). Both are reachable from `OperatorState` (pool via `system_db.pool()`). Creating a separate `MetricsState` would add a new state type, a new `with_state()` sub-router, and a new middleware registration — all for a single route. That complexity is unjustified.

### Backward-compatible wrappers

The four wrappers in `router.rs` (`build`, `build_with_aws`, `build_with_gcp`, `build_with_secret_fetchers`) each call `get_or_install_prometheus_handle()` internally and pass the result to `build_admin_router`. No changes are needed at existing call sites.

### Startup constraint

`get_or_install_prometheus_handle()` is called before `spawn_all_servers` in every server constructor. This ensures the recorder is active before any listener thread starts, so any metrics emitted in listener initialization are captured.

---

## gRPC Handler Instrumentation Pattern

Each of the 10 `impl Firestore` methods in `handler.rs` follows this contract:

1. Capture wall-clock start at handler entry (before rate-limit check and auth).
2. Statically determine the `method` constant using `obs_helpers` method name constants.
3. Execute existing handler logic (rate-limit check, auth, backend adapter call).
4. On handler return (both OK and `Err(Status)` paths):
   a. Compute elapsed seconds from captured start.
   b. Map the result to a `status` label via `grpc_status_label()`.
   c. Increment `embyr_grpc_requests_total{method, status}`.
   d. Record `embyr_grpc_request_duration_seconds{method}` with elapsed seconds.

The timing and counter increment happen at every exit point, including early returns (rate limit rejection, auth failure) — those exits are counted and timed.

`listen` handler: the stream setup phase is timed (from first client message receipt to the moment the response stream is sent). The per-message delivery loop is not timed in V1.

---

## Rate Limit Metrics Conversion

### `rate_limit_pg_timeout_total` (OBS-04, slice item 3)

`rate_limit.rs` (the `Err(_timeout)` arm of `tokio::time::timeout`):

Previous state: `tracing::warn!(project_id = project_id, "rate_limit_pg_timeout: ...")` — log only.

Implemented state: the `tracing::warn!` is retained AND `metrics::counter!("embyr_rate_limit_pg_timeout_total").increment(1)` fires on the same code path. Both signals fire together.

### `embyr_rate_limit_requests_total` (OBS-04, primary counter)

`RateLimiter::check()` returns `Result<RateLimitInfo, RateLimitInfo>`. The counter fires at every call return point:
- `Ok(_)` path: `outcome = "allowed"`
- `Err(_)` path: `outcome = "rejected"`

The counter fires regardless of which internal code path produced the result (Postgres path, in-process fallback, or disabled mode).

---

## Pool Metrics Background Task (OBS-05)

The system DB pool is accessed via `system_db.pool()` (returns `&sqlx::PgPool`, cloneable). The background task is spawned using `tokio::spawn` immediately after the system DB is initialized in each server constructor.

Task contract:
- Receives a `sqlx::PgPool` clone.
- Runs a `tokio::time::interval(Duration::from_secs(15))` loop.
- On each tick: calls `pool.size()` → `u32` and `pool.num_idle()` → `usize` (both non-async).
- Updates `embyr_pg_pool_size{pool="system"}` and `embyr_pg_pool_idle{pool="system"}`.
- Task runs for process lifetime with no shutdown coordination (gauge values are informational).

The `get_prometheus_metrics` handler also updates both gauges immediately before calling `handle.render()`, ensuring scrape freshness even if the background task tick has not fired recently (D-OBS-8 dual-update strategy).

`sqlx::PgPool` exposes `.size() -> u32` and `.num_idle() -> usize` as non-async public methods (confirmed stable in sqlx 0.7). Test environments using SQLite-backed `AnyPool` for the system DB use a conditional branch — pool gauges are omitted in that case.

---

## Slice-to-ADR Mapping

| Slice | Primary ADR reference | Key design decision settled |
|-------|----------------------|----------------------------|
| OBS-01 | ADR-016 §PrometheusHandle lifecycle; §GET /metrics endpoint | `OnceLock` installation pattern; `OperatorState` (not thin `MetricsState`); operator sub-router auth |
| OBS-02 | ADR-016 §Metric naming convention | `embyr_grpc_requests_total{method, status}`; 10 handler sites (not 9); `obs_helpers.grpc_status_label()` |
| OBS-03 | ADR-016 §Histogram bucket configuration | Bucket boundaries aligned to Firestore SLA 2s threshold; `listen` handler records setup latency only |
| OBS-04 | ADR-016 §rate_limit_pg_timeout_total conversion; §HIGH CARDINALITY | `embyr_rate_limit_requests_total{project_id, outcome}`; log retained alongside counter; no cardinality suppression in V1 |
| OBS-05 | ADR-016 §Pool gauge update strategy | Dual update: 15s background task + scrape-time update; system DB pool only |
