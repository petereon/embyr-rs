# Feature Delta: rate-limiter-fail-open

## Wave: DISCUSS / [REF] Reading Confirmation

✓ `docs/product/production-readiness-audit-2026-09-08.md` read for finding #20 (Medium, Security),
confirmed verbatim: *"Rate limiter fails open on database errors and conflates 'DB error' with
'allowed' (`.ok().flatten()` discards the error, `.unwrap_or(false)` on the exists-check). An
attacker who degrades the system DB (see #14) turns off rate limiting globally as a side effect."*
Cited location `crates/embyr-server/src/middleware/rate_limit.rs:171-181,217,232-238,250` has shifted
— two other features (`preauth-db-amplification`, `pool-sizing-and-limits`) touched this file since
the audit ran. Current line numbers, confirmed by direct read of the file as it exists today:

- **Line 238-253** (`check_pg`'s atomic `UPDATE ... RETURNING tokens`, via `query_scalar::<_, f64>()
  .fetch_optional(pool).await.ok().flatten()`): `sqlx::query_scalar::fetch_optional` returns
  `Ok(None)` for the LEGITIMATE case "0 rows updated" (either genuinely rate-limited or the row
  doesn't exist) and `Err(e)` for a REAL Postgres error (connection refused, pool-acquire failure,
  permission error, disk full, etc.). `.ok()` converts `Err(e)` to `None`, discarding `e` entirely.
  `.flatten()` then merges that manufactured `None` with the legitimate `Ok(None)` case into a single
  `None` value. **From this line onward, a genuine DB error is indistinguishable from "0 rows
  updated."**
- **Line 265-293** (the `None` arm — disambiguates "0 rows" into rate-limited vs. row-absent): runs a
  second query, `SELECT EXISTS(...) .fetch_one(pool).await.unwrap_or(false)` (line 267-273).
  `fetch_one` returns `Ok(true)`/`Ok(false)` on success, `Err(e)` on a real Postgres error.
  `.unwrap_or(false)` maps BOTH "genuinely queried, row does not exist" (`Ok(false)`) AND "the query
  itself failed" (`Err(e)`) to the same `false`. When `false`, lines 275-292 INSERT a fresh default
  row (`ON CONFLICT DO NOTHING`) and unconditionally return `Ok(RateLimitInfo { remaining: capacity -
  1.0, ... })` — **the request is allowed, with a full fresh bucket, regardless of whether the row
  was genuinely absent or Postgres was simply erroring.**
- **Line 296-308** (a third, narrower `.ok().flatten().unwrap_or(0.0)` inside the "genuinely
  rate-limited" branch, reached only when `row_exists` was confirmed `true`): a DB error here defaults
  `current` to `0.0`, used only to compute the `reset_ms` header value for an ALREADY-decided `Err(...)`
  (rejected) response. This does not change the allow/deny outcome — it degrades header precision on
  an already-fail-closed path. **Confirmed NOT part of the fail-open defect**; noted for completeness,
  not remediated by this feature.

✓ **The core security question, answered directly from the code**: `check_pg` NEVER surfaces a raw
`sqlx::Error` to its caller — it always resolves internally to `Ok(RateLimitInfo)` or
`Err(RateLimitInfo)` (both domain-level allow/deny outcomes, never a Rust `Result::Err` carrying the
DB error). This means the **only** path that can trigger `check_inner`'s existing timeout-driven
fallback to `check_in_process` (line 198-217, the bounded per-instance bucket) is a `tokio::time::
timeout` actually elapsing — i.e. the Postgres call not completing within 20ms. A `sqlx::Error` that
returns FAST (a live connection error, pool-acquire failure, or query error — anything that fails
before the 20ms deadline, which `pool-sizing-and-limits`' new `acquire_timeout` short-circuit makes
*more*, not less, likely) never reaches that fallback at all. It is swallowed inside `check_pg` itself
and, via the two `.ok()`/`.unwrap_or` calls above, resolved to **allow, with a freshly-inserted
full-capacity bucket** — **confirmed: the finding's claim is accurate. Non-timeout Postgres errors
currently fail open, unconditionally and unboundedly** (not the bounded 1x-capacity fallback
`check_in_process` would provide).

✓ `docs/evolution/2026-08-08-distributed-rate-limiting.md` (ADR-015) read for the ORIGINAL design
rationale. **D3: "Failure mode: fail open with per-instance fallback capped at 1x configured limit;
20ms Postgres timeout hard-coded; not configurable | Availability over strict enforcement during
degradation."** This was a DELIBERATE, documented decision — but scoped explicitly to the TIMEOUT
case, and explicitly BOUNDED (capped at 1x limit via the existing in-process token bucket, never
uncapped). `docs/product/jobs.yaml` JOB-11's own functional dimension restates the same bound
verbatim: *"per-instance fallback caps at 1x limit when system DB unreachable (not uncapped, not
2x)."* **This feature does not reopen or reverse D3** — it closes a gap where D3's own stated
guarantee is silently NOT honored for a second, distinct class of degradation (query errors, not
timeouts).

✓ `docs/evolution/2026-09-13-preauth-db-amplification.md` (finding #14, closed) read. Confirmed: added
a pre-`rate_limiter.check()` `ProjectId::new` charset guard, reducing pre-auth DB round-trips for
structurally-invalid `project_id`s. Explicitly, by construction (zero diff lines inside `check_pg`/
`check_inner`/`RateLimiter`), left this feature's exact concern untouched and named it as finding
#20's own job (§ Key Decisions row: *"Finding #20 (fail-open-on-DB-error, same file) | Explicitly out
of scope, confirmed untouched by construction"*). Confirms: a well-formed-but-garbage `project_id`
that IS provisioned, or any legitimate `project_id` during a real DB incident, still reaches
`check_pg` today — finding #14's fix narrows the ATTACK SURFACE for triggering DB load, it does not
change what happens once `check_pg` actually runs during degradation.

✓ `docs/evolution/2026-09-14-pool-sizing-and-limits.md` (finding #16, closed) read. Confirmed: added a
real `acquire_timeout` to the shared system pool (already had 5s) and made it env-overridable; a
saturated pool now fails FAST (single-digit seconds) with a clean `sqlx::Error` instead of hanging
toward sqlx's 30s default. Explicitly left `check_pg`/`check_inner`'s fail-open behavior untouched
(§ Follow-Up Work is silent on it; no diff lines in `rate_limit.rs` per that evolution doc's own Key
Files list). **This raises, not lowers, the real-world odds of finding #20 firing**: a saturated
shared system pool (the exact `rate_buckets` table's own pool) now produces a pool-acquire error
WELL UNDER the 20ms `check_pg` timeout window rather than hanging past it — meaning MORE degradation
scenarios now resolve as a fast `sqlx::Error` (hitting the buggy `.ok().flatten()`/`.unwrap_or(false)`
path) rather than a slow timeout (hitting the correct, bounded `check_in_process` fallback). Pool
saturation is directly attacker-reachable: any sustained high-concurrency load against the shared
system pool (the same pool auth, admin API, and `rate_buckets` all depend on) degrades it.

✓ `docs/evolution/2026-09-14-healthz-dependency-checks.md` (finding #15, ADR-078) read. Confirmed:
`/healthz` now performs a real `SystemDb::probe()` (`SELECT 1` + schema check, 3s timeout) against
the SAME shared system pool `check_pg` uses, giving an orchestrator an independent signal to stop
routing traffic to a pod whose system DB is down. **This reduces, but does not eliminate, this
finding's real-world blast radius**: a orchestrator only acts on a FULL `/healthz` failure (the probe
itself failing outright within its 3s timeout). A PARTIAL or INTERMITTENT degradation — brief
connection resets, transient permission blips, a momentarily-saturated pool that recovers within a
few requests, or contention isolated to `rate_buckets` specifically (e.g. row-level lock contention
under high per-project concurrency, which does not necessarily fail a `SELECT 1` against a DIFFERENT
connection) — can leave `/healthz` reporting healthy while `check_pg` intermittently errors. **This is
exactly the window where finding #20 is most dangerous**: the operator sees a healthy fleet while an
attacker (or a noisy neighbor) silently rides the fail-open bug past their rate limit. Confirmed: this
finding is genuinely unaddressed by #14, #15, or #16 — each closed a different, real gap in the same
neighborhood, none of them this one.

✓ `docs/product/jobs.yaml` JOB-11 (`fair-multitenancy`, P2 Sam Chen) read in full, including its
existing functional dimension's already-stated "capped at 1x, not uncapped" promise (quoted above).
Extended with a dated NOTE (this DISCUSS) rather than a new job — see § Persona & Job.

## Wave: DISCUSS / [REF] Investigation Findings

### Investigation 1 — the failure-mode decision: fail-open (bounded) vs. fail-closed vs. current (unbounded fail-open)

Three real options, evaluated on their merits rather than defaulting to either reflex:

**Option A — Always fail closed on any Postgres error inside `check_pg`** (reject the request,
`RESOURCE_EXHAUSTED` or a distinct `UNAVAILABLE`, whenever the DB check cannot be completed).
Rejected as the primary fix: (1) it directly contradicts ADR-015 D3's own deliberate,
still-valid rationale ("availability over strict enforcement during degradation") for the reason D3
was written — a rate-limiting SUBSYSTEM hiccup (e.g. one saturated connection, one transient
connection reset) would take down ALL traffic for a tenant, not just traffic that would have exceeded
its limit, over a problem unrelated to that tenant's own behavior; (2) it would make behavior
INCONSISTENT between the two kinds of DB degradation this code already distinguishes — a timeout
today correctly degrades to bounded per-instance enforcement (still serves legitimate traffic, just
without cluster-wide coordination), while an error would hard-reject everything; a security fix
should not make the SAME underlying problem (rate limiter cannot reach the DB) behave in two
contradictory ways depending on which failure mode Postgres happens to produce; (3) `/healthz`
(finding #15) already fails closed AT THE RIGHT LAYER for a genuinely dead system DB — a fully-down
Postgres already fails readiness and an orchestrator already stops routing traffic fleet-wide. Adding
a SECOND, narrower fail-closed mechanism inside the rate limiter itself for the exact same "system DB
is down" case is redundant for the scenario it handles best (full outage) and actively harmful for the
scenario it handles worst (a single flaky query on an otherwise-healthy pool).

**Option B — Current behavior (unbounded fail-open on any error)**: rejected outright — this is the
audit finding itself. An attacker who can induce ANY fast-returning Postgres error against the shared
system pool (not just a full outage — a `pool-sizing-and-limits`-shortened `acquire_timeout` breach
counts) gets unlimited, unbounded requests for as long as the degradation persists, with no local
enforcement of any kind. This directly defeats JOB-11's own stated fairness guarantee and is worse
than D3's own documented design ever intended.

**Option C — Bounded fail-open: route ANY `check_pg` failure (error OR timeout) through the SAME
existing per-instance fallback (`check_in_process`), never through an unconditional allow.**
**RECOMMENDED.** This does not invent a new failure mode — it makes `check_pg` honor the SAME
guarantee ADR-015 D3 and JOB-11 already promise for timeouts, for the other kind of DB failure too.
Concretely: an `Err` from the atomic UPDATE query, or an `Err` from the disambiguating EXISTS query,
should be treated exactly like a timeout — falls through to `check_in_process`, which still enforces
a real limit (capped at 1x configured capacity, per node) rather than allowing unconditionally. The
ONLY case that legitimately allows-with-a-fresh-bucket is a Postgres query that SUCCEEDS and
genuinely returns "this row does not exist" (the documented pre-migration-0018 project case) — that
remains unchanged, it is not a failure case at all.

This also directly answers the "fail open for a bounded short window, then fail closed" framing this
task raised as a possible middle path: a TIME-boxed escalation (allow briefly, then start rejecting)
would need new state (a clock, a counter, a per-project or global "how long has the DB been down"
tracker) this codebase does not have and ADR-015 never asked for. The simpler, already-proven, already
-consistent middle path is ENFORCEMENT-bounded, not time-bounded: never literally "off," always at
least the per-instance cap, for exactly as long as the DB is unreachable — reusing code that already
exists, is already tested, and is already the documented contract for the timeout case.

### Investigation 2 — the two `.ok()`/`.unwrap_or` collapses are two instances of one root pattern, not two separate bugs

Both defects have the identical shape: a `sqlx` call that can return `Err` (real failure) or a
legitimate `Ok(None)`/`Ok(false)` (a normal "not found" outcome) gets collapsed with `.ok()...` or
`.unwrap_or(...)` into a single value that then can't distinguish "PG errored" from "PG succeeded and
said no." The fix is one pattern applied twice: match on the `Result` explicitly, treat `Err` as "DB
check unavailable" (→ per-instance fallback), and only treat the genuine `Ok` "not found" value as the
legitimate business case it already handles (rate-limited, or pre-migration-project-gets-a-fresh-row).
This supports ONE right-sized story, not two — same file, same root cause, same fix shape, applied at
two call sites inside the same function.

### Investigation 3 — this is a conformance fix against an ALREADY-DOCUMENTED requirement, not new scope

JOB-11's functional dimension (`docs/product/jobs.yaml`, unchanged since `distributed-rate-limiting`
shipped) already states: *"per-instance fallback caps at 1x limit when system DB unreachable (not
uncapped, not 2x)."* This sentence was written to describe the TIMEOUT path (the only one the
`distributed-rate-limiting` feature actually implemented that way) but its wording makes no such
carve-out — it says "when system DB unreachable," full stop. A fast Postgres `Err` IS the system DB
being unreachable (just via a different failure signature than a timeout). This DISCUSS treats the
fix as bringing the CODE into conformance with a requirement JOB-11 already states, not as expanding
JOB-11's scope or inventing new behavior — the acceptance criteria below are testable directly against
that pre-existing sentence.

## Wave: DISCUSS / [REF] Open Design Questions (named, not locked)

- **OQ-RLFO-01 (fallback trigger granularity)**: whether EVERY `sqlx::Error` variant inside `check_pg`
  should route to `check_in_process`, or whether some narrow subset (e.g. a constraint violation on
  the `INSERT ... ON CONFLICT DO NOTHING`, which is already tolerated via `let _ =` on line 277-284)
  should keep its current handling. DISCUSS's own reading found only the two identified call sites
  (the UPDATE's `fetch_optional` and the EXISTS's `fetch_one`) actually drive the allow/deny decision
  — the tolerated `INSERT` error is already inert (its own row is optional; the response was already
  decided by the time it runs). DESIGN should confirm no other latent `Result`-swallowing exists in
  this function before finalizing.
- **OQ-RLFO-02 (observability of the new fallback trigger)**: whether a DB-error-triggered fallback
  should increment the SAME `embyr_rate_limit_pg_timeout_total` counter the timeout path already emits
  (renaming/repurposing it), or a new, distinctly-named counter (e.g.
  `embyr_rate_limit_pg_error_total`) so Sam Chen can distinguish "the DB was slow" from "the DB
  errored" when diagnosing an incident via the `/metrics` endpoint (`observability`, ADR-016). DISCUSS
  leans toward a distinct counter (these are operationally different signals worth telling apart) but
  does not lock the exact metric name.
- **OQ-RLFO-03 (log fidelity)**: whether the existing `tracing::warn!("rate_limit_pg_timeout: ...")`
  log line's wording should be generalized, or whether a second, distinct `tracing::warn!` should fire
  for the error path with the actual `sqlx::Error` embedded (subject to `sanitize-backend-error-
  messages`'s ADR-075 sweep discipline — this log line is server-side `tracing`, not a `tonic::Status`
  returned to a caller, so ADR-075's client-facing sanitization rule does not apply, but DESIGN should
  confirm this boundary explicitly rather than assume it).

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

- Feature type: **Backend/security-reliability fix** — no user-facing UI, no customer-visible surface
  beyond the rate-limit behavior itself; the "user" is the deployment operator (Sam Chen) who relies on
  the fairness guarantee, and indirectly every tenant whose fair-share protection this defect
  currently undermines. No journey artifact, no TUI mockup, no emotional-arc YAML (mirrors this
  session's established precedent for this class of finding: `pool-sizing-and-limits`,
  `healthz-dependency-checks`, `preauth-db-amplification`).
- Scope: **finding #20 (Medium, Security)** only. Related-but-separate, already-closed findings
  explicitly NOT reopened: #14 (pre-auth DB amplification — reduces attack surface for TRIGGERING
  `check_pg`, doesn't change what `check_pg` does once triggered), #16 (pool sizing/`acquire_timeout` —
  changes HOW FAST a saturated pool errors, doesn't change what happens on that error), #15
  (`/healthz` readiness — an independent, complementary signal at the ORCHESTRATOR layer, not a fix to
  the rate limiter's own internal decision logic).
- Failure-mode decision: **Option C, bounded fail-open via the existing per-instance fallback** (§
  Investigation 1) — locked as the DISCUSS recommendation, carried into the acceptance criteria below.
  DESIGN may revisit if it finds evidence DISCUSS's reading missed, but the trade-off analysis and
  rejected alternatives are recorded here for that review to engage with, not silently overridden.
- JTBD: reuse an existing job — **JOB-11 (`fair-multitenancy`)**, extended with a dated NOTE (§ Persona
  & Job) — not a new job.
- Walking Skeleton: **Yes** — a real running `embyr-server` against a real Postgres container
  (testcontainers), with `check_pg`'s underlying connection forced into a real error (not a timeout —
  e.g. terminating the specific backend connection mid-query, or pointing at a pool with `max_
  connections(0)`/an immediately-failing acquire) and proving the request is evaluated against the
  bounded per-instance bucket rather than allowed unconditionally.
- UX Research Depth: **None** — backend security/reliability fix; no emotional arc, no journey YAML,
  no TUI mockup applies.

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: **P2 Sam Chen (Service Operator / Platform Engineer)**, primary — the operator who
promised tenants a real, enforceable per-project rate limit (`distributed-rate-limiting`'s own
`JOB-11`) and needs that promise to hold even when the shared system Postgres degrades, not just when
it is fully healthy or fully down. Secondary, evil-user framing per BDD Confirmation Bias Defense
(Technique 2): an unnamed attacker/noisy-tenant who can induce Postgres errors against the shared
system pool (e.g. via sustained high-concurrency load, per `pool-sizing-and-limits`' own newly-fast
`acquire_timeout` breach) and who benefits, under the CURRENT bug, from an unconditional, unbounded
"rate limiting is off" side effect.

**Job**: **JOB-11 `fair-multitenancy`**, reused, extended with a dated NOTE rather than a new job (see
`docs/product/jobs.yaml`, NOTE added 2026-09-15). JOB-11's own functional dimension already states the
target behavior this feature brings the code into conformance with: *"per-instance fallback caps at 1x
limit when system DB unreachable (not uncapped, not 2x)."* This feature does not add a new promise —
it closes the gap between that promise and what `check_pg` currently does for one specific class of
"system DB unreachable" (a fast query error, as opposed to a slow timeout).

**Candidates considered and rejected**:
- **JOB-13 (`production-deployment`, P2 Sam Chen)** — considered because the fix lives in the same
  file/family of concerns as `pool-sizing-and-limits` and `healthz-dependency-checks`, both filed under
  JOB-13. Rejected as primary: those two features are about DEPLOYMENT-TIME configurability
  (env-var-driven pool sizing) and ORCHESTRATOR-FACING health signaling — genuinely different
  observable outcomes from this feature's own (a per-project rate-limit GUARANTEE holding under
  partial DB degradation). This finding's own JOB-11 functional dimension already names the exact
  target behavior verbatim; extending JOB-13 would require restating a promise JOB-11 already makes.
- **A new job** — considered and rejected: this is a conformance fix to an existing, already-documented
  requirement (§ Investigation 3), not a new user need or a new observable capability. Creating a new
  job would misrepresent this as expanding what Sam Chen can do, when the actual change is making an
  existing promise hold under a failure mode it previously silently excluded.
- **`infrastructure-only`** — considered because the change is internal error-handling logic with no
  new API surface. Rejected: Sam Chen makes a real, observable decision with this feature's output —
  whether the fairness guarantee (`distributed-rate-limiting`'s own headline outcome) can be trusted
  during a partial DB incident, provable via a real degraded-DB integration test and visible via the
  new/renamed Prometheus counter (OQ-RLFO-02) — satisfying Dimension 0's "real decision enabled" test.

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

**Signals checked**: >10 user stories? No (1). >3 bounded contexts/modules? No — 1 crate
(`embyr-server`), 1 file (`rate_limit.rs`), 2 call sites inside 1 function (`check_pg`), zero
`embyr-core` change (`RateLimitInfo` is unchanged; no new domain type needed). Walking skeleton >5
integration points? No (1: forcing a real, non-timeout Postgres error against `check_pg` and observing
the fallback). Estimated effort >2 weeks? No — this is a `match`-on-`Result` refactor at two call
sites reusing an ALREADY-EXISTING fallback function (`check_in_process`); well under 2 days including
a real-Postgres-error integration test. Multiple independent user outcomes? No — one outcome: "a
Postgres query error inside the rate-limit check degrades to the same bounded per-instance enforcement
a timeout already does, never to an unconditional allow."

**Verdict: PASS.** One right-sized story.

## Wave: DISCUSS / [REF] System Constraints

- ADR-015 D3's TIMEOUT-triggered fallback to `check_in_process` (20ms hard-coded, not configurable)
  must not regress — this feature extends the SET of conditions that trigger the fallback, it does not
  change the fallback mechanism itself or its existing 1x-capacity cap.
- The legitimate "row genuinely absent" case (pre-migration-0018 project, EXISTS query SUCCEEDS and
  returns `false`) must continue to insert a default row and allow the request — this is NOT a failure
  case and this feature must not turn it into a rejection or route it through `check_in_process`.
- `RateLimiter::check`'s public signature (`async fn check(&self, project_id: &str) -> Result<
  RateLimitInfo, RateLimitInfo>`) does not change — callers (9 gRPC handler sites, `rest_rate_limit_
  middleware`) are unaffected by construction.
- No new Cargo dependency; no new `embyr-core` type. `check_in_process` already exists and is already
  exercised by the existing timeout-fallback acceptance scaffold (`distributed_rate_limiting`'s own
  b13 tests) — this feature reuses it, it does not modify its own internal behavior.
- Metrics/logging additions (OQ-RLFO-02, OQ-RLFO-03) must follow the existing `metrics`-crate,
  lock-free-counter convention already established by `observability` (ADR-016) — no new
  instrumentation library.

## Wave: DISCUSS / [REF] User Stories

### US-01: Sam Chen's Per-Project Rate Limit Survives a Postgres Query Error — Not Just a Timeout

**job_id**: JOB-11 | **Release**: 1 (Walking Skeleton) | **Persona**: P2 Sam Chen

#### Elevator Pitch
**Before**: When the shared system Postgres pool returns a real error while `check_pg` is evaluating
a rate-limit decision (a dropped connection, a permission failure, a saturated pool's `acquire_
timeout` firing fast — anything short of the 20ms soft timeout), the error is silently discarded
(`.ok().flatten()`, `.unwrap_or(false)`) and the request is unconditionally ALLOWED with a freshly
inserted, full-capacity bucket. If the error persists (e.g. an attacker keeps the pool saturated),
EVERY subsequent request for EVERY project is allowed, indefinitely — rate limiting is silently,
unboundedly off, with no local enforcement at all. This directly contradicts `JOB-11`'s own documented
promise that a DB-unreachable fallback "caps at 1x limit... not uncapped."
**After**: Any Postgres error encountered while evaluating a project's rate-limit check — not just a
20ms timeout — degrades to the SAME bounded, per-instance token-bucket enforcement the timeout path
already uses. The project's requests are still capped, locally, at the configured rate; they are never
unconditionally allowed because the DB happened to error rather than merely run slow.
**Decision enabled**: Sam Chen can tell a tenant or an SRE reviewer, backed by a real test against an
injected Postgres error (not just a simulated timeout), that the per-project rate-limit guarantee
holds even during a partial database incident — an attacker who can make Postgres return errors
cannot use that as a side channel to disable rate limiting.

#### Who
- Sam Chen (P2) | Service Operator / Platform Engineer who has already promised tenants a
  cluster-wide, per-project rate limit (`distributed-rate-limiting`) | Needs that promise to survive
  every realistic Postgres degradation mode, not only the one (timeout) the original implementation
  happened to cover.
- Secondary (evil-user framing): an attacker or unusually noisy tenant whose own traffic (or another
  exploited path) degrades the shared system pool into returning fast errors — the party this fix
  denies a free "turn off rate limiting" side channel.

#### Solution
Replace the two `Result`-swallowing calls inside `check_pg` (`.ok().flatten()` on the atomic UPDATE's
`fetch_optional`, `.unwrap_or(false)` on the EXISTS check's `fetch_one`) with explicit `Result`
handling: a genuine `Err` on either query routes to the SAME existing `check_in_process` fallback the
20ms timeout already uses. Only a genuinely successful `Ok(false)` from the EXISTS check (row truly
absent) continues to insert a default row and allow — exactly as it does today for that legitimate
case.

#### Domain Examples

**Example 1 (Happy Path — Postgres healthy, existing behavior unchanged)**: Fernbank Analytics
(`project_id: fernbank-analytics`) sends requests at its configured 1000 RPS limit. The shared system
Postgres is healthy. `check_pg`'s atomic UPDATE succeeds every time; requests are allowed until the
`rate_buckets` row's tokens are exhausted, then rejected with `RESOURCE_EXHAUSTED` — byte-for-byte the
same behavior as before this fix.

**Example 2 (Core scenario — a real Postgres error, not a timeout)**: The shared system pool is
saturated (per `pool-sizing-and-limits`' own new fast `acquire_timeout`) by a burst of concurrent
admin-API activity. Solstice Retail (`project_id: solstice-retail`) sends a request; `check_pg`'s
atomic UPDATE fails fast with a pool-acquire error, well under the 20ms `check_inner` timeout window.
Today, this silently allows the request with a fresh full bucket. After this fix, Solstice Retail's
request is evaluated against its own per-instance in-memory bucket (capped at the same configured
capacity) instead — enforced, just not cluster-wide-coordinated, for the duration of the pool
saturation.

**Example 3 (Error/Boundary — sustained DB errors, the security scenario)**: An attacker
(`project_id: attacker-corp`, a never-provisioned or barely-provisioned project) sustains enough
concurrent load to keep the shared system pool erroring for several minutes. Today, EVERY request from
EVERY project — not just `attacker-corp`'s own — is unconditionally allowed for the entire window,
because each `check_pg` call independently hits the same fail-open bug. After this fix, every
project's requests are capped by their own per-instance bucket for the same window — the attacker
gains, at most, a single node's worth of local headroom (the same 1x-cap JOB-11 already promises for a
timeout), never an unbounded, cluster-wide "rate limiting is off."

#### UAT Scenarios (BDD)

```gherkin
Scenario: Rate limiting behaves exactly as today when Postgres is healthy
  Given the shared system Postgres pool is healthy
  And Fernbank Analytics' rate_buckets row has 2 tokens remaining
  When Fernbank Analytics sends 3 requests in quick succession
  Then the first 2 requests are allowed
  And the 3rd request is rejected with RESOURCE_EXHAUSTED
  And no fallback path is triggered

Scenario: A real Postgres error on the atomic UPDATE falls back to bounded per-instance enforcement, not an unconditional allow
  Given the shared system Postgres pool is returning connection errors (not merely slow)
  And Solstice Retail has never previously been rate-limited on this node
  When Solstice Retail sends more requests than its configured per-instance capacity allows
  Then requests beyond that per-instance capacity are rejected
  And no request is unconditionally allowed as a result of the Postgres error
  And no fresh full-capacity bucket is inserted into rate_buckets as a side effect of the error

Scenario: A real Postgres error on the disambiguating EXISTS check falls back to bounded per-instance enforcement
  Given the shared system Postgres pool's atomic UPDATE returns 0 rows updated
  And the follow-up SELECT EXISTS query itself fails with a Postgres error
  When a project sends a request during this condition
  Then the request is evaluated against the per-instance fallback bucket
  And the request is NOT unconditionally allowed on the assumption the project's row is merely absent

Scenario: A genuinely pre-migration project (no Postgres error, row truly absent) is still allowed with a fresh bucket
  Given the shared system Postgres pool is healthy
  And a project's rate_buckets row genuinely does not exist (predates migration 0018)
  When that project sends a request
  Then a default rate_buckets row is inserted for the project
  And the request is allowed
  And this behavior is unchanged from before this fix

Scenario: Sustained Postgres errors do not turn off rate limiting for any project
  Given the shared system Postgres pool continues returning errors for several consecutive requests
  When multiple different projects each send requests exceeding their own per-instance capacity during this window
  Then each project's excess requests are rejected by its own per-instance bucket
  And no project receives unlimited allowed requests for the duration of the Postgres degradation

Scenario: The existing 20ms-timeout fallback path is unchanged by this fix
  Given the shared system Postgres pool's atomic UPDATE takes longer than 20ms to respond
  When a project sends a request during this delay
  Then the request is evaluated against the per-instance fallback bucket, exactly as before this fix
  And the existing rate_limit_pg_timeout warning and its associated counter still fire
```

#### Acceptance Criteria
- [ ] AC-RLFO-01: with the shared system Postgres pool healthy, rate-limit allow/reject behavior is
      unchanged from before this fix (regression guard).
- [ ] AC-RLFO-02: a real (non-timeout) Postgres error on the atomic `UPDATE ... RETURNING tokens` query
      routes the request through the existing per-instance fallback bucket (`check_in_process`) —
      never through an unconditional allow, and never inserts a fresh full-capacity `rate_buckets` row
      as a side effect of the error.
- [ ] AC-RLFO-03: a real (non-timeout) Postgres error on the disambiguating `SELECT EXISTS` query
      routes the request through the same per-instance fallback bucket — never assumes "row absent,
      allow" on the basis of an error.
- [ ] AC-RLFO-04 (regression guard): a genuinely successful `SELECT EXISTS` query returning `false`
      (the pre-migration-0018 project case) continues to insert a default row and allow the request,
      unchanged from before this fix.
- [ ] AC-RLFO-05 (the security property): during a sustained window of real Postgres errors, no
      project's requests are unconditionally allowed — every project's excess requests beyond its own
      per-instance capacity are rejected, proven against a real, injected Postgres error (not a
      timeout), for at least two independent projects.
- [ ] AC-RLFO-06 (regression guard): the existing 20ms-timeout-triggered fallback (ADR-015 D3) and its
      associated `tracing::warn!`/counter continue to fire exactly as before this fix.

#### Outcome KPIs
- **Who**: Sam Chen (Service Operator/Platform Engineer) and every tenant relying on JOB-11's
  fair-multitenancy guarantee (distinct from, but harmed alongside, any tenant an attacker targets
  directly).
- **Does what**: a real Postgres error encountered while checking a project's rate limit results in
  that project's requests being capped by the existing bounded per-instance fallback, never
  unconditionally allowed.
- **By how much**: from "any non-timeout Postgres error → unlimited, unbounded requests allowed for
  every project for as long as the error persists" (confirmed defect, § Reading Confirmation) to
  "capped at 1x per-instance capacity per project, per node, for the same duration" — matching JOB-11's
  own already-documented promise for the timeout case.
- **Measured by**: AC-RLFO-05 — a real testcontainers-Postgres-backed integration test that injects a
  genuine, fast-returning Postgres error (e.g. terminating the pool's backend connection, or a
  zero-capacity/immediately-failing pool) during `check_pg` and asserts request throughput is bounded,
  not unlimited, for at least two independent projects during the injected window.
- **Baseline**: today's confirmed behavior — zero enforcement (unconditional allow, fresh full-capacity
  bucket inserted) on any non-timeout Postgres error, for every project, for as long as the error
  persists (§ Reading Confirmation).

#### Technical Notes
- Reuses `check_in_process` exactly as it exists today (no changes to that function) — only the
  TRIGGER condition inside `check_pg` (which errors now also route to it) changes.
- OQ-RLFO-01 (whether every `sqlx::Error` variant should trigger the fallback, or a narrower subset)
  and OQ-RLFO-02/03 (new-vs-reused metric name, log-line wording) are named DESIGN decisions — this
  story's ACs are written at the observable-outcome level and do not depend on either.
- No `embyr-core` change — `RateLimitInfo` and `RateLimiter::check`'s public signature are unchanged.
- Depends on nothing outside `embyr-server`; `embyr-core` and `embyr-agent` are untouched.
- Sanitization boundary (ADR-075) does not apply to this fix's own logging — the affected log lines are
  server-side `tracing::warn!`, never a `tonic::Status` returned to a caller — but DESIGN should
  confirm this explicitly (OQ-RLFO-03) rather than assume it.

## Wave: DISCUSS / [REF] Out of Scope

- **Finding #14 (preauth-db-amplification)** — already closed; reduces the ATTACK SURFACE for
  triggering `check_pg` with a garbage `project_id`, does not change `check_pg`'s own internal
  behavior. Not reopened.
- **Finding #16 (pool-sizing-and-limits)** — already closed; changes HOW FAST a saturated pool
  produces an error, does not change what `check_pg` does with that error. Not reopened.
- **Finding #15 (healthz-dependency-checks)** — already closed; an independent, orchestrator-facing
  signal for FULL system-DB outages, complementary to but not a substitute for this feature's own
  in-process fix for partial/intermittent degradation. Not reopened.
- **A time-boxed "fail open for N seconds, then fail closed" escalation mechanism** — considered (§
  Investigation 1, Option C discussion) and rejected in favor of the simpler, already-proven,
  enforcement-bounded (not time-bounded) fallback to `check_in_process`. Would require new state this
  codebase does not have and ADR-015 never called for.
- **Picking a different 20ms timeout value, or making it configurable** — unrelated to this finding;
  ADR-015 D3 explicitly hard-codes it, not touched here.
- **A pre-emptive concurrency limiter on the shared system pool** — `pool-sizing-and-limits`' own
  OQ-PSL-02, explicitly deferred there, not this feature's concern.

## Wave: DISCUSS / [REF] Walking Skeleton Strategy

A real running `embyr-server` against a real Postgres container (testcontainers), where `check_pg`'s
underlying query is forced into a genuine, fast-returning error — not a timeout — (e.g. terminating
the specific backend connection `pg_terminate_backend` mid-check, or constructing a pool with an
immediately-failing acquire), then issuing a request for a project and asserting it is evaluated
against the per-instance fallback bucket (bounded, capped at configured capacity) rather than allowed
unconditionally with a freshly-inserted full-capacity row. This single scenario demonstrates the
feature's entire required outcome; the remaining scenarios (healthy-path regression, EXISTS-query
error, legitimate-row-absent regression, sustained-error security property, existing-timeout
regression) reuse the identical proof shape.

## Wave: DISCUSS / [REF] Driving Ports

No new port, no new RPC, no new listener. This feature changes internal error-handling logic inside
`RateLimiter::check_pg`, reached the same way it is today: every existing gRPC handler's
`rate_limiter.check()` call site and `rest_rate_limit_middleware`.

## Wave: DISCUSS / [REF] Pre-requisites

- None blocking. `check_in_process` already exists, is already tested (via the existing
  timeout-fallback acceptance scaffold), and is directly reusable as the fix's own fallback target. The
  testcontainers-Postgres integration-test mechanism for forcing a real connection error already
  exists in this workspace's precedent (`pr08_realtime_listener_reconnect.rs`'s own connection-drop
  pattern) and is directly reusable for this feature's own error-injection proof.

## Wave: DISCUSS / [REF] Definition of Ready Validation

### DoR Checklist (9-item hard gate)

| DoR Item | US-01 |
|---|---|
| 1. Traces to a job_id | PASS — JOB-11, reused via dated NOTE; JOB-13 and a new job both explicitly considered and rejected (§ Persona & Job) |
| 2. Elevator Pitch complete | PASS — Before/After/Decision-enabled, real entry point (every existing gRPC/REST call site that already invokes `rate_limiter.check()`), observable output (bounded per-instance enforcement vs. unconditional allow) |
| 3. 3+ domain examples, real data | PASS — Fernbank Analytics (healthy-path regression), Solstice Retail (real error, bounded fallback), attacker-corp (sustained-error security scenario) |
| 4. UAT in Given/When/Then (3-7) | PASS — 6 scenarios |
| 5. AC derived from UAT | PASS — AC-RLFO-01 through 06 |
| 6. Right-sized (1-3 days, 3-7 scenarios) | PASS — a `match`-on-`Result` refactor at 2 call sites reusing an already-existing, already-tested fallback function; well under 2 days including a real-Postgres-error integration test |
| 7. Technical notes identify constraints | PASS — OQ-RLFO-01/02/03 named, not locked; ADR-015 D3's own unchanged mechanism identified as a hard constraint |
| 8. Outcome KPIs with numeric target | PASS — from "unbounded allow" to "capped at 1x per-instance capacity," proven by a real error-injection integration test across 2+ independent projects |
| 9. Prior-wave artifacts reconciled | PASS — audit finding #20, ADR-015/D3's own original rationale, JOB-11's own pre-existing functional-dimension promise, `preauth-db-amplification`/`pool-sizing-and-limits`/`healthz-dependency-checks`'s own explicit "left untouched"/complementary framing, all directly informed this feature's shape |

### DoR Status: **PASSED**

## Wave: DISCUSS / [REF] Wave Decisions Summary

### Key Decisions
- [D1] Persona/job: **P2 Sam Chen / JOB-11 (`fair-multitenancy`)**, reused via a dated NOTE — not
  JOB-13 (a different observable outcome: deployment-time configurability and orchestrator-facing
  health signaling, not the rate-limit guarantee itself) and not a new job (this is a conformance fix
  to an already-documented JOB-11 promise, not new scope) (§ Persona & Job).
- [D2] Failure-mode decision: **bounded fail-open via the existing per-instance fallback
  (`check_in_process`)**, the same mechanism ADR-015 D3 already uses for timeouts — not always-fail-
  closed (rejected: contradicts D3's own availability rationale, inconsistent with the timeout path,
  redundant with `/healthz`'s own full-outage handling) and not the current unbounded fail-open (the
  defect itself) (§ Investigation 1).
- [D3] Both `.ok().flatten()` (atomic UPDATE) and `.unwrap_or(false)` (EXISTS check) are one root
  pattern — a `Result`-swallowing collapse of "DB errored" into "legitimate not-found" — fixed
  identically at both call sites, not treated as two separate concerns (§ Investigation 2).
- [D4] This is a conformance fix against JOB-11's own already-documented "capped at 1x... not
  uncapped" promise, not an expansion of JOB-11's scope (§ Investigation 3).
- [D5] Findings #14, #15, #16 (all closed, same file/neighborhood) are confirmed complementary, not
  overlapping — none of them changes what `check_pg` does once a real Postgres error occurs, and #16
  specifically makes that error MORE likely to arrive fast (within the 20ms window) rather than as a
  timeout, raising this finding's real-world likelihood (§ Reading Confirmation).
- [D6] Exact fallback-trigger granularity (whether every `sqlx::Error` variant qualifies) and new
  metric/log naming are named DESIGN choices (OQ-RLFO-01/02/03) — DISCUSS locks only the observable,
  testable outcomes in the ACs.

### Requirements Summary
- Primary need: a real Postgres error encountered while evaluating a project's rate-limit check must
  degrade to the same bounded, per-instance enforcement a 20ms timeout already triggers — never to an
  unconditional allow with a fresh full-capacity bucket.
- Constraint: ADR-015 D3's timeout-triggered fallback and its 1x-capacity cap must not regress; the
  legitimate pre-migration-project "row genuinely absent" case must continue to allow with a fresh
  bucket, unchanged; `RateLimiter::check`'s public signature is unchanged.
- Success looks like: AC-RLFO-01 through AC-RLFO-06 all passing against a real running `embyr-server`
  and a real, injected Postgres connection error (not merely a timeout), with zero regression to the
  existing timeout fallback or the legitimate row-absent case.

### Handoff Package (to DESIGN — solution-architect)
- This feature-delta.md (DISCUSS section) — job grounding, exact current-line-number reading of the
  defect (§ Reading Confirmation), 1 user story with 6 UAT scenarios and 6 acceptance criteria, a
  recorded trade-off analysis with 2 rejected failure-mode alternatives (§ Investigation 1).
- Confirmed exact file/line targets for DESIGN: `crates/embyr-server/src/middleware/rate_limit.rs:
  238-253` (atomic UPDATE's `.ok().flatten()`), `:265-293` (EXISTS-check disambiguation branch,
  `.unwrap_or(false)` at line 273), `:296-308` (confirmed NOT part of the defect — header-precision-
  only, already fail-closed by branch context, left untouched).
- Open DESIGN-level choices: OQ-RLFO-01 (fallback-trigger granularity across `sqlx::Error` variants),
  OQ-RLFO-02 (new vs. reused Prometheus counter for the error-triggered fallback), OQ-RLFO-03 (log-line
  wording/fidelity, and confirming the ADR-075 sanitization boundary does not apply here).
- `docs/product/jobs.yaml` JOB-11 already extended with a dated NOTE recording this feature's framing
  — no further job-file changes expected from DESIGN.

## Wave: DESIGN / [REF] Reading Confirmation

✓ `crates/embyr-server/src/middleware/rate_limit.rs` re-read at DESIGN depth, current lines 183-352.
Confirmed byte-for-byte against DISCUSS's own citation (line numbers match exactly — no drift since
DISCUSS, unlike the audit's original citation which had shifted):
- `check_inner` (154-220): `pg_pool.is_some()` guard → `tokio::time::timeout(20ms, self.check_pg(...))`.
  `Ok(result_and_existed) => return result_and_existed` (204), `Err(_timeout) => warn! + counter +
  fall through` (206-216), then unconditional `self.check_in_process(project_id)` (219) — this IS the
  fallback call DISCUSS/this feature reuses. Confirmed single call site.
- `check_pg` (228-318): exactly as DISCUSS read it. `.ok().flatten()` at line 252-253 (UPDATE),
  `.unwrap_or(false)` at line 273 (EXISTS), third `.ok().flatten().unwrap_or(0.0)` at line 306-308
  (header-precision only, confirmed NOT part of the defect — reached only when `row_exists == true`,
  i.e. after the disambiguation this feature fixes has already run and already confirmed a genuine,
  non-error "rate-limited" outcome; a DB error here cannot change allow/deny, only `reset_ms` precision
  on an already-`Err` response — left untouched, per DISCUSS's own scoping).
- `check_in_process` (325-351): confirmed zero relationship to this feature's own diff — reads
  `self.buckets` (in-process `HashMap`), never touches `pool`. **Confirmed untouched by this design.**
- Confirmed via grep: `check_pg` is a private `async fn`, called from exactly one site
  (`check_inner:201`). No test, no other module, calls it directly — the signature change below has
  a blast radius of exactly one call site.

✓ **Load-bearing discovery: ADR-015 (`docs/product/architecture/adr-015-distributed-rate-limiter-postgres.md`,
lines 100-134, "`check_pg()` internal method — return type explanation") already specifies the exact
fix this feature makes** — and specifies it as the ORIGINAL design, not a new one:

> `async fn check_pg(&self, project_id: &str) -> Result<Result<RateLimitInfo, RateLimitInfo>, sqlx::Error>`
> ... `Err(e) => { tracing::warn!(...); // continue to in-process bucket (step 3) }`

ADR-015's own decision path (§ "Decision path on each `check()` call", step 2d) states: *"`Ok(Err(sqlx_error))`
(hard DB error within 20ms) → log WARN → step 3 [in-process fallback]."* **The shipped code in
`check_pg` never implemented this arm.** Instead of returning `Result<Result<RateLimitInfo,
RateLimitInfo>, sqlx::Error>` as documented, it collapses the error internally via `.ok().flatten()`/
`.unwrap_or(false)` before ever returning — `sqlx::Error` never leaves `check_pg`, so `check_inner`'s
`match` can never see it, and the `Ok(Err(sqlx_error)) => fall through` arm ADR-015 asked for was never
reachable code. **This reframes the fix**: it is not "apply an existing pattern (D3/timeout) to a new
code path" (DISCUSS's framing, still directionally correct) so much as "implement the `check_pg()`
return type and step-2d match arm ADR-015 already specified, word for word, on 2026-08-08, which
DELIVER silently did not build." This is the strongest possible evidence for the "no new ADR" judgment
below — the decision already exists in writing; only the code needs to catch up to it.

## Wave: DESIGN / [REF] Code-Level Design

### `check_pg` signature change

```rust
// Before:
async fn check_pg(&self, project_id: &str, pool: &sqlx::PgPool)
    -> (Result<RateLimitInfo, RateLimitInfo>, bool)

// After (restores ADR-015's own originally-specified shape):
async fn check_pg(&self, project_id: &str, pool: &sqlx::PgPool)
    -> Result<(Result<RateLimitInfo, RateLimitInfo>, bool), sqlx::Error>
```

### Call site 1 — atomic UPDATE (current lines 238-253)

Replace `.fetch_optional(pool).await.ok().flatten()` with `.fetch_optional(pool).await?`. The `?`
propagates a genuine `sqlx::Error` out of `check_pg` immediately — before the EXISTS query, before the
INSERT — so a real error on this query can never reach the "row absent, insert + allow" branch and can
never manufacture a fresh full-capacity bucket. `Ok(None)` (legitimate "0 rows updated") continues to
flow into the existing `match allowed { None => ... }` arm unchanged.

### Call site 2 — EXISTS check (current lines 265-293)

Replace `.fetch_one(pool).await.unwrap_or(false)` with `.fetch_one(pool).await?`. A genuine `sqlx::Error`
here propagates out of `check_pg` the same way, before the `if !row_exists` branch runs — it can never
be misread as "row absent" and can never trigger the INSERT-and-allow path. `Ok(true)`/`Ok(false)`
(the two legitimate, error-free outcomes) continue exactly as today: `Ok(false)` → insert default row +
allow (AC-RLFO-04, unchanged); `Ok(true)` → proceed to the existing "genuinely rate-limited" branch
(line 296-308, untouched, see Reading Confirmation).

### Every other return point in `check_pg` wraps in `Ok(...)`

Lines 262 (`Ok(RateLimitInfo{...}), true)`), 285-292 (`Ok(RateLimitInfo{...}), false)`), and 315
(`Err(RateLimitInfo{...}), true)`) each become `Ok((..., ...))` — the outer `Ok` marks "no Postgres
error occurred," the inner `Ok`/`Err` continues to carry the existing allow/reject business decision.
No change to any of the three existing values themselves.

### `check_inner` routing — new match arm (current lines 198-217)

```rust
if let Some(pool) = &self.pg_pool {
    match tokio::time::timeout(
        std::time::Duration::from_millis(RATE_LIMIT_PG_TIMEOUT_MS),
        self.check_pg(project_id, pool),
    )
    .await
    {
        Ok(Ok(result_and_existed)) => return result_and_existed,   // unchanged happy path
        Ok(Err(pg_error)) => {                                      // NEW arm — the fix
            tracing::warn!(
                project_id = project_id,
                error = %pg_error,
                "rate_limit_pg_error: Postgres check failed, falling back to in-process bucket",
            );
            metrics::counter!("embyr_rate_limit_pg_error_total").increment(1);
            // Fall through to in-process — identical fallthrough shape to the timeout arm below.
        }
        Err(_timeout) => {                                          // unchanged
            tracing::warn!(
                project_id = project_id,
                "rate_limit_pg_timeout: Postgres check exceeded {}ms, falling back to in-process bucket",
                RATE_LIMIT_PG_TIMEOUT_MS,
            );
            metrics::counter!("embyr_rate_limit_pg_timeout_total").increment(1);
        }
    }
}
self.check_in_process(project_id)
```

Only one new match arm (`Ok(Err(pg_error))`) is added. The `Ok(Ok(...))` arm's binding changes from
`Ok(result_and_existed)` to `Ok(Ok(result_and_existed))` (one added nesting level, zero behavior
change) and the `Err(_timeout)` arm is untouched byte-for-byte. The final `self.check_in_process(...)`
call outside the `if let` is **exactly the existing line 219 — not duplicated, not modified.**

### Doc-comment correction (required, not optional)

The doc comment directly above `check_pg` (current lines 222-227) reads: *"On Postgres error, returns
`Ok(info)` with a synthetic full-capacity token count (fail-open behaviour: DB errors are not
penalised)."* This sentence describes the bug being fixed and must not survive the fix — it would
mislead the next reader into believing the current (broken) behavior is intentional. Replace with:
*"On Postgres error, returns `Err(sqlx::Error)` — the caller (`check_inner`) routes this to the
per-instance fallback (`check_in_process`), never to an unconditional allow."*

### What does NOT change (confirmed additive, not a rewrite)

- `check_in_process` (325-351): zero diff.
- The `Err(_timeout)` arm and its `rate_limit_pg_timeout_total` counter / `tracing::warn!` wording:
  zero diff (AC-RLFO-06).
- `RateLimiter::check`'s public signature (140-150) and `check_inner`'s own return type
  (`(Result<RateLimitInfo, RateLimitInfo>, bool)`, line 183-186): zero diff — the new `?`/`Result`
  nesting is fully absorbed inside `check_pg`/`check_inner`'s internal `match`, never surfaces past
  `check_inner`. Confirms the DISCUSS-locked System Constraint ("`RateLimiter::check`'s public
  signature does not change") holds under this exact design, not just in principle.
- The tolerated `INSERT ... ON CONFLICT DO NOTHING` error (`let _ = ...`, lines 277-284): zero diff —
  already inert (OQ-RLFO-01), the response was already decided (`Ok(RateLimitInfo{...}), false)`)
  before this INSERT runs; its own failure changes nothing about this feature's allow/deny outcome.

## Wave: DESIGN / [REF] Open Question Resolutions

- **OQ-RLFO-01 (fallback-trigger granularity) — RESOLVED: every `sqlx::Error` variant routes to
  `check_in_process`, no narrowing by variant.** Matches ADR-015's own original step-2d wording ("hard
  DB error," no carve-out). A variant-based allowlist/denylist would add a second, undocumented
  classification surface with no requirement driving it — YAGNI. The one call site DISCUSS flagged as
  a candidate for narrower handling (the tolerated INSERT error) is confirmed inert and already
  excluded by construction (it doesn't reach the `?`-propagating call sites at all).
- **OQ-RLFO-02 (metric naming) — RESOLVED: new, distinct counter `embyr_rate_limit_pg_error_total`**,
  separate from the existing `embyr_rate_limit_pg_timeout_total`. Rationale: "the DB was slow" and "the
  DB errored" are operationally distinct signals for Sam Chen diagnosing an incident via `/metrics`
  (matches DISCUSS's own lean). This is the one place this design **adds** observable behavior beyond
  ADR-015's original text, which said hard errors "do not increment this counter" (ADR-015 line 208,
  written when the step-2d arm was aspirational/unimplemented) — recorded as an ADR-015 addendum below,
  not a new ADR, since it changes one operational detail, not the architecture.
- **OQ-RLFO-03 (log fidelity + ADR-075 boundary) — RESOLVED: a second, distinct `tracing::warn!`
  (`rate_limit_pg_error`) fires for the error path, with the real `sqlx::Error` embedded via `error =
  %pg_error`.** The existing `rate_limit_pg_timeout` line's wording is untouched. **ADR-075 boundary
  confirmed explicitly (not assumed): both `tracing::warn!` lines in this function are server-side
  structured logs, never constructed into a `tonic::Status` returned to any gRPC/REST caller** — grep
  of `check_pg`/`check_inner` confirms neither function ever touches `tonic::Status` or a response
  builder; the caller only ever sees the domain-level `RateLimitInfo`/rejection via `check()`'s existing
  `Result<RateLimitInfo, RateLimitInfo>`. ADR-075's client-facing sanitization sweep does not apply to
  this log line.

## Wave: DESIGN / [REF] Earned Trust — Fault-Injection Confirmation

No new port, adapter, or component boundary is introduced by this fix (confirmed below, § C4/Diagram
Note) — `check_pg` remains a private internal method of the same existing `RateLimiter` struct, talking
to the same existing `pg_pool`. There is therefore no new `probe()`-style construct to design. The
applicable form of Earned Trust here is: **does the design get empirically proven against a REAL
Postgres failure, not a mocked one, before it is trusted?** DISCUSS's own Walking Skeleton Strategy
(already locked, not re-litigated) answers yes: the acceptance test forces a genuine, fast-returning
Postgres error via `pg_terminate_backend` mid-check or an immediately-failing pool acquire (real
fault injection, matching this workspace's own established pattern in
`pr08_realtime_listener_reconnect.rs`), then asserts the request is evaluated against
`check_in_process`'s bounded bucket. This is the direct analogue of a `probe()` behavioral gate for
this fix's one external dependency (the shared Postgres pool) — DESIGN confirms the mechanism is
already specified and does not need a new one. Additionally, per this repo's own CLAUDE.md ("Mutation
Testing Strategy: per-feature"), a `cargo-mutants`-scoped run against `rate_limit.rs` after DELIVER
will independently prove the new `Ok(Err(pg_error))` arm and both `?`-propagation sites are actually
exercised by the test suite, not merely present in source — catching a regression to `.ok().flatten()`
that a compiler cannot catch by itself (Rust's type system enforces that `?` requires a `Result`
return type, but nothing prevents a future edit from re-introducing `.ok()` inside that `Result`; the
mutation-tested acceptance test is the enforcement layer for that specific regression).

## Wave: DESIGN / [REF] ADR Decision

**No new ADR. One short, dated addendum appended to `docs/product/architecture/adr-015-distributed-
rate-limiter-postgres.md`** (below its existing, unmodified `## Enforcement` section — the
immutable-supersede convention this session already established for small operational corrections,
e.g. `rate-limiter-project-id-validation`'s ADR-016 amendment note).

Justification: per § Reading Confirmation above, this feature does not make a new architectural
decision — ADR-015 already specifies `check_pg`'s exact return type
(`Result<Result<RateLimitInfo, RateLimitInfo>, sqlx::Error>`) and the exact step-2d routing
(`Ok(Err(sqlx_error)) → warn → in-process fallback`) that this feature implements. The ADR's decision
was correct; the 2026-08-08 DELIVER implementation of it was not. The addendum records two things: (1)
conformance — the code now matches what ADR-015 already said, no new decision was made to get there;
(2) the one genuine, small operational decision this feature DOES add beyond ADR-015's original text —
a new, distinct `embyr_rate_limit_pg_error_total` counter, where ADR-015's original "Metric" section
said hard errors increment no counter at all (§ OQ-RLFO-02). That one paragraph-level change is
addendum-sized, not new-ADR-sized: it doesn't add a component, change a failure mode, or introduce a
technology — it adds one Prometheus counter, following the exact convention ADR-016 already
establishes for every other rate-limit metric.

**Addendum text (to be appended to ADR-015):**

```markdown
## Addendum (2026-09-15) — Finding #20 Conformance Fix (`rate-limiter-fail-open`)

This ADR's own § "check_pg() internal method — return type explanation" (above) already specified
`check_pg`'s return type as `Result<Result<RateLimitInfo, RateLimitInfo>, sqlx::Error>` and step 2d
of the decision path as "`Ok(Err(sqlx_error))` (hard DB error) → log WARN → in-process fallback."
The 2026-08-08 implementation of this ADR did not build that arm: `check_pg` instead collapsed
`sqlx::Error` internally via `.ok().flatten()` (atomic UPDATE) and `.unwrap_or(false)` (EXISTS check),
so a real Postgres error was silently treated as "row absent," inserting a fresh full-capacity bucket
and unconditionally allowing the request — production-readiness-audit-2026-09-08 finding #20 (Medium,
Security).

`rate-limiter-fail-open` (2026-09-15) brings the code into conformance with this ADR's own
already-specified design. This is a bug fix against an existing decision, not a new decision, with
one exception: the § "Metric" section above states hard Postgres errors "do not increment this
counter" (`rate_limit_pg_timeout_total`) and are "logged at WARN level" only. This is superseded:
hard errors now additionally increment a new, distinct `embyr_rate_limit_pg_error_total` counter
(separate time series from the timeout counter), giving Sam Chen an operationally distinct signal for
"the DB errored" vs. "the DB was slow" via `/metrics` — consistent with ADR-016's existing metric
conventions. No other part of this ADR changes.
```

## Wave: DESIGN / [REF] External Integration Check

Postgres (the shared system DB) is an internal, first-party dependency of `embyr-server` — not a
third-party API, webhook, or OAuth provider. Per the contract-testing guidance (external integrations
= highest-risk boundary), this does not qualify: no consumer-driven contract test recommendation
applies to this feature. (For contrast: `WarpApiGatewayAdapter`-style third-party integrations do not
exist in this codebase; embyr-rs's only "external" dependency in this sense is Postgres itself, already
covered by this workspace's own testcontainers-based real-infrastructure integration-test convention,
not a contract-test tool.)

## Wave: DESIGN / [REF] C4 / Diagram Note

No C4 diagram update. This fix changes internal error-handling control flow inside one existing private
method (`check_pg`) of one existing component (`RateLimiter`, inside the `embyr-server` container). No
component is added, removed, or re-scoped; no container boundary changes; no new integration point is
introduced (the same `pg_pool: Option<Arc<PgPool>>` dependency, already documented in ADR-015's own
existing architecture narrative, is the only external dependency touched). A System Context or
Container diagram produced for this feature would be byte-for-byte identical to the one implied by
ADR-015 — reproducing it here would not carry new information. This judgment mirrors the "small enough
to not warrant re-diagramming" precedent already set by sibling findings #14/#16/#19 in this session
(none of which produced new C4 diagrams for the same reason: internal logic change, zero topology
change).

## Wave: DESIGN / [REF] Quality Gates

- [x] Requirements traced to components: AC-RLFO-01 through 06 all trace to the two call sites +
      `check_inner`'s routing match, § Code-Level Design.
- [x] Component boundaries: unchanged — no new component; `RateLimiter` remains a concrete struct per
      ADR-015's own still-valid "no port/trait interface" decision (not reopened by this feature).
- [x] Technology choices: none — zero new dependencies, zero new `embyr-core` types (constraint from
      DISCUSS § System Constraints, confirmed held).
- [x] Quality attributes: Reliability (bounded degradation, not unconditional allow) and Security
      (closes finding #20) both directly addressed; Performance unaffected (`?` propagation adds zero
      overhead vs. `.ok().flatten()` on the hot path); Observability extended (new counter, § OQ-RLFO-02).
- [x] Dependency-inversion compliance: N/A — this codebase's own ADR-015 already rejected a port/trait
      abstraction for `RateLimiter` (Alternative 4) for a still-valid reason (one implementation, no
      swap requirement); not reopened here.
- [x] C4 diagrams: explicitly assessed and justified as unnecessary, § C4/Diagram Note.
- [x] Integration patterns: unchanged — same `pg_pool` dependency, same fallback mechanism, only the
      trigger condition set is widened.
- [x] OSS preference: N/A — zero new dependencies.
- [x] AC behavioral, not implementation-coupled: confirmed — all 6 ACs (DISCUSS) describe observable
      allow/reject/fallback outcomes, never internal method names or match-arm structure.
- [x] External integrations: assessed, does not apply (§ External Integration Check).
- [x] Enforcement tooling: behavioral (real-Postgres-error acceptance test, DISCUSS-locked Walking
      Skeleton) + per-feature mutation testing (this repo's own CLAUDE.md convention) — no static-lint
      enforcement exists in this codebase for this class of regression; documented as a residual risk,
      not a gap this fix is expected to close (§ Earned Trust).
- [x] Peer review: pending, see below.

## Wave: DESIGN / Handoff Package (to DISTILL — acceptance-designer)

**Files requiring a change (DELIVER):**
1. `crates/embyr-server/src/middleware/rate_limit.rs` — `check_pg` (228-318, signature + two `?`
   propagation sites + `Ok(...)` wrapping of existing return values + doc-comment correction),
   `check_inner` (198-217, one new `Ok(Err(pg_error))` match arm). No other function in this file
   changes.

**Files confirmed to need NO change (blast radius, grep-verified):**
- `check_in_process` (325-351), the `Err(_timeout)` arm and its counter/log line, `RateLimiter::check`'s
  public signature (140-150), `check_inner`'s own return type (183-186) — all zero-diff, § What Does
  NOT Change.
- Every gRPC handler call site and `rest_rate_limit_middleware` — both consume only `check()`'s
  unchanged public `Result<RateLimitInfo, RateLimitInfo>`.
- `crates/embyr-core/*` — `RateLimitInfo` unmodified, zero IO-boundary impact.

**Documentation changes made this wave:**
- This feature-delta.md (this DESIGN section).
- `docs/product/architecture/adr-015-distributed-rate-limiter-postgres.md` — addendum appended (text
  above), ADR otherwise unmodified per immutable-supersede convention.

**For DISTILL:**
- Reuse the 6 UAT scenarios and 6 ACs from DISCUSS verbatim — this DESIGN did not change scope or add
  scenarios, only resolved OQ-RLFO-01/02/03 and confirmed the exact code shape.
- New acceptance test file needed for the error-injection scenarios (AC-RLFO-02/03/05) — this
  workspace's `tests/distributed_rate_limiting/acceptance/` directory, following the existing
  `b13_fallback_on_pg_failure.rs` naming/structure convention (that file covers the TIMEOUT path only,
  per its own header comment — confirmed a sibling, not a duplicate). AC-RLFO-01/04/06 are regression
  guards against already-existing tests (`b12_postgres_rate_limit.rs`, `b13_fallback_on_pg_failure.rs`)
  — DISTILL should confirm exact reuse vs. new-file placement.
- Error-injection mechanism per DISCUSS's own locked Walking Skeleton Strategy: `pg_terminate_backend`
  mid-query or an immediately-failing pool acquire — DISTILL picks the exact mechanism, both are valid
  per this design.
