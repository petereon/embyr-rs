# Evolution: card-payments-backend

**Date:** 2026-08-16
**Feature:** Backend half of card-payments — real Stripe Customer/Subscription provisioning, webhook-driven subscription sync, dunning suspend/recovery, nightly usage metering to Stripe Billing Meter Events, and real-time cumulative Free-plan cap enforcement (background sweeper + cache, never on the hot request path)
**Job:** JOB-14 (`manage-subscription`) — this feature's "make it real" half, exactly as `admin-api-v2` was JOB-10's "make it real" half of `user-admin-ui`
**ADRs:** ADR-020 (`docs/product/architecture/adr-020-cumulative-cap-check-architecture.md`), ADR-021 (`docs/product/architecture/adr-021-stripe-sdk-integration.md`)

## Business Context

The sibling frontend feature (`card-payments`, shipped 2026-08-11, see
`docs/evolution/2026-08-11-card-payments.md`) built the billing UI entirely against mock data —
`AppModel.subscription`/`Card`/`Invoice` had no backing Stripe object anywhere. This feature makes
that UI real: Chris Okafor's (P5, Account Admin) plan, payment status, and suspension state are now
backed by a real Stripe test-mode subscription and real webhook events, Dana's eventual invoice
reflects real itemized overage, and D-6's Free-plan hard-stop (no auto-upgrade) is enforced for the
first time instead of only being displayed from a mock-derived state.

13 architecture decisions were locked pre-DISCUSS via `/grill-me`
(`docs/decisions/card-payments/grill-me-decisions.md`, D-1..D-13) and shared unmodified between
`card-payments` and this feature. DISCUSS scoped 7 user stories (US-201..207) across 3 internal
Release Groups — deliberately **not** split into further feature-ids (unlike the
`card-payments`→`card-payments-backend` split itself), because the bounded-context and
walking-skeleton-integration-point oversized signals stayed under threshold and all 3 groups share
one migration-numbering sequence and the same locked decision set.

## Key Decisions

### D-1..D-13 (pre-DISCUSS, `grill-me-decisions.md`)

| # | Decision |
|---|---|
| D-1 | Hybrid billing: flat base subscription + metered overage |
| D-2 | Stripe as the payment processor |
| D-3 | Stripe Elements card capture — explicitly descoped from this feature (frontend-only, deferred) |
| D-4 | Two plan tiers only: Free + Pro |
| D-5 | Card required even for Free (out of scope this feature — card capture is frontend) |
| D-6 | Free-cap-exceeded = hard-stop/suspend, no auto-upgrade — made real for the first time by US-206/207 |
| D-7 | All 4 usage dimensions (reads/writes/deletes/storage) metered separately |
| D-8 | Daily batch job pushes usage to Stripe, reading `daily_project_metrics` |
| D-9 | Real-time Free-cap enforcement "extends the existing rate limiter" — **DISCUSS flagged this framing as needing correction** (different key: account not project; different time semantics: cumulative-since-cycle-start not continuously-refilling); DESIGN resolved it via ADR-020, not literal token-bucket reuse |
| D-10 | New `subscriptions` / `accounts.stripe_customer_id` / `processed_webhook_events` schema |
| D-11 | 5th webhook sub-router on `:9090`, sibling of `public_router` |
| D-12 | Stripe Smart Retries → suspend, same code path as Free-cap-exceeded suspension (one suspension mechanism, two triggers) |
| D-13 | Real Stripe test-mode API + Stripe CLI, no mocked payment port |

### DESIGN-wave decisions (ADR-020, ADR-021)

- **ADR-020 — Cumulative Free-plan cap check is a background-computed cache, not a rate-limiter
  extension or a live-per-request check.** Resolves D-9's open question. A `CapUsageRefresher`
  sweeper runs on an interval (`pg_try_advisory_lock`-guarded, single-writer-per-cycle), computes
  per-account cumulative usage across all of the account's projects, writes to a `CapStatusCache`,
  and only that background cycle calls `suspend_account_projects` on cap-exceeded. Rejected
  alternatives: (1) live per-request computation — adds a Postgres round-trip to the hot path,
  violating the existing 20ms-fail-open latency precedent; (2) incrementally-maintained running
  counter — another piece of mutable state that can drift from `daily_project_metrics`'s ground
  truth; (3) synchronous per-request suspend as DISCUSS's own journey diagram literally sketched —
  rejected as the enforcement mechanism (would need a request-path race guard), partially reused for
  the read side (the cache is still read on the request path, just never computed there). Bounded
  staleness (≤2× refresh interval) accepted as a trade-off given no locked sub-second SLA. Storage
  dimension explicitly flagged as a known gap, not silently absorbed (OQ-CP-1).
- **ADR-021 — Stripe integration via `async-stripe`, not hand-rolled `reqwest`.** Chosen for API
  surface coverage and, more importantly, webhook-signature-verification security-criticality —
  hand-rolling HMAC verification for a payment webhook is exactly the kind of security-sensitive
  code an audited SDK should own. Mirrors the existing `aws-sdk-secretsmanager` precedent of
  preferring a vendor SDK over a hand-rolled client for security-adjacent integrations.
- Postgres `UNIQUE` + `ON CONFLICT` for webhook idempotency (zero new dependency, mirrors
  `sdk_api_keys.key_hash` precedent); Stripe's own native idempotency-key parameter for usage-record
  pushes (reuses Stripe's server-side guarantee instead of a redundant local ledger, CPB-AD-06).

## Steps Completed

All 8 roadmap steps (`docs/feature/card-payments-backend/deliver/execution-log.json`) show complete
`PREPARE → RED_ACCEPTANCE → GREEN → COMMIT` traces (`des-verify-integrity` → exit 0, 8/8 steps
traced).

| Step | Name | Status |
|---|---|---|
| 01-01 | Slice 01 (US-201, Walking Skeleton) — real Stripe Customer provisioning + `GET /admin/v1/billing/subscription` | PASS |
| 01-02 | Slice 02 (US-202) — plan-change persistence + first implementation of `activate_account_projects` | PASS |
| 01-03 | Slice 03 (US-203) — webhook signature verification + subscription-sync dispatch | PASS |
| 01-04 | Slice 04 (US-204) — dunning suspend/recovery + first implementation of `suspend_account_projects` | PASS |
| 01-05 | Fix-up — migrate `StripeGateway` to typed `async-stripe-core` builders (ADR-021 compliance) | PASS (see Notable Findings — mid-task incident) |
| 02-01 | Slice 05 (US-205) — nightly usage metering to Stripe Billing Meter Events | PASS |
| 03-01 | Slice 06 (US-206) — cumulative cap-status compute + cache write (read side only) | PASS |
| 03-02 | Slice 07 (US-207) — cap-exceeded enforcement, extends `run_cycle` with reused `suspend_account_projects` | PASS |

10 commits total (`876ddde`..`acb7875` feature commits, plus `1b11ed2` for the unrelated
stale-binary fix), one per roadmap step plus the fix-up and housekeeping commits, each with a
`Step-ID:` trailer where applicable.

## Scenarios: 36/37 green, 1 honestly deferred

`cargo test -p embyr-server --test card_payments_backend_cpb0{1..7}_*` → 49 total test-fn runs (37
feature scenarios + 12 harness-level self-tests), 36 pass, 1 `#[ignore]`d (see Known Deferred /
Incomplete Items). Verified independently by the orchestrator on 2026-08-15, plus
`cargo check --workspace --tests` (clean) and `des-verify-integrity` (8/8 steps, exit 0).

## Quality Gates

- **Roadmap review (Phase 1):** APPROVED, 0 blockers. This time the call-graph dependency trace
  (the exact bug class that slipped through unreviewed on the sibling `card-payments` roadmap — a
  backward method-call dependency between two steps) was independently verified clean before
  dispatch — done up front rather than caught after the fact.
- **Per-step TDD (Phase 2):** 8/8 steps COMMIT/PASS, `des-verify-integrity` exit 0.
- **Post-merge integration + demo evidence (Phase 3.5):** each of the 7 stories' own real-Stripe,
  real-Postgres acceptance-test run stands as demo evidence (each story's Elevator Pitch "After"
  line names a real, curl-able HTTP endpoint) — all exit 0, all PASS.
- **Refactor L1-L6 (Phase 3):** dropped stale DISTILL/SCAFFOLD provenance comments (`0ed04f7`).
- **Adversarial review (Phase 4):** 1 BLOCKER found and fixed — see Notable Findings below.
- **Mutation testing (Phase 5, `per-feature`):** 129 mutants (diff-scoped vs `63a53d2`), kill rate
  91/95 = 95.8% (excluding 34 structurally-unviable), well past the 80% gate. See Lessons Learned
  below for the domain-logic gap it surfaced and closed.
- **Deliver integrity verification (Phase 6):** exit 0, 8/8 steps traced.

## Notable Findings

### Phase 4 — TOCTOU race in webhook idempotency (BLOCKER, fixed)

Adversarial review found the webhook handler's idempotency check used a SELECT-then-INSERT pattern:
two concurrent redeliveries of the same Stripe event could both observe "not yet processed" before
either had inserted its row, double-dispatching the event (e.g. double-suspending, double-crediting
an activation). Fixed (`ebcf0b5`) by replacing it with an atomic `INSERT ... ON CONFLICT DO NOTHING`
gated on `rows_affected() == 1`, using the `processed_webhook_events` primary key as the sole atomic
gate. Because the final DB state is identical whether one or two requests actually dispatch, a
regression test needed a genuinely concurrent trigger and an independent signal (a Prometheus
counter at the dispatch entry point) rather than asserting on row state, which added — this mirrors
how the sibling frontend feature's own evolution doc documented its Phase-4 catch (`usage.rs`/
`invoices.rs` rendering placeholder stubs despite correct model data): in both features, Phase 4
adversarial review earned its cost by catching one concrete, high-value defect that earlier
self-reported gates had missed.

### `async-stripe` companion-crate dependency gap (discovered mid-DELIVER, step 01-05)

Step 01-01 discovered that the pinned `async-stripe` 1.0.0-rc.8 **facade crate does not itself
expose per-resource typed request builders** (e.g. `Customer::create`) — this is not obvious from
the crate name alone. The typed builders live in separate companion crates
(`async-stripe-core`, `async-stripe-billing`, `async-stripe-webhook`), which had to be added to
`Cargo.toml` as their own dependencies. Step 01-01 shipped with a hand-rolled `reqwest` stopgap for
the two functions affected (explicitly flagged as a deviation in its own commit); step 01-05
migrated them to the real typed builders once the companion crates were confirmed to compile,
restoring full ADR-021 compliance.

### Stale `target/release` binary regression (infra, not this feature's business logic)

While card-payments-backend's own steps 876ddde/ac9f9b5/b3679f6 added migrations `0019`/`0020`, 4
tests in the unrelated `secrets_management` suite began failing with a 30-second "server must
start" timeout. Root-caused (`docs/analysis/2026-08-11-sm03-server-must-start-rca.md`): the test
harness's `embyr_server_binary()` helper preferred `target/release/embyr-server` whenever it merely
existed on disk, with no freshness check — and a release binary built once for an earlier, unrelated
purpose (most plausibly `production-readiness`'s Dockerfile/CI validation) had been sitting stale in
the shared `target/` directory, predating this feature's new migrations. The stale binary's own
`sqlx` migrator rejected the newer, already-migrated DB ("migration 19 was previously applied but is
missing in the resolved migrations") and exited in ~300ms — but `wait_for_healthy()` had no
process-liveness check, so a fast, already-logged, self-explanatory error was misread as a 30-second
hang. Fixed by flipping the binary-preference order to prefer the cargo-guaranteed-fresh
`target/debug/` binary (commit `1b11ed2`). Not specific to this feature's Stripe/billing logic, but
worth carrying forward as an infra lesson (see below) since the failure mode — a stale build
artifact outside version control, invisible to diff review — is a systemic property of this
workspace's independently-copied test-harness pattern, not a one-off typo.

## Known Deferred / Incomplete Items

- **AC-205-04** (a single project's Stripe failure does not abort the whole metering run) is
  honestly deferred: the production code (`Result`-wrapped per-project loop that continues past a
  `StripeError`) IS implemented and code-reviewable, but the real-Stripe test fixture (3 projects
  sharing 1 Stripe customer) has no non-hardcoded way to make exactly 1 of 3 structurally-identical
  real API calls fail — empirically confirmed 0/3 failures across live runs. Re-`#[ignore]`d rather
  than silently marked done. Flagged for a future DISTILL pass to redesign the fixture (e.g. an
  intentionally-malformed quantity value Stripe genuinely rejects).
- **Storage-dimension cap metering (OQ-CP-1)** is out of scope, documented: `daily_project_metrics`
  has no storage-bytes column, so only reads/writes/deletes are capped/metered in this V1. Follow-up
  requires a new `BackendAdapter::table_size()` port method against customer DBs.
- `STRIPE_*` secret rotation (dual-key window, OQ-CP-2) not built in V1 — follow-up if operationally
  needed.
- `EMBYR_CAP_CHECK_INTERVAL_SECS` default (30s) tuning at production scale (OQ-CP-3) — a DEVOPS-wave
  post-launch observation, not a DESIGN-wave decision.
- `GET /admin/v1/billing/metering-log` audit endpoint (OQ-CP-4) — illustrative in US-205's pitch,
  not locked; pursue in DEVOPS wave if wanted.
- Per-wave DESIGN peer review was skipped (background-dispatch default) despite ADR-020's D-9
  resolution being novel/contested enough to normally warrant one — flagged for the orchestrator;
  the mandatory consolidated review at end of DISTILL covered it instead.

## Lessons Learned

1. **Mutation testing caught a real domain-logic gap that integration tests were masking.** Phase 5
   surfaced 4 surviving mutants in `embyr-core::billing`'s pure functions
   (`SubscriptionStatus::parse` arm deletions for active/past_due/canceled, and `cap_exceeded`
   always returning `true`) — these functions had **zero direct unit tests**, relying entirely on
   the much slower, less precise integration-test suite to exercise them indirectly. Direct unit
   tests closed the gap (`57640dc`), confirmed by a scoped mutant re-run. The general lesson:
   integration-test-only coverage of pure domain functions can pass every acceptance scenario while
   leaving individual logic branches structurally unverified — mutation testing is what actually
   catches this, not code review or scenario count.
2. **A dependency-graph assumption ("the facade crate is the whole SDK") cost a stopgap
   implementation and a follow-up fix-up step.** `async-stripe`'s pre-1.0 crate layout splits typed
   request builders into companion crates not implied by the facade crate's own name — worth
   checking a crate's actual module/feature layout before committing to an SDK-adoption ADR, not
   just its top-level package description.
3. **A stale, out-of-band build artifact can silently break tests in an unrelated feature, and diff
   review will never find it.** The `target/release/embyr-server` regression was caused by a binary
   built for a completely different purpose weeks earlier, sitting in a shared `target/` directory
   with zero freshness check — invisible to every source diff this session reviewed, because the
   cause wasn't in any diff. Worth treating "prefer the build-tool-guaranteed-fresh artifact by
   default" as a standing convention for any test harness that spawns a prebuilt binary, not just a
   one-off fix for this workspace's 2 affected harness copies.
4. **Roadmap-review rigor on the dependency/call-graph trace paid off proactively this time.** The
   sibling `card-payments` feature's roadmap review missed a backward method-call dependency between
   two steps, caught only afterward by the orchestrator reading actual test files. This feature's
   roadmap review explicitly ran the same call-graph trace *before* dispatch and found it clean —
   turning a reactive catch into a preventive one.

## Key Files

- `crates/embyr-core/src/admin/billing.rs` — pure domain types + `compute_cap_status`/`cap_exceeded`
  (NO IO, enforced by `deny.toml`)
- `crates/embyr-server/src/adapters/stripe_gateway.rs` — sole `async-stripe` import site (ACL)
- `crates/embyr-server/src/adapters/cap_status_cache.rs`
- `crates/embyr-server/src/sweepers/cap_usage_refresher.rs` — ADR-020's background sweeper
- `crates/embyr-server/src/admin/handlers/{billing_subscription,webhooks_stripe,billing_metering}.rs`
- `crates/embyr-server/src/admin/middleware/stripe_signature.rs`
- `crates/embyr-server/src/admin/handlers/lifecycle.rs` — `activate_account_projects`/
  `suspend_account_projects`, both reusing the existing private `set_project_status` (D-12)
- `crates/embyr-server/src/admin/router.rs` — 5th sub-router (webhooks), new session/operator routes
- `migrations/0019_subscriptions.sql`, `migrations/0020_processed_webhook_events.sql`
- `docs/product/architecture/adr-020-cumulative-cap-check-architecture.md`
- `docs/product/architecture/adr-021-stripe-sdk-integration.md`
- `docs/product/architecture/brief.md` § Application Architecture — card-payments-backend
- `docs/decisions/card-payments/grill-me-decisions.md` — D-1..D-13, shared with `card-payments`
- `docs/analysis/2026-08-11-sm03-server-must-start-rca.md` — stale-binary RCA
- `tests/card_payments_backend/` — 37 acceptance scenarios (`cpb01`–`cpb07`) + shared harness
- `docs/feature/card-payments-backend/feature-delta.md` — full DISCUSS+DESIGN+DISTILL+DELIVER
  narrative (retained in place, not migrated)

## Follow-Up Work

- Redesign the AC-205-04 test fixture so a single-project Stripe failure can be exercised without
  real-infrastructure hardcoding (e.g. an intentionally-malformed quantity value).
- Storage-dimension cap metering (OQ-CP-1) — needs a new `BackendAdapter::table_size()` port method.
- `STRIPE_*` secret rotation / dual-key window (OQ-CP-2), if operationally needed.
- `EMBYR_CAP_CHECK_INTERVAL_SECS` tuning at production scale (OQ-CP-3) — DEVOPS-wave observation.
- `GET /admin/v1/billing/metering-log` audit endpoint (OQ-CP-4), if pursued.
- Outcome KPI event instrumentation for this feature's DISCUSS-defined KPIs — DEVOPS-wave scope.
- Consolidate the currently-duplicated `ServerProcess`/`embyr_server_binary()` test-harness copies
  into one shared module, so the stale-binary-preference fix (and the missing process-liveness
  check in `wait_for_healthy()`, RCA Root Cause B, not yet fixed) can't silently diverge between
  suites again.
