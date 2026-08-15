# ADR-020: Cumulative Free-Plan Cap Check — Background-Computed Cache, Not a Rate-Limiter Extension

## Status

Accepted

## Context

`card-payments-backend`'s DISCUSS wave (D-9, `docs/decisions/card-payments/grill-me-decisions.md`)
originally framed real-time Free-plan cap enforcement as extending "the existing Postgres
token-bucket rate limiter" (`crates/embyr-server/src/middleware/rate_limit.rs`, ADR-015). Having
read `rate_limit.rs` directly, DISCUSS corrected this framing itself
(`docs/feature/card-payments-backend/feature-delta.md` § Open Question — D-9's Rate-Limiter
Framing) and explicitly left the concrete mechanism to DESIGN:

- `RateLimiter`/`TokenBucket` is a **per-second, per-project request-RATE** mechanism: continuous
  refill based on wall-clock elapsed time, keyed by `project_id`.
- What D-6/D-7/D-9 actually need is a **monthly-cumulative-usage-vs-plan-cap comparison**: has this
  *account* (not project) used ≥100% of its Free-plan allowance for a *dimension*, summed across
  all its projects, since the start of the current billing cycle. Different key (account vs.
  project), different time semantics (cumulative-since-cycle-start vs. continuously-refilling),
  different reset boundary (billing cycle vs. none).
- AC-206-06 explicitly locks: "This is a NEW, separately-keyed mechanism from `TokenBucket`/
  `rate_buckets` — DESIGN must not literally extend `TokenBucket`'s per-second refill fields."

Three computation strategies were on the table (DISCUSS's own framing, Slice 06 Technical Notes):
**live-per-request**, **cached-and-periodically-refreshed**, or **incrementally-maintained**. The
Driving Ports section of the DISCUSS feature-delta states the cap check "modifies the existing
request path's internal behavior (a new gate inside the existing middleware chain)" for the
`:8080`/`:8081` Firestore gRPC/REST hot path — the same hot path `RateLimiter::check()` already
runs on for every request, currently bounded to a hard 20 ms Postgres timeout with in-process
fallback (ADR-015).

Slice 07's own Learning Hypothesis independently flags a second risk: "Wiring the cap-exceeded
trigger to `suspend_project()` ... surfaces a race condition the dunning trigger (US-204) didn't,
because this trigger fires from the request hot path rather than an async webhook handler."

## Decision

**The cumulative cap check is computed and enforced by a new background task
(`CapUsageRefresher`), not by any new per-request Postgres query or per-request computation on the
`:8080`/`:8081` hot path. The existing request-path `ProjectStatus` check (already present in the
auth interceptor, zero new code) is what actually blocks traffic once suspension is applied.**

### Component shape

1. **`CapUsageRefresher`** (`crates/embyr-server/src/sweepers/cap_usage_refresher.rs`) — a new
   Tokio-interval background task, structurally identical to the existing `QueryLogSweeper`/
   `SessionCleaner` precedent (admin-api-v2): runs every `EMBYR_CAP_CHECK_INTERVAL_SECS` (default
   `30`), guarded by a Postgres advisory lock (`pg_try_advisory_lock(fnv1a_hash("embyr_cap_check"))`)
   to avoid redundant duplicate computation across instances (not required for correctness — see
   Consequences — but consistent with the existing sweeper pattern and avoids wasted Postgres load).
   Each cycle:
   - Loads every account with `subscriptions.plan = 'free'` and `subscriptions.status IN
     ('active', 'free_cap_exceeded')`.
   - For each, computes per-dimension cumulative usage (reads/writes/deletes — see § Storage
     Dimension Gap below) by summing `daily_project_metrics` joined through `projects.account_id`,
     filtered to the current calendar-month cycle (see § Billing Cycle Boundary below).
   - Writes the result into the in-process `CapStatusCache`.
   - When a Free account's status transitions from below-cap to ≥100% on any dimension, calls
     `lifecycle::suspend_account_projects(account_id, ...)` (ADR-020 §"Reused Suspend Path" below)
     and sets `subscriptions.status = 'free_cap_exceeded'`.

2. **`CapStatusCache`** (`crates/embyr-server/src/adapters/cap_status_cache.rs`) — an in-process,
   per-instance `Arc<RwLock<HashMap<AccountId, CapStatus>>>`, structurally identical in shape to
   the existing `CredentialCache` (`Arc<RwLock<LruCache<...>>>`). Consulted by
   `GET /admin/v1/billing/subscription` (US-201/US-206, admin port `:9090`, session-authed — NOT
   the hot path) to populate the `cap_status` response field. A cache miss (account never yet
   refreshed, e.g. brand new) fails open: `cap_status` is omitted from the response (AC-206-04),
   never fabricated as exceeded.

3. **The `:8080`/`:8081` hot path is unmodified beyond what already exists.** The auth
   interceptor's existing `ProjectStatus` check (SPEC-locked, step 3 in the documented auth flow —
   `docs/product/architecture/brief.md` § Driving Ports, "Auth interceptor design") already returns
   `PermissionDenied` for any `Suspended` project, regardless of *why* it was suspended. This is the
   literal "gate inside the existing middleware chain" the DISCUSS Driving Ports note anticipated —
   it turns out to already exist; what changes is *who writes* `suspended`, not the read-side check.
   Net latency added to the Firestore hot path: **zero**.

### Reused suspend path (D-12 compliance)

`lifecycle.rs::set_project_status` (private, single-project) is widened to `pub(crate)` and gains
two new fan-out wrapper functions in the same file:

```
pub async fn suspend_account_projects(account_id: AccountId, state: &LifecycleDeps) -> Result<u32, sqlx::Error>;
pub async fn activate_account_projects(account_id: AccountId, state: &LifecycleDeps) -> Result<u32, sqlx::Error>;
```

Both iterate `SELECT id FROM projects WHERE account_id = $1 AND status = <source status>` and call
the *exact same* `set_project_status` per project — satisfying D-12 ("one suspension mechanism, two
triggers") directly and literally, not by convention. `set_project_status`'s signature is narrowed
from `&OperatorState` to a smaller `LifecycleDeps { system_db: Arc<SystemDb>, credential_cache:
Arc<CredentialCache> }` struct so it is callable from `WebhookState` (dunning, US-204) and the
`CapUsageRefresher` background task (US-207) without either needing the full `OperatorState`
(admin key, AWS/GCP fetchers, rate-limit capacity — none of which either caller needs).

### Billing cycle boundary

Free-plan accounts (the only accounts this check applies to, D-6/D-7) have no guaranteed Stripe
`Subscription` object — `subscriptions.current_period_start`/`current_period_end` are `NULL` until
an account upgrades to Pro (US-202). AC-206-05 locks "resets at the billing-cycle boundary, not
account-creation date" without specifying what a Free account's cycle boundary *is*. **Decision:
Free-plan cap-check cycles are UTC calendar months.** Pro-plan accounts' `current_period_start`/
`current_period_end` (sourced from Stripe) are used only for subscription *display* (US-201's
`current_period_end` field) — never for cap computation, because caps do not apply to Pro accounts
at all (D-6/D-7). This removes the ambiguity DISCUSS deliberately left open.

### Storage dimension — known gap, not silently absorbed

D-7 requires all four dimensions (reads, writes, deletes, storage) metered/capped separately.
`daily_project_metrics` (`migrations/0002_metrics.sql`) has `read_ops`/`write_ops`/`delete_ops`/
`listen_connections` — **no storage-bytes column**. The existing `GET /admin/v1/billing` handler
already hard-codes `log_storage_bytes = 0::BIGINT` with the comment "deferred; UI shows '—'"
(`crates/embyr-server/src/admin/handlers/billing.rs`). Computing real per-project document storage
size requires querying each project's *customer* Postgres (`pg_total_relation_size` against the
`documents` table) — a cross-database operation neither `daily_project_metrics` nor this feature's
batch job currently performs, and one that would require a new `BackendAdapter` port method
(`table_size()`), a real port change beyond this feature's locked scope.

**Decision: storage-dimension cap enforcement and metering are NOT implemented in this feature's
V1.** `cap_status`'s `storage` entry (if included at all) mirrors `billing.rs`'s existing
placeholder (omitted or `0`), consistent with the precedent this feature inherits rather than a new
gap it introduces. Reads/writes/deletes — 3 of 4 D-7 dimensions — are fully real. This is flagged
explicitly as **OQ-CP-1** (see feature-delta.md Open Questions) as a scoped follow-up, not silently
declared "done."

## Alternatives Considered

### Alternative 1: Live per-request computation (rejected)

Compute the cumulative-usage aggregate on every `:8080`/`:8081` request for Free-plan projects,
with a 20 ms timeout + fail-open fallback mirroring `RateLimiter::check_pg`.

**Rejected because:** the aggregate query (`SUM(...) FROM daily_project_metrics` joined across
every project under an account, filtered to a calendar-month range) is materially heavier than
`RateLimiter`'s single-row `UPDATE ... WHERE project_id = $1` — a multi-row, multi-project scan
executed on *every* request for every Free-plan account is a meaningfully different load profile,
not a drop-in reuse of the 20 ms pattern. Running this on every request for potentially thousands
of Free accounts multiplies Postgres load for a value that only needs to be accurate to within
tens of seconds (D-6's "hard-stop" is a trust/billing guarantee, not a sub-second SLA — KPI #5's
own numeric target was explicitly deferred to DESIGN precisely because no such SLA was locked).

### Alternative 2: Incrementally-maintained running counter (rejected)

Maintain a live per-account-per-dimension counter, incremented atomically on every write
(`UPDATE account_usage_counters SET writes = writes + 1 WHERE account_id = $1`), mirroring
`rate_buckets`' UPDATE shape.

**Rejected because:** (a) requires resolving `project_id → account_id` on every hot-path request
(an extra join/lookup not currently needed there), (b) introduces a second, independently-mutated
source of truth for usage that can drift from `daily_project_metrics` (the existing, already-proven
ground truth every other billing surface reads from) — directly violating this feature's own System
Constraint ("no consumer is permitted to hold its own drifting copy," DISCUSS § Journey Deep-Dive
Shared Artifacts Registry validation rule), and (c) adds write load to the hot path for a value
whose staleness tolerance (tens of seconds, per D-6's non-instant trust guarantee) does not require
per-write freshness.

### Alternative 3: Synchronous per-request suspend, as DISCUSS's journey diagram literally sketches (rejected as the enforcement mechanism, partially reused for the read side)

DISCUSS's System/Operator Journey diagram shows the cap check as a synchronous gate in the request
path, with `suspend_project()` called inline. Taking this literally reintroduces the exact race
condition Slice 07's own Learning Hypothesis names (concurrent in-flight requests for the same
account each observing "not yet suspended" and racing to suspend/reactivate) — a risk the dunning
trigger (US-204, async webhook handler, naturally serialized per Stripe event) does not have.

**Rejected as the enforcement mechanism** because moving enforcement to a background task
(single-threaded per refresh cycle, same concurrency shape as a webhook handler) eliminates this
race by construction rather than requiring new synchronization primitives in the hot path.
**Retained conceptually for the read side**: `cap_status` display still reflects "the system's
current best knowldge," just refreshed on an interval rather than per-request.

## Consequences

### Positive

- Zero latency added to the `:8080`/`:8081` Firestore hot path — better than the 20 ms bound
  `RateLimiter` accepts, because no new hot-path code exists at all.
- Eliminates the race condition Slice 07 explicitly flagged as a risk, by construction (background
  task has the same single-writer-per-cycle shape as the dunning webhook handler).
- `set_project_status` reuse is literal and direct (D-12 compliance verified by construction, not
  by convention) — both dunning (US-204) and cap-exceeded (US-207) triggers call the identical
  function via the new `suspend_account_projects`/`activate_account_projects` wrappers.
- `CapStatusCache` and `CapUsageRefresher` both reuse established in-process patterns
  (`CredentialCache`'s cache shape; `QueryLogSweeper`/`SessionCleaner`'s advisory-lock sweeper
  shape) — near-zero net-new architectural surface.
- Resolves KPI #5's deferred numeric target: enforcement latency is bounded by
  `EMBYR_CAP_CHECK_INTERVAL_SECS` (default 30 s), i.e., **worst-case ≤ 2× the interval (60 s)** from
  the write that crosses 100% to the project transitioning to `suspended`.

### Negative / Trade-offs

- Enforcement is not instantaneous — a Free account can exceed its cap by up to one refresh cycle's
  worth of additional usage before being suspended (bounded, not unbounded: reads/writes/deletes
  are still individually rate-limited by the *existing*, unrelated per-second `RateLimiter`, so this
  is not an unbounded overage window).
- `cap_status` displayed via `GET /admin/v1/billing/subscription` can be up to
  `EMBYR_CAP_CHECK_INTERVAL_SECS` stale relative to the true current cumulative usage. Acceptable:
  AC-206-04 requires fail-open on staleness, not zero staleness.
- Storage-dimension cap enforcement is explicitly not built in V1 (see § Storage Dimension Gap) —
  a real, documented functional gap against D-7's literal four-dimension requirement, not a silent
  omission.
- A new interval/config surface (`EMBYR_CAP_CHECK_INTERVAL_SECS`) and a new advisory-lock key are
  added to the composition root, mirroring existing sweeper wiring — small, consistent, low-risk
  addition.

## Enforcement

- `embyr-core::admin::billing` (the pure `CapStatus`/`compute_cap_status`/`cap_exceeded` types and
  functions) has zero IO imports — enforced by the existing `cargo-deny` `deny.toml` constraint on
  `embyr-core`, unchanged.
- Unit tests for `compute_cap_status`/`cap_exceeded` cover the boundary-inclusive ≥100% rule
  (AC-206-02) without any database or Stripe dependency (pure function tests).
- Integration test (DISTILL wave): a real Postgres-backed test drives `daily_project_metrics` rows
  across the 100% boundary and asserts `CapUsageRefresher`'s next cycle (a) updates `CapStatusCache`
  correctly and (b) calls `suspend_account_projects` exactly once per crossing (not once per cycle
  while remaining over cap — idempotent, does not re-suspend an already-suspended account
  needlessly, though doing so would still be harmless per `set_project_status`'s idempotent
  `WHERE status IN ('active','suspended')` clause).
- Race-condition-elimination claim (§ Alternative 3) is verified behaviorally: a DISTILL-wave test
  drives concurrent Firestore requests against a Free account mid-cap-crossing and asserts no
  double-suspension / no lost-update on `subscriptions.status`.
