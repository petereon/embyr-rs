# Slice 03 — Pro-Plan Invoice Forecast + Per-Database Usage Breakdown

**Feature:** card-payments
**Slice:** 03 of 08
**Estimate:** 1 day
**Stories:** US-103, US-104
**Depends on:** Slice 01 (Overview shell), Slice 02 (usage-derivation pure function reused)

---

## Goal

Pro-plan accounts see a Next Invoice card estimating overage cost by dimension. The Usage tab
(all plans) shows a real per-database reads/writes/deletes/storage table with 3-color stacked bars
and 5 KPI summary tiles — directly superseding the all-"—" placeholder table in the existing
`views/billing.rs`.

## Learning Hypothesis

Disproves: "Overage estimation math (usage × PRICING rates) is confusing without a dimension-by-
dimension breakdown." Confirms if succeeds: itemized breakdown + "estimated, projecting to period
end" framing is clear enough that Dana doesn't need a support conversation to understand her bill.

## IN Scope

- `NextInvoiceCard`: base + per-dimension overage line items (reads/writes/deletes at
  `PRICING.overage[dim]` per `overageUnit`; storage at `storagePerGB`), total, "Estimated —
  projecting to period end" label, illustrative-pricing footer disclosure. Renders only for Pro plan.
- Zero-overage case: "$49.00 base · no overage projected"
- Usage tab: replace `views/billing.rs`'s placeholder `"—"` cells with real mock reads/writes/
  deletes/storage per database; 3-color stacked bar per row (reads=blue, writes=accent,
  deletes=red); 5 KPI summary tiles totaling the table; preserve existing time-range selector and
  empty state ("No databases — nothing to bill")

## OUT Scope

- Real Stripe invoice preview API (`card-payments-backend`)
- Invoice *history* table (Slice 06 — this slice is the forward-looking estimate only)

## Acceptance Criteria

From US-103: AC-103-01 through AC-103-06.
From US-104: AC-104-01 through AC-104-05.

## Dependencies

- Slice 02's usage-derivation pure function (reused, not duplicated)
- Existing `views/billing.rs` table structure and time-range selector (extended in place)

## Technical Notes

This slice directly resolves the "V2 plan" doc-comment in the existing `views/billing.rs`
(replace placeholder table with real mock-backed data; real server-function wiring stays
`card-payments-backend` scope per ADR-007's migration contract).
