-- Migration 0038: usage_metering_pushed table (card-payments-backend, US-205, AC-205-03)
--
-- Idempotency ledger for nightly usage metering pushes to Stripe. Mirrors the
-- processed_webhook_events shape (migrations/0020): (project_id, dimension,
-- date) is the PRIMARY KEY, so a re-run's push is a no-op
-- INSERT ... ON CONFLICT DO NOTHING. Replaces relying on Stripe's own
-- idempotent-replay response timing (comparing the returned event's
-- `created` against this call's start second), which is racy at 1-second
-- granularity when a re-run happens within the same wall-clock second as
-- the original push.

CREATE TABLE usage_metering_pushed (
    project_id  VARCHAR(63) NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
    dimension   TEXT        NOT NULL,
    date        DATE        NOT NULL,
    pushed_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, dimension, date)
);
