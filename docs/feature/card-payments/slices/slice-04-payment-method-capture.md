# Slice 04 — Payment Method Capture (Card Modal)

**Feature:** card-payments
**Slice:** 04 of 08
**Estimate:** 1 day
**Stories:** US-105
**Depends on:** Slice 01 (CardModal shell + PaymentMethodCard exist)

---

## Goal

Chris can add or update a card via a Stripe-Elements-styled modal (brand detection, inline
validation) that updates the Payment Method card on submit. Implements D-3 and D-5's
card-required-even-for-Free policy at the UI layer.

## Learning Hypothesis

Disproves: "A Rust-native form (no real Stripe.js yet) can't convincingly model the
Stripe-Elements capture UX (brand detection, formatting, validation) closely enough to validate
the interaction design before the real JS interop shim exists."
Confirms if succeeds: form/validation/state-update behavior is fully specifiable and testable in
pure Rust now; the eventual JS interop shim (`card-payments-backend`) is a drop-in replacement for
the input widget only, not a redesign of the modal's interaction flow.

## IN Scope

- `CardModal`: number/expiry/CVC inputs styled per `.stripe-el` class family, brand
  auto-detection from number prefix, number formatting
- Inline validation (incomplete/invalid number disables Save)
- Submit updates `model.subscription.card` (replaces, not appends) + closes modal + confirmation
  toast
- PCI SAQ-A reassurance copy + "Secured by Stripe · test mode, no real charge" footer (always
  visible, matches design-reference.md verbatim)

## OUT Scope

- Real Stripe.js/Stripe Elements JS interop shim (iframe-based widget) — `card-payments-backend`
- Real token exchange with Stripe's API — `card-payments-backend`
- Luhn-check or full card-network validation — V1 uses prefix-based brand detection + length check
  only, matching the design prototype's `detectBrand()` helper

## Acceptance Criteria

From US-105: AC-105-01 through AC-105-06.

## Dependencies

- Slice 01 (CardModal shell, PaymentMethodCard)

## Technical Notes

The stubbed submit handler in this slice stands in for the real Stripe token exchange; the
migration to real Stripe Elements is a same-shape swap inside the submit `Action`, per ADR-007's
Resource/Action migration contract — not a rewrite of this slice's form/validation code.
