# ADR-069: Bound `embyr_rate_limit_requests_total{project_id}` Cardinality by Reusing the Rate Limiter's Own Existence Check

## Status

Accepted

## Context

`docs/product/production-readiness-audit-2026-09-08.md` finding #2 (Blocker): `RateLimiter::check()`
(`crates/embyr-server/src/middleware/rate_limit.rs:140-150`) unconditionally records
`metrics::counter!("embyr_rate_limit_requests_total", "project_id" => project_id.to_owned(), "outcome"
=> outcome)` using the raw, attacker-controllable `project_id` string extracted from the gRPC resource
path — **before `authenticate()` runs** (`crates/embyr-server/src/grpc/handler.rs:1267` vs. `:1272`).
`extract_project_id` (`handler.rs:99-107`) accepts any non-empty substring; the Prometheus recorder
(`observability.rs`, `metrics-exporter-prometheus`) has no eviction/TTL/max-cardinality guard. An
unauthenticated attacker sending N distinct garbage `project_id` strings creates N permanent Prometheus
time series, eventually OOM-ing `embyr-server`.

This directly invalidates the risk acceptance this codebase already recorded for this exact metric:
ADR-016 § "HIGH CARDINALITY: `project_id` label on `embyr_rate_limit_requests_total`" (D-OBS-7) accepted
unbounded-by-format cardinality on the assumption that growth is bounded by the count of **real,
registered** projects (≤10,000). That assumption held only because nobody had examined whether the
label value is validated before being used — it is not.

Three candidate mechanisms were named at DISCUSS (not decided there):
- (a) format-validate the `project_id` before labeling, sentinel fallback for anything that fails.
- (b) don't label by `project_id` at all pre-authentication; only the confirmed, post-auth value.
- (c) something else, found after reading the code.

### Why (a) alone is rejected

`embyr_core::domain::project::ProjectId::new()` (`crates/embyr-core/src/domain/project.rs:7-22`)
already validates project IDs against `^[a-z][a-z0-9-]{0,62}$`. Reusing it as a metrics-label gate was
considered. **It does not bound cardinality.** The valid-format space (lowercase letters, digits,
dashes, ≤63 chars) is astronomically large and entirely attacker-controlled — an attacker generating
fresh, format-valid random strings (e.g. `"a" + 20 random lowercase-alphanumeric chars`) produces
exactly the same unbounded growth as today, just format-valid unbounded growth instead of arbitrary
unbounded growth. A cheap format check answers "is this syntactically plausible," not "is this bounded
in count." It is retained nowhere in this design as a labeling gate (though it remains valid, unrelated,
existing input validation used elsewhere — `authenticate()` already calls it for its own purposes).

### Why naive (b) is rejected in its literal form

The literal reading of (b) — record the metric only after `authenticate()` confirms the project is
real, using a new "confirmed projects" set populated on auth success — breaks on **call ordering
within the same request**: `rate_limiter.check()` runs at `handler.rs:1267`, strictly before
`authenticate()` at `:1272`, in every one of the ~18 call sites (confirmed by grep — see § Blast Radius
in the feature-delta.md DESIGN section). A newly-registered real project's *very first* request would
still be mislabeled under a same-request "confirm via auth" scheme, because the confirmation write
(inside `authenticate()`) necessarily happens after the metric read (inside `check()`) for that same
request. This is not a rare edge case: every scenario in `tests/observability/acceptance/obs04_rate_limit_metrics.rs`
is a freshly-provisioned project's first-ever traffic in a fresh test process — a literal post-auth-set
implementation would fail 100% of those scenarios once implemented, not just the cold-start minority.
Fixing the ordering problem by moving metric emission out of `RateLimiter::check()` into each of the
~18 call sites (after `authenticate()` returns) was considered and rejected: it turns a one-file,
single-function fix into an 18-site scattered change for no additional correctness benefit — a better,
already-existing signal exists (see Decision).

## Decision

Reuse the **existence check the rate limiter already performs on the *same* Postgres round trip (or
in-process map lookup) it uses to make the allow/reject decision itself**, as the metric-label gate.
No new database round trip, no new data structure, no new pre-auth validation step.

**The load-bearing fact making this correct with zero cold-start gap:** `provision.rs::insert_rate_bucket_in_tx`
(`crates/embyr-server/src/admin/handlers/provision.rs:135-157`) inserts a `rate_buckets` row in the
*same transaction* as every project's `projects` row INSERT, in all four backend-mode provisioning
branches. Every genuinely-provisioned project therefore has a `rate_buckets` row from the moment it is
created — before any request, gRPC or REST, ever reaches it. Provisioning itself (gated by the operator
`EMBYR_ADMIN_KEY`, entirely out of an unauthenticated attacker's reach) is the authoritative "this
project is real" boundary — stronger than format validation, and already the source of truth the rate
limiter's own Postgres path consults to do its job.

Concretely, in `crates/embyr-server/src/middleware/rate_limit.rs`:

- `check_pg()` already computes, as an unavoidable byproduct of disambiguating "rejected" from "row
  absent" (lines 220-279 today), whether a `rate_buckets` row existed *before* this call. Surface that
  boolean instead of discarding it.
- `check_in_process()` already computes, via `buckets.entry(project_id).or_insert_with(...)`
  (lines 291-293 today), whether this exact string was already a key in the in-process map. Surface
  that boolean the same way (a strictly weaker, per-instance-lifetime version of the same signal — see
  § Consequences).
- `check()` uses this boolean — call it `known_existing` — to choose the Prometheus label:
  `known_existing` → the real `project_id`; otherwise → the constant sentinel `"unconfirmed"`.

The rate-limit bucket **key** passed into `check_inner`/`check_pg`/`check_in_process` is completely
unchanged — still the raw `project_id` string, still doing exactly what it does today. Only the
Prometheus **label** computation branches on a signal those functions already had to compute for
themselves.

## Consequences

### Positive

- Exactly one file changes: `crates/embyr-server/src/middleware/rate_limit.rs`. Zero changes to
  `grpc/handler.rs` (~18 call sites unaffected — `check()`'s public signature is unchanged), zero
  changes to `observability.rs`, zero changes to `embyr-core`, zero changes to `provision.rs` (its
  existing behavior is read as a signal, not modified).
- Zero new database round trips, zero new latency on the hot pre-auth path.
- Cardinality is bounded by exactly the same ceiling ADR-016/D-OBS-7 already accepted (≤10,000 real
  projects) — this decision does not introduce a new number, it makes the existing accepted number
  *actually true* instead of aspirational.
- A real project's *first-ever* request gets its correct label immediately (no cold-start gap),
  because provisioning creates the `rate_buckets` row before traffic exists — verified against
  `provision.rs:135-157`, not assumed.
- An attacker sending N distinct, never-repeated garbage `project_id` strings produces exactly one new
  label value (`"unconfirmed"`) for the life of the process, regardless of N.

### Negative / accepted residuals

- **Finding #14** (same audit, DB-amplification: `check_pg`'s "row absent → INSERT a default row and
  allow" branch, `rate_limit.rs:240-254`) is unchanged by this ADR — explicitly out of scope per
  DISCUSS. A side effect worth naming: if an attacker *reuses* the same fake `project_id` many times
  (not the N-distinct-strings attack this feature closes), finding #14 will insert a row for it on the
  first occurrence, and from the second occurrence onward this ADR's mechanism will read that row as
  "existing" and use the attacker's chosen string as a real label. This does not reopen the cardinality
  bound (a reused string contributes no new distinct label value — cardinality growth is driven by
  distinct values, not occurrence count), but it does mean a *small*, attacker-chosen set of repeated
  fake IDs can each earn one permanent, real-looking label. This is bounded by whatever finite set of
  IDs the attacker chooses to repeat — not by N total requests — and is a direct, inherited consequence
  of finding #14 being unfixed, not a new gap this ADR introduces. If #14 is ever fixed (e.g., `check_pg`
  rejecting rather than auto-inserting for absent rows), this residual disappears on its own with no
  further change needed here.
- **In-process fallback mode** (`check_in_process`, used when no `pg_pool` is configured, or as the
  post-20ms-timeout fallback per `RATE_LIMIT_PG_TIMEOUT_MS`): the "existing" signal is a per-instance,
  in-memory map lookup, not pre-seeded at provisioning time. A real project's first request that happens
  to land on a fresh instance (or fall back during a Postgres hiccup) before that instance has seen it
  before gets one-time `"unconfirmed"` labeling on that instance, self-correcting from the second request
  onward. Accepted: this mirrors the fallback path's already-documented fail-open/eventually-consistent
  trade-offs (ADR-015), and only affects the rare timeout/no-pg-configured path, not the primary
  Postgres-backed production path.
- Adds one new, permanent label value (`"unconfirmed"`) to `embyr_rate_limit_requests_total` visible in
  Grafana/Prometheus — an operator-facing, documented, intentional signal ("pre-auth traffic against a
  project_id not found in `rate_buckets` at check time"), not noise.

## Alternatives Considered

1. **Format-validate then sentinel-fallback** (candidate a) — rejected, see § Why (a) alone is
   rejected. Does not bound cardinality against a targeted attacker.
2. **Post-auth-confirmed label via a new tracked set, metric moved to after `authenticate()`** —
   rejected, see § Why naive (b) is rejected. Correct in spirit, wrong implementation: breaks
   first-request labeling for real projects and requires 18 scattered call-site edits for a benefit the
   selected mechanism gets for free.
3. **Cardinality-capped LRU/registry wrapper around the Prometheus label itself** (candidate c) —
   considered and rejected: an eviction-based cap (e.g., first 10,000 distinct values win a real slot)
   is poisonable — an attacker who floods first can permanently consume the entire budget before any
   real project ever gets a slot, converting an OOM DoS into a "deny real tenants their own accurate
   metric" DoS. The selected mechanism has no such race: real projects are guaranteed a "slot" by
   provisioning, unconditionally, regardless of what an attacker does first.
4. **Re-order the gRPC pipeline so `authenticate()` runs before the rate limiter** — out of scope per
   DISCUSS (§ Out of Scope); would address both finding #2 and #14 at once but is a materially larger,
   riskier change than this feature's narrow requirement, and would remove the rate limiter's own
   documented purpose of skipping Argon2id cost on requests that would be rejected anyway.

## Enforcement

No new automated enforcement mechanism is warranted beyond the regression tests named in the
feature-delta.md DESIGN handoff — this is a bounded, single-file, private-function-signature change
with no new port/adapter boundary and no new external dependency.
