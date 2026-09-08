# ADR-016: Prometheus Metrics via `metrics` Facade and `metrics-exporter-prometheus`

## Status

Accepted

## Context

embyr-rs has no Prometheus metrics infrastructure. Three concrete gaps drive this decision:

1. **`rate_limit_pg_timeout_total` is a log string, not a counter.** The `tracing::warn!` at
   `crates/embyr-server/src/middleware/rate_limit.rs:146` emits a structured log line with
   the text "rate_limit_pg_timeout". No `metrics::counter!` call exists. Prometheus cannot
   observe this event; Sam Chen cannot query it without grepping logs.

2. **The `metrics` crate is absent from the workspace.** `docs/product/architecture/brief.md`
   § Technology Choices listed `metrics = "0.22.x"` and `metrics-exporter-prometheus` as
   planned technology. Neither crate appears in the workspace `Cargo.toml` or in
   `crates/embyr-server/Cargo.toml`. The entry was aspirational; ADR-015 referenced the
   "existing `metrics` crate" before it was added. This ADR resolves the discrepancy.

3. **JOB-12 (service operator observability) requires a Prometheus scrape endpoint.** Sam
   Chen runs a Grafana deployment. Without `GET /metrics`, he cannot add embyr to his
   Prometheus scrape config and must diagnose incidents by grepping logs — measured MTTR of
   >30 minutes per the DISCUSS wave. The five metric families in the observability feature
   (OBS-01 through OBS-05) deliver sub-5-minute diagnosis.

**Operational simplicity constraint (rank 6 quality attribute):** No new infrastructure
dependency is acceptable. The solution must use the existing system Postgres DB and the
existing admin port (:9090). No metrics sidecar, no StatsD agent, no OTel collector.

**`embyr-core` IO-free invariant:** `embyr-core`'s `deny.toml` prohibits `tokio`, `sqlx`,
`axum`, and any IO crate. Instrumentation must be confined to `embyr-server`. The `metrics`
crate facade macro API (`metrics::counter!()` etc.) resolves through the global recorder
installed at the composition root — the macros themselves have no IO dependency and can be
called from anywhere, but in this codebase they are only called from `embyr-server` modules.

## Decision

**Use `metrics = "0.23"` (the facade crate) and `metrics-exporter-prometheus = "0.15"`
(the Prometheus text-format exporter). Expose the scrape endpoint at `GET /metrics` on
the existing admin port (:9090) guarded by existing Bearer `EMBYR_ADMIN_KEY` auth.**

Note: the brief.md technology table previously listed `0.22.x`. The authoritative version
pair is `metrics = "0.23"` + `metrics-exporter-prometheus = "0.15"`, confirmed compatible:
they share the same `Recorder` trait version. The brief.md entry is corrected by this ADR.

---

### Crate installation

Added to `[workspace.dependencies]` in the workspace root `Cargo.toml`:
- `metrics = "0.23"`
- `metrics-exporter-prometheus = "0.15"`

Added to `[dependencies]` in `crates/embyr-server/Cargo.toml`:
- `metrics.workspace = true`
- `metrics-exporter-prometheus.workspace = true`

`embyr-core` does NOT gain these dependencies. Its `deny.toml` lists `metrics-exporter-prometheus`
as disallowed (IO coupling). The `metrics` crate facade is also disallowed in `embyr-core`
to prevent instrumentation calls from crossing into the domain layer.

---

### Macro API

All instrumentation uses standard `metrics` crate macros:

```
metrics::counter!("embyr_grpc_requests_total", "method" => method, "status" => status).increment(1)
metrics::histogram!("embyr_grpc_request_duration_seconds", "method" => method).record(elapsed_secs)
metrics::gauge!("embyr_pg_pool_size", "pool" => "system").set(size_f64)
```

These macros resolve through the process-global recorder. They do not contain IO — they are
atomic operations on the recorder's in-memory data structures.

---

### PrometheusHandle lifecycle

`PrometheusBuilder::new().install_recorder()` installs the global recorder and returns a
`PrometheusHandle`. It panics on a second call within the same process.

**`OnceLock`-based idempotent installation:** A new module
`crates/embyr-server/src/observability.rs` exposes:

```
pub fn get_or_install_prometheus_handle() -> PrometheusHandle
```

Backed by `static PROMETHEUS_HANDLE: std::sync::OnceLock<PrometheusHandle>`. The initializer
lambda configures histogram bucket boundaries for `embyr_grpc_request_duration_seconds` via
`Matcher::Prefix` and calls `install_recorder()` exactly once. `PrometheusHandle` is `Clone`;
subsequent calls to `get_or_install_prometheus_handle()` return a clone of the same handle
without touching the global recorder.

This pattern makes all server constructors in `lib.rs` (test servers) and `main.rs`
(production) safe to call unconditionally — they all call `get_or_install_prometheus_handle()`.
In a test process that starts multiple `TestServer` instances, the recorder is installed on
the first call and reused thereafter.

**Startup ordering constraint (AC-OBS-01-05):**
`get_or_install_prometheus_handle()` MUST be called before any TCP listener opens. Production
`main.rs` call order:

```
install_recorder → migrate_db → run_probes → bind_listeners → serve
```

Metrics emitted during startup probes and migrations are captured; no signal is lost.

---

### Metric naming convention

Prefix `embyr_` prevents collision with Prometheus internals and identifies embyr as the
source in shared Grafana deployments.

| Metric | Type | Labels | Description |
|--------|------|--------|-------------|
| `embyr_grpc_requests_total` | counter | `method`, `status` | gRPC requests by method and tonic status |
| `embyr_grpc_request_duration_seconds` | histogram | `method` | Wall-clock request duration |
| `embyr_rate_limit_requests_total` | counter | `project_id`, `outcome` | Rate-limit decisions per project |
| `embyr_rate_limit_pg_timeout_total` | counter | none | Postgres token-bucket fallback events |
| `embyr_pg_pool_size` | gauge | `pool` | System DB total connections |
| `embyr_pg_pool_idle` | gauge | `pool` | System DB idle connections |

**`method` label values:** `GetDocument`, `CreateDocument`, `UpdateDocument`,
`DeleteDocument`, `BatchGetDocuments`, `BeginTransaction`, `Commit`, `Rollback`,
`RunQuery`, `Listen`. `RunAggregationQuery` will use `"RunAggregationQuery"` when
implemented. All 10 currently-implemented `impl Firestore` methods in `handler.rs` are
instrumented; AC-OBS-02-02 listed 9 (an artifact of draft state); the correct count is
all implemented handlers.

**`status` label values (tonic Code → lowercase string):**
`ok`, `not_found`, `unauthenticated`, `permission_denied`, `resource_exhausted`,
`internal`, `unavailable`, `aborted`, `already_exists`, `invalid_argument`,
`failed_precondition`, `unimplemented`.

Helper function `grpc_status_label(code: tonic::Code) -> &'static str` in
`crates/embyr-server/src/middleware/obs_helpers.rs` (new file) centralises this mapping.

**`outcome` label values:** `allowed`, `rejected`.

**`pool` label values:** `system` (system DB `PgPool` only; customer DB pools are per-project
on-demand and are not instrumented in V1 — unbounded cardinality; V2 concern).

---

### HIGH CARDINALITY: `project_id` label on `embyr_rate_limit_requests_total`

**Amended by ADR-069 (2026-09-09):** D-OBS-7's acceptance below assumed cardinality is bounded by the
count of *real, registered* projects. That assumption was not enforced in code — the label used
whatever raw `project_id` string an unauthenticated caller supplied, before any validation
(production-readiness-audit-2026-09-08.md finding #2, Blocker). ADR-069 makes this assumption true in
code by gating the label on the rate limiter's own pre-existing `rate_buckets` row-existence check,
with a constant `"unconfirmed"` sentinel for anything not already provisioned. The ≤10,000-projects
ceiling below is otherwise unchanged and remains the operative scaling assumption for legitimate
traffic.

Per D-OBS-7, the `project_id` label is retained in V1 despite the cardinality risk. This
delivers the auditor-visible evidence that JOB-11 fair-multitenancy is enforced.

At deployments up to ~10,000 projects, the resulting ~20,000 time series (2 outcome values
per project) is well within Prometheus defaults (~2M series). Above 10,000 projects, the
operator must use recording rules to aggregate:

```promql
sum by (outcome) (rate(embyr_rate_limit_requests_total[5m]))
```

This decision is documented here and must be included in the operator runbook. Re-evaluate
in V2 if any deployment exceeds 10,000 projects.

---

### Histogram bucket configuration

Bucket boundaries for `embyr_grpc_request_duration_seconds`:

```
[0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0]  (seconds)
```

These boundaries are aligned to the Firestore SLA threshold (p99 ≤ 2 s per AC-05c). The
`2.0` bucket enables `histogram_quantile(0.99, ...) > 2.0` as the SLO alert expression.

Applied at recorder installation via `Matcher::Prefix("embyr_grpc_request_duration")` in
the `PrometheusBuilder`. DELIVER must verify the exact builder method name for
`metrics-exporter-prometheus = "0.15"` (anticipated API: `.set_buckets_for_metric()`).

---

### `GET /metrics` endpoint

Added to the `operator_router` in `build_admin_router` (guarded by `operator_auth_middleware`,
requiring Bearer `EMBYR_ADMIN_KEY`). This is consistent with D-OBS-4 and the existing operator
route pattern.

`OperatorState` gains a new field: `prometheus_handle: metrics_exporter_prometheus::PrometheusHandle`.
`build_admin_router` gains a corresponding `prometheus_handle: PrometheusHandle` parameter.

A new handler module `crates/embyr-server/src/admin/handlers/prometheus_metrics.rs` (named
to avoid collision with the existing `metrics.rs` handler for project-level metrics) implements
`get_prometheus_metrics`:

```
Response: StatusCode::OK, Content-Type: "text/plain; charset=utf-8", body: handle.render()
```

Before `handle.render()`, the handler also updates the two pool gauges (`embyr_pg_pool_size`,
`embyr_pg_pool_idle`) from the system DB pool to ensure scrape freshness (D-OBS-8 dual-update).

**Backward-compatible wrapper functions:** `build`, `build_with_aws`, `build_with_gcp`, and
`build_with_secret_fetchers` in `router.rs` call `get_or_install_prometheus_handle()` internally.
No call-site changes are required in existing test code.

---

### `rate_limit_pg_timeout_total` conversion

The existing `tracing::warn!` at `rate_limit.rs:146` is RETAINED. A `metrics::counter!` call
is added alongside it at the same call site. Both fire on the same code path (20ms timeout).

Hard Postgres errors (errors returned within 20ms, not timeouts) are logged at WARN but do
NOT increment this counter, consistent with ADR-015 semantics.

---

### Pool gauge update strategy (D-OBS-8)

Two mechanisms update `embyr_pg_pool_size` and `embyr_pg_pool_idle`:

1. **Background task (15-second interval):** Spawned in the server startup code (both
   `main.rs` and all `start_test_server_*` functions in `lib.rs`) after the system DB pool
   is created. Receives `pool: sqlx::PgPool` (clone). Uses `tokio::time::interval`.

2. **Scrape-time update:** The `get_prometheus_metrics` handler updates both gauges
   immediately before `handle.render()`. This ensures the response reflects pool state
   at the moment of the HTTP request.

`sqlx::PgPool` exposes `.size() -> u32` and `.num_idle() -> usize` as non-async public
methods (confirmed stable in sqlx 0.7). DELIVER must add a conditional branch for test
environments using SQLite-backed `AnyPool` where these methods may not be available.

---

## Alternatives Considered

### Alternative 1: `prometheus` crate directly (rejected)

The `prometheus` crate (maintained by the Prometheus Rust team) provides Prometheus text
format without a facade layer. It uses its own `Registry` and `TextEncoder`.

**Rejected because:** No abstraction layer between instrumentation call sites and the
exposition format. Every crate that wants to emit metrics must import `prometheus` directly,
coupling it to the Prometheus format. Future format changes (e.g., OTLP, StatsD) require
rewriting every call site. The `metrics` facade provides identical `counter!/histogram!/gauge!`
macros with the recorder swappable at the composition root — the 5-10 call sites per feature
remain unchanged regardless of export format.

Additionally, `prometheus` does not have an equivalent to `metrics-exporter-prometheus`'s
`PrometheusHandle.render()` pattern; its `TextEncoder::encode()` requires explicit registry
access, which complicates the handler implementation.

### Alternative 2: OpenTelemetry SDK (rejected)

`opentelemetry = "0.21"` + `opentelemetry-prometheus` would provide vendor-neutral
instrumentation. The OTel SDK defines a standardized API across tracing, metrics, and logs.

**Rejected because:** OTel adds approximately 150-200 KB to the binary. This is not a hard
limit violation (embyr-rs has no bundle size constraint like the VSCode extension), but it
contradicts the operational simplicity quality attribute (rank 6). Sam's V1 requirement is
a Prometheus scrape — not an OTel collector pipeline. The `tracing-opentelemetry` crate can
bridge to OTel trace export in V2 without changing any `tracing::span!` or `tracing::info!`
call sites. Similarly, if OTel metrics become a requirement, the `metrics` facade allows
swapping the recorder to an OTel-backed implementation without touching the 20+ call sites
that will exist after the observability feature ships.

### Alternative 3: `tracing-subscriber` metrics bridge (rejected)

`tracing-timing` and related crates can derive histograms from tracing span lifetimes.
`tracing` spans already wrap gRPC handler calls; duration could be extracted from span events.

**Rejected because:** Cannot emit Prometheus text format directly — requires an additional
pipeline from spans to Prometheus exposition. Counter metrics (e.g., `embyr_grpc_requests_total`
with `status` label derived from the response) are not naturally derivable from span attributes
without a custom span visitor implementation. The `metrics::counter!` + `metrics::histogram!`
approach is 3-6 lines per handler site and produces the exact metric with the exact label
semantics defined in this ADR. The span-derivation approach introduces implicit mapping
conventions that would need documentation and testing of their own.

## Consequences

### Positive

- Sam Chen adds embyr to his Prometheus `scrape_configs` on day 1; target shows UP.
- All 5 metric families are available in one endpoint with standard Prometheus text format.
- `rate_limit_pg_timeout_total` is observable in Prometheus — no more log grepping to
  diagnose rate-limit fallback events.
- `metrics` macros are available to all future `embyr-server` components without import
  changes to `Cargo.toml`.
- Test constructors remain backward-compatible — `OnceLock` absorbs multiple
  `get_or_install_prometheus_handle()` calls safely.
- `embyr-core` is not modified — the IO-free invariant is preserved.

### Negative / Trade-offs

- `project_id` label on `embyr_rate_limit_requests_total` is high cardinality at deployments
  above 10,000 projects. Documented above and in operator runbook. Not suppressed in V1.
- `build_admin_router` gains a new `PrometheusHandle` parameter. The four backward-compatible
  wrappers absorb this change internally; external callers are unaffected.
- Prometheus metrics state is process-global. In acceptance tests running multiple `TestServer`
  instances in the same process, counter values accumulate across instances. Tests must assert
  `>= N` (monotonically), not `== N` for counter values.
- `install_recorder()` is not idempotent in `metrics-exporter-prometheus` without `OnceLock`.
  The `OnceLock` wrapper provides idempotency for the test process lifetime. This is a design
  constraint: if future code attempts to reset or replace the recorder, it will fail silently
  (OnceLock ignores subsequent initializer calls). There is no use case for recorder reset
  in this codebase.
- Brief.md technology table entry for `metrics` requires an update: version `0.22.x` →
  `0.23`; `metrics-exporter-prometheus` version `0.15` added explicitly.

## Enforcement

- **Workspace dependency pinning:** `metrics = "0.23"` in `[workspace.dependencies]`.
  Individual crates use `metrics.workspace = true`. Version drift is caught by `cargo update`
  output in CI.
- **`embyr-core` boundary:** `embyr-core/deny.toml` adds `metrics` and
  `metrics-exporter-prometheus` to the disallowed crate list. `cargo-deny` in CI enforces
  this. Any `use metrics::` in `embyr-core/src/` is a compile error (crate not in deps).
- **Behavioral probe (Earned Trust):** OBS-01 acceptance test scrapes `GET /metrics` with a
  valid Bearer token and asserts HTTP 200 + `Content-Type: text/plain; charset=utf-8` +
  body containing `# HELP` and `# TYPE` lines. This is the behavioral layer for the
  `PrometheusHandle` substrate dependency. The handle is assumed operational if the recorder
  installs without panic and `render()` returns a non-empty string — tested on every CI run.
- **ADR-015 metric reference corrected:** ADR-015 § Metric referred to the "existing
  `metrics` crate (0.22.x)". This ADR supersedes that reference; the crate is new in this
  feature at version 0.23.
