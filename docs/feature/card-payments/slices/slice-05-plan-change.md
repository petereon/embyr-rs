# Slice 05 — Plan Change (Upgrade / Downgrade)

**Feature:** card-payments
**Slice:** 05 of 08
**Estimate:** 1 day
**Stories:** US-106
**Depends on:** Slice 01 (UpgradeModal shell + PlanCard exist)

---

## Goal

Chris can upgrade Free→Pro or downgrade Pro→Free through a two-step modal (compare → confirm),
with an explicit, unskippable hard-cap warning on downgrade per D-6. Confirming updates the Plan
card immediately and clears a `free_cap_exceeded` suspension if one was active.

## Learning Hypothesis

Disproves: "A single-step plan toggle is sufficient — users don't need a comparison view before
committing." Confirms if succeeds: the two-step compare→confirm pattern (with a forced warning on
downgrade) is the right amount of friction — enough to prevent an accidental downgrade-into-
suspension, not so much that upgrading feels bureaucratic.

## IN Scope

- `UpgradeModal` compare step: `PlanColumn` × 2 (Free/Pro) from `PLAN_FEATURES`, included volumes
- Confirm step: Free→Pro updates `subscription.plan` immediately, closes modal, confirmation toast
- Downgrade path: mandatory confirmation step showing "Hard usage caps apply immediately —
  exceeding them suspends the console until you upgrade again." (verbatim from design-reference.md)
  before the downgrade can be confirmed
- Upgrading while `effectiveStatus == free_cap_exceeded` clears the suspended/read-only state
- Cancel/close at any step: no state changes

## OUT Scope

- Real Stripe subscription update API (`card-payments-backend`)
- Proration calculation display (out of scope for all of card-payments per grill-me's deferred
  "actual price points" note — illustrative pricing only)

## Acceptance Criteria

From US-106: AC-106-01 through AC-106-06.

## Dependencies

- Slice 01 (UpgradeModal shell, PlanCard)
- Slice 02's `capExceeded` derivation (reused to pre-select Pro and to test the suspension-clearing
  path)
