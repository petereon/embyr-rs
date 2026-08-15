# Slice 04 — Dunning Suspension and Payment Recovery

**Feature:** card-payments-backend
**Slice:** 04 of 07 | **Release:** 1 (Subscription & Payment Lifecycle)
**Estimate:** 1.5 days
**Stories:** US-204
**Depends on:** Slice 03 (webhook router/dispatch must exist)

---

## Goal

`invoice.payment_failed` (final-failure) suspends every project under an account; `invoice.payment_succeeded` reactivates them — both via the exact same `set_project_status` path used elsewhere (D-12).

## Learning Hypothesis

Disproves: "D-12's 'one suspension mechanism, two triggers' design has hidden per-trigger special-casing once implemented" (e.g. this trigger ends up needing its own parallel status-update logic because of some detail the existing operator-initiated `suspend_project` didn't anticipate, like multi-project accounts).
Confirms if succeeds: `lifecycle.rs`'s existing single-project `set_project_status` function generalizes cleanly to "suspend every project under this account" with no duplicated logic.

## IN Scope

- `invoice.payment_failed` (final-failure indicator only, not intermediate retries) suspends ALL active projects under the account
- `invoice.payment_succeeded` reactivates ALL suspended projects under the account
- Both call the identical `set_project_status(..., "suspended"/"active", ...)` function `lifecycle.rs` already exports
- Credential cache eviction on suspension, matching today's `suspend_project` behavior byte-for-byte

## OUT Scope

- Free-cap-exceeded triggering (Slice 07 — a different trigger, same mechanism, built later)
- Grace-window/retry-count configuration — governed by Stripe's own Smart Retries schedule (D-12), not application code

## Acceptance Criteria

- AC-204-01 through AC-204-05 (see feature-delta.md US-204)

## Dependencies

- Slice 03 complete (webhook dispatch)
- `stripe trigger invoice.payment_failed` / `invoice.payment_succeeded` (D-13 test synthesis)

## Effort Estimate

1.5 days. Reference class: `lifecycle.rs`'s `set_project_status` is a ~25-line reusable function; the new work is the multi-project-per-account fan-out and the final-vs-intermediate-retry distinction.
