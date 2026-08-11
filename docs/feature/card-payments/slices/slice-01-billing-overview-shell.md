# Slice 01 — Walking Skeleton: Billing Overview Shell

**Feature:** card-payments
**Slice:** 01 of 08
**Estimate:** 1 day
**Stories:** US-101
**Depends on:** none (existing `Section::Billing` nav link + `Tabs`/`Modal` primitives from user-admin-ui)

---

## Goal

Billing → Overview renders a real Plan card and Payment Method card from mock subscription data,
plus a navigable Overview/Usage/Invoices tab shell — replacing today's all-placeholder page. This
is the thinnest end-to-end cut across every activity in the journey (plan status, payment status,
tab navigation to usage/invoices, and a stub launch point for the upgrade/card modals).

## Learning Hypothesis

Disproves: "The existing `Section::Billing` + `Tabs`/`Modal` primitives from user-admin-ui don't
compose cleanly for a denser, multi-card billing layout."
Confirms if succeeds: The established Leptos TEA conventions (model.rs/msg.rs/update.rs) and
existing primitives extend to card-payments without new architectural patterns — remaining
7 slices can proceed as pure additive work.

## IN Scope

- `AppModel.subscription: Subscription` (plan, card: Option<Card>, stripe_customer_id,
  current_period_end, payment_failure: bool) + mock constructor in `data.rs`
- `views/billing/mod.rs` + `overview.rs` (new subdirectory, mirrors `views/db_detail/` precedent)
- `PlanCard`: Free/Pro badge, Free shows included volume (FREE_CAPS), Pro shows base price +
  renewal date
- `PaymentMethodCard`: brand/last4/expiry + green "On file" badge when card present; "No card on
  file" + "Add card" CTA when absent; mono `stripeCustomerId` display
- `Tabs` (Overview/Usage/Invoices) wired to `views/billing/mod.rs`; Usage and Invoices tabs render
  structural stubs this slice (real content in Slices 02-03, 06)
- `UpgradeModal`/`CardModal` open as empty shells (open/close only, no save logic yet)

## OUT Scope

- Cap Usage / Next Invoice card content (Slice 02-03)
- Real card capture / plan-change logic (Slice 04-05)
- Invoice table content (Slice 06)
- SuspensionBanner (Slice 07)
- Real Stripe data (`card-payments-backend`, follow-up feature)

## Acceptance Criteria

From US-101: AC-101-01 through AC-101-07 (see feature-delta.md).

## Dependencies

- `Section::Billing`, `Tabs`, `Modal` primitives (exist, user-admin-ui)
- `data.rs` mock module (exists, ADR-007 pattern)

## Pre-Slice Spike

None — extends proven primitives; no new technology introduced.
