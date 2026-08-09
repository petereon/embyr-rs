# Card Payments — Pre-DISCUSS Decisions

**Status:** Captured via `/grill-me`, 2026-08-09. Not yet executed — no DISCUSS wave started.
**Next step:** `/nw-discuss` for `card-payments` when ready to proceed, using this file as locked input.

## Context

No payment infrastructure exists in embyr-rs today. `GET /admin/v1/billing`
(`crates/embyr-server/src/admin/handlers/billing.rs`) already aggregates
per-project reads/writes/deletes/log-storage-bytes by date range for display,
but it is read-only reporting — no charging, no card storage, no
`stripe_customer_id`, no subscription/invoice schema anywhere.

## Decisions

| # | Question | Decision |
|---|---|---|
| 1 | Billing model | **Hybrid** — flat base subscription + metered overage on top |
| 2 | Payment processor | **Stripe** |
| 3 | Card capture UI | **Stripe Elements**, embedded in the Leptos/WASM admin SPA (PCI SAQ-A scope; needs a small JS interop shim since Stripe.js is JS-only) |
| 4 | Plan tiers | **Two: Free + Pro** |
| 5 | Free-tier card requirement | **Card required even for Free** — no anonymous free-riding; every account has a payment method on file from signup |
| 6 | Free-tier cap exceeded | **Hard-stop, suspend, manual upgrade required.** No silent auto-upgrade/auto-charge — avoids surprise billing and preserves trust |
| 7 | Overage metering granularity | **All four dimensions billed separately** — reads, writes, deletes, storage each get their own meter/price, mirroring the existing `billing.rs` aggregation (no new aggregation logic needed) |
| 8 | Metering cadence to Stripe | **Daily batch job** reading the existing `daily_project_metrics` table, pushing one Stripe usage record per project per dimension per day. Real-time per-operation push explicitly ruled out (not viable at embyr-rs's request volume) |
| 9 | Free-tier real-time cap enforcement | **Extends the existing Postgres token-bucket rate limiter** (`crates/embyr-server/src/middleware/rate_limit.rs`, built for JOB-11 fair-multitenancy) with a quota check — no new enforcement path, no live Stripe calls in the request hot path |
| 10 | Local schema + sync mechanism | **New `subscriptions` table** `{ account_id, plan (free\|pro), status, stripe_subscription_id, current_period_end }` + `accounts.stripe_customer_id` column + `processed_webhook_events` table (idempotency — Stripe retries webhook delivery). Kept current via Stripe webhooks (`customer.subscription.updated/deleted`, `invoice.payment_failed`), not polling |
| 11 | Webhook endpoint placement | **5th sub-router on `:9090` admin**, structurally identical to the existing `public_router` pattern in `admin/router.rs` (no session/operator auth `route_layer` applied) — but with its own `stripe_signature_middleware` verifying the `Stripe-Signature` header instead of no auth at all. Same port as operator/metrics/healthz; customer gRPC/REST stays clean on `:8080`/`:8081` |
| 12 | Pro-tier payment failure (dunning) | **Stripe Smart Retries** run first (no custom retry logic). On exhausted retries (subscription status `unpaid`/`past_due` past grace window), the webhook handler suspends the account's projects — **same code path** as the Free-cap-exceeded suspension (one suspension mechanism, two triggers) |
| 13 | Testing strategy | **Real Stripe test-mode API** (`sk_test_...`) over the network + **Stripe CLI** (`stripe trigger`) to synthesize webhook events for handler tests. No mocked `IPaymentGateway` port — consistent with this codebase's stated preference for real-infrastructure integration tests over mocks (testcontainers Postgres, real subprocess servers elsewhere) |

## Explicitly ruled out

- **Perpetual free tier without a card** — reopens unbounded cost exposure with no revenue backing it (the concern that started decisions 5–6)
- **Time-boxed trial instead of a capped free tier** — considered and reversed; user chose capped-free-with-card instead
- **Auto-upgrade to Pro on Free overage** — charges a card without an explicit "yes, bill me" action; higher chargeback/trust risk
- **Downgrade-to-Free on Pro payment failure instead of suspending** — a non-paying account could keep using infrastructure indefinitely, and Free requires a card anyway (decision 5), so this path never actually satisfies its own precondition
- **Blended "operation" unit for overage** (reads+writes+deletes weighted into one meter) — no existing weighting formula, adds complexity with no clear benefit over mirroring the existing 4-dimension split
- **Storage-only overage, unlimited ops on Pro** — risks a heavy-write customer costing more than their subscription covers
- **Polling Stripe instead of webhooks** — sync lag, against Stripe's own webhook-first design
- **Webhook route bypassing auth as a special case** — reframed: it's not a bypass, it's a structural sibling sub-router (precedent: `public_router` already works this way for signin/signout/oidc_callback)
- **Real-time per-operation usage push to Stripe** — not viable at embyr-rs's request rates

## Open — deferred to DISCUSS/DESIGN, not decided here

- Actual price points and included allowances per dimension (business/market decision, not architecture)
- Exact `subscriptions`/`processed_webhook_events` column types and migration numbering
- `ServerConfig` additions (`STRIPE_SECRET_KEY`, `STRIPE_WEBHOOK_SIGNING_SECRET`, `STRIPE_PUBLISHABLE_KEY`) — straightforward extension of the pattern from `production-readiness`, no open question
- Whether the rate-limiter's quota check needs a new `RateLimitInfo`-adjacent domain type in `embyr-core` or reuses the existing one
