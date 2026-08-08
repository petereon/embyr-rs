# Slice OBS-03 — gRPC Request Latency Histograms per Method

**Feature:** observability
**Slice:** OBS-03 of OBS-05
**Estimate:** 0.75 day
**Stories:** US-OBS-03
**Depends on:** OBS-02 complete (method label convention established)
**Status:** IMPLEMENTED (commit 87ffced, 2026-08-08)

---

## Goal

Record `embyr_grpc_request_duration_seconds{method}` histogram on every gRPC
handler call, capturing wall-clock time from request receipt to response send.
Sam can now compute p99 per method in Prometheus and alert on the 2-second
Firestore SLA threshold.

---

## Learning Hypothesis

Disproves: "Recording `Instant::now()` at handler entry and calling
`metrics::histogram!()` at handler exit is blocked by the embyr-core purity
constraint (no `std::time` in domain logic)."

Confirms if succeeds: The timing is captured entirely in `embyr-server` handler
code, not in `embyr-core`; `tokio::time::Instant::now()` in the handler preamble
is valid since handlers live in embyr-server (IO layer). Histogram appears in
/metrics with `_bucket`, `_count`, `_sum` suffix lines.

**Result:** CONFIRMED. All 10 handler sites instrumented with start capture +
elapsed record. Histogram appears with correct bucket boundaries in obs03
acceptance tests.

---

## IN Scope

- Add `let start = tokio::time::Instant::now()` at the beginning of each of
  the 10 gRPC handler call sites (same sites as OBS-02)
- After the handler returns (on both OK and error paths), record:
  `metrics::histogram!("embyr_grpc_request_duration_seconds", "method" => method).record(elapsed_secs)`
- Configure histogram bucket boundaries in the `PrometheusBuilder` call
  (`observability.rs`) to match Firestore SLA thresholds:
  `[0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0]`
- Bucket configuration applied in `observability.rs` `PrometheusBuilder` setup
  via `Matcher::Prefix("embyr_grpc_request_duration")`

**Correction from DISTILL brief:** The DISTILL brief listed "9 gRPC handler call
sites." The correct count is **10** — all currently implemented `impl Firestore`
handlers. Consistent with the OBS-02 correction.

## OUT Scope

- Admin port handler latency (those are low-volume; not worth instrumenting in V1)
- Streaming handler individual message latency (Listen streams) —
  only the stream setup/teardown latency is recorded in V1

---

## Acceptance Criteria

See `feature-delta.md` US-OBS-03 for AC-OBS-03-01 through AC-OBS-03-04.

Key gate: acceptance test fires a `RunQuery` with a collection scan and asserts
`embyr_grpc_request_duration_seconds_count{method="RunQuery"}` ≥ 1;
the `_sum` value is > 0. Also asserts the `_bucket` lines are present for
the configured boundary values.

---

## Dependencies

- OBS-02 complete (method label convention and helper function established)
- `PrometheusBuilder` bucket config in `observability.rs`

---

## Effort Estimate

0.75 day. Same 10 call sites as OBS-02 plus ~1 LOC per site for elapsed
calculation. Bucket config is ~10 LOC in `observability.rs`. Risk: `Matcher`
import from `metrics-exporter-prometheus` needed; API is stable at 0.15.
