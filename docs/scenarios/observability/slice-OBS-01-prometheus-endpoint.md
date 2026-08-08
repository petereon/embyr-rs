# Slice OBS-01 — Prometheus `/metrics` Endpoint (Walking Skeleton)

**Feature:** observability
**Slice:** OBS-01 of OBS-05
**Estimate:** 0.5 day
**Stories:** US-OBS-01
**Depends on:** admin-api-v2 complete (admin router architecture in place)
**Status:** IMPLEMENTED (commit 87ffced, 2026-08-08)

---

## Goal

Install the `metrics` + `metrics-exporter-prometheus` crates and expose
`GET /metrics` on the admin port (:9090) authenticated by Bearer admin_key.
Nothing is instrumented yet — the endpoint must return valid Prometheus
exposition format (even if empty) so Prometheus can scrape it without error.

This slice is the mandatory foundation. No other OBS slice ships without it.

---

## Learning Hypothesis

Disproves: "Installing a global Prometheus recorder conflicts with the existing
`metrics_adapter` (daily_project_metrics) or causes compile errors in embyr-core
(which must not import IO crates)."

Confirms if succeeds: `PrometheusBuilder::new().install_recorder()` executes at
startup in embyr-server lib.rs without touching embyr-core; the handle is
injectable into the admin router; Prometheus scrape returns HTTP 200 with
`Content-Type: text/plain; charset=utf-8`.

**Result:** CONFIRMED. OnceLock-based `get_or_install_prometheus_handle()` in
`src/observability.rs` installs cleanly without touching embyr-core. All 5 obs01
acceptance tests pass.

---

## IN Scope

- Add to `[workspace.dependencies]` in `Cargo.toml`:
  - `metrics = "0.23"`
  - `metrics-exporter-prometheus = "0.15"`
- Add to `embyr-server/Cargo.toml` `[dependencies]`:
  - `metrics.workspace = true`
  - `metrics-exporter-prometheus.workspace = true`
- `lib.rs`: call `get_or_install_prometheus_handle()` before any
  listener opens; pass the returned `PrometheusHandle` into `build_admin_router`
- Admin router: add `GET /metrics` to the operator sub-router (Bearer admin_key)
  using `OperatorState` extended with `prometheus_handle: PrometheusHandle`
- Handler: `async fn get_prometheus_metrics(State(s): State<OperatorState>) -> impl IntoResponse`
  updates pool gauges then returns `handle.render()` with `Content-Type: text/plain; charset=utf-8`

## OUT Scope

- Any actual metric instrumentation (counters, histograms, gauges) — those are
  OBS-02 through OBS-05
- Dashboard or alerting configuration (DEVOPS wave)
- `/metrics` on data ports :8080 or :8081

---

## Acceptance Criteria

See `feature-delta.md` US-OBS-01 for the full AC list (AC-OBS-01-01 through
AC-OBS-01-05).

Key gate: `curl -H "Authorization: Bearer $EMBYR_ADMIN_KEY" http://localhost:9090/metrics`
returns HTTP 200 with body containing `# HELP` and `# TYPE` lines (even if
no user-defined metrics exist yet, the exporter emits its own internal metrics).

---

## Dependencies

- `EMBYR_ADMIN_KEY` env var (existing)
- Admin port (:9090) running (existing)
- `metrics = "0.23"` and `metrics-exporter-prometheus = "0.15"` version pair
  confirmed compatible (see D-OBS-2-CORRECTION in feature-delta.md)

---

## Effort Estimate

0.5 day. The `metrics-exporter-prometheus` installation is ~10 LOC in lib.rs
and ~15 LOC for the handler + router wiring. The main risk is the `OperatorState`
struct change propagating to `build_admin_router` call sites; reference class:
DRL-05 added an 8th parameter to `build_admin_router` in one step.
