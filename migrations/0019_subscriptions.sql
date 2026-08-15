-- Migration 0019: subscriptions table + accounts.stripe_customer_id (card-payments-backend, US-201)
--
-- One row per account (not per project — an account can own multiple projects,
-- confirmed via the existing projects.account_id FK, migrations/0015). The row is
-- a read cache of Stripe's own subscription object; Stripe remains the source of
-- truth, kept in sync by the webhook handler (US-203).
--
-- plan/status are CHECK-constrained to the exact enumerated values D-4/D-6/D-12
-- lock (Free + Pro tiers only; four subscription statuses).
--
-- current_period_start/current_period_end are Pro-plan-display-only (ADR-020 §
-- Billing Cycle Boundary) — NULL for Free-plan accounts, which have no
-- guaranteed Stripe Subscription object and use the UTC-calendar-month cycle
-- for cap computation instead (never sourced from these columns).

ALTER TABLE accounts ADD COLUMN IF NOT EXISTS stripe_customer_id TEXT UNIQUE;

CREATE TABLE subscriptions (
    account_id              UUID         NOT NULL PRIMARY KEY
                                          REFERENCES accounts(id) ON DELETE CASCADE,
    plan                    VARCHAR(10)  NOT NULL DEFAULT 'free'
                                          CHECK (plan IN ('free', 'pro')),
    status                  VARCHAR(20)  NOT NULL DEFAULT 'active'
                                          CHECK (status IN (
                                              'active',
                                              'past_due',
                                              'free_cap_exceeded',
                                              'canceled'
                                          )),
    stripe_subscription_id  TEXT,
    current_period_start    TIMESTAMPTZ,
    current_period_end      TIMESTAMPTZ,
    updated_at               TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX subscriptions_status_idx ON subscriptions(status);
