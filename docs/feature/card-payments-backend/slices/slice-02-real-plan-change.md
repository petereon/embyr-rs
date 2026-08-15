# Slice 02 — Plan Change Persists to a Real Stripe Subscription

**Feature:** card-payments-backend
**Slice:** 02 of 07 | **Release:** 1 (Subscription & Payment Lifecycle)
**Estimate:** 1 day
**Stories:** US-202
**Depends on:** Slice 01 (`stripe_customer_id`, `subscriptions` row must already exist)

---

## Goal

`POST /admin/v1/billing/subscription` changes a real Stripe Subscription object; local state only updates after Stripe confirms.

## Learning Hypothesis

Disproves: "Optimistic local-state updates on plan change are safe" — i.e. this slice proves the opposite is required: local state must only change after Stripe confirms, or a failed API call silently desyncs local truth from Stripe's truth.
Confirms if succeeds: A write-through (not write-behind) pattern against a real external API is workable within existing request-handling latency norms.

## IN Scope

- `POST /admin/v1/billing/subscription` with `{"plan": "free" | "pro"}` — session-authed, Owner/Admin role (`check_rbac`)
- Real Stripe Subscription create/update call
- Local `subscriptions.plan` update ONLY after Stripe confirms
- Upgrading while `status='free_cap_exceeded'` clears the status and reactivates the account's projects (calls the same suspend/activate path as US-204/US-207)

## OUT Scope

- Downgrade-scheduling semantics (immediate vs. period-end) — DESIGN decision, not locked here
- Webhook-driven sync (Slice 03) — this slice is the direct-API-call path only
- Enterprise/third-tier plans (D-4 rules this out entirely)

## Acceptance Criteria

- AC-202-01 through AC-202-05 (see feature-delta.md US-202)

## Dependencies

- Slice 01 complete (real `stripe_customer_id` must exist)
- `check_rbac` role-gating pattern (existing, reused)

## Effort Estimate

1 day. Reference class: `check_rbac` and the RBAC-gated handler shape already exist elsewhere in `admin/handlers`; the new work is the Stripe Subscription API call and the write-through ordering.
