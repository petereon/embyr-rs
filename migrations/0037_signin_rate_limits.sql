-- admin-signin-hardening (ADR-076): per-source-IP token bucket backing
-- SigninRateLimiter. No FK (unlike rate_buckets) — key space is unbounded,
-- lazily populated by the atomic UPSERT in SigninRateLimiter::check_pg.
CREATE TABLE signin_rate_limits (
    source_key  VARCHAR(45)      NOT NULL PRIMARY KEY, -- max textual IPv6 length
    tokens      DOUBLE PRECISION NOT NULL,
    last_refill TIMESTAMPTZ      NOT NULL
);
