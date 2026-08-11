# Slice 08 — Test-Mode Payment Simulator (Dev-Only)

**Feature:** card-payments
**Slice:** 08 of 08
**Estimate:** 0.5 day
**Stories:** US-109
**Depends on:** Slice 07 (SuspensionBanner must exist to demo against)

---

## Goal

A dashed-border, amber "TEST MODE"-badged `TestClockCard` on Billing → Overview toggles
`subscription.payment_failure` via a Segmented control, letting Chris (or QA) see the
SuspensionBanner's red `past_due` state without a real Stripe webhook event.

## Learning Hypothesis

Disproves: "The SuspensionBanner's payment-failed state can only be verified via a real Stripe CLI
`stripe trigger` event, making it impractical to demo or regression-test in this UI-only slice."
Confirms if succeeds: a direct mock-state setter reproduces the exact same rendering path as a
real webhook-driven update would (same `effectiveStatus` derivation, same `SuspensionBanner`
component) — proving the derivation function is the right seam for the eventual real integration.

## IN Scope

- `TestClockCard`: dashed border, amber "TEST MODE" badge, `Segmented` control ("Payment
  succeeds" / "Payment fails")
- Toggling sets `model.subscription.payment_failure`, which flows through the existing
  `effectiveStatus` derivation (Slice 07) to immediately re-render `SuspensionBanner` — no page
  reload
- Toggle scoped to the currently-viewed account's mock state only
- Visual gating: card is styled and labeled as test-only (V1 requires only the visual marker;
  build-level `#[cfg]`/feature-flag gating for non-production builds is a DESIGN-wave decision)

## OUT Scope

- Real Stripe CLI `stripe trigger` integration — `card-payments-backend`
- Any production-facing use of this card

## Acceptance Criteria

From US-109: AC-109-01 through AC-109-05.

## Dependencies

- Slice 07 (SuspensionBanner + `effectiveStatus` derivation)

## Technical Notes

V1 direct setter (`Msg::SetPaymentFailure`) mirrors the JSX prototype for interaction-parity
testing. `card-payments-backend` should replace the setter's *source* (real webhook vs. this
toggle) without changing the derivation or `SuspensionBanner` component at all — same seam as
Slice 04/05's Resource/Action migration contract (ADR-007).
