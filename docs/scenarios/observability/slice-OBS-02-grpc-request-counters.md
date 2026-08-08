# Slice OBS-02 — gRPC Request Counters and Error Rates per Method

**Feature:** observability
**Slice:** OBS-02 of OBS-05
**Estimate:** 0.75 day
**Stories:** US-OBS-02
**Depends on:** OBS-01 complete (Prometheus recorder installed)
**Status:** IMPLEMENTED (commit 87ffced, 2026-08-08)

---

## Goal

Increment `embyr_grpc_requests_total{method, status}` on every gRPC handler
return — both success and error paths. Sam can now count requests per method
and compute error rates in PromQL.

---

## Learning Hypothesis

Disproves: "Adding metrics::counter! calls to all gRPC handler call sites
blocks the hot path or causes false error counts due to mismatched status label
values."

Confirms if succeeds: Scraping /metrics after a GetDocument call shows
`embyr_grpc_requests_total{method="GetDocument",status="ok"} 1` (or higher);
a failed auth attempt shows `embyr_grpc_requests_total{method="GetDocument",status="unauthenticated"} 1`.

**Result:** CONFIRMED. Walking skeleton `grpc_request_increments_embyr_grpc_requests_total`
passes. All 10 handlers instrumented (see correction note below).

---

## IN Scope

- Define metric name convention: `embyr_grpc_requests_total` with labels:
  - `method`: short gRPC method name — `GetDocument`, `CreateDocument`,
    `UpdateDocument`, `DeleteDocument`, `BatchGetDocuments`, `BeginTransaction`,
    `Commit`, `Rollback`, `RunQuery`, `Listen`
  - `status`: tonic status code as lowercase string (`"ok"`, `"not_found"`,
    `"unauthenticated"`, `"permission_denied"`, `"resource_exhausted"`,
    `"internal"`, `"unavailable"`, `"aborted"`, `"already_exists"`,
    `"invalid_argument"`, `"failed_precondition"`, `"unimplemented"`)
- Add `metrics::counter!("embyr_grpc_requests_total", "method" => method, "status" => status).increment(1)` call
  at the end of each gRPC handler call site in
  `crates/embyr-server/src/grpc/handler.rs`
- Counter incremented AFTER the result is determined (on both OK and error paths)
- Helper function `grpc_status_label(status_code: tonic::Code) -> &'static str`
  in `crates/embyr-server/src/middleware/obs_helpers.rs` (new file)

**Correction from DISTILL brief:** The DISTILL brief named the helper file
`middleware/metrics.rs`. The DESIGN wave resolved this as `obs_helpers.rs` to
avoid ambiguity with the existing `admin/handlers/metrics.rs` handler. The
DELIVER implementation used `obs_helpers.rs`.

**Correction from DISTILL brief:** The DISTILL brief listed 9 handler call sites;
the correct count is **10** (all currently implemented `impl Firestore` handlers:
`GetDocument`, `CreateDocument`, `UpdateDocument`, `DeleteDocument`,
`BatchGetDocuments`, `BeginTransaction`, `Commit`, `Rollback`, `RunQuery`, `Listen`).

## OUT Scope

- Latency histograms (OBS-03)
- Rate limit counters (OBS-04)
- Admin port or REST port instrumentation

---

## Acceptance Criteria

See `feature-delta.md` US-OBS-02 for AC-OBS-02-01 through AC-OBS-02-05.

Key gate: acceptance test fires `GetDocument` → asserts
`embyr_grpc_requests_total{method="GetDocument",status="ok"}` count ≥ 1;
fires a second `GetDocument` on an unknown project → asserts
`embyr_grpc_requests_total{method="GetDocument",status="not_found"}` count ≥ 1.

---

## Dependencies

- OBS-01 complete
- `embyr-server/src/grpc/handler.rs` — 10 handler call sites to instrument

---

## Effort Estimate

0.75 day. 10 call sites × ~3 LOC each = ~30 LOC. Risk: tonic 0.12 error type
mapping to status codes; the `tonic::Status::code()` method returns `tonic::Code`
which maps to a string via a new helper. Reference class: DRL-04 modified all
gRPC handler call sites in one step for rate-limit header attachment.
