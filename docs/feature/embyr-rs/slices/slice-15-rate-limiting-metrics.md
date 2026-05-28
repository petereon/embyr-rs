# Slice 15 — Rate Limiting + Usage Metrics

**Goal**: Per-project token-bucket rate limiting enforces configurable request limits; usage metrics are recorded per request.

## IN scope
- Token bucket per project per embyr instance (not distributed): `ratelimit.default_rps` (default 1000), `ratelimit.default_burst` (default 200), configurable per-project override
- `RESOURCE_EXHAUSTED: "rate limit exceeded"` when bucket empty
- `ratelimit.enabled` config flag (default: true)
- Ingress/egress byte tracking per request: written to `daily_project_metrics` at request completion
- CPU time tracking (wall-clock proxy): recorded alongside byte metrics
- Tombstone sweeper: background task purges `deleted_documents` older than 24h; runs every hour

## OUT scope
- Distributed rate limiting across instances (per-instance bucket is the contract)
- Billing aggregation / reporting
- Alert thresholds

## Learning Hypothesis
Disproves: "Per-project token-bucket rate limiting distorts tail latency because bucket refill requires a lock."
Confirms if: p99 latency increase from rate limiting is < 0.5ms when the bucket is not exhausted (warm path).

## Acceptance Criteria
- Burst of 300 requests when `default_burst=200`: first 200 succeed; next 100 return `RESOURCE_EXHAUSTED`
- `ratelimit.enabled=false`: no requests rejected regardless of rate
- `daily_project_metrics` row for today has correct `ingress_bytes` after 10 SDK reads
- Tombstone sweeper purges `deleted_documents` rows older than 24h on each hourly run
- p99 latency increase < 0.5ms with rate limiting enabled (measured with 1000 RPS load)

## Dependencies
All prior slices (cross-cutting)

## Effort estimate
≤1 day
