# Feature Delta: preauth-db-amplification

## Wave: DISCUSS / [REF] Reading Confirmation

✓ `docs/product/production-readiness-audit-2026-09-08.md` read for finding #14 (High, Reliability),
confirmed verbatim: *"Pre-authentication 3-round-trip amplification against the shared system DB —
an unknown `project_id` triggers UPDATE + SELECT EXISTS + INSERT before `authenticate()` ever runs,
on the same pool auth/sessions/admin API depend on."* Cites
`crates/embyr-server/src/middleware/rate_limit.rs:203-255`. Also read finding #20 (Medium, Security,
same file, already logged separately): *"Rate limiter fails open on database errors and conflates
'DB error' with 'allowed' (`.ok().flatten()` discards the error, `.unwrap_or(false)` on the
exists-check). An attacker who degrades the system DB (see #14) turns off rate limiting globally as a
side effect."* Confirmed #20 is explicitly out of scope for this feature — see § Out of Scope.

✓ `crates/embyr-server/src/middleware/rate_limit.rs` read in full (423 lines). Confirmed the exact
3-round-trip path lives in `check_pg`'s `None` branch (lines 264-293, current file): when the atomic
`UPDATE ... RETURNING tokens` (lines 238-253) matches zero rows, the code runs `SELECT EXISTS(...)`
(lines 267-272) to distinguish "genuinely rate-limited" from "row absent," and if absent, runs
`INSERT INTO rate_buckets ... ON CONFLICT DO NOTHING` (lines 277-283) before returning an
allowed/full-capacity result. A never-provisioned or garbage `project_id` deterministically hits all
three statements on **every single request** — this is not a one-time cold-start cost, it repeats for
every distinct never-provisioned string an attacker sends. Confirmed this runs against `pg_pool`, the
same `sqlx::PgPool` documented (`docs/SPEC.md`, ADR-015) as the shared **system** database — the same
pool auth (`admin_accounts`), sessions, and the admin API depend on, not a per-tenant customer
database.

✓ `crates/embyr-server/src/grpc/handler.rs` read (lines 1258-1296, representative of the ~15 call
sites `rate_limiter.check()` has in this file). Confirmed the exact pre-auth ordering finding #14
names: `handle_get_document` calls `Self::extract_project_id(&name)` (format check only — see below),
then `self.rate_limiter.check(&project_id).await` at line 1267, and only AFTER that returns `Ok` does
it call `self.authenticate(&project_id, &api_key).await?` at line 1272. A caller with a garbage,
never-authenticated `project_id` reaches the rate limiter's 3-round-trip path on every request, before
`authenticate()` has any chance to reject it. Confirmed this ordering repeats at all ~15 call sites
(`handler.rs:1267,1570,1809,2038,2264,2455,2522,2694,2734,2823,2957,3005,3293,3534,3675`) plus the
REST middleware (`rate_limit.rs:377-393`, `rest_rate_limit_middleware`, gated on the same
`rate_limiter.check()`).

✓ `crates/embyr-server/src/grpc/handler.rs:99-107` (`extract_project_id`) read. Confirmed the ONLY
validation applied before the `project_id` reaches `rate_limiter.check()` is: the resource name has
shape `projects/{pid}/...` AND `{pid}` is non-empty. No charset, length, or format check. A caller can
supply any non-empty, slash-free string — arbitrary case, symbols, whitespace, 1000+ characters — and
it reaches `check_pg` verbatim.

✓ `docs/SPEC.md:73` read: `project_id` "Must match `^[a-z][a-z0-9-]{0,62}$` (lowercase RFC 1123
hostname label: starts with a letter, max 63 chars, lowercase letters / digits / hyphens only)" — this
is the actual provisioning-time format for every REAL project. `crates/embyr-core/src/domain/
project.rs` read in full: `ProjectId::new`/`is_valid_project_id` (lines 7-17, 102-112) already
implements exactly this regex as a pure, `embyr-core`-resident (zero IO), already-unit-tested function
(11 existing unit tests, lines 24-100) — confirmed by `provision.rs:165`'s own comment ("same regex as
`ProjectId::new`"). This function is NOT currently called anywhere on the `rate_limiter.check()`
pre-auth path.

✓ `docs/evolution/2026-09-09-rate-limiter-project-id-validation.md` read in full (closed finding #2,
same file). Confirmed its own explicit scope note: *"Finding #14 (unrelated, same `rate_limit.rs`
file) was explicitly named as related-but-separate and not touched by this fix."* That feature added
the `known_existing`/`UNCONFIRMED_PROJECT_LABEL` signal purely to bound a Prometheus label — it did
NOT add any pre-DB format check and does NOT reduce round-trip count. Confirmed no overlap or
duplication risk with this feature.

## Wave: DISCUSS / [REF] Investigation Findings

### Investigation 1 — the 3-round-trip cost is deterministic and repeats per-request, not a one-time cold start

`check_pg`'s `None` branch is reached whenever the atomic `UPDATE` matches zero rows — true for BOTH
(a) a project that predates migration 0018 (a real, rare, one-time case the `INSERT ... ON CONFLICT DO
NOTHING` was designed for) and (b) any string that is not, and will never become, a real
`rate_buckets.project_id`. Case (b) has no cold-start property: the `INSERT` never creates a
persistent row an attacker's NEXT distinct garbage string would match, so a script cycling through N
distinct unknown strings costs 3×N round trips against the shared system pool, unboundedly, for as
long as the attacker sends traffic. This is the amplification the audit names — 3x load per
pre-auth, zero-cost-to-generate request, sustained indefinitely.

### Investigation 2 — a cheap, already-existing, IO-free syntactic check is the right-shaped fix; DISCUSS does not lock the exact wiring

The provisioning-time format (`^[a-z][a-z0-9-]{0,62}$`, `docs/SPEC.md:73`) is already implemented,
already tested, and already IO-free as `embyr_core::domain::project::ProjectId::new`/
`is_valid_project_id`. Any `project_id` failing this check cannot possibly correspond to any REAL
project, ever — provisioning itself would have rejected it. Rejecting such requests before they reach
`rate_limiter.check()`/`check_pg` eliminates all 3 round trips for that request, using a pure
in-memory string check with no new dependency, no schema change, and no new domain logic (Elephant
Carpaccio ladder: reuse, don't reinvent). This closes the DOMINANT, attacker-cheap slice of the
vector: an attacker generating random garbage strings at volume will, by construction, almost never
produce a string matching the charset by chance for any input longer than a few characters.

**What this does NOT close** (see Investigation 3): a syntactically valid but never-provisioned
`project_id` (e.g., a real-looking but non-existent name) still falls through to the existing 3
round trips — the syntactic check cannot distinguish "well-formed and real" from "well-formed and
never provisioned" without a DB lookup, which is the exact cost being eliminated. DISCUSS names this
as an accepted, documented residual (AC-05), not a gap this feature silently leaves unaddressed.

**DISCUSS's own recommendation**: reuse `ProjectId::new`/`is_valid_project_id` from `embyr-core`
directly (visibility is already `pub`) rather than duplicating the regex/charset logic in
`embyr-server`. This is named as **OQ-PDA-01** for DESIGN to confirm the exact call site and error
shape — DISCUSS locks only the observable outcome (zero round trips for syntactically-invalid input),
not the code path.

### Investigation 3 — well-formed-but-unprovisioned project_ids are a real, smaller residual risk; explicitly out of DISCUSS's required-outcome scope, not silently ignored

An attacker who specifically crafts candidate strings matching `^[a-z][a-z0-9-]{0,62}$` (e.g. guessing
real-looking project names) still reaches the full 3-round-trip path, because a syntactic check alone
cannot know whether a well-formed string is provisioned without a DB lookup — the same structural
argument the closed `rate-limiter-project-id-validation` feature (finding #2) already made about
existence not being decidable syntactically. This residual is smaller in practice: the charset-valid
string space an attacker must search is enormously larger than "any byte string," so per-request
attacker cost to hit this residual path is much higher than the zero-effort garbage case this feature
closes. DISCUSS does not treat closing this residual (e.g., a bloom filter or existence cache) as
in-scope — that would be a materially larger design (new shared cache, invalidation on
provision/delete) for a residual the audit itself does not separately call out as its own finding.
Named as **OQ-PDA-02** for DESIGN to note as a possible follow-up, not built here.

### Investigation 4 — this feature must not touch finding #20's fail-open behavior

Finding #20 lives in the SAME `check_pg` function this feature touches (`.ok().flatten()` at line 253
and 307, `.unwrap_or(false)` at line 273) — DB-error handling that fails open (treats a DB error as
"allowed"). This feature's fix (reject syntactically-invalid input before calling `check_pg` at all)
does not require touching any of those three lines, and DISCUSS explicitly forbids doing so as part of
this fix. Confirmed no unavoidable overlap: the new syntactic check is a guard BEFORE `check_inner`/
`check_pg` is ever called, not a modification of what happens once inside it.

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: **P2 Sam Chen (Service Operator / Platform Engineer)** — the operator who runs the shared
`embyr-server` deployment and depends on the system Postgres pool staying healthy for auth, sessions,
and the admin API, regardless of what unauthenticated traffic the public gRPC/REST surface receives.

**Job**: **JOB-11 `fair-multitenancy`**, reused. JOB-11's own functional dimension already promises
"one project cannot consume more than its fair share" of shared resources via the very
`rate_buckets`/`RateLimiter` mechanism this finding lives inside. This finding is a direct extension
of that same promise to its own enforcement path: today, an entity that isn't even a real tenant (a
garbage, never-authenticated `project_id`) can consume 3x the DB load of a real tenant's request
against the SAME shared pool the fairness mechanism itself depends on — undermining JOB-11's own
guarantee at its source rather than at the data plane JOB-11 already protects. This mirrors this
session's own established pattern of JOB-11 being extended for closely-related concerns on the same
file without becoming a new job (see `docs/product/jobs.yaml` JOB-11 NOTE entries for
`card-payments-backend` and `firestore-malformed-filter-shape-validation`).

**Candidates considered and rejected**:
- **JOB-12 (`observability`, P2 Sam Chen)** — about Prometheus metrics/dashboards for diagnosing
  production behavior. Rejected: this finding is about eliminating unnecessary DB round trips
  themselves, not about observing them; `rate-limiter-project-id-validation` (finding #2) already used
  JOB-12 for the metric-cardinality angle of this same file, and that feature explicitly named finding
  #14 as separate. No metrics/dashboard work is needed here.
- **JOB-13 (`production-deployment`, P2 Sam Chen)** — about `docker run`/CI/startup-configuration
  readiness. Rejected: this is a runtime request-handling behavior gap, not a
  deployment-configuration or CI gap.
- **`infrastructure-only`** — considered because the fix is a small, backend-only guard clause with no
  new user-facing surface. Rejected: a real persona (Sam Chen) makes a real decision with a real,
  observable output — see § Elevator Pitch below — so this qualifies as a JOB-11-traced story, not an
  infrastructure-only one, per Dimension 0 of the review criteria (a story enabling no user decision
  is infrastructure; this one does).

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

- Feature type: **Backend** — a reliability/DoS-adjacent hardening fix on an existing, already-shipped
  pre-auth code path. No new user-facing surface, no UI, no journey artifact needed (mirrors this
  session's established precedent for this class of finding:
  `rate-limiter-project-id-validation`, `admin-signin-hardening`).
- Scope: **single, well-localized finding** (#14 only). Finding #20 (fail-open-on-DB-error, same file)
  is explicitly OUT OF SCOPE per the task's own framing and confirmed by Investigation 4 — not
  bundled, not touched.
- JTBD: reuse an existing job — **JOB-11 (`fair-multitenancy`)**, not a new job (§ Persona & Job).
- Walking Skeleton: **Yes** — a real flood of garbage-`project_id` requests against a real running
  `embyr-server` and a real Postgres backend, proving zero additional DB round trips are generated,
  while a real known-project request remains unaffected.
- UX Research Depth: **None** — pre-auth backend request-handling fix; no emotional arc, no journey
  YAML, no TUI mockup applies (nothing user-facing changes for legitimate callers).

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

**Signals checked**: >10 user stories? No (1). >3 bounded contexts/modules? No — one file
(`crates/embyr-server/src/middleware/rate_limit.rs`), one existing pure function reused from
`embyr-core` (read-only import, no `embyr-core` changes). Walking skeleton >5 integration points? No
(2: the gRPC/REST request entry point, and the shared system Postgres pool whose absence of new
queries is being proven). Estimated effort >2 weeks? No — a guard clause using an already-implemented,
already-tested validator; well under 1 day. Multiple independent user outcomes? No — one outcome
(garbage `project_id` floods stop costing 3 DB round trips each).

**Verdict: PASS.** One right-sized story; no split needed.

## Wave: DISCUSS / [REF] System Constraints

- `embyr-core` remains IO-free and is not modified by this feature — `ProjectId::new`/
  `is_valid_project_id` already exists there, already pure, already tested; this feature only adds a
  NEW caller inside `embyr-server`, per OQ-PDA-01.
- Finding #20 (rate limiter fails open on DB errors, same file, `.ok().flatten()`/`.unwrap_or(false)`
  in `check_pg`) is explicitly OUT OF SCOPE — zero lines of that logic are touched by this feature
  (Investigation 4, AC-04).
- The observable rejection CONTRACT for a garbage `project_id` (eventual `Unauthenticated`/
  `InvalidArgument` outcome once `authenticate()` or existing validation runs) must not change for any
  caller — only the DB round-trip count on the way there is in scope.
- A syntactically-valid-but-never-provisioned `project_id` is an explicit, accepted residual
  (Investigation 3, AC-05) — this feature does not claim to close that narrower case.
- Exact call-site wiring (which function gains the guard, exact error path/status returned) is a
  DESIGN choice (OQ-PDA-01) — DISCUSS locks only the observable, testable outcome: zero DB round
  trips for syntactically-invalid `project_id` values.

## Wave: DISCUSS / [REF] User Stories

### US-01: The Shared System Database Stays Protected From Pre-Authentication Garbage-Project-ID Floods

**job_id**: JOB-11 | **Release**: 1 (Walking Skeleton) | **Persona**: P2 Sam Chen

#### Elevator Pitch
**Before**: Any caller — including one with no valid credentials — can send a `GetDocument` gRPC call
(or the equivalent REST/gRPC-Web route) to embyr-server's public `:8080`/`:8081` surface with a
`project_id` that will never match a real project. Today, the caller still eventually receives the
correct `Unauthenticated` rejection, but not before `rate_limit.rs`'s `check_pg` runs an `UPDATE`, a
`SELECT EXISTS`, and an `INSERT` against the shared system Postgres pool — 3 round trips per request,
repeatable indefinitely by cycling through distinct garbage strings, against the same pool auth,
sessions, and the admin API depend on.
**After**: the identical `GetDocument` call with the identical garbage `project_id`, sent to the same
public gRPC/REST endpoint, still returns the exact same `Unauthenticated` rejection to the caller — but
Sam Chen, watching the shared system Postgres pool's connection/query-count on the existing Prometheus
`/metrics` endpoint (JOB-12's own admin-port dashboard), sees zero additional queries land on that pool
for the flood, because the request is rejected before `check_pg` is ever reached.
**Decision enabled**: Sam Chen can size the shared system database pool's headroom for real
auth/session/admin traffic without budgeting for an unauthenticated attacker's ability to multiply
load 3x per garbage request, and can show a security reviewer the pool's own connection/query-count
metric staying flat under a synthetic garbage-`project_id` flood — closing finding #14 with observable
evidence, not an unaddressed gap.

#### Who
- Sam Chen (P2) | Service Operator / Platform Engineer running the shared `embyr-server` deployment |
  Needs the shared system database pool to stay available for real auth/session/admin traffic
  regardless of how much unauthenticated garbage traffic the public data-plane surface receives.

#### Solution
Reject requests whose `project_id` fails the same syntactic validation every real project's ID is
already required to pass at provisioning time (`embyr_core::domain::project::ProjectId`/
`is_valid_project_id`), before the request reaches `RateLimiter::check`/`check_pg`'s 3-round-trip
path. Exact call-site wiring is a DESIGN decision (OQ-PDA-01).

#### Domain Examples

**Example 1 (Happy Path — regression guard)**: Fernbank Analytics, an existing, provisioned project
(`project_id = "fernbank-analytics"`, has a `rate_buckets` row), sends a normal `GetDocument` request.
The request is unaffected by this fix — its `project_id` passes the syntactic check, and the existing
single-round-trip `UPDATE ... RETURNING tokens` path runs exactly as it does today.

**Example 2 (Attack/Boundary — the amplification vector this fix closes)**: An unauthenticated script
sends `GetDocument` requests at a sustained rate of 10,000/minute, cycling through garbage
`project_id` values like `"DROP_TABLE_123"`, `"' OR 1=1--"`, `"Not_Lowercase"`, and a 500-character
string — none matching the real project-naming format. Today, every one of these triggers 3 round
trips against the shared system database before `authenticate()` rejects it. After this fix, every one
is rejected with zero queries reaching that database.

**Example 3 (Edge/Boundary — documented residual, not silently claimed as fixed)**: A caller submits
`project_id = "acme-corp-2026"` — syntactically valid (matches `^[a-z][a-z0-9-]{0,62}$`) but never
provisioned. This fix does not change behavior here: the existing 3-round-trip migration-compatibility
path still runs, because a syntax check alone cannot distinguish "well-formed and real" from
"well-formed and never provisioned." This is Investigation 3's named, accepted residual.

**Example 4 (Error/Boundary — pre-existing, unaffected boundary)**: A caller submits an empty
`project_id` (`projects//documents/...`). This is already rejected by `extract_project_id` before ever
reaching the rate limiter today — zero round trips already, unaffected by and not claimed as part of
this fix.

#### UAT Scenarios (BDD)

```gherkin
Scenario: A known project's request is unaffected by the amplification fix
  Given Fernbank Analytics is a provisioned project with an existing rate-limit bucket
  When Fernbank Analytics sends a GetDocument request using its own project_id
  Then the request is rate-limit-checked using the existing single-round-trip path
  And the request proceeds to authentication exactly as before this fix

Scenario: Garbage project_id floods no longer touch the shared database
  Given an unauthenticated caller sends GetDocument requests using project_id values that could never
    match any real project's naming rules, such as "DROP_TABLE_123", "' OR 1=1--", and a 500-character string
  When 10,000 such requests are sent within one minute
  Then every request is rejected before any query reaches the shared system database
  And the shared system database's query and connection count show no measurable increase from this flood

Scenario: Rejected garbage requests fail exactly as before, just without the database cost
  Given a caller sends a request with a project_id that is not a syntactically valid project identifier
  When the request is rejected
  Then the caller receives the same eventual rejection outcome they would have received before this fix
  And no legitimate caller observes any behavior change

Scenario: A well-formed but unprovisioned project_id remains a documented edge case
  Given a project_id that matches the valid project-naming pattern but has never been provisioned
  When a request using that project_id is sent
  Then the existing migration-compatibility round-trip path still runs, unchanged by this fix
  And this is a deliberately accepted, documented boundary, not a silent regression

Scenario: The rate limiter's existing fail-open-on-database-error behavior is untouched
  Given the rate limiter's pre-existing behavior of treating a database error as an allowed request (finding #20, tracked separately)
  When this fix is applied
  Then that fail-open behavior is not modified by this feature
  And no line of that separate, already-tracked finding's logic is changed

@property
Scenario: Shared database load stays flat under a sustained garbage-project_id flood
  Given the shared system Postgres pool also serves authentication, session, and admin API traffic
  Then a sustained flood of syntactically-invalid, unauthenticated project_id requests produces zero additional queries against that pool
```

#### Acceptance Criteria
- [ ] AC-PDA-01: a request whose `project_id` does not match the provisioning-time format
      (`^[a-z][a-z0-9-]{0,62}$`) is rejected before any query reaches the shared system Postgres pool —
      zero `UPDATE`/`SELECT`/`INSERT` round trips for that request.
- [ ] AC-PDA-02 (regression guard): a request whose `project_id` is syntactically valid and belongs to
      an already-provisioned project is unaffected — identical round-trip behavior to today.
- [ ] AC-PDA-03 (regression guard): the final rejection outcome (status/response shape) returned to a
      caller with a syntactically-invalid `project_id` is unchanged from today — only the DB
      round-trip count changes, not the observable contract.
- [ ] AC-PDA-04 (regression guard): finding #20's fail-open-on-DB-error behavior in `check_pg` is not
      modified by this feature — zero lines of that logic path are touched.
- [ ] AC-PDA-05: a syntactically-valid-but-never-provisioned `project_id` is explicitly documented as
      out of scope for this fix's round-trip reduction — not silently regressed, not silently claimed
      as fixed.
- [ ] AC-PDA-06: under a sustained flood of syntactically-invalid `project_id` requests, the shared
      system Postgres pool shows no measurable increase in query/connection load attributable to this
      path — proven by a real load-style or query-count-assertion test, not by code inspection alone.

#### Outcome KPIs
- **Who**: Sam Chen (Service Operator/Platform Engineer) and, transitively, every real project sharing
  the system database pool with auth/session/admin traffic.
- **Does what**: the shared system Postgres pool no longer receives amplified round trips for
  pre-authentication requests carrying a syntactically-invalid, never-real `project_id`.
- **By how much**: from 3 DB round trips per garbage/unauthenticated request (confirmed baseline,
  § Reading Confirmation) to 0 round trips for the syntactically-invalid subset of that traffic — the
  dominant, zero-cost-to-generate share of the vector. Syntactically-valid-but-unprovisioned IDs remain
  at 3 round trips (AC-PDA-05, documented residual).
- **Measured by**: AC-PDA-06 — a query-count or connection-load assertion in an integration test run
  against a real Postgres instance, before and during a synthetic garbage-`project_id` flood.
- **Baseline**: 3 round trips per garbage `project_id` request today, confirmed by direct code reading
  of `check_pg`'s `None` branch (§ Reading Confirmation, Investigation 1).

#### Technical Notes
- Reuses `embyr_core::domain::project::ProjectId::new`/`is_valid_project_id` — already implemented,
  already unit-tested, IO-free. No new Cargo dependency, no schema change, no `embyr-core` edit
  required (OQ-PDA-01: DESIGN confirms exact call-site wiring and error path).
- OQ-PDA-02 (closing the smaller well-formed-but-unprovisioned residual, e.g. via a bloom
  filter/existence cache) is named as a possible future follow-up, explicitly not built by this
  feature (Investigation 3).
- Depends on nothing outside `embyr-server`; `embyr-core`'s existing `ProjectId` type is read, not
  modified; `embyr-agent` is untouched.
- Finding #20 (same file, fail-open-on-DB-error) must remain byte-for-byte unchanged by this feature
  (AC-PDA-04).

## Wave: DISCUSS / [REF] Out of Scope

- **Finding #20** (rate limiter fails open on database errors, same file) — explicitly a separate,
  already-logged finding per the task's own framing; not fixed, not touched, confirmed via
  Investigation 4.
- **Closing the well-formed-but-unprovisioned `project_id` residual** (Investigation 3, OQ-PDA-02) —
  would require a materially larger mechanism (existence cache/bloom filter with invalidation on
  provision/delete); not warranted by the audit's own finding, which names "unknown `project_id`" in
  the sense of never-matching-the-format garbage, not real-looking guesses.
- **Any redesign of the `RateLimiter`/`rate_buckets` mechanism itself** — this is a narrow guard added
  BEFORE the existing mechanism is invoked, not a change to `check`/`check_inner`/`check_pg`/
  `check_in_process`'s own logic.
- **Rate-limiting or validating any other pre-auth field** (e.g. `api_key` format) — this feature is
  scoped to the `project_id`-driven 3-round-trip amplification finding #14 names, nothing broader.

## Wave: DISCUSS / [REF] Walking Skeleton Strategy

US-01's own property scenario (shared database load stays flat under a sustained garbage-`project_id`
flood) is the walking skeleton: a real flood of syntactically-invalid `GetDocument` requests against a
real running `embyr-server` and a real Postgres backend, proving zero additional queries reach the
shared system pool — not a unit-test-only or mocked proof — while a real known-project request
(Example 1/Scenario 1) remains provably unaffected in the same run.

## Wave: DISCUSS / [REF] Driving Ports

Any Firestore data request naming a `project_id` in its resource path, pre-authentication — the ~15
gRPC call sites in `crates/embyr-server/src/grpc/handler.rs` that call `self.rate_limiter.check(...)`
(e.g. `GetDocument` at `handler.rs:1267`), plus `rest_rate_limit_middleware`
(`rate_limit.rs:377-393`) for REST/gRPC-Web routes carrying a `:project_id` path parameter. No new
endpoint — this is a guard on requests to existing, already-shipped entry points.

## Wave: DISCUSS / [REF] Pre-requisites

- None blocking. `embyr_core::domain::project::ProjectId::new`/`is_valid_project_id` already exists,
  already `pub`, already unit-tested — no new Cargo dependency, no migration, no schema change needed
  for the guard itself.

## Wave: DISCUSS / [REF] Definition of Ready Validation

### DoR Checklist (9-item hard gate)

| DoR Item | US-01 |
|---|---|
| 1. Traces to a job_id | PASS — JOB-11, reused, with 3 candidate alternatives explicitly reasoned against (§ Persona & Job) |
| 2. Elevator Pitch complete | PASS — Before/After/Decision-enabled, real entry point (public gRPC/REST Firestore data API, e.g. `GetDocument`), observable output (shared DB query/connection count does not move under a garbage flood) |
| 3. 3+ domain examples, real data | PASS — Fernbank Analytics (known project), a garbage-flood script with concrete example strings, "acme-corp-2026" residual case, empty-`project_id` pre-existing boundary |
| 4. UAT in Given/When/Then (3-7) | PASS — 6 scenarios (5 regular + 1 `@property`) |
| 5. AC derived from UAT | PASS — AC-PDA-01 through 06, each traced to a scenario above |
| 6. Right-sized (1-3 days, 3-7 scenarios) | PASS — single guard clause reusing an existing pure function; well under 1 day |
| 7. Technical notes identify constraints | PASS — OQ-PDA-01 (call-site wiring) and OQ-PDA-02 (residual follow-up) named, not locked; finding #20 boundary named |
| 8. Outcome KPIs with numeric target | PASS — 3 round trips → 0 round trips for the syntactically-invalid subset, measured by a real query-count/load test |
| 9. Prior-wave artifacts reconciled | PASS — audit finding #14/#20, `rate_limit.rs`, `handler.rs` call sites, `docs/SPEC.md:73`, `embyr-core/src/domain/project.rs`, `docs/evolution/2026-09-09-rate-limiter-project-id-validation.md`, `docs/product/jobs.yaml` JOB-11/12/13 all directly informed this feature's shape |

### DoR Status: **PASSED**

## Wave: DISCUSS / [REF] Wave Decisions Summary

### Key Decisions
- [D1] Persona/job: **P2 Sam Chen / JOB-11 (`fair-multitenancy`)**, reused — not JOB-12 (observability,
  already used for the metric-cardinality angle of this same file), JOB-13 (deployment/CI, unrelated
  dimension), or `infrastructure-only` (a real persona makes a real decision here) (§ Persona & Job).
- [D2] Finding #14 (this feature) and finding #20 (fail-open-on-DB-error, same file) are treated as
  **two separate, non-bundled findings** per the task's own framing — this feature touches zero lines
  of #20's logic (§ Investigation 4, AC-PDA-04).
- [D3] The fix reuses `embyr_core::domain::project::ProjectId::new`/`is_valid_project_id` — an
  already-implemented, already-tested, IO-free provisioning-format check — rather than inventing new
  validation logic, named OQ-PDA-01 for DESIGN to confirm exact call-site wiring (§ Investigation 2).
- [D4] Well-formed-but-never-provisioned `project_id` values remain a documented, accepted residual
  (OQ-PDA-02) — not closed by this feature, not silently claimed as fixed (§ Investigation 3).
- [D5] Single story, no split — one bounded context, one file touched in `embyr-server`, well under 1
  day of effort (§ Scope Assessment).

### Requirements Summary
- Primary need: a request carrying a `project_id` that can never correspond to a real project (fails
  the same format check provisioning already enforces) must be rejected before it costs the shared
  system database pool any query — with zero behavior change for real, known projects.
- Constraint: finding #20's fail-open-on-DB-error behavior in the same file must not be touched by this
  fix; the eventual rejection contract for garbage input must not change, only its DB cost.
- Success looks like: AC-PDA-01 through AC-PDA-06 all passing against a real running `embyr-server` and
  a real Postgres backend, with zero regression to any existing rate-limiter or Firestore-handler test.

### Handoff Package (to DESIGN — solution-architect)
- This feature-delta.md (DISCUSS section) — job grounding, blast-radius trace (all ~15 call sites),
  amplification-mechanism investigation, 1 user story with 6 UAT scenarios and 6 acceptance criteria.
- Confirmed exact file/line target for DESIGN: guard to add before `RateLimiter::check`/`check_inner`
  is invoked (all call sites in `crates/embyr-server/src/grpc/handler.rs` and
  `rest_rate_limit_middleware` in `crates/embyr-server/src/middleware/rate_limit.rs:377-393`), using
  `embyr_core::domain::project::ProjectId::new`/`is_valid_project_id` (already `pub`, already tested).
- Open DESIGN-level choices: OQ-PDA-01 (exact call-site wiring — a shared helper vs. per-call-site
  guard, given ~15 call sites; error/status shape returned) and OQ-PDA-02 (named-only, not built —
  whether a future feature should close the well-formed-but-unprovisioned residual).
- Flagged, out-of-scope, related finding for DESIGN's own awareness (not this feature's build target):
  finding #20 (fail-open-on-DB-error, same file) — must remain untouched by this feature's diff.

## Wave: DESIGN / [REF] Reading Confirmation

✓ `crates/embyr-server/src/grpc/handler.rs` read in full for every `rate_limiter.check()` call site
(15 confirmed: lines 1267, 1570, 1809, 2038, 2264, 2455, 2522, 2694, 2734, 2823, 2957, 3005, 3293,
3534, 3675). Traced each call site's `project_id`/`project_id_str` binding back to its source:
- **14 of 15** resolve through `Self::extract_project_id` (lines 99-107), either directly
  (`handle_get_document`, `handle_create_document`, `handle_begin_transaction`, `handle_commit`,
  `handle_batch_write`, `handle_rollback`, `handle_run_query`, `handle_batch_get_documents`,
  `handle_run_aggregation_query`, `handle_write` handshake) or transitively via
  `Self::parse_document_path` (line 113, used by `handle_update_document`/`handle_delete_document`)
  or `Self::parse_parent_prefix` (line 148, used by `handle_list_documents`/`handle_list_collection_ids`)
  — both of which themselves call `Self::extract_project_id` as their own first line.
- **1 of 15** (`handle_listen`, line 3534) resolves through a separate free function,
  `extract_project_id_from_listen_request` (lines 3900-3909), which duplicates
  `extract_project_id`'s shape but is not a method on `FirestoreService`.

**This means exactly TWO functions are the pre-`rate_limiter.check()` choke point for all 15 gRPC
call sites** — not 15 separate call sites needing 15 separate edits. This is a stronger reuse
opportunity than DISCUSS's own framing ("a shared helper vs. per-call-site guard, given ~15 call
sites") anticipated; no new helper is needed at all (see § Decision below).

✓ **Load-bearing discovery DISCUSS's own Technical Notes didn't name**: `authenticate()`
(`handler.rs:199-206`) — called immediately after `rate_limiter.check()` at all 15 sites — ALREADY
contains the exact fix:
```rust
let project_id = embyr_core::domain::project::ProjectId::new(project_id_str)
    .map_err(|e| Status::invalid_argument(e.to_string()))?;
```
This means **today's actual observable rejection for a charset-invalid `project_id` is already
`Status::invalid_argument`, not `Status::unauthenticated`** — DISCUSS's own Elevator Pitch text
("the caller still eventually receives the correct `Unauthenticated` rejection") is imprecise on
this one point: the real current behavior is `authenticate()`'s `ProjectId::new` check firing
*after* the 3 wasted round trips, not the `system_db` unauthenticated-lookup path. This does not
change any AC (AC-PDA-03 is about the outcome being unchanged, and it names no specific status
code) — it makes satisfying AC-PDA-03 exact and mechanical rather than approximate: **reuse the
identical `ProjectId::new(...).map_err(|e| Status::invalid_argument(e.to_string()))` expression,
verbatim, earlier**, guaranteeing byte-identical status code and message text, not merely
equivalent-shaped rejection.

✓ Confirmed via grep (`\.authenticate\(`) that `authenticate()` has exactly 15 call sites, one per
handler, always immediately after that handler's own `rate_limiter.check()` call using the identical
`project_id` variable — a 1:1 pairing across the whole file. No handler calls `authenticate()`
without having already called `rate_limiter.check()` with the same string.

✓ `crates/embyr-core/src/error.rs:22` read: `CoreError::InvalidArgument`'s `Display` impl is
`#[error("invalid argument: {0}")]`. Irrelevant to this fix directly (we reuse `e.to_string()`
verbatim, not the bare inner message), but confirms the message text produced is stable and
already covered by `ProjectId`'s own 11 existing unit tests (`embyr-core/src/domain/project.rs`).

✓ `crates/embyr-server/src/middleware/rate_limit.rs:377-393` (`rest_rate_limit_middleware`) read in
full. Confirmed it extracts `project_id` from the axum `Path` extractor directly — it does **not**
go through `handler.rs::extract_project_id` at all (different crate module, different parsing:
axum already splits the `:project_id` path segment, no `projects/{pid}/...` resource-name parsing
needed). This is a **third**, independent choke point requiring its own guard.

✓ Traced what `rest_rate_limit_middleware` currently allows through to, for the one route it gates
today (`accounts_bridge_dispatch`, `lib.rs:79`, dispatching `signInWithPassword`/`signUp`/
`sendOobCode`/`resetPassword`/`signInWithCustomToken`). Read
`crates/embyr-server/src/adapters/project_auth.rs:66-96` (`resolve_customer_db_adapter`) — confirmed
it ALSO already contains an equivalent guard:
```rust
let domain_project_id =
    ProjectId::new(project_id).map_err(|_| ProjectAuthError::ProjectNotFound)?;
```
Grepped `resolve_customer_db_adapter`/`ProjectAuthError::ProjectNotFound` across
`crates/embyr-server/src/rest/`: confirmed 3 of the 4 dispatched actions
(`sign_in_with_password.rs:104`, `sign_up.rs:164`, `reset_password.rs:143,260`) all map
`ProjectAuthError::ProjectNotFound` to the identical `pub(crate) fn invalid_api_key() -> Response`
(`rest/sign_up.rs:121-123`, `failure(StatusCode::UNAUTHORIZED, "INVALID_API_KEY")`).

**Peer review (iteration 1) caught a real gap here**: the 4th action, `sign_in_with_custom_token`
(`rest/sign_in.rs`), does NOT call `resolve_customer_db_adapter` and does NOT share the other
3 actions' rejection shape. Read `rest/sign_in.rs:84-111` in full: for a `project_id` matching no
row, it calls `state.system_db.get_client_identity_credential(&project_id)` (lines 101-105, returns
`Ok(None)` for any never-provisioned or charset-invalid string — no crash, just an empty lookup),
then returns `malformed_response()` (line 110):
```rust
fn malformed_response() -> (StatusCode, Json<SignInFailureResponse>) {
    (StatusCode::BAD_REQUEST, Json(SignInFailureResponse { reason: "MALFORMED_TOKEN" }))
}
```
i.e. **`400 {"reason":"MALFORMED_TOKEN"}`** — materially different from the other 3 actions'
**`401 {"error":{"code":401,...,"status":"INVALID_API_KEY"}}`**. An unconditional `invalid_api_key()`
guard in the REST middleware would have silently changed this one action's observable contract
(400→401, different body) — a real AC-PDA-03 violation this design's first pass missed. Fixed in
§ Decision below: the guard is now action-aware, not unconditional.

`malformed_response()` (`rest/sign_in.rs:63-70`) is currently a private `fn`, not `pub(crate)`.
Following the exact precedent already set for `invalid_api_key`/`hosted_identity_not_enabled`
(`rest/sign_up.rs:121,127`, both `pub(crate)` specifically so `sign_in_with_password.rs` and
`reset_password.rs` could reuse them), this design widens `malformed_response` to `pub(crate)` — a
visibility-only change, zero logic change, same pattern already used twice in this module family.

✓ Confirmed `crate::rest::sign_up::invalid_api_key` is `pub(crate)` (`rest/sign_up.rs:121`) and
`rest` is declared `pub mod rest;` in `lib.rs:10` — reachable from
`crate::middleware::rate_limit::rest_rate_limit_middleware` with a same-crate import, no
visibility change needed anywhere.

## Wave: DESIGN / [REF] Open Question Resolutions

**OQ-PDA-01 (exact call-site wiring) — RESOLVED: extend the two existing gRPC choke-point functions
in place; add one new guard to the REST middleware. No new shared helper function, no per-call-site
duplication.**

Four edits, four files, zero new functions (one existing function widened from private to
`pub(crate)` — a visibility change, not new logic):
1. `FirestoreService::extract_project_id` (`handler.rs:99-107`) — covers 14/15 gRPC call sites.
2. `extract_project_id_from_listen_request` (`handler.rs:3900-3909`) — covers the 15th (`Listen`).
3. `rest_rate_limit_middleware` (`rate_limit.rs:377-393`) — covers the REST/gRPC-Web
   `:project_id`-path-param route, branching on `action` (see revised code below — peer review
   iteration 1 caught that an unconditional guard would have broken `signInWithCustomToken`'s
   distinct rejection shape).
4. `rest/sign_in.rs:63` — widen `malformed_response` from private to `pub(crate)` so (3) can reuse
   it for the `signInWithCustomToken` action specifically.

Rejected alternative: a new shared `fn validate_project_id_or_reject(...)` helper called at all 15
gRPC sites individually. Rejected because the 15 call sites already collapse to 2 functions
(§ Reading Confirmation) — inserting a helper INSIDE those 2 functions, rather than calling a new
helper from all 15 original sites, is strictly less code and equally DRY. Ladder rung 2 (reuse
what's already the choke point) beats rung 7 (write a new abstraction) here.

**OQ-PDA-02 (well-formed-but-unprovisioned residual) — RESOLVED: not built, per DISCUSS's own
scope call; door left open at zero cost.** This design touches only the pre-`check_pg` entry guards
in 3 functions; it does not touch `check_pg`, `check_inner`, `check_in_process`, the `rate_buckets`
schema, or the `RateLimiter` struct itself. A future existence-cache/bloom-filter feature closing
this residual would compose additively (e.g., a new check inserted between this guard and
`rate_limiter.check()`) with zero rework of this feature's diff — the syntactic guard and any future
existence check are structurally independent layers, not alternatives competing for the same call
site.

## Wave: DESIGN / [REF] Decision — Exact Code Change

### 1. `crates/embyr-server/src/grpc/handler.rs:99-107` — `extract_project_id`

Before:
```rust
fn extract_project_id(name: &str) -> Result<&str, Status> {
    let mut parts = name.splitn(5, '/');
    match (parts.next(), parts.next()) {
        (Some("projects"), Some(pid)) if !pid.is_empty() => Ok(pid),
        _ => Err(Status::invalid_argument(format!(
            "invalid resource name: {name}"
        ))),
    }
}
```

After:
```rust
fn extract_project_id(name: &str) -> Result<&str, Status> {
    let mut parts = name.splitn(5, '/');
    match (parts.next(), parts.next()) {
        (Some("projects"), Some(pid)) if !pid.is_empty() => {
            // preauth-db-amplification (finding #14): reject a charset-invalid
            // project_id here, BEFORE rate_limiter.check()'s 3-round-trip
            // check_pg() path — not just before authenticate(). Verbatim reuse
            // of authenticate()'s own existing check (handler.rs:205-206) so
            // the observable Status/message is byte-identical to today's,
            // only earlier (AC-PDA-03). authenticate()'s own check is left in
            // place (defense-in-depth for any future caller of authenticate()
            // that skips this guard) — not removed by this fix.
            embyr_core::domain::project::ProjectId::new(pid)
                .map_err(|e| Status::invalid_argument(e.to_string()))?;
            Ok(pid)
        }
        _ => Err(Status::invalid_argument(format!(
            "invalid resource name: {name}"
        ))),
    }
}
```

Covers (transitively, with zero further edits): `handle_get_document`, `handle_create_document`,
`handle_update_document`/`handle_delete_document` (via `parse_document_path`),
`handle_list_documents`/`handle_list_collection_ids` (via `parse_parent_prefix`),
`handle_begin_transaction`, `handle_commit`, `handle_batch_write`, `handle_rollback`,
`handle_run_query`, `handle_batch_get_documents`, `handle_run_aggregation_query`, `handle_write`
handshake — 14 handlers, 0 of their own lines touched.

### 2. `crates/embyr-server/src/grpc/handler.rs:3900-3909` — `extract_project_id_from_listen_request`

Before:
```rust
fn extract_project_id_from_listen_request(msg: &ListenRequest) -> Result<String, Status> {
    let db = &msg.database;
    let mut parts = db.splitn(5, '/');
    match (parts.next(), parts.next()) {
        (Some("projects"), Some(pid)) if !pid.is_empty() => Ok(pid.to_string()),
        _ => Err(Status::invalid_argument(format!(
            "invalid database path in ListenRequest: {db}"
        ))),
    }
}
```

After:
```rust
fn extract_project_id_from_listen_request(msg: &ListenRequest) -> Result<String, Status> {
    let db = &msg.database;
    let mut parts = db.splitn(5, '/');
    match (parts.next(), parts.next()) {
        (Some("projects"), Some(pid)) if !pid.is_empty() => {
            // preauth-db-amplification (finding #14): same guard as
            // extract_project_id — see that function's comment.
            embyr_core::domain::project::ProjectId::new(pid)
                .map_err(|e| Status::invalid_argument(e.to_string()))?;
            Ok(pid.to_string())
        }
        _ => Err(Status::invalid_argument(format!(
            "invalid database path in ListenRequest: {db}"
        ))),
    }
}
```

Covers `handle_listen` (the 15th call site).

### 3. `crates/embyr-server/src/middleware/rate_limit.rs:377-393` — `rest_rate_limit_middleware`

Before:
```rust
pub async fn rest_rate_limit_middleware(
    State(rate_limiter): State<Arc<RateLimiter>>,
    Path(params): Path<HashMap<String, String>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let Some(project_id) = params.get("project_id") else {
        return next.run(request).await;
    };

    match rate_limiter.check(project_id).await {
        Ok(_info) => next.run(request).await,
        Err(info) => rest_rate_limit_rejection(project_id, &info),
    }
}
```

After (revised in peer review iteration 1 — see § Reading Confirmation: the guard must be
action-aware, `signInWithCustomToken` has its own distinct rejection shape):
```rust
pub async fn rest_rate_limit_middleware(
    State(rate_limiter): State<Arc<RateLimiter>>,
    Path(params): Path<HashMap<String, String>>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let Some(project_id) = params.get("project_id") else {
        return next.run(request).await;
    };

    // preauth-db-amplification (finding #14): reject a charset-invalid
    // project_id here, before rate_limiter.check()'s 3-round-trip path.
    // Reuses the SAME response the dispatched handler would itself have
    // produced for this exact case, per action, so the observable response
    // is unchanged, only earlier (AC-PDA-03):
    //   - signInWithCustomToken: rest/sign_in.rs's own malformed_response()
    //     (400 MALFORMED_TOKEN) — that handler never calls
    //     resolve_customer_db_adapter, so it does NOT share the other
    //     3 actions' shape.
    //   - every other action (signInWithPassword, signUp, sendOobCode,
    //     resetPassword, and any future action): resolve_customer_db_adapter's
    //     own ProjectId::new guard (adapters/project_auth.rs:76) maps to
    //     crate::rest::sign_up::invalid_api_key() (401 INVALID_API_KEY) —
    //     confirmed identical across all 3 of today's other dispatched actions.
    if embyr_core::domain::project::ProjectId::new(project_id.as_str()).is_err() {
        let action = params
            .get("action")
            .map(|s| s.trim_start_matches(':'))
            .unwrap_or_default();
        return if action == "signInWithCustomToken" {
            crate::rest::sign_in::malformed_response().into_response()
        } else {
            crate::rest::sign_up::invalid_api_key()
        };
    }

    match rate_limiter.check(project_id).await {
        Ok(_info) => next.run(request).await,
        Err(info) => rest_rate_limit_rejection(project_id, &info),
    }
}
```

Covers the REST/gRPC-Web `:project_id`-path-param route (currently: the `accounts:<verb>`
identity-bridge dispatch).

**Total production diff: 4 files, 3 functions extended + 1 visibility bump, ~30 lines added, 0 lines
deleted, 0 new functions, 0 new types, 0 new `CoreError` variant, 0 schema change, 0 new Cargo
dependency.**

## Wave: DESIGN / [REF] Contract Preservation Analysis (AC-PDA-03, AC-PDA-04)

**gRPC side**: today, a charset-invalid `project_id` flows past the old `extract_project_id`
(non-empty check only) → `rate_limiter.check()` (3 round trips, allowed — a fresh bucket always has
capacity) → `authenticate()`, whose own `ProjectId::new` check fires and returns
`Status::invalid_argument(e.to_string())`. No rate-limit headers are attached to this error path —
`attach_rate_limit_headers` is only called on the success path further down each handler, after
`authenticate()` returns `Ok`. After this fix: `extract_project_id`/
`extract_project_id_from_listen_request` reject with the byte-identical
`Status::invalid_argument(e.to_string())` — same `CoreError::InvalidArgument` value, same `Display`
text, same absence of rate-limit headers (there were never any to lose). **Zero observable
difference to the gRPC caller, confirmed by construction (identical expression), not merely by
inspection.**

**REST side**: today, the same charset-invalid `project_id` flows past
`rest_rate_limit_middleware` (3 round trips, allowed) → the dispatched handler, which rejects via
one of two shapes depending on `action` (§ Reading Confirmation, peer-review-iteration-1 finding):
`signInWithPassword`/`signUp`/`sendOobCode`/`resetPassword` reject via `resolve_customer_db_adapter`
→ `ProjectAuthError::ProjectNotFound` → `invalid_api_key()` (401, `INVALID_API_KEY`);
`signInWithCustomToken` rejects via its own `get_client_identity_credential` empty-lookup →
`malformed_response()` (400, `MALFORMED_TOKEN`). No rate-limit headers are attached on the allowed
path either way (`rest_rate_limit_middleware` only attaches headers in its own `Err` branch). After
this fix: the middleware branches on the same `action` path param and returns the matching response
directly — `malformed_response().into_response()` for `signInWithCustomToken`, `invalid_api_key()`
for every other action. **Same status, same body, same absence of headers, per action — zero
observable difference for all 4 dispatched actions**, not 3 of 4 (the gap peer review caught is now
closed by the action-aware branch in § Decision above).

**Finding #20 (AC-PDA-04)**: this fix adds code that runs strictly BEFORE `rate_limiter.check()`/
`check_inner`/`check_pg` are ever invoked, for the syntactically-invalid subset of requests. Zero
lines inside `check_pg` (including the `.ok().flatten()` at lines 253/307 and `.unwrap_or(false)` at
line 273 finding #20 names) are touched, read differently, or reordered by this diff. For a
syntactically VALID `project_id` (the case finding #20's fail-open behavior actually concerns), this
fix's new guard passes with zero effect — `ProjectId::new` succeeds and execution proceeds to
`rate_limiter.check()` exactly as today.

## Wave: DESIGN / [REF] ADR Decision — no new ADR

**Decision: no new ADR (ADR-077 not created).** Rationale, evaluated against this project's own bar
(same test `agent-field-path-validation` and `rate-limiter-project-id-validation` were held to,
contrasted with `sanitize-backend-error-messages`'s ADR-075 and `admin-signin-hardening`'s ADR-076,
both of which DID warrant one):
- Zero new component, port, adapter, type, or cross-cutting pattern. This fix extends 3 existing
  functions in place; it introduces no new abstraction.
- Zero new `CoreError` variant, proto field, schema change, or Cargo dependency.
- The architecture this fix operates within (`RateLimiter`/`check_pg`/Postgres-backed distributed
  token bucket) is `ADR-015`'s, unmodified — this fix only moves an already-existing, already-tested
  validation check to run earlier relative to that architecture's entry point. It does not change
  `ADR-015`'s decision, alternatives, or consequences in any way that would require superseding it.
- The one genuine design choice here (§ Open Question Resolutions, OQ-PDA-01: extend 2 existing
  choke-point functions + 1 new REST guard, vs. a new shared helper) is narrow, fully reversible, and
  recorded inline above with its rejected alternative — matching the same bar
  `agent-field-path-validation`'s own "no new ADR" decision applied.
- Matches this session's established precedent for this exact class of fix:
  `rate-limiter-project-id-validation` (same file!) and `stripe-webhook-secret-required` also
  produced no new ADR for narrow, single/few-file pre-existing-check-reordering security hardening.

## Wave: DESIGN / [REF] C4 Diagrams — none produced (scope justification)

No new container, component, process, or integration boundary is introduced, moved, or removed. The
gRPC `:8080` `FirestoreServer`, the REST/gRPC-Web `:8081` axum router, `RateLimiter`, and the shared
system Postgres pool are all pre-existing and structurally unchanged — only the ORDER in which two
already-existing checks run, within 3 already-existing functions, changes. Producing a System
Context/Container diagram would repeat `adr-001-process-topology.md`'s existing diagram unchanged.
Matches session precedent (`agent-field-path-validation`, `rate-limiter-project-id-validation`,
`stripe-webhook-secret-required` — none produced a C4 diagram for the same reason).

## Wave: DESIGN / [REF] Earned Trust / Probe Applicability — not applicable, and why

Principle-12 probing applies to adapters/ports depending on something external (filesystem, time,
subprocess, vendor SDK, config source, network, kernel syscall semantics). This fix touches none of
those as NEW dependencies: `ProjectId::new`/`is_valid_project_id` is a pure, IO-free, in-memory
charset check (`embyr-core` remains IO-free, unaffected — confirmed by this design's own reading),
and the REST guard's `invalid_api_key()` call is a pure in-memory response construction. No adapter,
no driven port, no external dependency is introduced, removed, or newly exercised by this fix — the
`sqlx::PgPool`/`pg_pool` dependency this fix REDUCES exposure to is pre-existing and already governed
by `ADR-015`'s own design (20 ms timeout, fallback to in-process bucket), unmodified here. **No probe
is needed for this fix.** The correctness guarantee this fix depends on (the charset regex behaving
correctly) is already enforced by `embyr-core`'s own pre-existing 11-test suite
(`crates/embyr-core/src/domain/project.rs`), reused, not duplicated, by this fix.

## Wave: DESIGN / [REF] External Integrations / Enforcement Tooling

- **External integrations**: none. No third-party API, webhook, or OAuth provider is touched — no
  contract-testing annotation applies.
- **Architectural enforcement**: `deny.toml`'s existing `embyr-core`-must-stay-IO-free rule is
  unaffected — this fix calls an already-IO-free function from `embyr-server`/`embyr-core` callers
  that already depend on `embyr-core`; no new dependency edge is added. No new architectural
  invariant is introduced by this fix, so no new enforcement tooling is warranted. The correctness
  guarantee (charset regex behavior) is already covered by `embyr-core`'s own existing unit-test
  suite, reused as-is.

## Wave: DESIGN / Handoff Package (to DISTILL — acceptance-designer)

**Files requiring a change (4, all in `crates/embyr-server/src/`):**
1. `grpc/handler.rs` — extend `extract_project_id` (lines 99-107) and
   `extract_project_id_from_listen_request` (lines 3900-3909) per § Decision above. No other line in
   this file changes; `authenticate()` (lines 199-206) is left untouched (its own check becomes a
   defense-in-depth no-op on the guarded path, not dead code — still the correct behavior for any
   future caller of `authenticate()` that bypasses these two functions).
2. `middleware/rate_limit.rs` — extend `rest_rate_limit_middleware` (lines 377-393) per § Decision
   above, branching on the `action` path param. No line inside
   `RateLimiter`/`check`/`check_inner`/`check_pg`/`check_in_process` changes — AC-PDA-04 (finding #20
   non-interaction) is structurally guaranteed, not just intended.
3. `rest/sign_in.rs` — widen `malformed_response` (line 63) from private to `pub(crate)`. Zero logic
   change; same visibility-widening pattern already used for `invalid_api_key`/
   `hosted_identity_not_enabled` in `rest/sign_up.rs`.

**Files confirmed to need NO change:**
- `embyr-core/src/domain/project.rs` — `ProjectId::new`/`is_valid_project_id` reused as-is, already
  tested (11 existing unit tests), not modified.
- `embyr-core` anywhere else — remains IO-free, no new dependency added.
- `adapters/project_auth.rs` — `resolve_customer_db_adapter`'s own guard is reused (its response
  shape is mirrored, not its code path), not modified.
- `rest/sign_in_with_password.rs`, `rest/sign_up.rs` (function bodies — only one sibling `fn`'s
  visibility keyword changes), `rest/reset_password.rs` — unmodified; this fix closes the gap
  upstream of them, at the middleware boundary, not at their own already-existing guards.
- `rest/sign_in.rs`'s own `sign_in_with_custom_token` handler body — unmodified; only
  `malformed_response`'s visibility keyword changes, its body is untouched.

**Documentation changes made this wave:**
- No new ADR (§ ADR Decision above).
- No new C4 diagram (§ C4 Diagrams above).
- This DESIGN section, appended to this same `feature-delta.md`.

**Regression guards DISTILL/DELIVER must run:**
- `embyr-core`'s own `project.rs` unit-test module (11 tests) — unmodified, reused as the shared
  charset-validator correctness guard.
- Existing gRPC/REST integration suites exercising every one of the 15 gRPC handlers and the REST
  `accounts:<verb>` dispatch with a KNOWN, provisioned `project_id` — must continue passing
  unmodified (regression guard for AC-PDA-02: this fix must not affect legitimate traffic).
- New acceptance tests DISTILL must author for this feature's own 6 UAT scenarios (AC-PDA-01 through
  AC-PDA-06), per the Walking Skeleton Strategy DISCUSS already fixed: a real flood of
  syntactically-invalid `project_id` gRPC/REST requests against a real running `embyr-server` and a
  real Postgres backend, asserting zero additional `rate_buckets` queries land (AC-PDA-06 — a
  query-count or connection-load assertion, not code inspection), while a known-project request in
  the same run is unaffected (AC-PDA-02).
- Full workspace `cargo test` — run once, at the pre-commit gate, per this repo's own token-discipline
  convention (CLAUDE.md).

**Locked DESIGN decisions for DISTILL/DELIVER to build against:**
- [DD1] Guard added in exactly 3 functions across 4 files (one file change is a visibility bump, not
  a guard) — `extract_project_id`, `extract_project_id_from_listen_request` (both `handler.rs`),
  `rest_rate_limit_middleware` (`rate_limit.rs`), plus `rest/sign_in.rs`'s `malformed_response`
  widened to `pub(crate)`. No new shared helper function; no per-call-site duplication across the 15
  gRPC sites (OQ-PDA-01).
- [DD2] gRPC guard reuses `authenticate()`'s own exact expression
  (`ProjectId::new(pid).map_err(|e| Status::invalid_argument(e.to_string()))`) verbatim — guarantees
  byte-identical rejection contract (AC-PDA-03), not merely equivalent-shaped.
- [DD3] REST guard is action-aware (peer review iteration 1 finding, resolved): reuses
  `crate::rest::sign_in::malformed_response()` verbatim for `signInWithCustomToken` and
  `crate::rest::sign_up::invalid_api_key()` verbatim for every other action — guarantees identical
  rejection contract for all 4 dispatched actions, independently confirmed for each, not assumed
  uniform.
- [DD4] Zero lines inside `check_pg`/`check_inner`/`check_in_process`/`RateLimiter` change — finding
  #20 (AC-PDA-04) is structurally untouched by construction.
- [DD5] OQ-PDA-02 (well-formed-but-unprovisioned residual) intentionally not built; the guard's
  placement (strictly before `rate_limiter.check()`, structurally independent of `check_pg`'s own
  logic) leaves a future existence-cache/bloom-filter feature free to compose additively, at zero
  rework cost to this fix.
- [DD6] No new ADR, no new C4 diagram, no new probe (§ ADR Decision, § C4 Diagrams, § Earned Trust
  above) — all three are conscious "not applicable" determinations, not silent omissions.

## Wave: DISTILL / [REF] Reading Confirmation

✓ DESIGN's line numbers verified against current source, unchanged: `handler.rs:99-107`
(`extract_project_id`), `handler.rs:199-206` (`authenticate`'s own `ProjectId::new` check),
`handler.rs:3900-3909` (`extract_project_id_from_listen_request`), `rate_limit.rs:228-318`
(`check_pg`, DESIGN cited slightly older 203-293 — same logic, only line-shifted),
`rate_limit.rs:377-393` (`rest_rate_limit_middleware`), `rest/sign_in.rs:63`
(`malformed_response`), `rest/sign_up.rs:121` (`invalid_api_key`). `lib.rs:79-90`
(`accounts_bridge_dispatch`) confirmed the exact REST route pattern
(`/v1/projects/:project_id/accounts:action`) and `action` param extraction
(`trim_start_matches(':')`) DESIGN's REST guard code relies on.

✓ Existing test conventions read for consistency: `tests/distributed_rate_limiting/` (same
`rate_limit.rs` file's own DRL feature, `b11`-`b16`, `DrlTestContext` harness — real Postgres via
testcontainers-rs, real gRPC/REST server via `start_distributed_grpc_server`/
`start_test_server_with_rate_limit`), `tests/rest_rate_limiting/acceptance/rest_calls_are_rate_limited.rs`
(REST harness pattern), `tests/observability/acceptance/obs05_postgres_pool_metrics.rs` (confirmed
NO existing Prometheus metric counts `rate_buckets` queries specifically — only pool size/idle
gauges and `embyr_rate_limit_requests_total`/`embyr_rate_limit_pg_timeout_total`).

## Wave: DISTILL / [REF] Scenario List

Extends the existing `distributed_rate_limiting` acceptance suite (same file this feature touches)
rather than a new top-level test directory — matches the `rate-limiter-project-id-validation`
feature's own precedent (`b15`/`b16` in the same directory, same `DrlTestContext` harness).

| # | Scenario | Tags | AC |
|---|---|---|---|
| 1 | Garbage project_id flood never invokes RateLimiter::check() while known project unaffected | `@walking_skeleton @driving_port @real-io @property` | AC-PDA-01/02/06 |
| 2 | Single garbage project_id GetDocument never invokes RateLimiter::check() | `@driving_port @real-io` | AC-PDA-01 |
| 3 | Garbage project_id GetDocument returns same InvalidArgument status as before the fix | `@driving_port @real-io` | AC-PDA-03 |
| 4 | Well-formed-but-unprovisioned project_id still invokes RateLimiter::check() (gRPC) | `@driving_port @real-io` | AC-PDA-05 |
| 5 | Empty project_id rejected before reaching rate limiter, exactly as today | `@driving_port @real-io @regression` | Example 4 |
| 6 | Listen RPC with garbage project_id never invokes RateLimiter::check() | `@driving_port @real-io` | AC-PDA-01 |
| 7 | signInWithCustomToken with garbage project_id returns 400 MALFORMED_TOKEN, no DB check | `@driving_port @real-io` | AC-PDA-01/03 |
| 8 | signUp (representative of every other action) with garbage project_id returns 401 INVALID_API_KEY, no DB check | `@driving_port @real-io` | AC-PDA-01/03 |
| 9 | Known provisioned project_id REST request unaffected by the guard | `@driving_port @real-io` | AC-PDA-02 |
| 10 | Well-formed-but-unprovisioned project_id still reaches shared database via REST | `@driving_port @real-io` | AC-PDA-05 |

10 scenarios (1 walking skeleton + 9 focused); 6/10 (60%) are error/boundary-path scenarios
(#2,3,4,6,7,8 assert on rejection paths), exceeding the 40% mandate. All Tier A (production
composition root — real `embyr-server`, real Postgres via testcontainers; no Tier B: this is a
single guard-clause diff, not a ≥3-scenario domain-rich chained journey — Mandate 10 conditions
for Tier B do not hold).

## Wave: DISTILL / [REF] Walking Skeleton Strategy

Strategy C (Real local resources) — matches this project's Architecture of Reference for
driven-internal ports (Postgres via testcontainers) and driving ports (real gRPC/REST server).
No external/non-deterministic ports in scope. Scenario 1 (above) is the walking skeleton, per
DISCUSS's own Walking Skeleton Strategy section — a real flood against a real server + real DB,
proving zero additional rate-limit checks while a known project stays unaffected.

## Wave: DISTILL / [REF] Adapter Coverage

| Adapter | `@real-io` scenario | Covered by |
|---|---|---|
| gRPC `FirestoreClient` (GetDocument, Listen) | YES | Scenarios 1,2,3,4,5,6 |
| REST `accounts:<verb>` dispatch | YES | Scenarios 7,8,9,10 |
| Postgres `rate_buckets` (via `RateLimiter::with_pg`) | YES | All 10 scenarios (real testcontainers Postgres) |
| Admin `:9090 /metrics` (Prometheus scrape) | YES | Scenarios 1,2,4,6,7,8,10 (round-trip observability) |

No new driven adapter is introduced by this feature (DESIGN § Handoff Package) — all four rows are
pre-existing adapters already exercised with real I/O by the `distributed_rate_limiting`/
`observability` suites; this feature's tests add coverage of the NEW guard's interaction with them.

## Wave: DISTILL / [REF] DB-Round-Trip Observability Mechanism

Chosen: the already-existing `embyr_rate_limit_requests_total{project_id="unconfirmed",
outcome="allowed"}` Prometheus counter (OBS-04/ADR-069), scraped from the real admin `:9090
/metrics` endpoint, asserted as a before/after delta per scenario. It increments exactly once per
`RateLimiter::check()` INVOCATION for any never-before-seen `project_id` — a delta of 0 across a
request is direct, structural proof `check()`/`check_pg`'s 3-round-trip path never ran; a delta of
1 is direct proof it did.

Two alternatives were tried empirically and rejected (see
`tests/distributed_rate_limiting/common/mod.rs::metric_value` doc comment and
`docs/feature/preauth-db-amplification/distill/red-classification.md` § Mechanism note for the
full writeup, including the two spurious failures that surfaced each rejection):

1. **`pg_stat_user_tables` scan/insert counters** — lags real commits by Postgres's own internal
   `PGSTAT_MIN_INTERVAL` (~1s) stats flush; produced false-zero deltas for fast sequential requests.
2. **`rate_buckets` row EXISTENCE** — invalid for garbage/never-provisioned `project_id`s
   specifically: the table has a FK to `projects(id)` (migration 0018), so `check_pg`'s own
   `INSERT ... ON CONFLICT DO NOTHING` silently fails whenever no `projects` row exists — row count
   stays 0 whether or not the 3-round-trip path ran. (Row existence remains valid and is still used
   for the KNOWN-project regression scenarios, where a real `projects` row satisfies the FK.)

A wall-clock timing bound was not attempted at all — this session's own repeated
"empirically-re-derive, don't estimate" precedent (see MEMORY.md `admin-signin-hardening`'s
TOTP/CPU-contention finding) ruled it out before trying, once a structural DB-native/production
counter was confirmed available.

## Wave: DISTILL / [REF] Scaffolds

None. This feature extends 3 EXISTING, already-compiling production functions (`extract_project_id`,
`extract_project_id_from_listen_request`, `rest_rate_limit_middleware`) plus one visibility bump
(`rest/sign_in.rs::malformed_response`) — no new production module is introduced, so Mandate 7's
scaffold-stub requirement does not apply. All 10 acceptance tests compile and run against the real,
unmodified functions today; RED is expressed via assertion failure (see red-classification.md), not
`ImportError`/`NotImplementedError`.

## Wave: DISTILL / [REF] Test Placement

`tests/distributed_rate_limiting/acceptance/b17_preauth_project_id_amplification.rs` (gRPC, 6
scenarios) and `b18_preauth_project_id_amplification_rest.rs` (REST, 4 scenarios), registered as
`[[test]]` targets `drl_b17_preauth_project_id_amplification`/
`drl_b18_preauth_project_id_amplification_rest` in `crates/embyr-server/Cargo.toml`. Extends the
existing `tests/distributed_rate_limiting/common/mod.rs` harness (added `metric_value` helper and
`garbage_project_ids()` fixture) rather than creating a new top-level test directory — same file
under test (`rate_limit.rs`), same established DRL harness precedent.

## Wave: DISTILL / [REF] Pre-requisites

- Prior wave driving ports (DISCUSS § Driving Ports) confirmed unchanged: ~15 gRPC call sites (all
  funnel through 2 choke-point functions per DESIGN's own reading) + `rest_rate_limit_middleware`.
- DEVOPS wave: not run for this feature (backend-only guard-clause fix, no new environment/infra
  surface — mirrors DISCUSS's own "Orchestrator Decisions" call). Default environment matrix
  (clean | with-pre-commit | with-stale-config) applies uninstrumented; no environment-specific
  behavior is introduced by this fix.

## Wave: DISTILL / [REF] RED Verification Result

`docs/feature/preauth-db-amplification/distill/red-classification.md` — 5/10 scenarios fail today
for the correct reason (MISSING_FUNCTIONALITY: `RateLimiter::check()` genuinely invoked when it
should have been guarded); 5/10 pass today as intentional regression guards (contract-preservation,
residual-not-closed, pre-existing-boundary-unaffected) and must stay green after DELIVER's diff.
Zero scenarios fail for the wrong reason (no import/fixture/setup errors). Zero stray Docker
containers left behind post-run.

## Wave: DISTILL / [REF] Mandate Compliance Evidence

- **CM-A** (Mandate 1, driving ports only): all 10 tests invoke via `FirestoreClient` (tonic gRPC
  client) or `reqwest::Client` (REST HTTP) against a real running server — zero direct imports of
  `extract_project_id`/`check_pg`/internal handler methods from test code (verified: neither
  `b17_*.rs` nor `b18_*.rs` imports anything from `embyr_server::grpc::handler` or
  `embyr_server::middleware::rate_limit` directly; only `embyr_server::{TestServer,
  start_test_server_with_distributed_rate_limit}` composition-root entry points via `common/mod.rs`).
- **CM-B** (Mandate 2, business language): scenario/test names and doc comments describe
  observable behavior ("never invokes RateLimiter::check()", "returns same status as before the
  fix") — technical terms (Postgres, gRPC, REST, status codes) appear only in doc-comment rationale
  and step bodies, matching this project's own established convention for backend-reliability
  features (no stakeholder-facing UI exists for this class of fix).
- **CM-C** (Mandate 3, user journey completeness): walking skeleton (scenario 1) covers full
  before/after journey — garbage flood has zero DB cost AND a legitimate tenant's request is
  simultaneously provably unaffected in the same run, matching Sam Chen's own elevator pitch
  ("size the pool without budgeting for attacker amplification").
- **CM-D** (Mandate 4, pure function extraction): not applicable — this feature reuses an
  already-pure, already-tested function (`ProjectId::new`/`is_valid_project_id`, 11 existing unit
  tests in `embyr-core`); no new business logic is introduced requiring extraction.

## Wave: DISTILL / [REF] Peer Review Result

`nw-acceptance-designer-reviewer` (Sentinel) — **approved**, zero blockers/high. One **medium**
finding: AC-PDA-04 (finding #20's fail-open-on-DB-error must not be modified) has no dedicated
runtime acceptance test, since "these lines were not touched" is a diff-inspection fact, not a
runtime-observable behavior a driving-port test can assert. Reviewer confirmed this is a legitimate,
already-documented scope decision (this file's own Contract Preservation Analysis + AC-PDA-04 row
above) and recommends DELIVER's own code reviewer explicitly diff-check that none of the 4 touched
production files modify any line inside `check_pg`/`check_inner`/`check_in_process` — carried
forward as a DELIVER-handoff action item, not a re-opened DISTILL gap. All other dimensions scored
8-9/9; all three applicable mandates (CM-A hexagonal boundary, CM-B business language, CM-C user
journey completeness) passed.

**DELIVER handoff action item**: at COMMIT phase, diff-check that `check_pg`'s `.ok().flatten()`
(lines ~253, ~307) and `.unwrap_or(false)` (line ~273) are byte-for-byte unchanged.
