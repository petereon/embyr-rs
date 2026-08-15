-- Migration 0020: processed_webhook_events table (card-payments-backend, US-203)
--
-- Idempotency ledger for Stripe webhook delivery. Stripe redelivers events on
-- non-2xx/timeout responses; event_id is the PRIMARY KEY so a redelivered
-- event is a no-op INSERT ... ON CONFLICT DO NOTHING, mirroring the existing
-- sdk_api_keys.key_hash UNIQUE-constraint idempotency shape (migrations/0012).
--
-- No FK to subscriptions/accounts — a webhook event may arrive for an account
-- that does not yet exist locally (e.g. the very first customer.subscription
-- event racing US-201's lazy provisioning); the dedupe ledger must accept it
-- unconditionally by event_id alone.

CREATE TABLE processed_webhook_events (
    event_id      TEXT        PRIMARY KEY,
    event_type    TEXT        NOT NULL,
    processed_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
