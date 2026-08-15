# Slice 03 — Webhook Ingestion Keeps Subscription State in Sync

**Feature:** card-payments-backend
**Slice:** 03 of 07 | **Release:** 1 (Subscription & Payment Lifecycle)
**Estimate:** 1.5 days
**Stories:** US-203
**Depends on:** Slice 01 (`subscriptions` table must exist to sync into)

---

## Goal

A 5th sub-router (`POST /admin/v1/webhooks/stripe`) verifies Stripe signatures, dedupes by `event.id`, and syncs `customer.subscription.*` events into the local `subscriptions` row.

## Learning Hypothesis

Disproves: "Webhook signature verification + idempotency cannot be meaningfully exercised without a mocked port, contradicting D-13's no-mock stance."
Confirms if succeeds: Stripe CLI's `stripe trigger` is sufficient to exercise real signed webhook delivery end-to-end in tests, with no mocked `IPaymentGateway`-style port needed.

## IN Scope

- New migration: `processed_webhook_events` table (`event_id` unique, `processed_at`)
- 5th sub-router on `:9090`, structural sibling of `public_router` (D-11)
- `stripe_signature_middleware` verifying `Stripe-Signature` against `STRIPE_WEBHOOK_SIGNING_SECRET`
- Idempotent processing: redelivered `event.id` is a 200 no-op
- `customer.subscription.updated` / `customer.subscription.deleted` handlers syncing the local `subscriptions` row
- Unhandled-but-validly-signed event types return 200 (forward-compatible, not an error)

## OUT Scope

- `invoice.payment_failed` / `invoice.payment_succeeded` handling (Slice 04 — dunning is a distinct handler even though it shares this router)
- Reconciliation/backfill of missed events — not in this slice; DEVOPS-wave KPI #1 measurement is a separate, later concern

## Acceptance Criteria

- AC-203-01 through AC-203-06 (see feature-delta.md US-203)

## Dependencies

- `STRIPE_WEBHOOK_SIGNING_SECRET` env var (per `config.rs` pattern)
- Stripe CLI installed for `stripe trigger` test synthesis (D-13)
- Slice 01 complete

## Effort Estimate

1.5 days. Reference class: `router.rs`'s existing 4-sub-router pattern is directly reusable for router structure; the new work is signature verification middleware + idempotency table + event-type dispatch.
