# Slice 01 — Real Subscription Record Exists (Walking Skeleton)

**Feature:** card-payments-backend
**Slice:** 01 of 07 | **Release:** 1 (Subscription & Payment Lifecycle)
**Estimate:** 1.5 days
**Stories:** US-201
**Depends on:** none (Walking Skeleton)

---

## Goal

`GET /admin/v1/billing/subscription` returns a real, Stripe-backed subscription record, auto-provisioning a Stripe Customer on first call.

## Learning Hypothesis

Disproves: "A real Stripe SDK network round-trip cannot integrate cleanly inside an existing async Axum session-authed handler at acceptable latency, with no mocked port to fall back on (D-13)."
Confirms if succeeds: The existing `UserAdminState`/session-auth pattern extends cleanly to a real external-API-calling handler, and D-13's no-mock constraint is workable in practice, not just in principle.

## IN Scope

- Migration `0019_subscriptions.sql`: `subscriptions` table (`account_id`, `plan`, `status`, `stripe_subscription_id`, `current_period_end`)
- Migration: `accounts.stripe_customer_id` column
- `GET /admin/v1/billing/subscription` — session-authed, any role
- Lazy Stripe Customer provisioning on first call (idempotent — never creates a duplicate Customer for the same account)
- `STRIPE_SECRET_KEY` resolved via the `config.rs` secret-manager pattern

## OUT Scope

- Plan changes (Slice 02)
- Webhook ingestion (Slice 03)
- Card capture (explicitly out of scope for the whole feature — see feature-delta.md § Out of Scope)
- `cap_status` field (Slice 06)

## Acceptance Criteria

- AC-201-01 through AC-201-06 (see feature-delta.md US-201)

## Dependencies

- `STRIPE_SECRET_KEY` env var (or AWS/GCP secret ARN, per `config.rs` pattern)
- Real Stripe test-mode account (`sk_test_...`), per D-13
- System DB running (Postgres)

## Effort Estimate

1.5 days. Reference class: `config.rs`'s existing secret-resolution pattern is directly reusable; the new work is the Stripe SDK call + idempotent provisioning logic + one migration.
