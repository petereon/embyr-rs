# Slice 02 — Free-Plan Cap Usage Visibility

**Feature:** card-payments
**Slice:** 02 of 08
**Estimate:** 1 day
**Stories:** US-102
**Depends on:** Slice 01 (Overview tab shell)

---

## Goal

Free-plan accounts see a Cap Usage card on Billing → Overview: one progress bar per usage
dimension (reads/writes/deletes/storage), colored accent/amber/red by proximity to the cap, with
an expandable per-database breakdown.

## Learning Hypothesis

Disproves: "Cap proximity as a percentage/color isn't enough signal — admins need raw numbers to
trust the bar." (If review/demo feedback shows the bar alone is insufficient, per-dimension raw
numbers must be promoted from the expand-affordance to always-visible — informs Slice 03/04 layout.)
Confirms if succeeds: Amber-at-80%/red-at-100% bars with an expand-for-detail pattern is sufficient
for Chris to decide whether to upgrade, without needing raw numbers up front.

## IN Scope

- Cap ratio derivation: `usage / FREE_CAPS[dimension]` — pure function, no duplicated stored state
  (per project's functional-where-practical paradigm)
- `CapUsageCard`: 4 bars (reads=zap, writes=activity, deletes=trash, storageGB=database icons),
  color thresholds (accent <80%, amber 80-99%, red ≥100%), percentage label per bar
- Expand affordance revealing per-database contribution, summing to the card total
- Card renders only when `subscription.plan == Free`
- Mock usage data varying across the 3 seeded accounts (healthy, near-cap, at-cap) to exercise all
  three color states in the demo

## OUT Scope

- Next Invoice card (Pro plan) — Slice 03
- Usage tab per-database table — Slice 03
- Real usage data from `daily_project_metrics` (`card-payments-backend`)

## Acceptance Criteria

From US-102: AC-102-01 through AC-102-06.

## Dependencies

- Slice 01 (Overview tab renders `PlanCard`; `CapUsageCard` sits alongside it)
