# Slice 06 — Invoice History

**Feature:** card-payments
**Slice:** 06 of 08
**Estimate:** 1 day (likely &lt;1 day; kept as its own slice for a clean independent demo)
**Stories:** US-107
**Depends on:** Slice 01 (Invoices tab shell)

---

## Goal

Billing → Invoices shows a table of past and upcoming invoices (Date/Period/Base/Overage/Total/
Status) with PDF download on paid invoices, and the documented Free-plan empty state.

## Learning Hypothesis

Disproves: "Invoice history is low-priority and can be deferred past the walking skeleton without
losing demo coherence." Confirms if succeeds: a standalone, independently reviewable slice proves
invoice history has no hidden coupling to plan-change or payment-capture state beyond what's
already modeled (`subscription.plan` gates the empty state; nothing else).

## IN Scope

- `BillingInvoicesTab`: table columns Date/Period/Base/Overage/Total/Status; PDF-download link on
  `status: paid`, none on `status: upcoming`
- Free-plan empty state: "The Free plan has no recurring charges — invoices appear once you're on
  Pro." (verbatim)
- Invoice history persists across plan changes (mock `AppModel.invoices` is independent of
  `subscription.plan`, not cleared on downgrade)
- Mock: 3-4 invoices per Pro-plan seed account (mix of paid + one upcoming)

## OUT Scope

- Real Stripe invoice list API (`card-payments-backend`)
- Real PDF generation (mock link is a stub href in V1)

## Acceptance Criteria

From US-107: AC-107-01 through AC-107-04.

## Dependencies

- Slice 01 (Invoices tab shell)
