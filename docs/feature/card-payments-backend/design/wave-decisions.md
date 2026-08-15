# DESIGN Decisions — card-payments-backend

## Key Decisions

- [D1] **D-9's "extends the rate limiter" framing resolved as CREATE NEW, not extend**: the
  cumulative Free-plan cap check is a new background-computed subsystem (`CapUsageRefresher` +
  `CapStatusCache`), keyed by account (not project), reset on a billing-cycle boundary (not
  continuous refill). `RateLimiter`/`TokenBucket` (ADR-015) is untouched. (see:
  `docs/product/architecture/adr-020-cumulative-cap-check-architecture.md`)
- [D2] **Enforcement moved off the `:8080`/`:8081` hot path entirely**: the background task
  applies suspension by writing `projects.status='suspended'`; the *existing* auth-interceptor
  status check (already on the hot path) is what blocks subsequent traffic. Zero new latency on
  the Firestore data plane; eliminates the request-hot-path race condition Slice 07 flagged by
  construction. (see: ADR-020 § Alternatives Considered)
- [D3] **Stripe integration via `async-stripe`, not hand-rolled `reqwest`**: API surface (5
  resource areas: Customer/Subscription/Invoice/UsageRecord/webhook-signature) and
  webhook-signature security-criticality favor a maintained typed SDK, mirroring this project's
  own `aws-sdk-secretsmanager` precedent (full vendor SDK adopted when surface/risk justify it)
  over its `GcpSecretFetcher` precedent (hand-rolled, single-endpoint). (see:
  `docs/product/architecture/adr-021-stripe-sdk-integration.md`)
- [D4] **`lifecycle::set_project_status` reused literally for both new suspend triggers**:
  visibility widened, signature narrowed to a new `LifecycleDeps` struct, two new fan-out wrappers
  (`suspend_account_projects`/`activate_account_projects`) added — D-12's "one mechanism, two
  triggers" is structural, not conventional. (see: feature-delta.md § Component Decomposition)
- [D5] **Free-plan cap-check billing cycle = UTC calendar month**: Free accounts have no
  guaranteed Stripe `Subscription` object; Stripe's `current_period_start`/`current_period_end`
  are Pro-display-only, never consulted for cap computation. (see: ADR-020 § Billing Cycle
  Boundary)
- [D6] **Storage dimension NOT metered/capped in V1 — flagged, not silently dropped**:
  `daily_project_metrics` has no storage-bytes column; `billing.rs` already hard-codes storage as
  a placeholder. Real storage metering requires a new `BackendAdapter::table_size()` port method
  against customer DBs — out of this feature's locked scope. (see: OQ-CP-1)
- [D7] **Webhook idempotency via local ledger; usage-record idempotency via Stripe's native
  mechanism**: `processed_webhook_events` (local UNIQUE-constraint table, mirrors
  `sdk_api_keys.key_hash` precedent) for webhooks; Stripe's own idempotency-key parameter for
  usage-record pushes (no redundant local ledger).
- [D8] **New `WebhookState`, separate from `OperatorState`/`UserAdminState`**: mirrors the
  admin-api-v2 B-AD-07 precedent ("different dependency graphs; merging creates unnecessary
  coupling").
- [D9] **`run_metering` is HTTP-triggered (operator route), not a Tokio-interval sweeper**: DISCUSS
  Slice 05 explicitly descopes scheduling infrastructure to DEVOPS-wave — deliberate asymmetry
  with `CapUsageRefresher` (which IS an in-process interval sweeper), not an inconsistency.

## Architecture Summary

- Pattern: Hexagonal (ports-and-adapters), unchanged — extends the existing `embyr-core`/
  `embyr-server` boundary, zero new crates.
- Paradigm: Functional-where-practical Rust, unchanged (project `CLAUDE.md`).
- Key components: `StripeGateway` (adapter), `CapUsageRefresher` + `CapStatusCache` (new
  background-computed subsystem), `embyr-core::admin::billing` (pure domain types), 5th webhook
  sub-router + `WebhookState`, `lifecycle.rs` extended for cross-trigger suspend reuse.

## Reuse Analysis

| Existing Component | File | Overlap | Decision | Justification |
|-------------------|------|---------|----------|---------------|
| `router.rs` 4-sub-router pattern | `crates/embyr-server/src/admin/router.rs` | New webhook sub-router | EXTEND | 5th sub-router, structural sibling of `public_router` (D-11 locked) |
| `lifecycle.rs::set_project_status` | `crates/embyr-server/src/admin/handlers/lifecycle.rs` | Suspend/activate | EXTEND | D-12 mandates the literal same function for both new triggers |
| `config.rs` secret-resolution (ADR-018) | `crates/embyr-server/src/config.rs` | Env-var + AWS/GCP secret sourcing | EXTEND | 3 new resolver calls, zero new resolution logic |
| `middleware::rate_limit::RateLimiter` | `crates/embyr-server/src/middleware/rate_limit.rs` | Per-request quota | **CREATE NEW** (not extended) | Different key/time-semantics/reset-boundary; AC-206-06 locks this explicitly |
| `billing.rs` aggregation query shape | `crates/embyr-server/src/admin/handlers/billing.rs` | Account-scoped usage aggregation | EXTEND (pattern, new query text) | Same join/GROUP BY shape, different date-range predicate |
| `credential_cache.rs` (`CredentialCache`) | `crates/embyr-server/src/adapters/credential_cache.rs` | In-process per-instance cache | PATTERN REUSE | `CapStatusCache` follows the identical shape, different key/value types |
| `sweepers::query_log_sweeper`/`session_cleaner` | `crates/embyr-server/src/sweepers/` | Advisory-lock interval task | PATTERN REUSE | `CapUsageRefresher` follows the identical shape |
| `sdk_api_keys.key_hash` UNIQUE idempotency | migrations | Idempotent-write pattern | PATTERN REUSE | `processed_webhook_events(event_id PK)` follows the identical shape |
| `admin/state.rs` Operator/UserAdmin split (B-AD-07) | `crates/embyr-server/src/admin/state.rs` | Per-router state struct | PATTERN REUSE → CREATE NEW `WebhookState` | Different dependency graph (Stripe gateway + webhook secret) |
| `daily_project_metrics` schema | `migrations/0002_metrics.sql` | 4-dimension usage | **GAP, flagged** | No storage column; pre-existing gap, not newly introduced |

## Technology Stack

- `async-stripe` (MIT): Stripe API + webhook signature verification. See ADR-021.
- Postgres `UNIQUE`+`ON CONFLICT` (existing `sqlx`): webhook idempotency ledger.
- `tokio::time::interval` + `pg_try_advisory_lock` (existing pattern): `CapUsageRefresher`.

## Constraints Established

- The `:8080`/`:8081` Firestore hot path gains zero new code for this feature — all cap-check
  computation and enforcement happen in a background task.
- `embyr-core::admin::billing` is IO-free (enforced by existing `deny.toml`).
- `StripeGateway` is the sole `async-stripe` import site (code-review convention, mirrors existing
  AWS/GCP fetcher isolation).
- No mocked payment port (D-13, carried forward) — `StripeGateway` has no trait interface, mirrors
  the `RateLimiter` "concrete struct, one implementation" precedent (ADR-015).

## Upstream Changes

- None that contradict DISCUSS's locked decisions. One DESIGN-discovered gap surfaced explicitly,
  not silently absorbed: storage-dimension metering has no data source today (OQ-CP-1) — this
  narrows D-7's "four dimensions" claim to "three of four, real; one, placeholder" for this
  feature's V1, documented as a scoped follow-up rather than a contradiction of the locked
  decision.
