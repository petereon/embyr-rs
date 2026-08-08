# Slice OBS-04 — Rate Limit Allow/Reject Counters per Project

**Feature:** observability
**Slice:** OBS-04 of OBS-05
**Estimate:** 0.5 day
**Stories:** US-OBS-04
**Depends on:** OBS-01 complete (Prometheus recorder installed)
**Status:** IMPLEMENTED (commit 87ffced, 2026-08-08)

---

## Goal

Increment `embyr_rate_limit_requests_total{project_id, outcome}` on every
`RateLimiter::check()` call — both `Ok(allowed)` and `Err(rejected)` arms.
Sam can now prove per-project fair-multitenancy enforcement to auditors and
validate that JOB-11 rate limits are working across nodes.

---

## Learning Hypothesis

Disproves: "Adding a project_id label to a high-cardinality counter causes
Prometheus memory exhaustion for deployments with thousands of projects."

Validates: This is a HIGH cardinality risk. The slice must document the
trade-off: `project_id` label on a per-request counter is appropriate for
embyr deployments up to ~10,000 projects (within Prometheus defaults).
If cardinality is a concern, operators can use recording rules to aggregate.
This is a locked decision (D-OBS-7) rather than ignored.

Confirms if succeeds: After a rate-limit rejection, scraping /metrics shows
`embyr_rate_limit_requests_total{project_id="fintech-acme",outcome="rejected"} 1`
and the allowed counter for the same project reflects the bucket fill pattern.

**Result:** CONFIRMED. All three obs04 acceptance tests pass. High-cardinality
trade-off documented in ADR-016 and operator runbook.

---

## IN Scope

- Instrument `RateLimiter::check()` return value in
  `crates/embyr-server/src/middleware/rate_limit.rs`:
  - `Ok(_)` arm: `metrics::counter!("embyr_rate_limit_requests_total", "project_id" => project_id.to_owned(), "outcome" => "allowed").increment(1)`
  - `Err(_)` arm: `metrics::counter!("embyr_rate_limit_requests_total", "project_id" => project_id.to_owned(), "outcome" => "rejected").increment(1)`
- Also increment `embyr_rate_limit_pg_timeout_total` (no labels, low cardinality)
  at the `tracing::warn!` call site in `check()` — this replaces the existing
  log-only signal with a proper counter while retaining the `tracing::warn!`
- `metrics.workspace = true` dependency already present after OBS-01

## OUT Scope

- Per-project rate limit configuration changes (future ADR, D4 from DRL)
- `embyr_rate_limit_requests_total` by method (not needed in V1)
- Alerting thresholds (DEVOPS wave)

---

## Acceptance Criteria

See `feature-delta.md` US-OBS-04 for AC-OBS-04-01 through AC-OBS-04-04.

Key gate: acceptance test exhausts the rate bucket for project "fintech-acme",
then asserts `embyr_rate_limit_requests_total{project_id="fintech-acme",outcome="rejected"}`
> 0 and `embyr_rate_limit_requests_total{project_id="fintech-acme",outcome="allowed"}`
matches the bucket capacity used before exhaustion.

---

## Dependencies

- OBS-01 complete
- `crates/embyr-server/src/middleware/rate_limit.rs` — `Ok` and `Err` return arms of `check()`
- Existing `drl_b12_postgres_rate_limit` acceptance tests as reference for
  exhaustion scenario setup

---

## Effort Estimate

0.5 day. 2 return arms in `rate_limit.rs` plus 1 new counter for pg timeout.
Risk: `project_id` label ownership — `check()` receives `&str`, which must be
converted to an owned `String` for the label value (the `metrics!` macro requires
`'static` or owned). Reference: existing `tracing::warn!` at line 148 already
uses `project_id = project_id` label on the structured log.
