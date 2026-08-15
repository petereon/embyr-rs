# DISCUSS Decisions — card-payments-backend

## Key Decisions

- [D1] **job_id reuse, not a new job**: Every story traces to JOB-14 (`manage-subscription`), extended via cross-reference notes rather than a new job or an in-place JOB-14 rewrite. Rationale: this feature is JOB-14's "make it real" backend half, exactly as `admin-api-v2` was JOB-10's backend half of `user-admin-ui` — no new user motivation exists here, only a new mechanism (see: `docs/product/jobs.yaml`, `feature-delta.md` § Persona & Job).
- [D2] **Scope Assessment verdict: OVERSIZED (2/5 signals — effort >2 weeks, multiple independent shippable outcomes), split into 3 internal Release Groups within ONE feature-id**, not a further feature-id split. Rationale: bounded-context (3, not >3) and WS-integration-point (3, not >5) signals stayed under threshold — a materially weaker oversized signal than `card-payments`' own 5/5 — and all 3 groups share one migration-numbering sequence and one locked D-1..D-13 decision set, unlike the clean crate-boundary split between `card-payments` and this feature (see: `feature-delta.md` § Scope Assessment).
- [D3] **Walking Skeleton: YES, narrowly scoped to Slice 01 only** (Decision 2 = "Depends", brownfield evaluated first). Rationale: much scaffolding (router pattern, secret-resolution pattern, suspend mechanism) is directly reusable; the one genuinely new risk is a real Stripe SDK network round-trip inside an existing session-authed handler under D-13's no-mock constraint — WS burns that down alone before webhook/batch/rate-limiter work begins (see: `feature-delta.md` § Story Map, Walking Skeleton).
- [D4] **D-9's "extends the rate limiter" framing corrected, not accepted at face value**: after reading `rate_limit.rs` directly, D-9 is documented as requiring a genuinely new, separately-keyed (account, not project) monthly-cumulative mechanism — not literal `TokenBucket`/`rate_buckets` reuse. The concrete computation mechanism (live/cached/incremental) is explicitly left open for DESIGN (see: `feature-delta.md` § Open Question — D-9's Rate-Limiter Framing).
- [D5] **Real Stripe Elements JS interop shim (D-3) explicitly descoped**, despite being listed as recommended follow-up work in `docs/evolution/2026-08-11-card-payments.md`. Rationale: this dispatch's Decision 1 locks this feature as backend-only with no frontend/UI work; the evolution doc's suggestion is superseded, not silently re-absorbed (see: `feature-delta.md` § Out of Scope, § Prior Wave Consultation contradiction-check note).
- [D6] **Compute-then-enforce split for real-time cap enforcement**: Slice 06 (compute `cap_status`, read-only) and Slice 07 (wire the suspend action) are deliberately two separate slices rather than one, so the riskiest new architecture (§ Open Question) is validated in isolation before any mutating/suspending action depends on it.
- [D7] **No mocked payment port for DISTILL** (D-13, hard constraint, carried forward as a System Constraint): every acceptance test must exercise real Stripe test-mode API calls and/or Stripe CLI `stripe trigger` synthesis — consistent with this project's established real-infrastructure-over-mocks testing philosophy (testcontainers Postgres, real subprocess servers elsewhere).

## Requirements Summary

- Primary jobs/user needs: Make JOB-14's billing/payment self-service experience (plan visibility, card status, usage-vs-cap, upgrade/downgrade, invoice history, suspension recovery) real and financially accurate, replacing the `card-payments` frontend's mock `AppModel` data with a real Stripe-backed subscription/webhook/metering/enforcement system.
- Walking skeleton scope: Slice 01 (US-201) only — real Stripe Customer provisioning + `subscriptions` schema + one read endpoint.
- Feature type: Backend (APIs, webhooks, batch jobs, rate-limiter extension — no frontend/UI work, per locked Decision 1).

## Constraints Established

- Migrations for this feature start at `0019` (next after `0018_rate_buckets.sql`).
- `subscriptions.status`/cap-exceeded/payment-failed derivations MUST resolve to the single `set_project_status` code path in `lifecycle.rs` regardless of trigger (D-12) — verified directly in US-204's and US-207's own AC, not merely assumed.
- New IO-free domain types live in `crates/embyr-core/src/admin/`, enforced by `deny.toml`; Stripe SDK calls live in `embyr-server`'s adapters layer only.
- `STRIPE_SECRET_KEY`/`STRIPE_WEBHOOK_SIGNING_SECRET`/`STRIPE_PUBLISHABLE_KEY` resolved via the existing `config.rs` secret-manager pattern, not a new pattern.
- Real-time cap-status computation must fail open on unavailability/staleness, mirroring `rate_limit.rs`'s existing Postgres-timeout fail-open precedent — never fabricate an over-cap reading.

## Upstream Changes

- None that contradict `card-payments`' DISCUSS output. One clarification (not a contradiction): the evolution doc's Follow-Up Work list included the Stripe Elements JS interop shim as `card-payments-backend` scope; this dispatch's Decision 1 explicitly descopes it from THIS feature (see D5 above) — documented as a scope refinement, not a reversal of any locked decision.
- No DISCOVER-wave evidence exists for this feature (mirrors `card-payments`' own gap) — this feature inherits JOB-14's already-validated opportunity score (17) rather than re-deriving one.
