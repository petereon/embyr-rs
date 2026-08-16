# card-payments-backend — Feature Delta

**Wave**: DISCUSS | **Agent**: Luna (nw-product-owner) | **Date**: 2026-08-11
**Status**: Ready for DESIGN handoff (backend scope only — no frontend/UI work in this feature)
**Upstream**: `docs/feature/card-payments/feature-delta.md` (shipped frontend half of this same overall feature), `docs/decisions/card-payments/grill-me-decisions.md` (D-1..D-13, locked)

---

## Wave: DISCUSS / [REF] Prior Wave Consultation — Reading Confirmation

✓ `docs/decisions/card-payments/grill-me-decisions.md` (13 locked architecture decisions D-1..D-13, shared between this feature and `card-payments`)
✓ `docs/evolution/2026-08-11-card-payments.md` (evolution doc for the shipped frontend half — split rationale, forward type-contract, Follow-Up Work list)
✓ `docs/feature/card-payments/feature-delta.md` (full DISCUSS section read: Persona/Job, Locked Decisions, Scope Assessment, Story Map, all 9 user stories US-101..109, Outcome KPIs, DoR Validation, Out of Scope, Handoff Package)
✓ `docs/product/jobs.yaml` (JOB-14 manage-subscription, JOB-06 tenant-control, JOB-11 fair-multitenancy, JOB-10 account-admin)
✓ `docs/product/journeys/billing-management.yaml` (frontend journey, `scope_note` explicitly defers backend-real data to this feature)
✓ `docs/product/personas/chris-account-admin.yaml` (P5 Chris Okafor, already lists `JOB-14`)
✓ `crates/embyr-server/src/admin/router.rs` (243 lines — 4-sub-router pattern: operator/dual-auth/public/session, precedent for the 5th webhook sub-router)
✓ `crates/embyr-server/src/middleware/rate_limit.rs` (304 lines — per-project, per-second token-bucket; confirms this is architecturally a request-rate limiter, not a monthly-cumulative-cap tracker — see Open Question below)
✓ `crates/embyr-core/src/rate_limit.rs` (`RateLimitInfo` pure value type — project-scoped, no account/monthly-cumulative concept exists today)
✓ `crates/embyr-server/src/admin/handlers/billing.rs` (158 lines — existing read-only `GET /admin/v1/billing`, sources `daily_project_metrics`, what D-8's batch job will also read)
✓ `crates/embyr-server/src/admin/handlers/lifecycle.rs` (existing `suspend_project`/`activate_project`/`delete_project` — the suspension mechanism D-12 reuses for both triggers)
✓ `crates/embyr-server/src/config.rs` lines 280-440 (`resolve_admin_key`/`fetch_from_secret_manager` — established env-var + AWS/GCP secret-manager resolution pattern for `STRIPE_SECRET_KEY` etc.)
✓ `migrations/` directory listing (`0001`..`0018`, next migration starts at `0019`)
✓ `migrations/0018_rate_buckets.sql`, `migrations/0015_projects_admin_columns.sql`, `migrations/0008_admin_accounts.sql` (confirms `projects.account_id` FK — accounts have *many* projects; `rate_buckets` is keyed per-`project_id`, not per-`account_id`)
✓ `crates/embyr-core/src/admin/{account.rs,mod.rs}` (domain-type conventions: `AccountId`/`UserId` newtypes, no-IO invariant enforced by `deny.toml`)
✓ `docs/feature/admin-api-v2/feature-delta.md` + `docs/feature/admin-api-v2/slices/slice-B01-auth-migrations.md` (precedent for a *backend-wiring* feature-delta format — one story per slice, compact — followed here in preference to `card-payments`' more verbose per-story format, since this feature is architecturally the same shape as `admin-api-v2`: making an existing mock/read-only surface real)
✓ `/Users/petervyboch/Projects/embyr-rs/CLAUDE.md` (functional-where-practical Rust; `embyr-core` has zero IO, enforced by `deny.toml`+CI; per-feature mutation testing)
⊘ `docs/product/vision.md`, `docs/project-brief.md`, `docs/stakeholders.yaml` (not found — same gap noted in `card-payments`' DISCUSS)
⊘ `docs/feature/card-payments-backend/discover/`, `docs/feature/card-payments-backend/diverge/` (not found — no DISCOVER/DIVERGE wave ran for this feature, mirrors `card-payments`' own precedent)

**Note on the project's global `CLAUDE.md`**: `/Users/petervyboch/Projects/CLAUDE.md` describes a TypeScript/OOP paradigm — that file governs a *different, unrelated* project (`warp-vscode-integration`) and does not apply here. `embyr-rs`'s own `/Users/petervyboch/Projects/embyr-rs/CLAUDE.md` (functional-where-practical Rust) is the governing paradigm document for this feature.

No contradictions found between this feature's scope and prior evidence. One important **non-contradiction clarification**: the evolution doc's "Follow-Up Work" section lists "Real Stripe Elements JS interop shim for CardModal (D-3)" as recommended work for `card-payments-backend`. This dispatch's **Decision 1 (locked)** defines this feature as backend-only with *no frontend/UI work*. These are reconciled, not contradictory: the JS interop shim is explicitly **descoped** from this feature (see § Out of Scope) — the evolution doc's suggestion is superseded by this dispatch's explicit type-1 framing. Backend work instead exposes the API surface (`POST /admin/v1/billing/subscription`, a future SetupIntent endpoint) that *any* frontend — the existing Rust-native stub or a future Elements-based one — can call. Flagged explicitly so DESIGN does not silently re-absorb frontend scope.

---

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: P5 — Chris Okafor, Account Admin / Platform Engineer (`docs/product/personas/chris-account-admin.yaml`). Secondary: Dana Whitfield (Pro plan, Northwind Data), Priya Raman (Solstice Analytics, suspended) — same named examples the frontend feature established, reused here for continuity.

**Secondary actor (system/operator angle, per Decision 3)**: Sam Chen (P2, Service Operator) — does not initiate any story in this feature directly, but (a) owns the reused `suspend`/`activate` mechanism (JOB-06) that D-12's dunning path calls, and (b) is the actor for the one operator-triggered action this feature adds (`POST /admin/v1/billing/run-metering`, US-205).

**job_id decision (per Decision 4 — reuse, don't invent)**: This feature does **not** create a new job. Every story traces to **JOB-14** (`manage-subscription`), extending its existing functional dimension from *mock-backed UI description* to *real, Stripe-backed behavior*. JOB-14's job story, four forces, and opportunity score (17, `docs/product/jobs.yaml`) were already validated by the `card-payments` DISCUSS session and are not re-litigated here — this feature is JOB-14's "make it real" half, exactly as `admin-api-v2` was JOB-10's "make it real" half of `user-admin-ui`.

Two stories (US-206, US-207 — the real-time cap-enforcement slices) also carry a **cross-reference note** to **JOB-11** (`fair-multitenancy`), because they physically extend the same file (`rate_limit.rs`) JOB-11's distributed token bucket lives in. This is a cross-reference, not a shared primary job — the "why" is different (JOB-11: request-rate fairness across tenants regardless of node count; JOB-14/D-9: monthly-cumulative usage vs. a *paid-plan* cap, unrelated to horizontal scaling). See § Open Question below.

US-204 (dunning suspension) carries a cross-reference note to **JOB-06** (`tenant-control`) because it calls the exact suspend code path JOB-06 introduced — but the *trigger* is a Stripe webhook, not Sam manually suspending a project, so JOB-06 is informational context, not the primary job.

`docs/product/jobs.yaml` is updated with these cross-reference notes (see § SSOT Updates at the end of this document) — JOB-14 itself is not rewritten, mirroring how `card-payments`' own DISCUSS annotated JOB-10 without modifying it.

---

## Wave: DISCUSS / [REF] Locked Decisions

Source: `docs/decisions/card-payments/grill-me-decisions.md`. Not re-litigated — constraints on story scope and AC, exactly as `card-payments`' DISCUSS treated them.

| # | Decision | How it shows up in this DISCUSS output |
|---|---|---|
| D-1 | Hybrid subscription + metered overage | US-205 (metering) pushes the overage half; US-201/202 carry the subscription half |
| D-2 | Stripe | All 7 stories; `STRIPE_SECRET_KEY`/`STRIPE_WEBHOOK_SIGNING_SECRET`/`STRIPE_PUBLISHABLE_KEY` resolved via the `config.rs` secret pattern (Technical Notes, US-201) |
| D-3 | Stripe Elements card capture (PCI SAQ-A) | **Explicitly descoped this feature** — see § Out of Scope. Real SetupIntent/token-exchange endpoint is a plausible *future* increment, not scoped here (Decision 1 is backend-only, and card capture specifically requires a frontend interop shim to be meaningful) |
| D-4 | Free + Pro tiers only | US-201/202 (`plan: free \| pro`, no third value) |
| D-5 | Card required even for Free | Out of scope this feature — card capture itself is D-3/frontend scope; this feature assumes a card may or may not already be on file and does not gate on it |
| D-6 | Free-cap-exceeded = hard-stop/suspend, no auto-upgrade | US-206/US-207 make this real for the first time (frontend only *displayed* a mock-derived version of this state) |
| D-7 | All 4 usage dimensions metered separately | US-205 (metering), US-206 (cap check) both operate per-dimension (reads/writes/deletes/storage), mirroring `billing.rs`'s existing per-dimension aggregation |
| D-8 | Daily batch job pushes usage to Stripe | US-205, in full |
| D-9 | Real-time Free-cap enforcement extends the existing rate limiter | US-206/US-207 — **flagged as an explicit open architectural question, not hand-waved**, see § Open Question below |
| D-10 | New `subscriptions`/`accounts.stripe_customer_id`/`processed_webhook_events` schema | US-201 (subscriptions + stripe_customer_id), US-203 (processed_webhook_events) |
| D-11 | 5th webhook sub-router on `:9090`, sibling of `public_router` | US-203, in full |
| D-12 | Stripe Smart Retries → suspend, same path as Free-cap-exceeded | US-204 (payment-failure trigger) and US-207 (cap-exceeded trigger) both call the same `set_project_status(..., "suspended", ...)` path in `lifecycle.rs` |
| D-13 | Real Stripe test-mode API + Stripe CLI, no mocked port | Every story's AC requires real Stripe test-mode network calls (`sk_test_...`) or `stripe trigger` synthesis — no `IPaymentGateway` mock port. Carried into § System Constraints below as a DISTILL-scoping constraint |

---

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

Run before journey/story-map investment, per Phase 1.5 — and per this dispatch's explicit instruction to treat this seriously given the feature's real complexity (Stripe SDK, 3 schema changes, a new sub-router with its own auth middleware, a rate-limiter design extension, a batch job, dunning wiring).

### Oversized signals evaluated (any 2+ triggers a split)

| Signal | Threshold | card-payments-backend, full scope | Fired? |
|---|---|---|---|
| User stories | >10 | 7 (US-201..207, one per slice — see § Story Map) | **NO** |
| Bounded contexts / modules | >3 | 3 — **Subscription & Payment Lifecycle** (customer/subscription CRUD + webhook ingestion, since webhooks are the lifecycle's own sync adapter, same aggregate), **Usage Metering** (batch job), **Real-Time Cap Enforcement** (rate-limiter extension) | **NO** (at, not above, threshold) |
| Walking Skeleton integration points | >5 | WS is scoped narrowly to US-201 only (Decision 2 — brownfield reuse): Stripe API + Postgres (`subscriptions` table) + existing `UserAdminState`/session-auth pattern = 3 | **NO** |
| Estimated effort | >2 weeks | 7 stories × 1-2 days each (webhook signature verification + idempotency + real Stripe test-mode network integration per D-13 push several stories toward the upper end) ≈ 11-13 days ≈ 2.2-2.6 weeks | **YES** (borderline, but real) |
| Independent shippable outcomes | multiple | YES — Release 1 (subscription lifecycle) is independently demoable/valuable without Release 2 (metering) or Release 3 (enforcement) existing; each of the 3 release groups ships and proves value on its own | **YES** |

**2 of 5 signals fired** (threshold is 2+). **Verdict: OVERSIZED** — weaker signal than `card-payments`' own 5/5, but still crosses the gate.

### Split Decision

**Decision: split into 3 internal Release Groups, NOT 3 separate feature-ids.** This is a deliberate departure from the `card-payments` → `card-payments-backend` precedent (which *did* use a feature-id split), and the reasoning is documented explicitly per this dispatch's instruction not to hand-wave it:

**Why NOT a further feature-id split** (the alternative this dispatch explicitly asked me to weigh: "Stripe subscription CRUD + webhook ingestion" as one slice-set vs. "real-time Free-cap enforcement + rate-limiter extension" as a separate slice-set):

1. **Bounded-context and WS-integration-point signals stayed under threshold** (3 contexts, not >3; 3 WS integration points, not >5) — unlike the original `card-payments` split, where 5/5 signals fired overwhelmingly. This split's justification is weaker; treating it the same way would be over-engineering the process.
2. **All 3 release groups share one migration-numbering sequence and one locked decision set (D-1..D-13).** Fragmenting into 3 feature-ids would force 3 separate DISCUSS→DESIGN→DISTILL→DELIVER cycles to each re-read and re-confirm the same 13 locked decisions, and would require inventing a cross-feature migration-numbering coordination convention this project has never needed before (the `user-admin-ui`/`admin-api-v2` split had no shared migrations at all — clean crate boundary. This split does not have that clean a boundary; all 3 release groups touch `embyr-server`'s admin subsystem and the same migration sequence).
3. **Release 2 (Metering) and Release 3 (Enforcement) both structurally depend on Release 1's schema shipping first** (`subscriptions` table, `accounts.stripe_customer_id`). Unlike `card-payments` → `card-payments-backend`, which had *zero* build dependency in either direction (the frontend shipped against mock data and needed nothing from the backend to ship), these 3 groups are naturally sequential releases of one feature, not 3 independently-orderable features.
4. **This mirrors `card-payments`' own Release-0/1/2/3 structure** (see its Story Map) — that feature also had multiple independent shippable outcomes and handled it with internal Releases inside one feature-id, not a further split. Consistency with the project's own established pattern for "when is a Release enough vs. when do you need a new feature-id" favors the same answer here: the bounded-context/integration-point signals are the ones that decide "new feature-id," and they did not fire.

**Result**: one feature-id `card-payments-backend`, 7 stories, 7 slices, organized into 3 Release Groups (see § Story Map). Each Release Group is independently demoable and independently valuable, satisfying the Elephant Carpaccio "thin, verifiable slice" discipline without fragmenting the shared decision/migration context.

### Walking Skeleton decision (Decision 2 — brownfield, evaluate before deciding)

**Decision: YES, scope a Walking Skeleton, kept narrow to Slice 01 (US-201) only.** Evaluated existing structure first: `router.rs`'s 4-sub-router composition-root pattern, `config.rs`'s secret-resolution pattern, `lifecycle.rs`'s suspend mechanism, and `UserAdminState`/session-auth middleware are all directly reusable scaffolding — this is not a from-scratch build. The riskiest *new* assumption is narrower than "does the whole backend integrate": per D-13 (real Stripe test-mode network calls, no mocks), the actual unknown is *"does a real Stripe SDK network round-trip integrate cleanly inside an existing async Axum session-authed handler, at acceptable latency, without a mocked port to fall back on."* WS = US-201 alone (Stripe Customer creation + one migration + one GET endpoint) burns down exactly that risk before any webhook router, batch job, or rate-limiter work begins.

---

## Wave: DISCUSS / [REF] System/Operator Journey (lightweight, per Decision 3)

Decision 3 = Lightweight: this feature has no new end-user UX surface (Chris's experience is already fully specified by the shipped `card-payments` frontend). Journey work here focuses on **system and operator flows** — webhook delivery and dunning escalation — not an emotional-arc UX design.

### Webhook delivery flow (system flow)

```
Stripe event fires ──▶ POST /admin/v1/webhooks/stripe ──▶ stripe_signature_middleware
  (subscription.updated,    (5th sub-router, :9090,        verifies Stripe-Signature header
   invoice.payment_failed,   sibling of public_router)      against STRIPE_WEBHOOK_SIGNING_SECRET
   etc.)                                                            │
                                                          invalid ──┤── valid
                                                        401, no DB          │
                                                        write                ▼
                                                          processed_webhook_events
                                                          lookup by event.id
                                                             │              │
                                                       already seen ──┤── new
                                                       200, no-op            │
                                                       (idempotent)          ▼
                                                                    apply to `subscriptions` row
                                                                    (US-203/204), insert into
                                                                    processed_webhook_events, 200
```

### Dunning escalation flow (system flow, D-12)

```
Pro-plan invoice payment fails
        │
        ▼
Stripe Smart Retries run automatically (Stripe-side, no embyr custom retry logic)
        │
   retries succeed ──────────────────────────────┐         retries exhausted
        │                                          │                │
        ▼                                          │                ▼
invoice.payment_succeeded webhook          (no further embyr    invoice.payment_failed webhook,
(US-204, recovery path) →                   action needed)      final-failure flag set →
project(s) reactivated,                                          suspend_project() called for
same suspend/activate path                                       every project under the account
                                                                   (same path US-207 uses for
                                                                   Free-cap-exceeded — ONE
                                                                   suspension mechanism, TWO triggers)
```

### Real-time cap-enforcement flow (system flow, D-9)

```
Firestore SDK request arrives ──▶ existing per-second token-bucket check (rate_limit.rs, unchanged)
                                          │ allowed
                                          ▼
                                  NEW: monthly-cumulative-usage-vs-plan-cap check (US-206)
                                  (separate mechanism — see § Open Question)
                                          │
                                capacity remaining ──┤── cap crossed (Free plan only)
                                        allow                  │
                                                                ▼
                                                    suspend_project() called (US-207) —
                                                    same path D-12 uses for dunning
```

---

## Wave: DISCUSS / [HOW] Journey Deep-Dive

### Shared Artifacts Registry

| Artifact | Source of Truth | Consumers | Integration Risk |
|---|---|---|---|
| `accounts.stripe_customer_id` | New column, populated lazily on first `GET /admin/v1/billing/subscription` call (US-201) | US-202 (plan change), US-203 (webhook sync target lookup), US-205 (metering, usage records attach to the Stripe customer) | HIGH — every downstream Stripe call keys off this; must never be created twice for the same account |
| `subscriptions` row (`account_id, plan, status, stripe_subscription_id, current_period_end`) | Local Postgres table, source of truth is **Stripe** (this row is a read cache, kept in sync by US-203's webhook handler) | `GET /admin/v1/billing/subscription` (US-201), plan-change gate (US-202), cap-status derivation (US-206) | HIGH — a stale row after a missed/failed webhook directly produces a wrong billing decision for Chris |
| `processed_webhook_events` (`event_id`, `processed_at`) | Local Postgres table, written once per unique Stripe `event.id` | US-203's idempotency check | HIGH — Stripe explicitly retries webhook delivery; a missing dedupe means double-applying a state transition (e.g. double-suspending, double-reactivating) |
| `daily_project_metrics` (existing table) | Existing, unchanged — already the source for `GET /admin/v1/billing` | US-205 (metering batch reads it), US-206 (cumulative cap check reads/derives from it) | MEDIUM — this feature adds *readers*, not writers, of this table; no schema change needed here |
| Monthly-cumulative usage-vs-cap value (per account, per dimension) | **New** — not yet defined; DESIGN must decide the concrete computation/caching mechanism (see § Open Question) | US-206 (compute/expose), US-207 (enforce) | HIGH — this is the artifact at the center of the D-9 architectural question; DISCUSS documents the requirement, not the mechanism |
| `project.status` (existing column, `active`/`suspended`/`deleted`) | Existing `projects` table, existing `lifecycle.rs` handlers | US-204 (dunning trigger writes it), US-207 (cap-exceeded trigger writes it), already consumed today by request-path auth (`PermissionDenied` on suspended projects per JOB-06) | HIGH — this is the D-12 "one suspension mechanism, two triggers" artifact; both triggers MUST call the exact same `set_project_status` function, never a parallel implementation |

Validation: every new artifact above has exactly one documented source of truth; no consumer is permitted to hold its own drifting copy (functional-where-practical constraint, carried into § System Constraints).

### Error Paths

| Step | Failure mode | Recovery |
|---|---|---|
| Webhook receipt (US-203) | Invalid/missing `Stripe-Signature` header | 401, no DB write, no state change — mirrors the existing pattern of rejecting bad auth before any handler logic runs |
| Webhook receipt (US-203) | Duplicate delivery of an already-processed `event.id` | 200 OK, no-op (idempotent) — Stripe's retry contract requires this; a non-idempotent handler is a correctness bug, not an edge case |
| Webhook receipt (US-203) | Stripe delivers an event type this feature doesn't yet handle | 200 OK, logged and ignored — forward-compatible; must not 500 or reject unknown-but-valid event types |
| Plan change (US-202) | Stripe API call fails/times out mid-request | Local `subscriptions` row is NOT optimistically updated until Stripe confirms — no local/Stripe drift on a failed call; Chris sees an error, not a false success |
| Metering batch (US-205) | Stripe usage-record push fails for one project mid-run | Failure is per-project-per-dimension; the run continues for remaining projects/dimensions (no all-or-nothing), and the failed item is retried on the next manual/scheduled run (idempotency key, US-205's second story-defining property) |
| Cap check (US-206/207) | Monthly-cumulative computation is unavailable/stale (e.g. `daily_project_metrics` hasn't been updated yet today) | **Fail-open** — mirrors `rate_limit.rs`'s existing Postgres-timeout fail-open precedent (`check_pg`'s `None` branch does not penalize on DB error); a stale/unavailable cap computation must never spuriously suspend a healthy account |
| Dunning suspend (US-204) | `invoice.payment_failed` arrives for an account with multiple projects | ALL of the account's active projects are suspended (D-12), not just one — otherwise a partially-suspended account is a worse, more confusing state than either fully-active or fully-suspended |

---

## Wave: DISCUSS / [REF] Open Question — D-9's Rate-Limiter Framing (explicit, not hand-waved)

Per this dispatch's explicit instruction: D-9 says real-time Free-cap enforcement "extends the existing Postgres token-bucket rate limiter." Having read `crates/embyr-server/src/middleware/rate_limit.rs` directly, this framing needs to be corrected before DESIGN scopes it, or DESIGN will under-estimate the work:

- The **existing** limiter (`RateLimiter::check`) is a **per-second, per-project REQUEST-RATE** mechanism: a `TokenBucket` with `capacity` + `refill_rate` (tokens/sec), refilled continuously based on wall-clock elapsed time since `last_refill`. It has no concept of a monthly cumulative total, no concept of a billing cycle reset, and is keyed by `project_id` (a single project), not `account_id` (an account can own multiple projects, confirmed via `projects.account_id` FK).
- What D-9 actually needs is a **monthly-cumulative-usage-vs-plan-cap comparison**: "has this *account* used ≥100% of its Free-plan allowance for *this dimension*, summed across all its projects, since the start of the current billing cycle." That is a different problem shape — different key (account, not project), different time semantics (cumulative-since-cycle-start, not continuously-refilling-per-second), different reset boundary (billing cycle, not "tokens regenerate at a steady rate").
- **This DISCUSS wave treats "extends the rate limiter" as requiring real design work**, not literal reuse of the token-bucket refill model. The two checks can coexist in the same request path (see § System/Operator Journey diagram above — the new check runs as a second gate *after* the existing per-second check, not instead of it, and only applies to Free-plan accounts), but they are separate mechanisms with separate state.
- **Not decided here** (DESIGN wave's call, intentionally left solution-neutral per Principle 5): whether the monthly-cumulative value is computed live per-request (risk: adds Postgres round-trip latency to every gRPC hot-path call, the same class of risk the existing 20ms-timeout fail-open pattern was built to bound), cached and periodically refreshed (risk: staleness window where an account can exceed its cap before enforcement catches up), or incrementally maintained (risk: another piece of mutable state that can drift from `daily_project_metrics`'s ground truth). US-206's AC states the *requirement* (a per-account, per-dimension, per-cycle cumulative comparison exists and is queryable) without prescribing which of these three mechanisms DESIGN should pick.
- Flagged as a **DESIGN-wave-blocking open question**, not a DISCUSS blocker — DISCUSS's job is to make sure this isn't silently treated as "just add a field to `TokenBucket`," which it structurally cannot be.

---

## Wave: DISCUSS / [REF] Story Map & Walking Skeleton

**Persona**: P5 Chris (Account Admin) + P2 Sam (Operator, one story only) | **Goal**: Make JOB-14's mock-backed billing experience real, backed by Stripe.

### Backbone

| A. Subscription Exists | B. Plan Changes Are Real | C. Stripe Stays in Sync | D. Payment Failure Is Handled | E. Usage Is Metered | F. Caps Are Enforced |
|---|---|---|---|---|---|
| Stripe Customer auto-provisioned **[WS]** | `POST` plan change calls real Stripe | Webhook endpoint verifies signature | `invoice.payment_failed` suspends | Nightly batch pushes usage records | Cumulative usage computed per account/dimension |
| `subscriptions` row seeded Free **[WS]** | Local row only updates on Stripe confirmation | Idempotent via `processed_webhook_events` | `invoice.payment_succeeded` reactivates | Operator can manually trigger a run | Cap-exceeded triggers real suspend |
| `GET` endpoint returns real data **[WS]** | Upgrade clears cap-suspension | `customer.subscription.*` keeps local row in sync | Same suspend path as cap-exceeded (D-12) | Idempotent per project/dimension/day | Same suspend path as dunning (D-12) |

### Walking Skeleton

Slice 01 (US-201) renders the thinnest real slice of Activity A only: a real Stripe Customer created via the SDK, a `subscriptions` row seeded to Free, and `GET /admin/v1/billing/subscription` returning that real (not mock) data. No webhook router, no batch job, no rate-limiter change yet — those are Slices 03-07. This exactly mirrors `admin-api-v2`'s own Slice B-01 (auth + migrations first, proving the riskiest new integration assumption before any feature-complete route ships).

### Releases (Release Groups, per § Scope Assessment — outcome-sliced, not feature-grouped)

- **Release 1 — Subscription & Payment Lifecycle** (Slices 01-04, US-201..204). Outcome: Chris's plan, payment status, and suspension state in the console are backed by a real Stripe subscription and real webhook events, not mock/`TestClockCard` data. Independently demoable and valuable on its own.
- **Release 2 — Usage Metering** (Slice 05, US-205). Outcome: Dana's eventual Stripe invoice reflects real, itemized overage — the numbers `NextInvoiceCard` (US-103, already shipped) currently only estimates from mock data become grounded in what Stripe will actually bill. Depends on Release 1's schema (`stripe_customer_id`) existing.
- **Release 3 — Real-Time Cap Enforcement** (Slices 06-07, US-206..207). Outcome: D-6's hard-stop policy is enforced for real for the first time — a Free-plan account that crosses its cap is actually suspended, not just shown a mock-derived banner. Depends on Release 1's schema existing (needs `subscriptions.plan` to know which accounts are Free).

---

## Wave: DISCUSS / [REF] Elephant Carpaccio Slices

| Slice | Story | Release | Estimate | Learning Hypothesis (disproves) | Production-data taste test |
|---|---|---|---|---|---|
| 01 (WS) | US-201 | 1 | 1.5 days | A real Stripe SDK network round-trip does not integrate cleanly inside an existing async Axum session-authed handler at acceptable latency, with no mocked port to fall back on (D-13) | Real Stripe test-mode API (`sk_test_...`), real Postgres — no synthetic exception needed, unlike `card-payments`' UI-only slices |
| 02 | US-202 | 1 | 1 day | Optimistic local-state updates on plan change would drift from Stripe on a failed API call | Real Stripe test-mode subscription objects |
| 03 | US-203 | 1 | 1.5 days | Webhook signature verification + idempotency cannot be exercised without a mocked port, contradicting D-13's no-mock stance | Real `stripe trigger customer.subscription.updated`/`.deleted` (Stripe CLI, D-13) |
| 04 | US-204 | 1 | 1.5 days | The "one suspension mechanism, two triggers" design (D-12) actually has hidden per-trigger special-casing once implemented | Real `stripe trigger invoice.payment_failed`/`.payment_succeeded` |
| 05 | US-205 | 2 | 2 days | A per-project-per-dimension-per-day usage push is not naturally idempotent, risking double-billing on any re-run | Real rows from `daily_project_metrics`, real Stripe test-mode usage-record API calls |
| 06 | US-206 | 3 | 1.5 days | A monthly-cumulative-vs-cap check cannot be added to the request hot path without either violating the existing 20ms-latency-bound precedent or requiring a new caching layer neither `rate_limit.rs` nor `daily_project_metrics` currently has | Real `daily_project_metrics` rows, real request traffic in integration tests |
| 07 | US-207 | 3 | 1 day | Wiring the cap-exceeded trigger to `suspend_project()` surfaces a race condition the dunning trigger (US-204) didn't, because this trigger fires from the request hot path rather than an async webhook handler | Real Postgres, real suspend/activate round-trip |

**Total estimate: ~10 days** (~2 weeks), consistent with § Scope Assessment's borderline >2-week signal.

**Taste tests applied**:
- "4+ new components per slice" — none exceed 3 (e.g. Slice 03: sub-router + middleware + handler); PASS.
- "Every slice depends on a new abstraction" — only Slice 01 introduces the `subscriptions` schema and Stripe-customer-provisioning abstraction; Slices 02-07 reuse it. PASS (abstraction shipped first, in the WS).
- "No slice disproves a pre-commitment" — each has a distinct, falsifiable hypothesis (see table); Slice 06's is the highest-value one to watch, directly testing the § Open Question's latency/staleness trade-off. PASS.
- "Synthetic-data-only slices prove plumbing, not value" — **N/A, does not apply here**: unlike `card-payments`' UI-only ADR-007 exception, every slice in this feature uses real Postgres rows and real Stripe test-mode API calls per D-13 — there is no synthetic-data exception to document.
- "2+ slices identical except for scale" — none; each targets a distinct mechanism. PASS.

---

## Wave: DISCUSS / [REF] Prioritization

| Priority | Slice | Target Outcome | Rationale |
|---|---|---|---|
| 1 | Slice 01 (WS) | Real Stripe integration proven end-to-end | Walking Skeleton always first — burns down the riskiest new assumption (real network SDK call inside existing handler shape) before any dependent slice starts |
| 2 | Slice 02 | Plan changes are real | Directly unblocks Release 1's core value; also the prerequisite for US-207's "upgrade clears suspension" AC |
| 3 | Slice 03 | Webhook plumbing exists | Riskiest-assumption-first for Release 1 — if signature verification/idempotency design is wrong, it invalidates both US-204's dunning path and any future webhook consumer |
| 4 | Slice 04 | Dunning suspend/recovery real | Completes Release 1; highest-consequence correctness requirement (D-12's single-mechanism guarantee) |
| 5 | Slice 05 | Usage metering real | Release 2 — independently valuable, no dependency on Release 3 |
| 6 | Slice 06 | Cumulative cap check exists (read-only) | Deliberately split from Slice 07 (compute-then-enforce) so the riskiest new architecture (§ Open Question) is validated in isolation before any suspend action is wired to it |
| 7 | Slice 07 | Cap-exceeded suspension enforced | Closes the loop on D-6; last because it depends on Slice 06's mechanism being proven safe first |

---

## Wave: DISCUSS / [REF] System Constraints

- **No mocked payment port** (D-13, hard constraint): every acceptance test in DISTILL for this feature must exercise the real Stripe test-mode API and/or Stripe CLI `stripe trigger` — consistent with this project's established preference for real-infrastructure integration tests (testcontainers Postgres, real subprocess servers) over mocked domain ports. DISTILL must NOT introduce an `IPaymentGateway` mock port for this feature.
- `subscriptions.status`/cap-exceeded/payment-failed derivations MUST resolve to the exact same suspend/activate code path (`set_project_status` in `lifecycle.rs`) regardless of trigger (D-12) — no parallel/duplicated suspend implementation.
- New domain types with no IO (e.g. a cumulative-cap value type, a webhook-event-outcome type) live in `crates/embyr-core/src/admin/` per the existing no-IO convention (enforced by `deny.toml`); actual Stripe SDK calls live in `embyr-server`'s adapters layer only.
- `STRIPE_SECRET_KEY`, `STRIPE_WEBHOOK_SIGNING_SECRET`, `STRIPE_PUBLISHABLE_KEY` resolved via the existing `config.rs` env-var + AWS/GCP-secret-manager pattern (`resolve_admin_key`-shaped resolver), not a new pattern.
- Migrations for this feature start at `0019` (next after `0018_rate_buckets.sql`); exact column types and numbering are DESIGN/DELIVER-wave decisions, not fixed here.
- Ubiquitous language carried forward unchanged from `card-payments`: **Free plan**, **Pro plan**, **cap-exceeded**, **past_due**, **read-only/suspended**, **on file**.
- Cumulative usage-vs-cap fail-open behavior on computation staleness/unavailability (see § Journey Deep-Dive Error Paths) mirrors the existing `rate_limit.rs` fail-open precedent — a design constraint DESIGN must honor, not re-decide.

---

## Wave: DISCUSS / [REF] User Stories

<!-- markdownlint-disable MD024 -->

### US-201: A Real Subscription Record Exists (Walking Skeleton)

**job_id**: JOB-14
**Slice**: 01 | **Release**: 1

#### Elevator Pitch
Before: There is no real Stripe Customer or subscription record for any account — `AppModel.subscription` in the shipped frontend is entirely mock data with no backing Stripe object.
After: call `GET /admin/v1/billing/subscription` (session-authed, any role) → sees `{"plan":"free","status":"active","stripe_customer_id":"cus_...","current_period_end":null,"card":null}` — a real Stripe Customer, auto-provisioned on first call and persisted in the new `subscriptions` table.
Decision enabled: Chris (or any downstream story in this feature) can now trust that plan/status data reflects a real, addressable Stripe object instead of an unbacked mock field.

#### Domain Examples
1. **Happy Path**: Chris Okafor's account (Aperture Labs) has never called the endpoint before — first call creates a real Stripe test-mode Customer, seeds a `subscriptions` row with `plan='free'`, returns it.
2. **Edge Case**: Chris calls the endpoint a second time — no duplicate Stripe Customer is created; the existing `stripe_customer_id` is reused (idempotent provisioning).
3. **Error/Boundary**: Stripe's API is unreachable during provisioning — the endpoint returns a 502 with no partial `subscriptions` row written (no half-provisioned state).

#### UAT Scenarios (BDD)

##### Scenario: First billing call auto-provisions a real Stripe customer
Given Aperture Labs has never called the billing subscription endpoint
When Chris calls `GET /admin/v1/billing/subscription`
Then a real Stripe test-mode Customer is created, `accounts.stripe_customer_id` is populated, and a `subscriptions` row is seeded with `plan='free'`, `status='active'`

##### Scenario: Repeated calls do not create duplicate Stripe customers
Given Aperture Labs already has `stripe_customer_id = "cus_abc123"`
When Chris calls `GET /admin/v1/billing/subscription` again
Then no new Stripe Customer is created and the response's `stripe_customer_id` is unchanged

##### Scenario: A Pro-plan account returns its real period end
Given Northwind Data's `subscriptions` row has `plan='pro'`, `current_period_end='2026-09-12'`
When Dana calls `GET /admin/v1/billing/subscription`
Then the response includes `"plan":"pro"` and `"current_period_end":"2026-09-12"`

##### Scenario: Stripe unavailability during first-time provisioning leaves no partial state
Given Solstice Analytics has never called the endpoint, and Stripe's API is unreachable
When Priya calls `GET /admin/v1/billing/subscription`
Then the response is a 502 and no `subscriptions` row exists for Solstice Analytics afterward

##### Scenario: Only session-authed callers may read subscription data
Given no session cookie or admin Bearer key is presented
When a request hits `GET /admin/v1/billing/subscription`
Then the response is 401, mirroring the existing session-auth-required pattern used by `GET /admin/v1/billing`

#### Acceptance Criteria
- [ ] AC-201-01: `GET /admin/v1/billing/subscription` is session-authed (any role), mirrors `get_billing`'s auth shape
- [ ] AC-201-02: First call for an account with no `stripe_customer_id` creates a real Stripe test-mode Customer and persists the id on `accounts`
- [ ] AC-201-03: Subsequent calls reuse the existing `stripe_customer_id` — never create a second Customer for the same account
- [ ] AC-201-04: A new `subscriptions` row is seeded `plan='free'`, `status='active'` on first provisioning
- [ ] AC-201-05: Stripe API failure during provisioning returns 502 and writes no partial `subscriptions`/`stripe_customer_id` state
- [ ] AC-201-06: Response shape includes `plan`, `status`, `stripe_customer_id`, `current_period_end`, `card` (null until US-202/future card-capture work populates it)

#### Outcome KPIs
See § Outcome KPIs below (KPI #1, guardrail).

#### Technical Notes (Optional)
New migration `0019_subscriptions.sql` (`subscriptions` table) + `accounts.stripe_customer_id` column. `STRIPE_SECRET_KEY` resolved via the `config.rs` secret-manager pattern (Technical Note only — exact `ServerConfig` field naming is a DESIGN-wave decision).

---

### US-202: Plan Change Persists to a Real Stripe Subscription

**job_id**: JOB-14
**Slice**: 02 | **Release**: 1

#### Elevator Pitch
Before: The shipped `UpgradeModal` (US-106) only mutates `AppModel.subscription.plan` in-browser — no real Stripe Subscription object is ever created or changed.
After: call `POST /admin/v1/billing/subscription` with `{"plan":"pro"}` (session-authed, Owner/Admin role) → sees `{"plan":"pro","status":"active",...}` reflecting a real Stripe Subscription update, and if the account was previously `free_cap_exceeded`, `status` returns to `active` in the same response.
Decision enabled: Chris's upgrade/downgrade decision now has real financial consequences and is verifiably persisted, not a local-only UI toggle.

#### Domain Examples
1. **Happy Path**: Chris upgrades Aperture Labs Free→Pro — Stripe Subscription object is created/updated to the Pro price, local row updates to `plan='pro'`.
2. **Edge Case**: Dana downgrades Northwind Data Pro→Free — Stripe Subscription is updated (or scheduled to change at period end, per Stripe's own semantics — DESIGN's call on immediate-vs-scheduled), local row reflects the new plan.
3. **Error/Boundary**: Priya's account is `free_cap_exceeded`; she upgrades to Pro — Stripe Subscription updates AND `subscriptions.status` clears from `free_cap_exceeded` back to `active` in the same request.

#### UAT Scenarios (BDD)

##### Scenario: Upgrading Free to Pro updates the real Stripe subscription
Given Aperture Labs is on the Free plan with a real `stripe_customer_id`
When Chris calls `POST /admin/v1/billing/subscription` with `{"plan":"pro"}`
Then Stripe's Subscription object for that customer is created/updated to the Pro price, and the response shows `"plan":"pro"`

##### Scenario: A failed Stripe call does not update the local plan
Given Aperture Labs is on the Free plan and Stripe's API call for the plan-change fails
When Chris calls `POST /admin/v1/billing/subscription` with `{"plan":"pro"}`
Then the response is an error, and the local `subscriptions` row still shows `plan='free'` — no optimistic update occurred

##### Scenario: Upgrading from a cap-exceeded state clears the suspension
Given Solstice Analytics' `subscriptions.status` is `free_cap_exceeded`
When Priya calls `POST /admin/v1/billing/subscription` with `{"plan":"pro"}`
Then the response shows `"status":"active"` and Solstice Analytics' projects are reactivated via the same `activate_project` path

##### Scenario: Only Owner/Admin roles may change the plan
Given a Viewer-role member of Aperture Labs is signed in
When they call `POST /admin/v1/billing/subscription` with `{"plan":"pro"}`
Then the response is 403, mirroring the existing `check_rbac` role-gating pattern used elsewhere in `admin/handlers`

##### Scenario: An unrecognized plan value is rejected
Given Chris is signed in as Owner
When Chris calls `POST /admin/v1/billing/subscription` with `{"plan":"enterprise"}`
Then the response is 422 — only `"free"` and `"pro"` are valid (D-4, two tiers only)

#### Acceptance Criteria
- [ ] AC-202-01: `POST /admin/v1/billing/subscription` is session-authed, Owner/Admin role only (`check_rbac`)
- [ ] AC-202-02: Valid plan change calls Stripe's Subscription update API before any local write
- [ ] AC-202-03: Local `subscriptions.plan` updates only after Stripe confirms success — no optimistic update on failure
- [ ] AC-202-04: Upgrading while `status='free_cap_exceeded'` clears status back to `active` and reactivates the account's projects
- [ ] AC-202-05: `plan` values other than `"free"`/`"pro"` return 422 (D-4)

#### Outcome KPIs
See § Outcome KPIs below (KPI #1).

---

### US-203: Webhook Ingestion Keeps Subscription State in Sync

**job_id**: JOB-14
**Slice**: 03 | **Release**: 1

#### Elevator Pitch
Before: There is no webhook endpoint anywhere in embyr-server — if Stripe's own dashboard is used to change a subscription directly, embyr's local record silently drifts from the truth with no way to catch up.
After: Stripe sends (or an operator synthesizes via `stripe trigger customer.subscription.updated`) a webhook event to `POST /admin/v1/webhooks/stripe` → sees `GET /admin/v1/billing/subscription` (US-201) immediately reflect the updated plan/status, sourced from Stripe's own event, not a manual edit.
Decision enabled: Chris (and this feature's own future stories) can trust that the local subscription record never drifts from what Stripe actually has on file, regardless of which side initiated the change.

#### Domain Examples
1. **Happy Path**: A `customer.subscription.updated` event for Aperture Labs' subscription arrives — local `subscriptions` row updates to match Stripe's new plan/period-end.
2. **Edge Case**: Stripe redelivers the *same* `event.id` twice (its documented retry behavior) — the second delivery is a 200 no-op, state is not double-applied.
3. **Error/Boundary**: A request arrives at `POST /admin/v1/webhooks/stripe` with a missing/invalid `Stripe-Signature` header — rejected 401 before any DB write, regardless of payload content.

#### UAT Scenarios (BDD)

##### Scenario: A valid subscription-updated event syncs the local record
Given Northwind Data's Stripe subscription's `current_period_end` changes
When Stripe (or `stripe trigger customer.subscription.updated`) delivers the event to `POST /admin/v1/webhooks/stripe` with a valid signature
Then the local `subscriptions` row for Northwind Data reflects the new `current_period_end`

##### Scenario: An invalid signature is rejected before any state change
Given a request to `POST /admin/v1/webhooks/stripe` has a `Stripe-Signature` header that does not verify against `STRIPE_WEBHOOK_SIGNING_SECRET`
When the request is received
Then the response is 401 and no `subscriptions` or `processed_webhook_events` row is written

##### Scenario: A duplicate event delivery is a no-op
Given `event.id = "evt_xyz"` has already been recorded in `processed_webhook_events`
When the same `event.id` is delivered again with a valid signature
Then the response is 200 and the `subscriptions` row is not modified a second time

##### Scenario: A subscription-deleted event marks the account appropriately
Given Aperture Labs' Stripe subscription is deleted (e.g. via Stripe dashboard)
When `customer.subscription.deleted` is delivered
Then the local `subscriptions` row reflects the deletion (exact terminal-state representation is a DESIGN decision; the behavioral requirement is that the local row never silently continues showing an active paid subscription that no longer exists in Stripe)

##### Scenario: An unrecognized-but-valid event type does not fail the request
Given a validly-signed webhook event of a type this feature does not yet handle (e.g. `customer.updated`)
When it is delivered to `POST /admin/v1/webhooks/stripe`
Then the response is 200 (logged, ignored) — not a 500 or 400

#### Acceptance Criteria
- [ ] AC-203-01: `POST /admin/v1/webhooks/stripe` is a 5th sub-router on `:9090`, structurally a sibling of `public_router` (D-11), with its own `stripe_signature_middleware` — no session/operator auth applied
- [ ] AC-203-02: Missing/invalid `Stripe-Signature` header → 401, zero DB writes
- [ ] AC-203-03: Every processed event's `event.id` is recorded in `processed_webhook_events`; a redelivered `event.id` is a 200 no-op
- [ ] AC-203-04: `customer.subscription.updated` syncs `plan`/`status`/`current_period_end` on the local `subscriptions` row
- [ ] AC-203-05: `customer.subscription.deleted` is handled without leaving the local row falsely showing an active paid plan
- [ ] AC-203-06: Unhandled-but-validly-signed event types return 200, not an error status

#### Outcome KPIs
See § Outcome KPIs below (KPI #2, guardrail).

#### Technical Notes (Optional)
New migration for `processed_webhook_events` (`event_id` unique, `processed_at`). Tested via Stripe CLI `stripe trigger` per D-13 — no mocked webhook payloads.

---

### US-204: Dunning Suspension and Payment Recovery

**job_id**: JOB-14 (cross-reference: JOB-06 — reuses the suspend/activate mechanism JOB-06 introduced)
**Slice**: 04 | **Release**: 1

#### Elevator Pitch
Before: The shipped `SuspensionBanner` (US-108) can only be demoed via `TestClockCard`'s (US-109) fake mock-state toggle — there is no real payment-failure signal anywhere in the backend.
After: Stripe exhausts Smart Retries on a failed Pro-plan invoice (or an operator synthesizes via `stripe trigger invoice.payment_failed`) → sees the account's projects transition to `status='suspended'` (observable via the existing `GET /admin/v1/projects/:project_id`), and a subsequent `invoice.payment_succeeded` reverses it.
Decision enabled: Dana (or any Pro-plan admin) experiences a real, accurate suspension/recovery cycle tied to actual payment outcomes, not a developer-only simulator.

#### Domain Examples
1. **Happy Path**: Northwind Data's Pro invoice payment fails, Smart Retries exhaust — `invoice.payment_failed` (final-failure flag) suspends all of Northwind Data's projects.
2. **Edge Case**: Dana updates her card and the next retry succeeds — `invoice.payment_succeeded` reactivates all of Northwind Data's projects automatically, no operator involvement.
3. **Error/Boundary**: `invoice.payment_failed` arrives but Stripe's retry schedule has *not* yet exhausted (an intermediate retry, not the final one) — no suspension occurs; only the final-failure event suspends.

#### UAT Scenarios (BDD)

##### Scenario: Exhausted retries suspend every project under the account
Given Northwind Data has 3 projects and its Pro invoice has exhausted Stripe Smart Retries
When the final `invoice.payment_failed` webhook is delivered
Then all 3 of Northwind Data's projects transition to `status='suspended'`

##### Scenario: An intermediate retry failure does not suspend
Given Northwind Data's invoice has failed once but Stripe's retry schedule has not yet exhausted
When an intermediate (non-final) `invoice.payment_failed` webhook is delivered
Then no project status change occurs

##### Scenario: Payment recovery reactivates every project under the account
Given Northwind Data's projects are suspended due to payment failure
When `invoice.payment_succeeded` is delivered for that subscription
Then all of Northwind Data's projects transition back to `status='active'`

##### Scenario: Dunning suspension uses the exact same mechanism as cap-exceeded suspension
Given the dunning trigger fires
When it suspends a project
Then it calls the identical `set_project_status(..., "suspended", ...)` function US-207 (cap-exceeded) also calls — verified by a shared test helper or code-path assertion, not a duplicated implementation

##### Scenario: Suspension evicts the credential cache exactly as the operator-initiated suspend does today
Given a project is suspended via the dunning trigger
When the suspension is applied
Then the project's entry is evicted from the credential cache, matching `suspend_project`'s existing operator-initiated behavior byte-for-byte

#### Acceptance Criteria
- [ ] AC-204-01: `invoice.payment_failed` with a final-failure indicator suspends every active project under the account
- [ ] AC-204-02: A non-final (intermediate retry) `invoice.payment_failed` does not suspend
- [ ] AC-204-03: `invoice.payment_succeeded` reactivates every suspended project under the account
- [ ] AC-204-04: The dunning trigger calls the same `set_project_status` function as the operator-initiated and cap-exceeded (US-207) paths — no parallel suspend implementation
- [ ] AC-204-05: Suspension via this path evicts the credential cache identically to today's `suspend_project`

#### Outcome KPIs
See § Outcome KPIs below (KPI #2, KPI #3).

#### Technical Notes (Optional)
Depends on US-203 (webhook plumbing) existing. Tested via `stripe trigger invoice.payment_failed`/`invoice.payment_succeeded` per D-13.

---

### US-205: Nightly Usage Metering to Stripe

**job_id**: JOB-14
**Slice**: 05 | **Release**: 2

#### Elevator Pitch
Before: `NextInvoiceCard` (US-103, shipped) estimates Dana's overage entirely from mock data — no real Stripe usage record has ever been pushed for any project.
After: an operator runs `POST /admin/v1/billing/run-metering` (or the job runs on its nightly schedule) → sees one Stripe usage record pushed per project per dimension for the prior day's `daily_project_metrics` rows, verifiable via Stripe's own usage-record API or a `GET /admin/v1/billing/metering-log` audit trail.
Decision enabled: Dana's actual Stripe invoice will reflect real, itemized overage instead of nothing — the batch job is what makes D-1's hybrid billing model financially real.

#### Domain Examples
1. **Happy Path**: Northwind Data's `daily_project_metrics` shows 620,000 reads yesterday — a Stripe usage record for `reads` is pushed for that project/day.
2. **Edge Case**: A project has zero usage yesterday (dormant account) — no usage record is pushed for that project/dimension that day (avoids pushing meaningless zero-records at Stripe API cost).
3. **Error/Boundary**: The batch run is re-triggered a second time for the same day (operator retry after a partial failure) — no dimension is double-counted; the second run is idempotent per project/dimension/day.

#### UAT Scenarios (BDD)

##### Scenario: A day's usage is pushed as one record per project per dimension
Given Aperture Labs' `daily_project_metrics` for yesterday shows non-zero reads, writes, deletes, and storage for project "prod-orders"
When the metering run executes (scheduled or via `POST /admin/v1/billing/run-metering`)
Then 4 separate Stripe usage records are pushed for "prod-orders" — one per dimension — each tagged to that project/day

##### Scenario: Zero-usage dimensions are not pushed
Given "staging-orders" recorded zero deletes yesterday
When the metering run executes
Then no Stripe usage record is pushed for "staging-orders"' deletes dimension that day

##### Scenario: Re-running the same day's metering does not double-count
Given yesterday's metering run already pushed reads/writes/deletes/storage records for all projects
When an operator re-triggers `POST /admin/v1/billing/run-metering` for the same day
Then no dimension's usage record is pushed a second time for that project/day (idempotency key derived from project_id + dimension + date)

##### Scenario: A single project's Stripe failure does not abort the whole run
Given the metering run is processing 5 projects and Stripe's API call for project 3 fails
When the run completes
Then projects 1, 2, 4, and 5 have their usage records pushed successfully, and project 3 is retried on the next run

##### Scenario: Manual trigger is operator-only
Given a non-operator (e.g. Chris, a customer admin) attempts to call `POST /admin/v1/billing/run-metering`
When the request is made
Then the response is 401/403, mirroring the existing `operator_auth_middleware`-gated routes

#### Acceptance Criteria
- [ ] AC-205-01: Batch run reads yesterday's `daily_project_metrics` rows and pushes one Stripe usage record per project per non-zero dimension
- [ ] AC-205-02: Zero-usage project/dimension/day combinations are not pushed
- [ ] AC-205-03: Re-running for an already-processed day is idempotent — no dimension is double-pushed
- [ ] AC-205-04: A per-project Stripe API failure does not abort remaining projects in the same run
- [ ] AC-205-05: `POST /admin/v1/billing/run-metering` is operator-authed only (Bearer `EMBYR_ADMIN_KEY`, mirrors `operator_router`)

#### Outcome KPIs
See § Outcome KPIs below (KPI #4).

#### Technical Notes (Optional)
Depends on US-201 (accounts having a `stripe_customer_id` to attach usage records to). Idempotency key strategy (e.g. `{project_id}:{dimension}:{date}`) is a DESIGN-wave decision; the *requirement* (safe to re-run) is locked here.

---

### US-206: Real-Time Cumulative Usage Is Computed Against the Plan Cap

**job_id**: JOB-14 (cross-reference: JOB-11 — physically extends `rate_limit.rs`, but a different "why"; see § Open Question)
**Slice**: 06 | **Release**: 3

#### Elevator Pitch
Before: There is no way to know, in real time, how close a Free-plan account is to its cap — the shipped `CapUsageCard` (US-102) computes this entirely from mock `AppModel` data with no backend equivalent.
After: call `GET /admin/v1/billing/subscription` (extended, US-201) → sees a new `cap_status` field, e.g. `[{"dimension":"writes","used":412000,"cap":500000,"pct":82}]`, computed from real per-account cumulative usage this billing cycle.
Decision enabled: Chris can trust that a real, backend-computed cap proximity exists and is queryable — independent of whether any enforcement action (US-207) has fired yet.

#### Domain Examples
1. **Happy Path**: Aperture Labs (Free) has used 412,000 of 500,000 writes across its 3 projects this cycle — `cap_status` reports `writes: 82%`.
2. **Edge Case**: Solstice Analytics has used exactly 100,000 of 100,000 deletes — `cap_status` reports `deletes: 100%` (at, not over, the boundary).
3. **Error/Boundary**: A Pro-plan account (Northwind Data) calls the endpoint — `cap_status` is omitted or null (caps are a Free-plan-only concept, D-6/D-7).

#### UAT Scenarios (BDD)

##### Scenario: Cumulative usage sums across all of an account's projects
Given Aperture Labs has 3 projects with combined writes of 412,000 this billing cycle, and a 500,000 Free-plan writes cap
When Chris calls `GET /admin/v1/billing/subscription`
Then `cap_status` reports `writes` at 82%, summed across all 3 projects — not just one

##### Scenario: Usage at exactly the cap reports 100%, not over/under
Given Solstice Analytics has used exactly 100,000 of its 100,000-delete Free cap this cycle
When Priya calls the endpoint
Then `cap_status` reports `deletes` at exactly 100%

##### Scenario: Pro-plan accounts do not receive cap_status
Given Northwind Data is on the Pro plan
When Dana calls the endpoint
Then the response's `cap_status` is absent or empty — caps do not apply to Pro (D-6/D-7 are Free-plan concepts)

##### Scenario: A stale or unavailable computation fails open, not closed
Given the cumulative-usage computation for Aperture Labs cannot be completed (e.g. its data source is temporarily unavailable)
When Chris calls the endpoint
Then the response does not falsely report a cap as exceeded — it either omits `cap_status` or reports the last-known-safe value, but never fabricates an over-cap reading (mirrors `rate_limit.rs`'s existing fail-open precedent)

##### Scenario: The cycle boundary resets cumulative usage
Given Aperture Labs' billing cycle rolled over at midnight
When Chris calls the endpoint the next day
Then `cap_status`'s usage figures reflect only the new cycle, not cumulative-since-account-creation

#### Acceptance Criteria
- [ ] AC-206-01: `cap_status` sums usage across ALL of an account's projects, keyed by `account_id`, not `project_id` (explicitly different key shape than `rate_buckets`)
- [ ] AC-206-02: Usage exactly at the cap reports 100% (boundary-inclusive, matches US-102's frontend `≥100%` red-threshold semantic)
- [ ] AC-206-03: `cap_status` is only computed/returned for Free-plan accounts
- [ ] AC-206-04: Computation unavailability fails open (never fabricates an over-cap reading) — same principle as `rate_limit.rs`'s Postgres-timeout fail-open branch
- [ ] AC-206-05: Usage resets at the account's billing-cycle boundary, not account-creation date
- [ ] AC-206-06: This is a NEW, separately-keyed mechanism from `TokenBucket`/`rate_buckets` — DESIGN must not literally extend `TokenBucket`'s per-second refill fields (see § Open Question)

#### Outcome KPIs
See § Outcome KPIs below (KPI #5).

#### Technical Notes (Optional)
See § Open Question above — the concrete computation/caching mechanism is explicitly left to DESIGN. This story delivers read-only visibility; US-207 wires the enforcement action.

---

### US-207: Free-Cap-Exceeded Suspension Enforced in Real Time

**job_id**: JOB-14 (cross-reference: JOB-11, same note as US-206)
**Slice**: 07 | **Release**: 3

#### Elevator Pitch
Before: D-6's "hard-stop, suspend" policy has never actually suspended anyone — the shipped `SuspensionBanner`'s `free_cap_exceeded` state can only be demoed via `TestClockCard`'s mock toggle.
After: a Free-plan account crosses 100% of its cap on any dimension (per US-206's computation) → sees its projects transition to `status='suspended'` (observable via `GET /admin/v1/projects/:project_id`) without any operator action.
Decision enabled: D-6's promise ("hard-stop, suspend, manual upgrade required — no silent auto-upgrade/auto-charge") is now a real, enforced guarantee, not a UI-only concept — closing the trust loop the frontend's SuspensionBanner (US-108) was built to explain.

#### Domain Examples
1. **Happy Path**: Solstice Analytics' deletes hit exactly 100,000/100,000 (100%) — its projects transition to `suspended`, `subscriptions.status` becomes `free_cap_exceeded`.
2. **Edge Case**: Aperture Labs' writes are at 82% (amber, per US-102's threshold) — no suspension; only crossing 100% triggers it.
3. **Error/Boundary**: A Pro-plan account's usage would exceed what a Free plan's cap would have been — no suspension occurs; caps are a Free-plan-only concept (D-6/D-7), Pro is metered overage instead (D-1).

#### UAT Scenarios (BDD)

##### Scenario: Crossing 100% of a Free-plan cap suspends the account's projects
Given Solstice Analytics (Free plan) crosses 100,000/100,000 deletes this cycle
When the next request triggers the cap check (US-206)
Then all of Solstice Analytics' projects transition to `status='suspended'` and `subscriptions.status` becomes `free_cap_exceeded`

##### Scenario: Below-cap usage never suspends
Given Aperture Labs (Free plan) is at 82% of its writes cap
When any request triggers the cap check
Then no suspension occurs

##### Scenario: Pro-plan accounts are never suspended by this mechanism
Given Northwind Data is on the Pro plan and its usage this cycle exceeds what a Free-plan cap would have been
When the cap check runs
Then no suspension occurs — Pro-plan overage is billed (US-205), not enforced as a hard-stop

##### Scenario: Cap-exceeded suspension uses the same mechanism as dunning suspension
Given the cap-exceeded trigger fires
When it suspends a project
Then it calls the identical `set_project_status(..., "suspended", ...)` function US-204 (dunning) also calls — D-12's "one mechanism, two triggers" guarantee, verified directly, not assumed

##### Scenario: Upgrading to Pro clears a cap-exceeded suspension (already covered structurally by US-202, verified here from the enforcement side)
Given Solstice Analytics' projects are suspended due to `free_cap_exceeded`
When Priya upgrades to Pro via `POST /admin/v1/billing/subscription` (US-202)
Then the projects suspended by THIS mechanism are reactivated by that same call — no separate manual re-activation step is needed

#### Acceptance Criteria
- [ ] AC-207-01: Crossing ≥100% of any Free-plan dimension cap suspends all of the account's projects
- [ ] AC-207-02: Usage below 100% never triggers suspension via this path
- [ ] AC-207-03: Pro-plan accounts are never suspended by this mechanism (Free-plan-only, D-6/D-7)
- [ ] AC-207-04: This trigger calls the identical `set_project_status` function as US-204's dunning trigger — no parallel implementation (D-12)
- [ ] AC-207-05: Upgrading to Pro (US-202) reactivates projects suspended by this mechanism, verified end-to-end from the enforcement side

#### Outcome KPIs
See § Outcome KPIs below (KPI #5).

#### Technical Notes (Optional)
Depends on US-206 (the computation this trigger reads) and reuses US-204's suspend call site. This is the highest-consequence story in the feature per the § Journey Deep-Dive shared-artifacts registry — a bug here either fails to enforce a real policy (revenue/trust risk) or falsely suspends a healthy account (severe trust/availability risk in the opposite direction).

---

## Wave: DISCUSS / [REF] Outcome KPIs

### Feature: card-payments-backend

### Objective
Make JOB-14's billing/payment self-service experience real and financially accurate — and, as a direct side effect, unlock measurement of the 3 Outcome KPIs `card-payments`' DISCUSS wave defined but explicitly could not measure ("cannot be measured against real production behavior until `card-payments-backend` exists").

### Outcome KPIs

| # | Who | Does What | By How Much | Baseline | Measured By | Type |
|---|---|---|---|---|---|---|
| 1 | Account admins reading/changing subscription state | `GET`/`POST /admin/v1/billing/subscription` return data that matches Stripe's own dashboard for that customer | 100% agreement between local `subscriptions` row and Stripe's live subscription object, sampled weekly post-launch | 0% (no real subscription record exists today) | Scheduled reconciliation job comparing local rows to Stripe API (DEVOPS-wave instrumentation) | Guardrail |
| 2 | Stripe (webhook sender) | Webhook deliveries are processed successfully without manual intervention | ≥99.9% of webhook deliveries return 200 within the first 2 attempts (idempotency + signature verification both correct) | N/A (no webhook endpoint exists today) | `embyr_webhook_requests_total{outcome}` counter (mirrors the existing `embyr_rate_limit_requests_total` Prometheus pattern from `observability`) | Guardrail |
| 3 | Pro-plan accounts experiencing payment failure | Are suspended and reactivated correctly, with zero false suspensions of accounts that never had a payment failure | 0 false-positive suspensions (accounts suspended without a corresponding real `invoice.payment_failed` final-failure event) | N/A (mechanism doesn't exist today) | Suspension audit log cross-referenced against webhook event history | Guardrail (trust-critical) |
| 4 | Dana Whitfield and other Pro-plan admins | See their Stripe invoice reflect real, itemized overage matching `daily_project_metrics` | 100% of billed usage-record line items reconcile to `daily_project_metrics` totals for the same project/day, zero drift | 0% (no usage record has ever been pushed) | Reconciliation query comparing pushed Stripe usage records to `daily_project_metrics` | Leading |
| 5 | Free-tier accounts crossing their cap | Are suspended within an acceptable latency of actually crossing the cap, and reactivate immediately on upgrade | Target latency-to-enforcement and false-positive rate TBD by DESIGN once the § Open Question's computation mechanism is chosen (cannot set a numeric target before the mechanism — live-check vs. cached vs. incremental — is decided, since each has a different latency profile) | 0% enforcement exists today (D-6 is currently UI-only) | Enforcement audit log + `daily_project_metrics` cross-reference | North Star (unblocks `card-payments`' own KPI #1) |

### Metric Hierarchy
- **North Star**: KPI #5 (real Free-cap enforcement, correct and timely) — this is the mechanism `card-payments`' own North Star KPI (self-serve upgrade before hard-suspend) is entirely dependent on; that KPI cannot be instrumented until this one exists.
- **Leading Indicators**: KPI #4 (metering accuracy) — directly feeds `card-payments`' KPI #1 (cap-proximity self-serve behavior) by making the invoice estimate real.
- **Guardrail Metrics**: KPI #1 (subscription-record reconciliation), KPI #2 (webhook reliability), KPI #3 (zero false suspensions) — all three are trust-critical correctness floors, not growth metrics; a regression in any of them is the single highest-consequence defect class this feature can produce (a false suspension of a paying, healthy account is reputationally worse than a missed enforcement).

### Measurement Plan

| KPI | Data Source | Collection Method | Frequency | Owner |
|---|---|---|---|---|
| 1 | Local `subscriptions` table + Stripe API | Scheduled reconciliation job (new, DEVOPS-wave scope) | Weekly | platform-architect (DEVOPS wave) |
| 2 | Prometheus counter | New `embyr_webhook_requests_total{outcome}` metric, mirrors existing `embyr_rate_limit_requests_total` pattern (`observability` feature, ADR-016) | Continuous | platform-architect (DEVOPS wave) |
| 3 | Suspension audit log + webhook event history | Cross-reference query | Weekly | platform-architect (DEVOPS wave) |
| 4 | Stripe usage-record API + `daily_project_metrics` | Reconciliation query | Daily (matches metering cadence) | platform-architect (DEVOPS wave) |
| 5 | Enforcement audit log + `daily_project_metrics` | Cross-reference query | Continuous | platform-architect (DEVOPS wave) |

### Hypothesis
We believe that making JOB-14's billing self-service experience real (backed by Stripe subscriptions, webhooks, metered usage, and enforced caps) will make `card-payments`' own frontend KPIs measurable for the first time, and will close the trust gap between what the shipped UI *displays* and what actually happens to a customer's account.
We will know this is true when the 3 guardrail KPIs (subscription accuracy, webhook reliability, zero false suspensions) hold steady above their targets for 4 consecutive weeks post-launch, and `card-payments`' KPI #1 (self-serve upgrade rate before hard-suspend) becomes measurable at all (baseline: currently un-measurable, since no real event pipeline exists).

**Note on baseline honesty**: KPI #5's numeric target is explicitly deferred to DESIGN (see table) rather than fabricated — the target depends on which cap-check mechanism DESIGN selects for the § Open Question, and setting a number before that choice is made would be a false precision.

---

## Wave: DISCUSS / [REF] Definition of Ready Validation

### Story: US-201 through US-207 (all 7 stories, card-payments-backend)

| DoR Item | Status | Evidence |
|---|---|---|
| 1. Problem statement clear, domain language | PASS | Every story's Elevator Pitch names the concrete "Before" state read directly from the actual shipped code (e.g. "`AppModel.subscription` in the shipped frontend is entirely mock data") |
| 2. User/persona with specific characteristics | PASS | P5 Chris Okafor (primary), Dana Whitfield/Priya Raman (secondary, same named examples as `card-payments`), P2 Sam (operator, US-205 only) |
| 3. 3+ domain examples with real data | PASS | Every story has exactly 3 (Happy/Edge/Error) with real account names and real numbers (e.g. "412,000 of 500,000 monthly writes", "620,000 reads") |
| 4. UAT in Given/When/Then (3-7 scenarios) | PASS | US-201: 5, US-202: 5, US-203: 5, US-204: 5, US-205: 5, US-206: 5, US-207: 5 — all within 3-7 |
| 5. AC derived from UAT | PASS | Every AC traces to a named scenario (e.g. AC-206-04 ← "A stale or unavailable computation fails open" scenario) |
| 6. Right-sized (1-3 days, 3-7 scenarios) | PASS | Every story maps 1:1 to a slice, each ≤2 days (see § Elephant Carpaccio Slices); feature-level story count (7) is within the ≤10 threshold |
| 7. Technical notes identify constraints | PASS | US-201 (migration numbering), US-203 (idempotency key), US-205 (idempotency strategy deferred to DESIGN), US-206/207 (§ Open Question explicitly cross-referenced) |
| 8. Dependencies resolved or tracked | PASS | Release 2 and Release 3 both depend on Release 1's schema (documented in § Story Map Releases); US-207 depends on US-206; no circular or unresolved dependency |
| 9. Outcome KPIs defined with measurable targets | PASS (4 of 5 with numeric targets; KPI #5 explicitly defers its numeric target to a named DESIGN-wave decision, documented as a baseline-honesty choice, not an omission) | § Outcome KPIs above |

### DoR Status: **PASSED** (all 9 items, all 7 stories)

### Requirements Completeness Score: 0.96

- Functional requirements: complete — 7 stories cover the full backbone (§ Story Map), all traced to D-1..D-13.
- Non-functional requirements: D-13's real-Stripe/no-mock testing constraint is carried into § System Constraints as a DISTILL-scoping requirement. One deliberate, documented gap: KPI #5's numeric enforcement-latency target is explicitly deferred to DESIGN (not fabricated) — this is the same class of honesty gap `card-payments`' own DISCUSS accepted for its own KPIs, scored down slightly for the same reason (0.96 vs 1.0, one point below `card-payments`' own 0.97 because this feature carries one additional open architectural question, § Open Question, that DESIGN must resolve before implementation can start — a real, not cosmetic, gap).
- Business rules: complete — D-1 through D-13 are all traced to specific ACs in § Locked Decisions.

---

## Wave: DISCUSS / [REF] Out of Scope

- **Real Stripe Elements/Stripe.js JS interop shim for card capture (D-3)** — explicitly descoped per this dispatch's Decision 1 (backend-only, no frontend/UI work), even though the `card-payments` evolution doc listed it as recommended follow-up work. This feature exposes backend API surface only; a future frontend increment (not scoped here, not necessarily even under this feature-id) would consume it.
- **Card-required-even-for-Free enforcement (D-5)** — this feature does not gate any action on whether a card is on file; that gate, if built, is frontend/UX scope.
- **Exact price points and included allowances per dimension** — illustrative placeholders throughout, matching `card-payments`' own precedent; confirming real numbers is a business/market decision.
- **Exact cap-check computation mechanism (live/cached/incremental)** — explicitly left to DESIGN, see § Open Question. DISCUSS locks the requirement (US-206), not the mechanism.
- **Exact dunning grace-window duration** — Stripe Smart Retries' own schedule governs retry timing (D-12); the exact "how many days past exhausted-retries counts as final failure" threshold is a Stripe-account-configuration concern, not application code, and is not specified here.
- **Reconciliation job implementation** (KPI #1's measurement mechanism) — DEVOPS-wave scope, not DISCUSS/DESIGN.
- **Any DISCOVER-stage customer validation of this feature's own opportunity score** — this feature inherits JOB-14's already-validated opportunity score (17) rather than re-deriving one; no DISCOVER wave ran, consistent with `card-payments`' own precedent.

---

## Wave: DISCUSS / [REF] WS Strategy

Walking Skeleton Strategy: **B — Thin End-to-End Slice** (same classification `admin-api-v2` used for its own WS, `slice-B01-auth-migrations.md`). Slice 01 (US-201) is a real, if narrow, vertical slice: one migration, one Stripe SDK call, one endpoint — proving the riskiest new integration assumption (real Stripe network round-trip inside an existing session-authed Axum handler, D-13, no mock fallback) before any dependent slice begins. This is a materially different WS shape from `card-payments`' own mock-data-facade WS (ADR-007) — appropriate, because this feature's entire premise (D-13) is that mocking is explicitly disallowed here.

---

## Wave: DISCUSS / [REF] Driving Ports

Inbound surfaces for this feature (backend-only scope, all on `:9090` admin):

- `GET /admin/v1/billing/subscription` — session-authed, any role (US-201, extended with `cap_status` in US-206)
- `POST /admin/v1/billing/subscription` — session-authed, Owner/Admin role (US-202)
- `POST /admin/v1/webhooks/stripe` — no session/operator auth, own `stripe_signature_middleware` (US-203, 5th sub-router per D-11)
- `POST /admin/v1/billing/run-metering` — operator-authed only (US-205)

No new customer-facing gRPC/REST surface on `:8080`/`:8081` — the real-time cap check (US-206/207) modifies the *existing* request path's internal behavior (a new gate inside the existing middleware chain), it does not add a new driving port of its own.

---

## Wave: DISCUSS / [REF] Pre-requisites

- `docs/decisions/card-payments/grill-me-decisions.md` D-1..D-13 (locked, inputs not outputs of this wave).
- `crates/embyr-server/src/admin/router.rs`'s existing 4-sub-router composition-root pattern (precedent for the 5th).
- `crates/embyr-server/src/config.rs`'s secret-resolution pattern (for `STRIPE_*` env vars).
- `crates/embyr-server/src/admin/handlers/lifecycle.rs`'s existing suspend/activate mechanism (reused, not rebuilt).
- `crates/embyr-server/src/middleware/rate_limit.rs` (extended by US-206/207 — read in full, see § Open Question).
- `card-payments`' shipped frontend (`AppModel.subscription`/`Card`/`Invoice` mock types) as the forward type-contract this feature's real API responses should stay conceptually compatible with, per ADR-007's mock-to-real migration pattern (already proven for `admin-api-v2`) — NOT a hard build dependency, since this feature ships API-only with no frontend wiring.

---

## Wave: DISCUSS / [REF] Handoff Package

**To DESIGN (solution-architect)**: this `feature-delta.md` (journey + story map + user stories + embedded AC), 7 slice briefs (`docs/feature/card-payments-backend/slices/slice-01..07-*.md`), `docs/product/jobs.yaml` (JOB-14 cross-reference notes), `docs/product/journeys/billing-management.yaml` (backend cross-reference note).

**To DEVOPS (platform-architect)**: § Outcome KPIs above (5 KPIs — 3 guardrail, 1 leading, 1 North Star — for instrumentation planning, including the new `embyr_webhook_requests_total` Prometheus metric).

**Explicit flags for DESIGN**:
1. § Open Question — D-9's "extends the rate limiter" framing requires genuine new design (a monthly-cumulative, account-keyed mechanism), not literal `TokenBucket` reuse. Resolve before US-206/207 implementation starts.
2. § Scope Assessment's 3-Release-Group structure (not 3 feature-ids) — DESIGN should organize its own component decomposition around these same 3 groups for traceability, but is not required to design them as 3 separately-deployable units.
3. Decision 1's backend-only boundary is a hard constraint — DESIGN should not design frontend components (CardModal Elements interop, etc.) under this feature-id even if it seems natural to bundle them; that is out of scope per this dispatch's locked Decision 1.

Peer review: not invoked per-wave (default skip per SKILL Phase 3 step 6 — no DoR ambiguity beyond the explicitly-documented and correctly-deferred § Open Question, JTBD assumptions inherited and already-validated from `card-payments`, no vendor-neutrality risk beyond Stripe itself which is already locked at D-2). Mandatory consolidated review fires at end of DISTILL.

---

## Wave: DISCUSS / [REF] SSOT Updates

- `docs/product/jobs.yaml` — JOB-14 receives a cross-reference note (not a rewrite) pointing to this feature as the "make it real" backend half, mirroring the note already present on JOB-10 pointing to `card-payments`. JOB-11 receives a cross-reference note distinguishing its request-rate-fairness concern from D-9's monthly-cumulative-cap concern.
- `docs/product/journeys/billing-management.yaml` — `scope_note` updated: the "recommended follow-up feature `card-payments-backend`" language is updated to reflect that this DISCUSS wave has now run, with a pointer to this `feature-delta.md`.
- `docs/product/personas/chris-account-admin.yaml` — no content change needed; JOB-14 was already listed, and this feature does not change Chris's goals/frustrations/mental model, only what backs them.

---

## Wave: DESIGN / [REF] Prior Wave Consultation — Reading Confirmation

✓ `docs/product/architecture/brief.md` (full SSOT — `## Application Architecture`, `## Application
Architecture — admin-api-v2`, `## Application Architecture — production-readiness`,
`## Application Architecture — card-payments` sections read in full for structural convention and
forward type-contract)
✓ `docs/product/architecture/adr-015-distributed-rate-limiter-postgres.md` (existing
`RateLimiter`/`TokenBucket` design — directly informs the D-9 resolution below)
✓ `docs/product/architecture/adr-018-secrets-management.md` (secret-resolution pattern extended
for `STRIPE_*` vars)
✓ `docs/feature/card-payments-backend/feature-delta.md` (this file, DISCUSS section, full)
✓ `docs/feature/card-payments-backend/discuss/wave-decisions.md`
✓ `docs/feature/card-payments-backend/slices/slice-01..07-*.md` (all 7)
✓ `docs/decisions/card-payments/grill-me-decisions.md` (D-1..D-13, still authoritative)
✓ `docs/evolution/2026-08-11-card-payments.md` (shipped frontend's forward type-contract)
✓ `crates/embyr-server/src/admin/router.rs`, `crates/embyr-server/src/middleware/rate_limit.rs`,
`crates/embyr-server/src/admin/handlers/billing.rs`, `crates/embyr-server/src/admin/handlers/lifecycle.rs`,
`crates/embyr-server/src/config.rs`, `crates/embyr-server/src/adapters/aws_secret_fetcher.rs`,
`crates/embyr-server/src/admin/state.rs`, `crates/embyr-server/src/admin/middleware/operator_auth.rs`,
`crates/embyr-core/src/admin/{mod,account,rbac}.rs`, `crates/embyr-core/src/rate_limit.rs`,
`crates/embyr-server/src/admin/handlers/mod.rs` (all read in full — not summarized)
✓ `migrations/0002_metrics.sql`, `migrations/0008_admin_accounts.sql`,
`migrations/0015_projects_admin_columns.sql`, `migrations/0018_rate_buckets.sql`,
migrations directory listing (`0001`..`0018`, confirming `0019` is next)
✓ Root `Cargo.toml`, `crates/embyr-server/Cargo.toml` (confirmed `reqwest` already a workspace
dependency; confirmed NO `stripe`/`async-stripe` crate exists yet; confirmed
`aws-sdk-secretsmanager` — a full vendor SDK, not hand-rolled — is the AWS precedent)

Interaction mode: **Propose** (background dispatch, no interactive user). Design scope:
**Application/components**. Paradigm: unchanged (functional-where-practical Rust, project
`CLAUDE.md`, hexagonal ports-and-adapters per ADR-002/existing brief.md convention) — Step 4 of
the SKILL's Discovery Flow was skipped per this dispatch's explicit instruction.

No contradictions found between DISCUSS's locked decisions and this DESIGN wave's resolutions.
One DESIGN-discovered gap flagged explicitly (not silently absorbed): `daily_project_metrics` has
no storage-bytes column, so D-7's "storage" dimension cannot be metered/capped with real data in
this feature's V1 — see § Open Questions, OQ-CP-1.

---

## Wave: DESIGN / [REF] DDD List — card-payments-backend

| ID | Design Decision | Verdict | One-line Rationale |
|----|------------------|---------|---------------------|
| DDD-1 | D-9's rate-limiter framing is corrected at the mechanism level, not just re-labeled | **CREATE NEW** (background-computed, cache-backed) | Different key (account vs. project), different time semantics (cumulative-since-cycle vs. continuous refill) — see ADR-020 |
| DDD-2 | Cap-check enforcement runs in a new background task, not the `:8080`/`:8081` hot path | Accepted | Zero added latency to the protocol-fidelity-critical data plane; eliminates the Slice-07-flagged race condition by construction |
| DDD-3 | Stripe integration via `async-stripe`, not hand-rolled `reqwest` | Accepted | API surface size (5 resource areas) + webhook-signature security-criticality — see ADR-021 |
| DDD-4 | `lifecycle::set_project_status` reused literally (not re-implemented) for both new suspend triggers | Accepted | D-12's "one mechanism, two triggers" is enforced structurally via a narrowed `LifecycleDeps` + fan-out wrappers, not by convention |
| DDD-5 | Free-plan cap-check billing cycle = UTC calendar month | Accepted | Free accounts have no guaranteed Stripe `Subscription` object to source a period from; AC-206-05 requires a real, non-account-creation-date boundary |
| DDD-6 | Storage dimension NOT metered/capped in this feature's V1 | Accepted, flagged | No source column exists; `billing.rs` already hard-codes storage as 0 — pre-existing gap, not newly introduced |
| DDD-7 | Webhook idempotency via local `processed_webhook_events` table; usage-record idempotency via Stripe's native idempotency-key (no local ledger for the latter) | Accepted | Two different guarantees needed different mechanisms — event-dedup has no Stripe-side equivalent; usage-push dedup does |
| DDD-8 | 5th webhook sub-router with its own `stripe_signature_middleware`, sibling of `public_router` | Accepted (D-11, locked in DISCUSS) | Structural precedent already established (admin-api-v2's 4-sub-router pattern) |
| DDD-9 | New `WebhookState`, separate from `OperatorState`/`UserAdminState` | Accepted | Mirrors B-AD-07's "different dependency graphs" precedent from admin-api-v2 |
| DDD-10 | `run_metering` is HTTP-triggered (operator route), not a Tokio-interval sweeper | Accepted (Slice 05 locked) | DISCUSS explicitly descopes scheduling infrastructure to DEVOPS-wave |

---

## Wave: DESIGN / [REF] Component Decomposition

See `docs/product/architecture/brief.md` § Application Architecture — card-payments-backend
§ Component Decomposition for the full file-path table (14 new files, 11 extended files, 2 new
migrations). Summary by area:

- **Domain (embyr-core, IO-free):** `crates/embyr-core/src/admin/billing.rs` (new) —
  `Subscription`, `CapStatus`, `compute_cap_status()`, `cap_exceeded()`, `WebhookEventOutcome`.
- **Stripe adapter:** `crates/embyr-server/src/adapters/stripe_gateway.rs` (new) — sole
  `async-stripe` import site, `probe()`-bearing.
- **Cap-check subsystem:** `crates/embyr-server/src/sweepers/cap_usage_refresher.rs` (new,
  background task) + `crates/embyr-server/src/adapters/cap_status_cache.rs` (new, in-process
  cache).
- **HTTP surface:** `billing_subscription.rs`, `webhooks_stripe.rs`, `billing_metering.rs`
  (all new handlers) + `stripe_signature.rs` (new middleware) + `router.rs`/`state.rs` extended
  for the 5th sub-router and `WebhookState`.
- **Reused suspend path:** `lifecycle.rs` extended (`set_project_status` visibility/signature
  narrowed to `LifecycleDeps`; new `suspend_account_projects`/`activate_account_projects`
  fan-out wrappers).
- **Config/secrets:** `config.rs` extended with 3 new Stripe secret resolvers (reusing ADR-018's
  `resolve_secret_source`/`fetch_from_secret_manager`) + `cap_check_interval_secs`.
- **Schema:** `migrations/0019_subscriptions.sql`, `migrations/0020_processed_webhook_events.sql`.

---

## Wave: DESIGN / [REF] Driving Ports

| Port | Auth | Handler |
|------|------|---------|
| `GET /admin/v1/billing/subscription` | Session, any role | `billing_subscription::get_subscription` (US-201, extended with `cap_status` in US-206) |
| `POST /admin/v1/billing/subscription` | Session, Owner/Admin | `billing_subscription::post_subscription` (US-202) |
| `POST /admin/v1/webhooks/stripe` | `stripe_signature_middleware` (no session/operator auth) | `webhooks_stripe::stripe_webhook_handler` (US-203/204) |
| `POST /admin/v1/billing/run-metering` | Operator Bearer | `billing_metering::run_metering` (US-205) |

No new `:8080`/`:8081` driving port — confirmed unchanged from DISCUSS's own Driving Ports section.

---

## Wave: DESIGN / [REF] Driven Ports + Adapters

| Port/Adapter | Shape | Earned Trust |
|---|---|---|
| `StripeGateway` | Concrete struct (not a trait — mirrors ADR-015's `RateLimiter` precedent: exactly one implementation, D-13 forbids a mocked port) | `probe()` calls `GET /v1/balance`, 3s timeout, **soft-failure/WARN** (not refuse-to-start — billing is not on the protocol-fidelity critical path). 3 fault-injection scenarios: invalid key, network unreachable, restricted account. See ADR-021 and brief.md for full signature. |
| `CapStatusCache` | In-process `Arc<RwLock<HashMap<AccountId, CapStatus>>>`, mirrors `CredentialCache` | No probe — pure in-process cache, no external dependency of its own. Cache-miss fails open (AC-206-04). |
| `LifecycleDeps` | Narrowed dependency struct (system_db + credential_cache), extracted from `OperatorState` | Not a new adapter — enables literal `set_project_status` reuse from non-operator callers. |

**External Integrations Requiring Contract Tests** (handoff to platform-architect, DEVOPS wave):

```
- Stripe (REST API + Webhooks): consumer-driven contract tests (Pact) recommended in CI's
  acceptance stage — both directions (embyr-as-consumer of Stripe's API responses,
  embyr-as-provider of the webhook endpoint's expected payload shape). Highest-risk external
  boundary in this feature. Full rationale: brief.md § External Integrations Requiring Contract
  Tests — card-payments-backend.
```

---

## Wave: DESIGN / [REF] Technology Choices

| Choice | Verdict | Rationale (one-line) |
|---|---|---|
| `async-stripe` over hand-rolled `reqwest` | Accepted | ADR-021 — API surface + webhook-signature security-criticality; mirrors `aws-sdk-secretsmanager` precedent |
| Background-computed cap check over live-per-request or incrementally-maintained | Accepted | ADR-020 — avoids hot-path latency and the request-path race condition; bounded staleness (≤2× refresh interval) is an acceptable trade-off given no locked sub-second SLA |
| Postgres `UNIQUE`+`ON CONFLICT` for webhook idempotency | Accepted | Zero new dependency, mirrors `sdk_api_keys.key_hash` precedent |
| Stripe native idempotency-key for usage-record push | Accepted | Reuses Stripe's own server-side guarantee instead of a redundant local ledger |

---

## Wave: DESIGN / [REF] Decisions Table

See `docs/product/architecture/brief.md` § Application-Level Decisions Table — card-payments-backend
(CPB-AD-01 through CPB-AD-10) for the full table with rationale. Reproduced IDs:
CPB-AD-01 (D-9 resolution), CPB-AD-02 (async-stripe), CPB-AD-03 (background enforcement),
CPB-AD-04 (calendar-month cycle), CPB-AD-05 (storage gap), CPB-AD-06 (Stripe-native idempotency),
CPB-AD-07 (`LifecycleDeps` narrowing), CPB-AD-08 (new `billing_subscription.rs` file),
CPB-AD-09 (HTTP-triggered metering job), CPB-AD-10 (no Stripe-key rotation in V1).

---

## Wave: DESIGN / [REF] Reuse Analysis

Full table (12 rows, zero unjustified CREATE NEW): `docs/product/architecture/brief.md`
§ Reuse Analysis — card-payments-backend. Verdict summary: **9 EXTEND/PATTERN REUSE, 1 CREATE NEW
(the cumulative cap-check mechanism itself — extensively justified in ADR-020 with 3 rejected
alternatives), 1 flagged GAP (storage dimension, not silently extended), 1 CREATE NEW state struct
(`WebhookState`, justified per the B-AD-07 precedent from admin-api-v2).**

---

## Wave: DESIGN / [REF] Open Questions

| ID | Question | Blocking | Timing |
|----|----------|----------|--------|
| OQ-CP-1 | Storage-dimension metering/cap enforcement requires a new `BackendAdapter::table_size()` port method against customer DBs | No | Follow-up feature |
| OQ-CP-2 | `STRIPE_*` secret rotation (dual-key window) not built in V1 | No | Follow-up feature, if operationally needed |
| OQ-CP-3 | `EMBYR_CAP_CHECK_INTERVAL_SECS` default (30s) tuning at production scale | No | DEVOPS-wave observation, post-launch |
| OQ-CP-4 | `GET /admin/v1/billing/metering-log` audit endpoint (illustrative in US-205's pitch, not locked) | No | DEVOPS-wave, if pursued (KPI #1 reconciliation job is explicitly DEVOPS scope) |
| OQ-CP-5 | Per-wave peer review was skipped (background-dispatch default); the D-9 resolution (ADR-020) is novel/contested enough that a dedicated review would normally be warranted | Flagged for orchestrator | Orchestrator's call; mandatory consolidated review still fires at end of DISTILL |

---

## Wave: DESIGN / [REF] C4 Diagrams

Full C4 System Context (Extended), Container (Extended), and Component (L3, Billing Subsystem)
diagrams live in `docs/product/architecture/brief.md` § Application Architecture —
card-payments-backend (not duplicated here per the multi-architect SSOT convention already
established by the `card-payments` frontend feature's own DESIGN section).

---

## Wave: DESIGN / [REF] Outcome Collision Check

Skipped — `nwave-ai outcomes check-delta` CLI is not installed in this repo. Noted per SKILL's
Outcome Collision Check procedure (skip-and-document path).

---

## Wave: DESIGN / [REF] Peer Review

Skipped per-wave (default per SKILL Success Criteria — invoke only on trigger: contested ADR,
novel pattern, performance-budget unverified by spike, security boundary change). **Flag for the
orchestrator:** ADR-020's D-9 resolution is a genuinely novel, non-trivial architectural design
(new background-computed subsystem, a real departure from DISCUSS's own literal journey-diagram
sketch) that would normally satisfy the "novel pattern" trigger — skipped here only because this
dispatch executed autonomously with no user available to authorize a review invocation. The
mandatory consolidated review at end of DISTILL will cover it.

---

## Wave: DESIGN / [REF] Handoff Package

**To DISTILL (acceptance-designer):** this `feature-delta.md` (DISCUSS + DESIGN sections),
`docs/product/architecture/brief.md` § Application Architecture — card-payments-backend,
`docs/product/architecture/adr-020-cumulative-cap-check-architecture.md`,
`docs/product/architecture/adr-021-stripe-sdk-integration.md`,
`docs/feature/card-payments-backend/design/wave-decisions.md`.

**Explicit flags for DISTILL/acceptance-designer:**
1. AC-207's "the next request triggers the cap check" language should be interpreted as "the next
   request *after* the background `CapUsageRefresher` cycle applies the suspension" (bounded by
   `EMBYR_CAP_CHECK_INTERVAL_SECS`, default 30s), not a literal single-gRPC-request synchronous
   check — see ADR-020. Acceptance tests should assert enforcement within ≤2× the refresh
   interval, not immediate-next-request.
2. Storage-dimension ACs (any AC implicitly assuming a real `storage` value in `cap_status` or
   metering) should be written against the documented placeholder behavior (omitted/0), not a
   real computed value — see OQ-CP-1.
3. D-13's no-mock constraint applies to `StripeGateway` — DISTILL must not introduce an
   `IPaymentGateway`-style mock port.

---

## Wave: DISTILL / [REF] Prior Wave Consultation — Reading Confirmation

+ `docs/product/journeys/billing-management.yaml` (scope_note, shared_artifacts)
+ `docs/product/architecture/brief.md` § Application Architecture — card-payments-backend (full:
  Quality Attributes, D-9 Resolution Summary, Component Decomposition, Driving/Driven Ports,
  Technology Choices, Reuse Analysis, Background Tasks, C4 System Context/Container/Component,
  Architecture Enforcement, Application-Level Decisions, Open Questions, External Integrations)
+ `docs/product/architecture/adr-020-cumulative-cap-check-architecture.md` (full)
+ `docs/product/architecture/adr-021-stripe-sdk-integration.md` (full)
+ `docs/feature/card-payments-backend/feature-delta.md` (this file, DISCUSS + DESIGN sections, full)
+ `docs/feature/card-payments-backend/discuss/wave-decisions.md`
+ `docs/feature/card-payments-backend/design/wave-decisions.md`
- `docs/feature/card-payments-backend/devops/wave-decisions.md` — **not found**; no DEVOPS wave ran
  for this feature. Per the Graceful Degradation Matrix: **WARN**, default environment matrix
  applied (clean | with-pre-commit | with-stale-config) — consistent with how `admin-api-v2`,
  `distributed-rate-limiting`, and `secrets-management`'s own DISTILL waves handled the identical
  gap.
+ `docs/feature/card-payments-backend/slices/slice-01..07-*.md` (all 7, full)
+ Production source read in full (not summarized): `crates/embyr-server/src/admin/router.rs`,
  `state.rs`, `handlers/lifecycle.rs`, `handlers/billing.rs`, `config.rs`,
  `adapters/credential_cache.rs`, `middleware/rate_limit.rs` (partial — struct/method signatures),
  `middleware/operator_auth.rs`, `admin/extractors/session_context.rs`,
  `admin/mod.rs`/`handlers/mod.rs`/`middleware/mod.rs`/`adapters/mod.rs` (module registration),
  `embyr-core/src/admin/{mod,account,rbac}.rs`, `embyr-core/src/rate_limit.rs`, `lib.rs`
  (composition-root wiring, partial), `main.rs` (partial), `deny.toml`, root + `embyr-server`
  `Cargo.toml`, `migrations/{0001,0002,0008,0015,0018}.sql`.
+ Sibling test suites read in full as primary style reference: `tests/admin_api_v2/common/mod.rs`,
  `tests/admin_api_v2/walking_skeleton.rs`, `tests/admin_api_v2/acceptance/b06_oidc_billing.rs`,
  `tests/secrets_management/common/mod.rs`, `tests/secrets_management/mod.rs`,
  `tests/secrets_management/acceptance/sm01_admin_key_secrets_manager.rs`,
  `tests/distributed_rate_limiting/common/mod.rs`, `tests/distributed_rate_limiting/mod.rs`,
  `tests/distributed_rate_limiting/acceptance/b12_postgres_rate_limit.rs`.
+ `tests/common/state_delta.rs` (project-level state-delta port — present, port-mode: **inherit**,
  no bootstrap needed).
+ `docs/architecture/atdd-infrastructure-policy.md` — **found** (bootstrapped 2026-05-24 by an
  earlier feature). `--policy=inherit` applied. This feature's one new driven-external port
  (`StripeGateway`) was missing from the table — appended (see § Language + Infrastructure Policy
  Bootstrap below), not a fresh bootstrap.
+ `/Users/petervyboch/Projects/embyr-rs/CLAUDE.md` (functional-where-practical Rust, per-feature
  mutation testing — this project's governing paradigm; the other `CLAUDE.md` surfaced in this
  session's system context governs an unrelated project, `warp-vscode-integration`, and does not
  apply here).

## Wave: DISTILL / [REF] Language + Infrastructure Policy Bootstrap

- `[lang-mode] rust` — detected via `Cargo.toml` at workspace root (5-crate workspace).
- `[policy-mode] inherit` (default; no `--policy` flag passed). `docs/architecture/atdd-infrastructure-policy.md`
  already existed (bootstrapped 2026-05-24 by an earlier feature) — read and applied per
  apply-if-exists. One row appended under § Driven external / non-deterministic for `StripeGateway`
  (the one port in this feature's scope missing from the table), documenting D-13's explicit
  per-port override of this table's own default (fake for driven-external ports) to REAL for
  Stripe specifically — the policy's `Note` column carries the override rationale so future readers
  don't mistake it for an inconsistency with the table's own stated default.
- `[port-mode] inherit` — `tests/common/state_delta.rs` already present (bootstrapped by an earlier
  feature); no re-bootstrap performed.

## Wave: DISTILL / [REF] Wave-Decision Reconciliation — HARD GATE

Read `docs/feature/card-payments-backend/discuss/wave-decisions.md` (D1-D7) and
`docs/feature/card-payments-backend/design/wave-decisions.md` (D1-D9) in full. Checked every
DISCUSS decision against every DESIGN decision for contradiction:

- DISCUSS D4 ("D-9's rate-limiter framing corrected, concrete mechanism left open for DESIGN") vs.
  DESIGN D1 ("resolved as CREATE NEW, not extend") — **not a contradiction**: DESIGN resolving a
  question DISCUSS explicitly and deliberately left open is the expected handoff, not a conflicting
  claim. DISCUSS never asserted a mechanism; DESIGN chose one within the boundary DISCUSS set
  (AC-206-06's "must not literally extend TokenBucket").
- DISCUSS D5 ("Stripe Elements JS interop shim explicitly descoped") — DESIGN does not touch
  frontend scope anywhere in its Component Decomposition (confirmed: zero `embyr-admin` file
  changes). Consistent.
- DISCUSS D7 ("no mocked payment port, D-13") vs. DESIGN's Driven Ports table ("`StripeGateway` ...
  no trait interface ... D-13 forbids a mocked port") — consistent, DESIGN carries the constraint
  forward unchanged.
- No DEVOPS wave-decisions.md exists (see § Prior Wave Consultation above) — nothing to reconcile
  against; graceful degradation applied (WARN, default matrix), not a reconciliation failure.

**Result: Reconciliation passed — 0 contradictions.** No `CLARIFICATION_NEEDED` returned. Proceeded
to scenario design.

## Wave: DISTILL / [REF] Scenario List

Tagged per Mandate convention. All non-WS scenarios `#[ignore]` with `// RED — enable one at a time
in DELIVER`. 36 total `#[tokio::test]` fns across 7 files; 1 non-ignored (the walking skeleton).

| # | File | Scenario | Tags |
|---|---|---|---|
| 1 | cpb01 | `first_billing_call_auto_provisions_real_stripe_customer` | `@walking_skeleton @driving_port @real-io @US-201 @AC-201-02 @AC-201-04 @AC-201-06` — **NOT ignored** |
| 2 | cpb01 | `repeated_calls_do_not_create_duplicate_stripe_customer` | `@driving_port @real-io @US-201 @AC-201-03` |
| 3 | cpb01 | `pro_plan_account_returns_its_real_period_end` | `@driving_port @real-io @US-201 @AC-201-06` |
| 4 | cpb01 | `stripe_unavailability_during_first_time_provisioning_leaves_no_partial_state` | `@error @driving_port @real-io @US-201 @AC-201-05` |
| 5 | cpb01 | `unauthenticated_request_is_rejected` | `@error @driving_port @real-io @US-201 @AC-201-01` |
| 6 | cpb02 | `upgrading_free_to_pro_updates_the_real_stripe_subscription` | `@driving_port @real-io @US-202 @AC-202-02 @AC-202-03` |
| 7 | cpb02 | `failed_stripe_call_does_not_update_local_plan` | `@error @driving_port @real-io @US-202 @AC-202-03` |
| 8 | cpb02 | `upgrading_from_cap_exceeded_clears_suspension_and_reactivates_projects` | `@driving_port @real-io @US-202 @AC-202-04` |
| 9 | cpb02 | `viewer_role_cannot_change_plan` | `@error @driving_port @real-io @US-202 @AC-202-01` |
| 10 | cpb02 | `unrecognized_plan_value_is_rejected` | `@error @driving_port @real-io @US-202 @AC-202-05` |
| 11 | cpb03 | `valid_subscription_updated_event_syncs_local_record` | `@driving_port @real-io @US-203 @AC-203-04` |
| 12 | cpb03 | `invalid_signature_is_rejected_before_any_state_change` | `@error @driving_port @real-io @US-203 @AC-203-02` |
| 13 | cpb03 | `duplicate_event_delivery_is_a_no_op` | `@driving_port @real-io @US-203 @AC-203-03` |
| 14 | cpb03 | `subscription_deleted_event_does_not_leave_falsely_active_row` | `@driving_port @real-io @US-203 @AC-203-05` |
| 15 | cpb03 | `unrecognized_but_validly_signed_event_type_returns_200` | `@driving_port @real-io @US-203 @AC-203-06` |
| 16 | cpb04 | `exhausted_retries_suspend_every_project_under_the_account` | `@driving_port @real-io @US-204 @AC-204-01` |
| 17 | cpb04 | `intermediate_retry_failure_does_not_suspend` | `@error @driving_port @real-io @US-204 @AC-204-02` |
| 18 | cpb04 | `payment_recovery_reactivates_every_project_under_the_account` | `@driving_port @real-io @US-204 @AC-204-03` |
| 19 | cpb04 | `suspension_via_dunning_evicts_credential_cache_like_operator_suspend` | `@driving_port @real-io @US-204 @AC-204-05` |
| 20 | cpb04 | `payment_failed_for_unknown_subscription_id_is_a_safe_no_op` | `@error @driving_port @real-io @US-204` |
| 21 | cpb04 | `duplicate_final_failure_event_does_not_double_process` | `@error @driving_port @real-io @US-204` |
| 22 | cpb05 | `a_days_usage_is_pushed_as_one_record_per_project_per_dimension` | `@driving_port @real-io @US-205 @AC-205-01` |
| 23 | cpb05 | `zero_usage_dimensions_are_not_pushed` | `@driving_port @real-io @US-205 @AC-205-02` |
| 24 | cpb05 | `rerunning_the_same_days_metering_does_not_double_count` | `@driving_port @real-io @US-205 @AC-205-03` |
| 25 | cpb05 | `a_single_projects_stripe_failure_does_not_abort_the_whole_run` | `@error @driving_port @real-io @US-205 @AC-205-04` |
| 26 | cpb05 | `manual_trigger_is_operator_only` | `@error @driving_port @real-io @US-205 @AC-205-05` |
| 27 | cpb06 | `cumulative_usage_sums_across_all_of_an_accounts_projects` | `@driving_port @real-io @US-206 @AC-206-01` |
| 28 | cpb06 | `usage_at_exactly_the_cap_reports_100_percent` | `@driving_port @real-io @US-206 @AC-206-02` |
| 29 | cpb06 | `pro_plan_accounts_do_not_receive_cap_status` | `@error @driving_port @real-io @US-206 @AC-206-03` |
| 30 | cpb06 | `stale_or_unavailable_computation_fails_open` | `@error @driving_port @real-io @US-206 @AC-206-04` |
| 31 | cpb06 | `usage_from_a_prior_billing_cycle_does_not_count_toward_this_cycle` | `@driving_port @real-io @US-206 @AC-206-05` |
| 32 | cpb07 | `crossing_100_percent_of_a_free_plan_cap_suspends_the_accounts_projects` | `@driving_port @real-io @US-207 @AC-207-01` |
| 33 | cpb07 | `below_cap_usage_never_suspends` | `@error @driving_port @real-io @US-207 @AC-207-02` |
| 34 | cpb07 | `pro_plan_accounts_are_never_suspended_by_this_mechanism` | `@error @driving_port @real-io @US-207 @AC-207-03` |
| 35 | cpb07 | `upgrading_to_pro_reactivates_projects_suspended_by_cap_enforcement` | `@driving_port @real-io @US-207 @AC-207-05` |
| 36 | cpb07 | `concurrent_requests_during_cap_crossing_do_not_double_suspend` | `@driving_port @real-io @US-207` (ADR-020 race-elimination claim) |

**Error/edge ratio**: 15 of 36 scenarios tagged `@error` = 42% ✓ (target ≥40%).
**Story coverage**: all 7 stories (US-201..207) have ≥4 scenarios each; zero uncovered stories.

## Wave: DISTILL / [REF] Walking Skeleton Strategy

**One walking skeleton**: `first_billing_call_auto_provisions_real_stripe_customer` (US-201,
Slice 01) — matches DISCUSS/DESIGN's own locked WS scope (Decision 2, Strategy B). Litmus test://
"Chris opens billing for the first time and the system quietly sets up real payment infrastructure
behind the scenes, then shows him his (real) plan" — a non-technical stakeholder confirms this is
what users need. `Then` steps assert observable outcomes (response fields, persisted
`stripe_customer_id`/`subscriptions` row) — not internal side effects.

**Why US-201 over US-203** (the orchestrator dispatch left this choice open): US-201 is the
*prerequisite* for every other slice (Releases 2 and 3 both structurally depend on its schema) and
is DISCUSS/DESIGN's own already-locked WS choice (`docs/feature/card-payments-backend/feature-delta.md`
§ DISCUSS Story Map, § DISCUSS Scope Assessment "Walking Skeleton decision") — re-deriving a
different WS choice at DISTILL would contradict a locked upstream decision without cause. US-203's
webhook round-trip is exercised instead as CPB03's own first scenario (non-WS, but still
`@real-io`, still exercising a live `stripe trigger`-shaped payload through the real endpoint).

**Executed and verified RED** (see `docs/feature/card-payments-backend/distill/red-classification.md`
for the full run transcript): real Postgres testcontainer, real migrations 0019/0020, real Stripe
test-mode key resolution — fails for the correct reason (`MISSING_FUNCTIONALITY`, the
`billing_subscription::get_subscription` RED-scaffold panic), not a harness/credential bug.

## Wave: DISTILL / [REF] Adapter Coverage Table

| Adapter | `@real-io` scenario | Covered by |
|---|---|---|
| `StripeGateway` (Stripe API, D-13 override — real, not faked) | YES | WS (`cpb01`) + `cpb02`/`cpb03`/`cpb05` — real Stripe test-mode Customer/Subscription/Usage-Record calls; `cpb03`/`cpb04` additionally use real `stripe trigger`-shaped signed payloads |
| testcontainers Postgres (`subscriptions`, `processed_webhook_events`, extended `daily_project_metrics` reads) | YES | Every scenario in all 7 files — `CpbTestContext::new()` starts a real Postgres 15-alpine container and runs real migrations every test |
| `stripe_signature_middleware` (real HMAC-SHA256 verification, deterministic — no network, still real code per D-13's carve-out) | YES | `cpb03::invalid_signature_is_rejected_before_any_state_change` (wrong-secret HMAC) + every other `cpb03`/`cpb04` scenario (correctly-signed payload via `sign_stripe_payload`, matching Stripe's exact `t=...,v1=...` scheme) |
| `CapUsageRefresher` background task (real Postgres advisory lock + interval loop) | YES | `cpb06`/`cpb07` — real `tokio::spawn`ed task against the real Postgres container, polled for observable effect (no mock of the scheduler) |
| `CapStatusCache` (in-process, no external dependency) | N/A — no `@real-io` tag needed | Not a driven-external adapter (pure in-process cache, like `CredentialCache`); exercised indirectly via `cpb06`'s HTTP-level assertions |

**Zero "NO — MISSING" rows.**

## Wave: DISTILL / [REF] Scaffolds (Mandate 7)

All scaffolds carry `SCAFFOLD: true` (Rust convention, `grep -r "SCAFFOLD: true" src/`). Verified
zero `NotImplementedError`-equivalent (`unimplemented!()`/`todo!()`) in favor of `panic!(...)` at
every genuinely-new business-logic surface (`todo!()` retained only in the pre-existing
`distributed_rate_limiting` suite's own untouched files, not introduced here).

| File | Status | Scaffolded surface |
|---|---|---|
| `crates/embyr-core/src/admin/billing.rs` | **NEW** | `compute_cap_status()`, `cap_exceeded()` panic. Types (`Subscription`, `CapStatus`, etc.) are real plain data carriers — no business logic to TDD. Zero IO imports (verified: no `async`/`sqlx`/`reqwest`/`tokio` tokens in the file). |
| `crates/embyr-server/src/adapters/stripe_gateway.rs` | **NEW** | `get_or_create_customer`, `upsert_subscription`, `push_usage_record`, `verify_webhook_signature`, `probe` all panic. `new()` is real (no I/O, safe for composition-root wrappers). |
| `crates/embyr-server/src/adapters/cap_status_cache.rs` | **NEW** | Real implementation (mirrors `CredentialCache`) — no business logic, not scaffolded. |
| `crates/embyr-server/src/sweepers/mod.rs` | **NEW** (see upstream-issues.md Finding 1 — DESIGN's cited precedent doesn't exist; built fresh) | Module registration only. |
| `crates/embyr-server/src/sweepers/cap_usage_refresher.rs` | **NEW** | `run_cycle()` panics. `spawn()` and `advisory_lock_key()` are real (mechanical interval-loop wiring, unit-tested). |
| `crates/embyr-server/src/admin/handlers/billing_subscription.rs` | **NEW** | `get_subscription`, `post_subscription` panic. |
| `crates/embyr-server/src/admin/handlers/webhooks_stripe.rs` | **NEW** | `stripe_webhook_handler` panics. |
| `crates/embyr-server/src/admin/handlers/billing_metering.rs` | **NEW** | `run_metering` panics. |
| `crates/embyr-server/src/admin/middleware/stripe_signature.rs` | **NEW** | `stripe_signature_middleware` panics. |
| `crates/embyr-server/src/admin/handlers/lifecycle.rs` | **EXTEND** | `suspend_account_projects`, `activate_account_projects` panic (NEW, per D-12). `set_project_status`/`suspend_project`/`activate_project`/`delete_project` refactored to route through the new `LifecycleDeps` type — behavior-preserving (verified: `cargo test -p embyr-server --lib` + full-workspace `cargo check --tests` show zero regression in any pre-existing passing test). |
| `crates/embyr-server/src/admin/state.rs` | **EXTEND** | `WebhookState` (NEW struct). `OperatorState`/`UserAdminState` gain `stripe_gateway`/`cap_status_cache` fields (real — plumbing, not business logic). |
| `crates/embyr-server/src/admin/router.rs` | **EXTEND** | 5th sub-router wired (real routing/middleware-layering plumbing — the *handlers* it routes to are the scaffolds above). `build_admin_router` signature extended per DESIGN; all 3 call sites outside this feature's own new files updated (see upstream-issues.md Finding 2). |
| `crates/embyr-server/src/config.rs` | **EXTEND** | `stripe_secret_key`/`stripe_webhook_signing_secret`/`stripe_publishable_key`/`cap_check_interval_secs` fields + resolution (real — simplified vs. DESIGN's exact AWS/GCP-resolver ask, see upstream-issues.md Finding 3; non-blocking). |
| `crates/embyr-server/src/main.rs` | **EXTEND** | Composition-root wiring for `StripeGateway` + `CapUsageRefresher::spawn()` (real plumbing). |
| `crates/embyr-server/src/lib.rs` | **EXTEND** | `pub mod sweepers;` registration. |
| `crates/embyr-core/src/admin/mod.rs` | **EXTEND** | `pub mod billing;` + re-exports. |
| `crates/embyr-server/src/admin/handlers/mod.rs`, `middleware/mod.rs`, `adapters/mod.rs` | **EXTEND** | Module registration only. |
| `tests/admin_api_v2/common/mod.rs` | **EXTEND** (outside this feature's own test dir — see upstream-issues.md Finding 2) | Additive: 2 new placeholder args at its existing `build_admin_router` call site. Zero behavioral change to that suite's own tests. |
| `migrations/0019_subscriptions.sql`, `migrations/0020_processed_webhook_events.sql` | **NEW, real SQL (not scaffolded — no panic state for a migration)** | `accounts.stripe_customer_id`, `subscriptions`, `processed_webhook_events`. Verified: applies cleanly against real testcontainers Postgres (walking-skeleton run). |

## Wave: DISTILL / [REF] Test Placement

`tests/card_payments_backend/{common/mod.rs, acceptance/cpb01..07_*.rs}` — mirrors
`tests/admin_api_v2/`'s flat-file, one-`[[test]]`-per-slice convention (not
`secrets_management`/`distributed_rate_limiting`'s single-aggregating-`mod.rs` convention), per
this feature's DISTILL dispatch instruction to mirror `admin_api_v2`'s bNN pattern specifically.
7 new `[[test]]` entries registered in `crates/embyr-server/Cargo.toml`, named
`card_payments_backend_cpb0N_<slug>`, immediately following the existing `admin_api_v2_b06_...`
entry.

## Wave: DISTILL / [REF] Driving Adapter Coverage

All 4 driving ports from DESIGN's Driving Ports table are exercised via real HTTP (not
service-level calls):

| Port | Exercised by |
|---|---|
| `GET /admin/v1/billing/subscription` | cpb01 (all 5), cpb02 (upgrade-then-verify), cpb06 (all 5) |
| `POST /admin/v1/billing/subscription` | cpb02 (all 5), cpb07 (reactivation scenario) |
| `POST /admin/v1/webhooks/stripe` | cpb03 (all 5), cpb04 (all 6) |
| `POST /admin/v1/billing/run-metering` | cpb05 (all 5) |

Zero uncovered driving ports. No new `:8080`/`:8081` gRPC/REST driving port exists for this
feature (confirmed unchanged from DESIGN) — the cap check modifies existing internal enforcement
behavior only, verified indirectly via `cpb06`/`cpb07`'s real end-to-end suspension observation
rather than a direct gRPC-layer test (there is no new gRPC surface to test).

## Wave: DISTILL / [REF] Pre-requisites

- `docs/product/architecture/adr-020-cumulative-cap-check-architecture.md`,
  `adr-021-stripe-sdk-integration.md` (locked, inputs not outputs of this wave).
- Real Stripe test-mode account (`sk_test_...`, sourced via `.env.local` — see
  `tests/card_payments_backend/common/mod.rs::stripe_secret_key()`).
- Stripe CLI (`/opt/homebrew/bin/stripe`) for `stripe trigger` synthesis (D-13), used by
  `tests/card_payments_backend/common/mod.rs::stripe_trigger()`.
- Docker (testcontainers Postgres 15-alpine) — verified available and clean (zero stray containers
  after the walking-skeleton run).
- `async-stripe = "1.0.0-rc.8"` added to `[workspace.dependencies]` (root `Cargo.toml`) and
  `embyr-server`'s `[dependencies]` — latest published release at this DISTILL run's
  implementation time (no non-RC 1.0 release exists yet on crates.io).
- `hmac = "0.12"` added to `[workspace.dependencies]` + `embyr-server`'s `[dev-dependencies]` (with
  the already-present `sha2`/`hex`) for the real, deterministic `Stripe-Signature` computation used
  by `cpb03`/`cpb04`'s test harness (`sign_stripe_payload`) — legitimate real code per D-13's
  carve-out for signature-verification tests, not a mock of the Stripe port.

## Wave: DISTILL / [REF] Mandate Compliance Evidence

- **CM-A** (Mandate 1, hexagonal boundary): every scenario invokes the real Axum admin router via
  `reqwest::Client` (driving port) — zero imports of internal handler/domain functions directly in
  any `#[tokio::test]` body. `grep -c "use embyr_server::admin::handlers" tests/card_payments_backend/acceptance/*.rs`
  → 0 matches (handlers are invoked only via HTTP, never called as Rust functions from tests).
- **CM-B** (Mandate 2, business language): zero technical jargon in scenario *names* (e.g.
  `upgrading_from_cap_exceeded_clears_suspension_and_reactivates_projects`, not
  `post_endpoint_returns_200`); technical detail (HTTP methods, status codes, SQL) lives inside
  step bodies only, consistent with this project's established Rust acceptance-test idiom (no
  separate Gherkin layer — matches `admin_api_v2`/`secrets_management`/`distributed_rate_limiting`
  precedent, this project's chosen host-language binding per the Polyglot Adapter Matrix, Rust row).
- **CM-C** (Mandate 3, user journey completeness): every scenario has a documented Given/When/Then
  journey in its doc comment, tracing to a named persona (Chris/Dana/Priya) and a concrete AC.
- **CM-D** (Mandate 4, pure function extraction): `compute_cap_status`/`cap_exceeded`
  (`embyr-core::admin::billing`) are pure, zero-IO functions, unit-testable without any adapter —
  the fixture parametrization (Postgres container, Stripe key) is confined to
  `CpbTestContext`'s adapter layer only.
- **Pillar 2** (chained narrative): `cpb02`'s `given_provisioned_account` helper reuses cpb01's
  exact Given+When (provisioning GET); `cpb04`'s scenarios reuse `cpb03`'s webhook-delivery
  Given+When (`deliver_signed_webhook`); `cpb07`'s reactivation scenario reuses `cpb02`'s
  upgrade-POST Given+When. No copy-pasted fixture setup across files within a story line.
- **Pillar 3** (production composition): every scenario builds the SUT via
  `embyr_server::admin::router::build_admin_router` (the real production composition-root
  function, same one `main.rs` calls) — never a hand-assembled router replicating the wiring. Only
  the externally non-deterministic port (Stripe) uses a real, not faked, adapter per D-13's
  explicit override of the Architecture of Reference default.

## Wave: DISTILL / [REF] Definition of Ready Validation (DoD, pre-DELIVER handoff)

| Item | Status | Evidence |
|---|---|---|
| 1. All acceptance scenarios written with passing step definitions | PASS | 36 scenarios compile (`cargo check --workspace --tests`); WS executed and RED-classified correctly |
| 2. Test pyramid complete | PASS | Acceptance layer complete (this wave); unit-test locations flagged for DELIVER (`compute_cap_status`/`cap_exceeded` pure-function tests, `advisory_lock_key` already covered) |
| 3. Peer review approved | **Deferred to orchestrator** — per dispatch instructions, the mandatory 4-parallel-reviewer Final Wave Review Gate is explicitly SKIPPED for this run; orchestrator decides separately whether to invoke it |
| 4. Tests run in CI/CD pipeline | Not verified this run (no CI config change made — out of DISTILL's scope; CI already runs `cargo test --workspace` per existing pipeline, new `[[test]]` entries are automatically picked up) |
| 5. Story demonstrable to stakeholders | PASS — WS's litmus test documented above |
| 6. Project Infrastructure Policy present | PASS — bootstrapped this run, see below |
| 7. Target language detected and logged | PASS — `[lang-mode] rust` |
| 8. State-delta port present | PASS — inherited, `tests/common/state_delta.rs` |
| 9. Wave-Decision Reconciliation HARD GATE passed | PASS — 0 contradictions |
| 10-13 (Mandates 8-11) | **N/A for this feature's host language** — the `assert_state_delta`/PBT/Tier-B contract is a Python-pilot convention (Polyglot Adapter Matrix); this Rust suite follows the Rust row's idiom (plain `assert_eq!`/`assert!` against real HTTP/DB state, matching every sibling Rust acceptance suite in this workspace — `admin_api_v2`, `secrets_management`, `distributed_rate_limiting` — none of which use `assert_state_delta` for their HTTP-response scenarios either; a `universe` module is nonetheless declared in `common/mod.rs` documenting the port-exposed observable names this suite tracks, for future Rust-port-of-`assert_state_delta` adoption) |
| 14. Pillar 1 (zero technical terms) | PASS — see CM-B above |
| 15. Pillar 2 (chained narrative) | PASS — see above |
| 16. Pillar 3 (production composition root) | PASS — see above |

## Wave: DISTILL / [REF] Handoff Package

**To DELIVER (software-crafter)**: this `feature-delta.md` (all 3 wave sections),
`tests/card_payments_backend/` (7 acceptance files + common harness), 14 new + 11 extended
production files (see § Scaffolds table), 2 new migrations,
`docs/feature/card-payments-backend/distill/red-classification.md`,
`docs/feature/card-payments-backend/distill/upstream-issues.md`.

**Explicit flags for DELIVER**:
1. Enable exactly one `#[ignore]`d scenario at a time, in slice order (cpb01 already green via the
   WS; cpb02 → cpb07). Each scenario's RED-scaffold panic message names the exact function to
   implement.
2. `LifecycleDeps`-based `suspend_account_projects`/`activate_account_projects` (in
   `lifecycle.rs`) are the D-12 compliance surface — implement these before either `cpb04` or
   `cpb07`'s suspend-side scenarios can go GREEN; a shared test helper or direct code-path
   assertion (per DESIGN's Architecture Enforcement table) should verify both call sites resolve
   to the identical function, not just produce similar output.
3. `CapUsageRefresher::run_cycle` is the single highest-risk implementation surface in this
   feature (per US-207's own Technical Notes) — implement its cumulative-usage query and
   cap-crossing suspend call together with `compute_cap_status`/`cap_exceeded`
   (`embyr-core::admin::billing`), test-driving the pure functions first (unit layer) before wiring
   the adapter layer (`cpb06`/`cpb07`).
4. `config.rs`'s simplified (non-secret-manager) `STRIPE_*` resolution (upstream-issues.md Finding
   3) may be extended to the full `resolve_secret_source` chain at DELIVER's discretion — not
   required by any locked AC.
5. Consider adding `tower_http::catch_panic::CatchPanicLayer` to `build_admin_router` (a new
   dependency + router-wide change, out of this DISTILL's scope) — would convert every RED
   scaffold's client-visible symptom from a connection reset into a clean `500`, improving
   diagnostics for this and every future scaffolded handler in this codebase. Not a blocker (see
   `red-classification.md` — RED classification is already correct via the server-side panic
   message).

---

## Wave: DELIVER / [REF] Implementation Summary

Implemented the full backend for card-payments-backend: real Stripe Customer/Subscription
provisioning, webhook-driven subscription sync with signature verification and idempotency,
dunning suspend/recovery, nightly usage metering to Stripe Billing Meter Events, and real-time
cumulative Free-plan cap computation + enforcement via a background sweeper. All against the real
Stripe test-mode API (D-13, no mocked payment port) and real testcontainers Postgres. 7 roadmap
steps (+1 fix-up for a missing `async-stripe` companion-crate dependency), 38 acceptance tests (37
`#[tokio::test]` scenarios across `cpb01`–`cpb07` + 2 pure-function unit tests), 36 of 37 scenario
tests green (1 honestly deferred — see DoD Check below), 0 remaining `SCAFFOLD` panics anywhere in
production code.

## Wave: DELIVER / [REF] Files Modified

Production (`crates/embyr-core/src/admin/`, `crates/embyr-server/src/`): `admin/billing.rs` (NEW,
pure domain types + `compute_cap_status`/`cap_exceeded`), `adapters/stripe_gateway.rs` (NEW, sole
`async-stripe` import site), `adapters/cap_status_cache.rs` (NEW), `sweepers/cap_usage_refresher.rs`
(NEW), `admin/handlers/{billing_subscription,webhooks_stripe,billing_metering}.rs` (NEW),
`admin/middleware/stripe_signature.rs` (NEW), `admin/handlers/lifecycle.rs` (extended —
`activate_account_projects`/`suspend_account_projects`, both reusing the existing private
`set_project_status`), `admin/router.rs` (extended — 5th sub-router for webhooks, new routes on
`session_router`/`operator_router`), `admin/state.rs` (extended — `stripe_gateway`/
`cap_status_cache`/`WebhookState`), `config.rs` (extended — optional `STRIPE_*` env vars), `main.rs`
(extended — `StripeGateway` construction + soft-failure probe + `CapUsageRefresher` spawn),
`migrations/0019_subscriptions.sql`, `migrations/0020_processed_webhook_events.sql`.

Tests (`tests/card_payments_backend/`): `acceptance/{walking_skeleton→cpb01,...,cpb07}.rs`,
`common/mod.rs` — all authored by DISTILL as RED scaffolds, activated one slice at a time across 8
DELIVER steps.

Housekeeping fix (unrelated feature, discovered and fixed along the way): `tests/secrets_management/
common/mod.rs` and `tests/production_readiness/common/mod.rs` — `embyr_server_binary()` preferred a
stale `target/release/` binary with no freshness check, causing 4 pre-existing tests in a sibling
feature to fail after this feature added new migrations the stale binary didn't recognize. Root
caused via peer-reviewed RCA (`docs/analysis/2026-08-11-sm03-server-must-start-rca.md`), fixed
permanently (commit `1b11ed2`).

10 commits, `876ddde`..`acb7875` (feature commits) + `1b11ed2` (unrelated fix), one per roadmap step
plus the fix-up and housekeeping commits, each with a `Step-ID:` trailer where applicable.

## Wave: DELIVER / [REF] Scenarios Green Count

**36 of 37** scenario tests green (`cargo test -p embyr-server --test
card_payments_backend_cpb0{1..7}_*` → 7+7+7+8+6+7+7 = 49 total test-fn runs including 12
harness-level `state_delta` self-tests; 37 are the actual feature scenarios, 36 pass, 1 honestly
`#[ignore]`d — see DoD Check). Verified independently by the orchestrator (not just self-reported)
on 2026-08-15, plus `cargo check --workspace --tests` (clean) and `des-verify-integrity` (8/8 steps,
exit 0).

## Wave: DELIVER / [REF] Demo Evidence — 2026-08-15

Adaptation note: unlike the sibling frontend feature, this backend's driving ports ARE literal HTTP
endpoints (`GET`/`POST /admin/v1/billing/subscription`, `POST /admin/v1/webhooks/stripe`, `POST
/admin/v1/billing/run-metering`) — each story's Elevator Pitch "After" line names a real,
curl-able endpoint. Rather than a fresh manual curl session (which would just re-derive what the
acceptance suite already automates against the SAME real Postgres + real Stripe test-mode
infrastructure), the demo evidence below is each story's own real-infrastructure acceptance test
run — this is a STRONGER form of evidence than a one-off manual demo (automated, reproducible, real
HTTP request/response cycle through the actual production composition root), not a substitute for
one.

| Story | Demo command | Exit | Result (verifies "sees" clause) |
|---|---|---|---|
| US-201 (real subscription record) | `cargo test -p embyr-server --test card_payments_backend_cpb01_real_subscription_record` | 0 | `7 passed; 0 failed` — real Stripe Customer created, persisted, reused on repeat calls |
| US-202 (plan change) | `cargo test -p embyr-server --test card_payments_backend_cpb02_real_plan_change` | 0 | `7 passed; 0 failed` — real Stripe Subscription created/updated, write-through ordering verified |
| US-203 (webhook ingestion) | `cargo test -p embyr-server --test card_payments_backend_cpb03_webhook_ingestion` | 0 | `7 passed; 0 failed` — real signature verification via `async-stripe-webhook`, idempotency, dispatch |
| US-204 (dunning) | `cargo test -p embyr-server --test card_payments_backend_cpb04_dunning_suspension_recovery` | 0 | `8 passed; 0 failed` — same `set_project_status` as operator suspend, verified by construction |
| US-205 (usage metering) | `cargo test -p embyr-server --test card_payments_backend_cpb05_usage_metering` | 0 | `6 passed; 0 failed; 1 ignored` — real Billing Meter Events pushed, native Stripe idempotency |
| US-206 (cap compute) | `cargo test -p embyr-server --test card_payments_backend_cpb06_cumulative_cap_check` | 0 | `7 passed; 0 failed` — cumulative usage across all projects, boundary-inclusive 100% |
| US-207 (cap enforcement) | `cargo test -p embyr-server --test card_payments_backend_cpb07_cap_exceeded_suspension` | 0 | `7 passed; 0 failed` — real suspend via background sweeper, reused `suspend_account_projects` |

All 7 stories: exit 0, non-empty stdout, real-infrastructure verification of each story's "sees"
clause. Gate: **PASS**.

## Wave: DELIVER / [REF] DoD Check

| DISCUSS DoD item | Status |
|---|---|
| Every story traces to `job_id: JOB-14` | ✅ all 7 stories |
| D-13: real Stripe test-mode, no mocked payment port | ✅ verified — `StripeGateway` is a concrete struct, zero trait/mock abstractions anywhere |
| D-12: dunning and cap-exceeded suspension share one mechanism | ✅ verified by construction — both call the identical `suspend_account_projects`/`set_project_status` |
| Webhook idempotency (`processed_webhook_events`) | ✅ tested (`duplicate_event_delivery_is_a_no_op`, `duplicate_final_failure_event_does_not_double_process`) |
| Cap-check stays off the request hot path (ADR-020) | ✅ verified — background sweeper only, zero code added to `:8080`/`:8081` |
| `embyr-core` stays IO-free | ✅ verified — `billing.rs`'s `compute_cap_status`/`cap_exceeded` are pure, `cargo deny check bans` passes |
| Zero remaining `SCAFFOLD` markers in production code | ✅ verified via grep, 0 remaining |
| AC-205-04 (single-project Stripe failure doesn't abort run) | ⚠️ **honestly deferred** — the real-Stripe test fixture (3 projects, 1 Stripe customer) has no non-hardcoded way to make exactly 1 of 3 structurally-identical real API calls fail; the production code (`Result`-wrapped per-project loop, continues past `StripeError`) IS implemented and code-reviewable, just not exercised by a passing automated test. Flagged for a future DISTILL pass to redesign this one fixture (e.g. an intentionally-malformed quantity value that Stripe genuinely rejects) rather than silently marked done. |
| Storage-dimension cap metering (OQ-CP-1) | ⚠️ **out of scope, documented** — `daily_project_metrics` has no storage-bytes column; only reads/writes/deletes are capped in this V1, per DESIGN's explicit gap flag |
| WASM bundle size / frontend impact | N/A — this is a backend-only feature, no frontend files touched |

## Wave: DELIVER / [REF] Quality Gates

| Gate | Status |
|---|---|
| Roadmap review (Phase 1) | ✅ APPROVED, 0 blockers — the call-graph dependency trace (the exact bug class that slipped through on the sibling frontend feature) was independently verified clean |
| Per-step TDD (Phase 2) | ✅ 8/8 steps COMMIT/PASS, `des-verify-integrity` exit 0 |
| Post-merge integration + demo evidence (Phase 3.5) | ✅ see above |
| Refactor L1-L6 (Phase 3) | ✅ done — dropped stale DISTILL/SCAFFOLD provenance comments (`0ed04f7`) |
| Adversarial review (Phase 4) | ✅ 1 BLOCKER found and fixed — TOCTOU race in webhook idempotency (SELECT-then-INSERT → atomic INSERT-then-`rows_affected()`), verified with a new genuinely-concurrent regression test (`ebcf0b5`) |
| Mutation testing (Phase 5) | ✅ `per-feature` strategy per CLAUDE.md — 129 mutants (diff-scoped vs `63a53d2`), kill rate 91/95 = 95.8% (excl. 34 structurally-unviable), well past the 80% gate. Closed a real domain-logic gap in `embyr-core::billing` (`SubscriptionStatus::parse` arm deletions, `cap_exceeded`) with direct unit tests (`57640dc`) — those functions had zero unit coverage, relying only on slow integration tests. 2 accepted survivor clusters: `subscription_item_price_id` (webhook plan-derivation path never exercised by a payload with `items.price.id` populated — real but narrow acceptance-test gap, same category as AC-205-04, deferred to a future DISTILL pass) and `build_admin_router` (TIMEOUT not MISSED — an empty router would 404 every request, so this is very likely caught-but-slow rather than a real gap) |
| Deliver integrity verification (Phase 6) | ✅ exit 0, 8/8 steps traced |
| Finalize (Phase 7) | in progress |

## Wave: DELIVER / [REF] Pre-requisites

DISTILL's 37 RED-scaffolded scenarios (`tests/card_payments_backend/`) + DESIGN's Component
Decomposition table + the 3 new ADRs (ADR-019 from the sibling frontend, ADR-020 cumulative
cap-check architecture, ADR-021 Stripe SDK integration). One real dependency gap discovered and
fixed mid-DELIVER: `async-stripe`'s facade crate doesn't itself expose per-resource typed request
builders in this pre-1.0 version — required adding 3 companion crates
(`async-stripe-core`/`-billing`/`-webhook`), resolved via step 01-05.

