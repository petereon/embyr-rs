# Slice 07 — Suspension Banner (Cross-Cutting)

**Feature:** card-payments
**Slice:** 07 of 08
**Estimate:** 1 day
**Stories:** US-108
**Depends on:** Slice 05 (UpgradeModal, for the cap-exceeded CTA target), Slice 04 (CardModal, for
the payment-failed CTA target)

---

## Goal

A `SuspensionBanner` renders above every routed section (Dashboard, Databases, Billing, etc.) —
not just Billing — whenever the account is read-only, explaining why in plain language and routing
to the correct recovery action. Implements D-6's hard-stop as a legible, actionable UI state
rather than a silent lockout.

## Learning Hypothesis

Disproves: "Surfacing suspension status only inside Billing → Overview is sufficient — users will
find it." (Risk: if the banner isn't global, a suspended user clicking around Dashboard/Databases
sees no explanation at all, which is worse than no feature.)
Confirms if succeeds: rendering the banner in `ShellView` above all `<Show>`-routed content, gated
on a single pure-derived `readOnly` boolean, is a correct and sufficient cross-cutting placement.

## IN Scope

- `effectiveStatus`/`capExceeded`/`readOnly` derivation as a pure function over
  `subscription` + `usage_totals` + `FREE_CAPS` (per CLAUDE.md's functional-where-practical
  preference — not duplicated stored booleans)
- `SuspensionBanner` component: amber state ("You've reached your Free plan limits for this
  cycle." + "Upgrade to Pro" CTA) for `free_cap_exceeded`; red state ("We couldn't process your
  last payment." + "Update payment method" CTA) for any other read-only cause
- Rendered in `ShellView` above all routed `Section` content
- Banner CTA opens the correct modal (Upgrade for cap-exceeded, Card for payment-failed)
- No banner renders when `effectiveStatus == active`

## OUT Scope

- Actually disabling/graying out write actions elsewhere in the console when `readOnly == true` —
  this banner is advisory visibility only in card-payments; real enforcement (blocking writes) is
  the rate-limiter extension in `card-payments-backend`. Flagged explicitly so DESIGN does not
  assume this slice blocks any action beyond rendering the banner.
- Real webhook-driven status transitions (`card-payments-backend`)

## Acceptance Criteria

From US-108: AC-108-01 through AC-108-06.

## Dependencies

- Slice 04 (CardModal — payment-failed CTA target)
- Slice 05 (UpgradeModal — cap-exceeded CTA target)
- Slice 02's cap-ratio derivation (reused for `capExceeded`)
