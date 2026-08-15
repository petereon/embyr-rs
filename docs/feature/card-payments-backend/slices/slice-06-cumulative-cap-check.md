# Slice 06 — Real-Time Cumulative Usage Is Computed Against the Plan Cap

**Feature:** card-payments-backend
**Slice:** 06 of 07 | **Release:** 3 (Real-Time Cap Enforcement)
**Estimate:** 1.5 days
**Stories:** US-206
**Depends on:** Slice 01 (`subscriptions.plan` needed to know which accounts are Free)

---

## Goal

A new, separately-keyed (account, not project) monthly-cumulative usage-vs-cap computation is exposed read-only via `GET /admin/v1/billing/subscription`'s new `cap_status` field — deliberately NOT yet wired to any enforcement action.

## Learning Hypothesis

Disproves: "A monthly-cumulative-vs-cap check can be added to the request hot path without either violating the existing 20ms-latency-bound precedent (`rate_limit.rs`'s `RATE_LIMIT_PG_TIMEOUT_MS`) or requiring a new caching layer neither `rate_limit.rs` nor `daily_project_metrics` currently has."
Confirms if succeeds: A viable computation strategy exists that respects the existing latency-bound precedent — proven in isolation, before any suspend action depends on it (Slice 07).

## IN Scope

- `cap_status` field on `GET /admin/v1/billing/subscription`: per-dimension `{used, cap, pct}`, summed across ALL of the account's projects
- Free-plan-only (Pro accounts get no `cap_status`, D-6/D-7)
- Boundary-inclusive: exactly-at-cap reports 100% (matches frontend's `≥100%` red threshold, US-102)
- Fail-open on computation unavailability/staleness (mirrors `rate_limit.rs`'s existing Postgres-timeout fail-open branch)
- Resets at billing-cycle boundary, not account-creation date

## OUT Scope

- **Any enforcement/suspension action** — this slice is read-only visibility. Slice 07 wires the trigger.
- The concrete computation mechanism (live-per-request vs. cached vs. incrementally-maintained) — explicitly a DESIGN-wave decision, see feature-delta.md § Open Question

## Acceptance Criteria

- AC-206-01 through AC-206-06 (see feature-delta.md US-206)

## Dependencies

- Slice 01 complete
- `daily_project_metrics` (existing, read-only consumer)

## Effort Estimate

1.5 days. Reference class: no direct precedent exists in this codebase for account-keyed (vs. project-keyed) aggregation in the request hot path — this is genuinely new work, hence the explicit § Open Question flag rather than a reuse claim.
