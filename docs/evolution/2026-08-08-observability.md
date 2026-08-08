# Evolution — observability

**Feature ID:** `observability`
**Closed:** 2026-08-08
**Wave sequence:** DISCUSS → DESIGN → DISTILL → DELIVER
**Commits:** 87ffced (all OBS-01–05 implementation), b0ff2f1 (DES artifacts)

---

## Feature Summary

Added Prometheus-format observability to embyr-rs. Before this feature, Sam Chen (service operator) had zero instrumentation: all diagnosis required grepping unstructured `tracing::info!/warn!` log lines, and incident MTTR exceeded 30 minutes. The `metrics` crate was absent from the workspace despite being listed as "planned" in `brief.md`.

5 delivery slices shipped, all in a single implementation commit (87ffced):

- **OBS-01** — `PrometheusHandle` installed via `OnceLock`; `GET /metrics` on admin port (:9090) behind Bearer admin_key auth
- **OBS-02** — `embyr_grpc_requests_total{method, status}` counter on all 10 gRPC handler call sites
- **OBS-03** — `embyr_grpc_request_duration_seconds{method}` histogram with 12-bucket Firestore-SLA-aligned boundaries
- **OBS-04** — `embyr_rate_limit_requests_total{project_id, outcome}` counter in `RateLimiter::check()`; `embyr_rate_limit_pg_timeout_total` counter alongside the existing `tracing::warn!`
- **OBS-05** — `embyr_pg_pool_size{pool}` and `embyr_pg_pool_idle{pool}` gauges updated by a 15-second background task and at each scrape

Walking skeleton confirmed: `grpc_request_increments_embyr_grpc_requests_total` passes. Full end-to-end loop closed — a gRPC call → counter instrumented in handler → `PrometheusHandle.render()` → `GET /metrics` → Sam sees the count.

---

## Business Context

**JOB-12 (system-observability)** — new job added this wave. Primary driver for all 5 slices.

Sam runs the embyr SaaS multi-tenant deployment. He provisions projects (JOB-02), enforces rate limits (JOB-11), and now can diagnose a P1 incident within 5 minutes of a Prometheus alert firing using only a Grafana dashboard — without accessing Postgres or grepping logs.

**JOB-11 (fair-multitenancy)** — enabled as a secondary outcome. OBS-04 makes rate-limit enforcement observable: Sam can show auditors `embyr_rate_limit_requests_total{project_id="fintech-acme",outcome="rejected"}` as per-project enforcement evidence, replacing the prior live SQL query on `rate_buckets`.

**D-OBS-2-CORRECTION:** A key DISCUSS finding: the task prompt claimed "`metrics` crate already in use for `rate_limit_pg_timeout_total`." Inspection of the actual code showed this was FALSE — `rate_limit_pg_timeout_total` was a string in a `tracing::warn!()` call. The `metrics` crate was entirely absent from `Cargo.toml`. All 5 slices added net-new instrumentation with no pre-existing foundation.

---

## Key Decisions (D-OBS-1–8 + ADR-016)

All 8 decisions were locked in the DISCUSS wave. DESIGN wave recorded no re-opened decisions.

| ID | Decision | Rationale |
|----|----------|-----------|
| D-OBS-1 | Metrics endpoint on :9090 (admin port), not :8080/:8081 | Admin port is network-isolated (internal only per architecture brief); operator data must not be visible to SDK clients |
| D-OBS-2-CORRECTION | Add `metrics = "0.23"` as a NEW workspace dependency | Crate was absent; task prompt claim that it existed was incorrect — DISCUSS wave corrected before DESIGN |
| D-OBS-3 | `metrics-exporter-prometheus = "0.15"` for the Prometheus scrape endpoint | Standard pair with `metrics 0.23`; `install_recorder()` is the ergonomic API |
| D-OBS-4 | `GET /metrics` on operator sub-router (Bearer admin_key) | Consistent with all other admin port routes; defense-in-depth alongside network policy |
| D-OBS-5 | Counter instrumentation at gRPC handler call sites, not Tower middleware | `project_id` lives in proto body; middleware cannot extract it before auth; method name is known at the call site |
| D-OBS-6 | `embyr_` prefix; `method` = short gRPC name; `status` = tonic Code as lowercase string | Avoids collision with Prometheus internals; short names reduce cardinality; readable in Grafana |
| D-OBS-7 | `project_id` label on `embyr_rate_limit_requests_total` is HIGH CARDINALITY — not suppressed | Acceptable for deployments ≤ 10,000 projects; auditor-visible evidence for JOB-11 is the V1 priority; V2 recording rules document provides the scaling path |
| D-OBS-8 | Pool stats updated by 15-second background task AND on each `/metrics` handler invocation | Belt-and-suspenders: background task keeps gauges fresh during long scrape intervals; handler update ensures scrape response reflects current moment |

**ADR-016** (`docs/product/architecture/adr-016-prometheus-metrics.md`): primary architectural record. Documents crate version pair, `OnceLock` installation pattern, `OperatorState` extension decision (vs. thin `MetricsState`), metric naming convention, HIGH CARDINALITY note for `project_id` label, histogram bucket configuration, and three rejected alternatives (direct `prometheus` crate, OpenTelemetry SDK, `tracing-subscriber` metrics bridge).

---

## Steps Completed

All 5 slices delivered in a single implementation commit (87ffced). RED_ACCEPTANCE phase driven by acceptance tests in each slice's test file.

| Slice | Stories | Key Deliverable | DES Outcome |
|-------|---------|-----------------|-------------|
| OBS-01 | US-OBS-01 | `observability.rs` (OnceLock), `prometheus_metrics.rs` handler, `GET /metrics` route on operator sub-router | GREEN + COMMIT PASS; RED_ACCEPTANCE PASS |
| OBS-02 | US-OBS-02 | `obs_helpers.rs` (`grpc_status_label()` + 10 method constants), counter calls at all 10 gRPC handler sites | GREEN + COMMIT PASS; RED_ACCEPTANCE PASS |
| OBS-03 | US-OBS-03 | Histogram recording at all 10 handler sites; 12-bucket boundary config in `PrometheusBuilder` | GREEN + COMMIT PASS; RED_ACCEPTANCE PASS |
| OBS-04 | US-OBS-04 | `embyr_rate_limit_requests_total{project_id, outcome}` + `embyr_rate_limit_pg_timeout_total` in `rate_limit.rs` | GREEN + COMMIT PASS; RED_ACCEPTANCE PASS |
| OBS-05 | US-OBS-05 | Pool gauge background task; scrape-time gauge update in handler | GREEN + COMMIT PASS; RED_ACCEPTANCE PASS |

**Unit test phases (PREPARE + RED_UNIT):** Skipped as NOT_APPLICABLE for all 5 slices — orchestrator-crafter merge; acceptance tests drove GREEN directly.

**Key new production files:**
- `crates/embyr-server/src/observability.rs` — `get_or_install_prometheus_handle()` backed by `static PROMETHEUS_HANDLE: OnceLock<PrometheusHandle>`; histogram bucket configuration
- `crates/embyr-server/src/middleware/obs_helpers.rs` — `grpc_status_label(code: tonic::Code) -> &'static str`; `&'static str` constants for all 10 gRPC method names
- `crates/embyr-server/src/admin/handlers/prometheus_metrics.rs` — `get_prometheus_metrics` axum handler; pool gauge update before `handle.render()`

**Corrected from DISTILL slice brief (slice-OBS-02):** Helper file is `obs_helpers.rs`, not `metrics.rs` (the DISTILL brief named it `middleware/metrics.rs` to avoid naming conflict, but the DESIGN wave resolved it as `obs_helpers.rs` to be unambiguous alongside the existing project-metrics handler).

---

## Lessons Learned

**DISCUSS inspection averted a design assumption failure.** The DISCUSS wave verified actual source code before writing any design decisions. The finding that `rate_limit_pg_timeout_total` was a log string (not a counter) and that the `metrics` crate was absent changed the scope framing of D-OBS-2 from "extend existing instrumentation" to "add net-new foundation." Had this not been caught, the DELIVER agent would have spent time searching for non-existent counter call sites.

**OnceLock is the idiomatic Rust solution for process-global recorder singletons.** `PrometheusBuilder::install_recorder()` panics on a second call; multiple test server instances in the same process would have caused panics. The `OnceLock<PrometheusHandle>` wrapper in `observability.rs` makes the installation idempotent. This is a general pattern for any global singleton that must be safely called from multiple server constructors.

**Walking skeleton scope: OBS-01 alone is not the walking skeleton.** OBS-01 delivers an empty Prometheus endpoint — technically functional but carrying no user value without a metric to observe. The walking skeleton is the first OBS-02 test: after a gRPC call, `GET /metrics` shows `embyr_grpc_requests_total` incremented. This closes the full diagnostic loop and provides non-technical stakeholder validation.

**Scrape-time pool gauge update (D-OBS-8) is worth the duplication.** The 15-second background task means the maximum gauge staleness is 15 seconds. Adding the pre-render update in the handler brings freshness to the exact moment of the scrape. Two lines of code in the handler; zero new infrastructure. The belt-and-suspenders pattern applies whenever gauges are updated asynchronously but freshness at scrape time is important.

**10 gRPC handlers, not 9.** AC-OBS-02-02 and the DISTILL slice briefs counted 9 methods (an artifact of early draft state that excluded `CreateDocument` and `Listen` while including the unimplemented `RunAggregationQuery`). The final implementation correctly instruments all 10 existing `impl Firestore` handlers. This discrepancy was caught in the DESIGN Metric Contract section and documented in ADR-016.

---

## Issues Encountered

No issues encountered. The single-commit delivery (87ffced) had no partial states, compilation failures, or test regressions. All 5 acceptance test files passed in sequence. The D-OBS-2-CORRECTION was identified and resolved in DISCUSS before DELIVER began.

---

## Migrated Artifacts

| Type | Source (transient) | Destination (permanent) |
|------|-------------------|------------------------|
| Architecture decisions | `docs/feature/observability/feature-delta.md` §§ DESIGN wave | `docs/architecture/observability/architecture-decisions.md` |
| ADR-016 | `docs/product/architecture/adr-016-prometheus-metrics.md` | Already in permanent location — no migration |
| Slice briefs | `docs/feature/observability/slices/slice-OBS-0{1..5}-*.md` | `docs/scenarios/observability/slice-OBS-0{1..5}-*.md` |
| Acceptance tests | `tests/observability/` | Already in permanent location (source tree) |
| DES traces | `docs/feature/observability/deliver/execution-log.json` + `roadmap.json` | Referenced from this evolution doc |
