# Slice 05 — Nightly Usage Metering to Stripe

**Feature:** card-payments-backend
**Slice:** 05 of 07 | **Release:** 2 (Usage Metering)
**Estimate:** 2 days
**Stories:** US-205
**Depends on:** Slice 01 (`stripe_customer_id` must exist to attach usage records to)

---

## Goal

A batch job pushes one Stripe usage record per project per non-zero dimension per day, reading `daily_project_metrics` (D-8). Operator-triggerable for testing/demo.

## Learning Hypothesis

Disproves: "A per-project-per-dimension-per-day usage push is not naturally idempotent, risking double-billing on any re-run."
Confirms if succeeds: An idempotency key derived from `{project_id, dimension, date}` is sufficient to make the entire batch safely re-runnable without any external locking/coordination.

## IN Scope

- `POST /admin/v1/billing/run-metering` — operator-authed only (mirrors `operator_router`)
- Reads yesterday's `daily_project_metrics` rows
- Pushes one Stripe usage record per project per non-zero dimension (reads/writes/deletes/storage, D-7)
- Idempotent re-run: no dimension double-pushed for an already-processed project/day
- Per-project failure isolation: one project's Stripe API failure does not abort the rest of the run

## OUT Scope

- Scheduled/cron wiring itself (DEVOPS-wave scope — this slice builds the job and its manual trigger; scheduling infrastructure is separate)
- Pricing/rate calculation display (that's the frontend's `NextInvoiceCard`, already shipped against mock data)
- Backfill of historical days before this feature ships

## Acceptance Criteria

- AC-205-01 through AC-205-05 (see feature-delta.md US-205)

## Dependencies

- Slice 01 complete
- Real `daily_project_metrics` rows (existing table, unchanged)
- Real Stripe test-mode usage-record API (D-13)

## Effort Estimate

2 days. Reference class: `billing.rs`'s existing `daily_project_metrics` aggregation query is directly reusable as the read side; the new work is the Stripe usage-record push + idempotency key design + per-project failure isolation.
