# Feature Delta: rate-limiter-project-id-validation

## Wave: DISCUSS / [REF] Prior Wave Consultation — Reading Confirmation

✓ `docs/product/production-readiness-audit-2026-09-08.md` — read in full (all 41 findings + bloat
list + notes). Finding #2's exact wording confirmed: "Unbounded Prometheus label cardinality
driven by an unauthenticated, unvalidated `project_id` — rate limiter runs *before* `authenticate()`,
`extract_project_id` accepts any non-empty string, registry has no eviction. Unauthenticated remote
OOM." Location cited: `crates/embyr-server/src/middleware/rate_limit.rs:143-148`;
`crates/embyr-server/src/grpc/handler.rs:99-107,1267,1272`; `crates/embyr-server/src/observability.rs:39-47`.
Severity: **Blocker**.
✓ Finding #14 (related, same file) also read: "Pre-authentication 3-round-trip amplification against
the shared system DB... an unknown project_id triggers UPDATE + SELECT EXISTS + INSERT before
authenticate() ever runs." Confirmed: both #2 and #14 share the SAME root architectural pattern —
the rate limiter runs on unvalidated input before authentication — but they are separate,
independently-tracked findings with separate remediations (#2 is a metrics-cardinality/memory
exhaustion vector; #14 is a database-amplification vector). **This feature scopes to #2 only.**
#14 remains "Not started" in the audit and is not touched here.
✓ `crates/embyr-server/src/grpc/handler.rs` read around lines 96-107 (`extract_project_id`) and
1258-1276 (`handle_get_document`). Confirmed exactly: `extract_project_id` parses
`projects/{pid}/databases/(default)/documents/...` and accepts ANY non-empty `pid` substring —
zero format validation, zero existence check. Confirmed the call ordering in `handle_get_document`
(representative of every RPC handler, per the audit's own citation of this as the pattern):
`extract_project_id` (line 1264) → `self.rate_limiter.check(&project_id)` (line 1267, unauthenticated)
→ `self.authenticate(&project_id, &api_key)` (line 1272, only now does auth happen). No investigation
performed into redesigning this ordering — that is explicitly DESIGN's call, not this DISCUSS's.
✓ `crates/embyr-server/src/middleware/rate_limit.rs` read in full (lines 1-310+). Confirmed the exact
mechanism: `RateLimiter::check()` (lines 140-150) calls `check_inner()` for the allow/reject decision,
then UNCONDITIONALLY records `metrics::counter!("embyr_rate_limit_requests_total", "project_id" =>
project_id.to_owned(), "outcome" => outcome).increment(1)` — using the raw, attacker-controllable
`project_id` string directly as a Prometheus label value, regardless of whether that project_id
corresponds to any real, authenticated project. This confirms the audit's finding precisely: the
metrics side-channel is entirely independent of the rate-limiting allow/reject logic itself (which is
in `check_inner`/`check_pg`/`check_in_process`, all of which are read and understood but explicitly
NOT the concern of this feature — see § Business Context).
✓ `crates/embyr-server/src/observability.rs` read in full (49 lines). Confirmed "no eviction" means
exactly what it says: `PrometheusHandle` is installed once via `OnceLock` at startup
(`get_or_install_prometheus_handle`) and is never told to expire, cap, or evict any label combination
for the life of the process — every unique `metrics::counter!(...)` label set ever recorded is
retained in the underlying `metrics-exporter-prometheus` registry (an in-memory `HashMap`-backed
structure) until process restart. There is no TTL, no LRU, no max-cardinality guard configured
anywhere in this file or at the `PrometheusBuilder::new()` call site.
✓ **Cross-reference found, not named by the task, directly relevant to this feature's regression
scope**: `tests/observability/acceptance/obs04_rate_limit_metrics.rs` (the OBS-04 acceptance test for
exactly this metric) is entirely `#[ignore]`'d `todo!()` scaffolding — 3 test function stubs, zero
implemented assertions, same for its 4 sibling files (`obs01`/`obs02`/`obs03`/`obs05`, 35 total
`#[ignore]`/`todo!()` occurrences across the whole `tests/observability/` suite). Its own doc comment
explicitly documents the assumption this vulnerability breaks: `"HIGH CARDINALITY (D-OBS-7): one time
series per project per outcome; acceptable ≤10k projects."` D-OBS-7's own design assumption was that
cardinality is bounded by the number of REAL, registered projects (≤10k) — it did not anticipate an
UNAUTHENTICATED caller supplying an unbounded number of FAKE project_id strings before any project
lookup occurs. This is the precise mechanism by which this finding invalidates D-OBS-7's own stated
assumption, not a new concern layered on top of it. Because these test files are unimplemented stubs,
AC-3 (from the task) — "existing observability tests must continue to pass unmodified" — is
technically trivially true today (ignored tests always "pass"); this DISCUSS states that requirement
honestly below rather than implying these are currently-exercised regression assertions.
✓ `docs/product/jobs.yaml` — read (JOB-01 through JOB-16 read in full; JOB-11 `fair-multitenancy`,
JOB-12 `observability`, and JOB-13 `production-deployment`, all persona P2 Sam Chen, evaluated as
candidates — see § Persona & Job for the reasoning between them). No dedicated
"metrics-cardinality-security" or "pre-auth-hardening" job exists.
✓ `docs/feature/stripe-webhook-secret-required/feature-delta.md` — read in full to confirm this
project's own established single-file, Tier-1-only `feature-delta.md` convention (`## Wave: DISCUSS /
[REF] {Section}` heading format) and its own precedent for reasoning explicitly against the
nearest-alternative job before committing to a reuse decision. This DISCUSS mirrors that convention
and structure directly, including reusing that feature's own attacker persona ("Marcus Webb") for
narrative continuity, since both features describe the same class of unauthenticated attacker against
the same production deployment shape.

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

- Feature type: **Security fix** (unauthenticated resource-exhaustion vector via an observability
  side-channel), not an SDK-developer-facing feature.
- JTBD: **reuse JOB-12** (`observability`, P2 Sam Chen) — see § Persona & Job for reasoning against
  JOB-11 and JOB-13.
- Decision 4 (full JTBD path vs. infrastructure-only): **Yes — full JTBD path**, per explicit task
  instruction. This is a real, unauthenticated, remote-exploitable security vulnerability against a
  real production deployment — not infrastructure-only scaffolding.
- Walking Skeleton: **Yes** — single story, no further slicing (mirrors `stripe-webhook-secret-
  required`'s own single-story precedent for a confined, single-mechanism security fix).
- UX Research Depth: **Lightweight** — an operator-facing, backend security/observability fix, not an
  end-user journey; no ASCII TUI mockups or emotional-arc journey YAML warranted.
- **The exact bounding mechanism is explicitly NOT this DISCUSS's call** — three candidate approaches
  are named below as DESIGN's own starting point (see § Business Context), but DISCUSS locks only the
  REQUIREMENT: unauthenticated, attacker-chosen input cannot cause unbounded Prometheus label growth.

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: **P2 Sam Chen (Service Operator / Platform Engineer)** — unchanged from JOB-11/12/13's
existing profile. Not an SDK developer (P1 Alex) — this finding was surfaced by a production-
readiness security scan against operational infrastructure Sam Chen alone depends on and operates.

**Job**: **JOB-12 `observability`**, reused, EXTENDED (not replaced). Candidates considered and
rejected:
- **JOB-11 `fair-multitenancy`** (persona Sam Chen, feature `distributed-rate-limiting`) — the nearer
  candidate by file location (`rate_limit.rs` is JOB-11's own home), but the WRONG fit: JOB-11's job
  story and functional dimension are entirely about the CORRECTNESS of the allow/reject decision
  across a horizontally-scaled cluster (`"per-project rate limits enforced consistently regardless of
  how many server instances are running"`). This finding does not touch that decision logic at all —
  `check_inner`/`check_pg`/`check_in_process` are unaffected; confirmed by direct reading, not
  assumed. The harm here is entirely in the METRICS side-channel the rate limiter happens to emit,
  not in what the rate limiter decides.
- **JOB-13 `production-deployment`** (persona Sam Chen, feature `production-readiness`) — the job
  `stripe-webhook-secret-required` (finding #1, same audit) reused, for a fail-fast STARTUP
  config-validation shape. Rejected here: this finding is not a startup-configuration gap (no env var
  is missing or wrongly defaulted) — it is a RUNTIME behavior of an already-correctly-configured,
  already-running server's own metrics instrumentation. The remediation shape (bound a label's
  cardinality) does not fit JOB-13's own "server exits non-zero with named missing var" pattern.
- **JOB-12 `observability`** (feature `observability`) — the correct fit. JOB-12's own functional
  dimension names the EXACT metric this finding is about, verbatim: `"embyr_rate_limit_requests_total
  {project_id,outcome} updated on every rate-limit check."` JOB-12's own job story is specifically
  about Sam Chen's ability to trust the `/metrics` surface: `"I want to query a Prometheus /metrics
  endpoint... so I can diagnose the problem before the customer escalates."` A metrics endpoint an
  unauthenticated attacker can turn into a memory-exhaustion weapon directly undermines that job's own
  emotional dimension (`"feel confident quoting SLOs to customers because the metrics prove the system
  is behaving correctly"`) — the diagnostic tool becomes the attack surface. This is also a direct,
  evidenced violation of an assumption JOB-12's own prior extension already documented in code (D-OBS-7,
  `obs04_rate_limit_metrics.rs`'s own doc comment — see § Reading Confirmation) — this feature closes
  that assumption's gap, the same "make it real / close a gap in an already-covered surface" pattern
  this session has used repeatedly (e.g., `customer-db-transaction-sweeper` → JOB-12).

## Wave: DISCUSS / [REF] Business Context

Today, every gRPC request handler (representative: `handle_get_document`, `handler.rs:1258-1276`)
extracts a `project_id` from the resource-path string via `extract_project_id` (`handler.rs:99-107`),
which accepts ANY non-empty substring in the `projects/{pid}/...` position — no format check, no
existence check. That raw string is passed directly to `RateLimiter::check(&project_id)`
(`handler.rs:1267`) **before `authenticate()` runs at all** (`handler.rs:1272`). `RateLimiter::check`
(`rate_limit.rs:140-150`) unconditionally records
`metrics::counter!("embyr_rate_limit_requests_total", "project_id" => project_id.to_owned(), "outcome"
=> outcome)` — using the raw, attacker-controlled string as a permanent Prometheus label value. The
Prometheus recorder (`observability.rs`, installed once via `OnceLock`, `metrics-exporter-prometheus`)
has no TTL, no max-cardinality guard, and no eviction policy of any kind: every unique label
combination ever seen is retained in memory for the life of the process.

The result: an unauthenticated attacker who sends requests with millions of distinct,
attacker-chosen `project_id` strings (no credential, no valid project, no prior interaction with the
system required) causes the Prometheus registry to grow by one new permanent time series per unique
string sent — an unbounded, purely attacker-driven memory-growth vector that eventually OOMs the
`embyr-server` process, taking down every real tenant's traffic on that instance. This is a genuine
denial-of-service, not a theoretical one: the mechanism requires zero authentication and zero rate
limiting of the metrics-recording call itself (the rate LIMITER's own instrumentation is what is
unbounded — the irony the audit's own finding names directly).

This also concretely invalidates an assumption this codebase's own prior `observability` feature
already documented in code: `obs04_rate_limit_metrics.rs`'s own doc comment states `"HIGH CARDINALITY
(D-OBS-7): one time series per project per outcome; acceptable ≤10k projects"` — an assumption that
cardinality is bounded by the count of REAL, registered projects. That assumption held only because
nobody had yet examined whether the label value is validated before being used — it is not.

**The core design question this DISCUSS surfaces but does NOT solve** (DESIGN's own investigation):
how should the metric label be bounded? Three candidates, none decided here:
- **(a) Validate-then-label, sentinel fallback**: record the `project_id` label only for a project_id
  that has passed a cheap format/existence check; anything that fails falls back to a constant
  sentinel label (e.g., `"invalid"`) — bounded cardinality regardless of attacker input volume.
- **(b) No project_id label pre-authentication**: don't label by `project_id` at all for the portion
  of the pipeline that runs before `authenticate()` succeeds; only attach the real label once the
  project_id is confirmed authenticated/real.
- **(c) Some other bounding mechanism** DESIGN may find more appropriate after its own deeper reading
  of the code (e.g., a cardinality-capped registry wrapper, a pre-auth-vs-post-auth metric split, or a
  different metric name entirely for pre-auth attempts).

This DISCUSS locks only the REQUIREMENT — unauthenticated, attacker-chosen input must not cause
unbounded Prometheus label growth — not the mechanism.

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

**Signals checked**: >10 user stories? No (1). >3 bounded contexts/modules? No — confined to
`embyr-server`'s own middleware layer (`rate_limit.rs`) and its instrumentation surface
(`observability.rs`); `handler.rs`'s `extract_project_id`/call-ordering is read for context but this
feature does not require changing the pre-auth/post-auth call ordering itself (that redesign, if ever
undertaken, is a separate, larger architectural question spanning both findings #2 and #14 — see
§ Out of Scope). Walking skeleton >5 integration points? No (a handful: one real gRPC request with a
real registered project, one real gRPC request with an attacker-chosen garbage project_id, one real
`GET :9090/metrics` scrape to observe the resulting label set — all against the same running server
instance). Estimated effort >2 weeks? No — this is a single-function-scale fix per the audit's own
overall framing ("most fixes are single-function"); no new domain concept, no new adapter trait, no
new bounded context. Multiple independent user outcomes? No — "unauthenticated input cannot grow
metrics cardinality unboundedly" is a single outcome; the legitimate-traffic-unaffected and
malformed-input-handled-safely cases are verification points of that ONE outcome, not separate
stories.

**Scope Assessment: PASS** — 1 user story, 1 bounded context (`embyr-server` middleware +
observability layer), estimated ≤2 days, 6 UAT scenarios (within the 3-7 right-sized range).

## Wave: DISCUSS / [REF] System Constraints

- The actual rate-limiting ALLOW/REJECT decision logic (`check_inner`/`check_pg`/`check_in_process`)
  is explicitly OUT OF SCOPE for behavior change — this is a fix to the metrics-recording side-channel
  only. Legitimate, authenticated per-project rate-limit enforcement must be provably unaffected
  (regression guard).
- The bounding mechanism must not depend on authentication having already succeeded to make a
  DECISION about whether to rate-limit (the rate limiter must still run pre-auth, per its own
  documented purpose — "skip Argon2id on requests that would be rate-limited anyway") — only the
  METRIC LABEL for pre-auth/unvalidated input is in scope for bounding, not the pipeline ordering
  itself.
- No new external dependency; the Prometheus recorder, the `metrics` crate macros, and the rate
  limiter all already exist (`observability`, `distributed-rate-limiting` features).
- Out of scope for this feature (see § Out of Scope): finding #14 (same root pattern, DB amplification,
  separate blocker) and finding #39 (same cardinality mechanism, different route — Stripe webhook
  `event_type` label) — related, not fixed here.

## Wave: DISCUSS / [REF] User Stories

### US-01: Unauthenticated Attacker-Chosen Project IDs Cannot Grow Metrics Cardinality Without Bound

**job_id**: JOB-12

#### Elevator Pitch
**Before**: Sam Chen operates a production embyr deployment and relies on `GET :9090/metrics`
(JOB-12's own diagnostic surface) to catch problems before customers escalate. An unauthenticated
attacker with no credential of any kind — "Marcus Webb," reusing the same attacker persona this
session's Stripe-webhook fix already established — writes a script that sends millions of gRPC
requests, each with a freshly-generated random string as the `project_id` in the resource path
(`projects/<random-uuid>/databases/(default)/documents/x`). Because `extract_project_id` accepts any
non-empty string and the rate limiter records a Prometheus counter labeled by that raw string BEFORE
`authenticate()` ever runs, each request creates a brand-new, permanent time series in the metrics
registry. The registry has no eviction. Marcus's script runs for a few minutes and the
`embyr-server` process OOMs and crashes — taking down every real tenant's traffic on that instance,
with zero authentication required and zero trace beyond an ordinary-looking flood of gRPC calls.
**After**: Sending any number of requests with arbitrary, malformed, or nonexistent `project_id`
strings does not create new, unique Prometheus label values for the rate-limiter's own metrics — the
registry's relevant metric family stays bounded regardless of how much distinct garbage an attacker
sends. Sam Chen's real, authenticated tenants (e.g., Meridian Health) keep their own accurate
per-project metric exactly as JOB-12 already promises.
**Decision enabled**: Sam Chen can trust that the `/metrics` endpoint he built his production
alerting and SLO dashboards on (JOB-12's own core promise) is itself safe to expose to a deployment
reachable by unauthenticated internet traffic — he does not need to add a separate cardinality-limiting
proxy in front of Prometheus, and he does not need to worry that a single unauthenticated attacker can
OOM his server just by hitting the gRPC port with garbage project IDs.

#### Who
- Sam Chen (P2) | Service operator running embyr-server in production, exposed to unauthenticated
  internet traffic on the gRPC/REST listeners, relying on JOB-12's `/metrics` surface for proactive
  alerting | Needs the metrics surface itself to be attack-resistant, not merely accurate for
  well-behaved traffic.

#### Solution
Bound the Prometheus label cardinality the rate limiter's own metrics recording can contribute for
unauthenticated, unvalidated `project_id` input, while leaving the actual rate-limiting allow/reject
decision and the accurate per-project metric for real, authenticated traffic unchanged. The exact
mechanism (validate-then-label with a sentinel fallback; no project_id label pre-authentication; or
another bounding approach DESIGN finds more appropriate after reading the code) is DESIGN's decision,
not fixed here.

#### Domain Examples

**Example 1 (Happy Path — real, authenticated project)**: Meridian Health's real, registered project
`meridian-health-prod` is within its rate limit. Meridian Health's SDK client calls `GetDocument`. The
rate limiter records `embyr_rate_limit_requests_total{project_id="meridian-health-prod",outcome=
"allowed"}` exactly as JOB-12's own functional dimension already promises — one stable, accurate time
series for this real tenant.

**Example 2 (Edge Case — malformed but non-empty project_id from a buggy client)**: A misconfigured or
outdated SDK client sends a resource name with a garbled, truncated `project_id` segment (e.g. a
stray `%20`-encoded fragment left over from a client-side string-concatenation bug) that is non-empty
but corresponds to no real project. Today this creates a brand-new permanent label. After this
feature, this input does not create a new permanent label distinct from a small, bounded set — the
request still receives a rate-limit decision (fail-safe, not silently dropped), but the metrics
side-channel does not treat it as a novel tenant.

**Example 3 (Error/Boundary — the exploit this feature closes)**: Marcus Webb, an unauthenticated
attacker with no credential of any kind, sends 2 million `GetDocument` requests, each with a freshly
generated random UUID string as the `project_id`
(`projects/6f2e1a3c-....../databases/(default)/documents/x`, a different UUID every time). Before this
feature: 2 million new, permanent Prometheus label combinations are created in the process's metrics
registry, the registry's memory footprint grows proportionally to attacker input volume with no
ceiling, and the `embyr-server` process eventually OOMs and crashes. After this feature: sending the
same 2 million requests results in the relevant metric family holding no more than a small, bounded
number of distinct label values attributable to this flood — the server's memory footprint for this
metric does not grow with attacker-chosen input volume, and the process does not OOM.

#### UAT Scenarios (BDD)

```gherkin
Scenario: Legitimate authenticated project traffic keeps an accurate, stable per-project metric
  Given Meridian Health's real, registered project "meridian-health-prod" is within its rate limit
  When Meridian Health's SDK client calls GetDocument
  Then embyr_rate_limit_requests_total{project_id="meridian-health-prod",outcome="allowed"} increments by 1
  And the request is allowed exactly as it is today

Scenario: A flood of attacker-chosen project IDs does not grow the metrics registry unboundedly
  Given no authenticated session exists for any of the following requests
  When Marcus Webb sends 10,000 GetDocument requests, each carrying a distinct, freshly-generated
    random string as the project_id in the resource path
  Then the embyr_rate_limit_requests_total metric family gains no more than a small, bounded number
    of new distinct label combinations attributable to these 10,000 requests
  And the server process's memory usage attributable to this metric does not grow proportionally
    with the 10,000 distinct attacker-chosen inputs

Scenario: Rate limiting itself keeps rejecting requests correctly for a real, over-limit project
  Given Meridian Health's project "meridian-health-prod" has an exhausted token bucket
  When Meridian Health's SDK client calls GetDocument again
  Then the request is rejected with RESOURCE_EXHAUSTED exactly as it is today
  And this rejection decision is unaffected by the cardinality-bounding change

Scenario: A single malformed project_id still receives a rate-limit decision without crashing
  Given an SDK client bug produces a malformed, non-empty project_id string in the resource path
  When that request reaches the rate limiter before authenticate() runs
  Then the server returns a rate-limit decision (allowed or rejected) without panicking
  And no new permanently-retained label distinct from the bounded set is created for this input

Scenario: The existing observability metrics test suite's documented contract is preserved
  Given the existing tests/observability acceptance test suite from the observability feature (ADR-016)
  When this feature's changes are applied
  Then the suite's files are unmodified by this feature
  And the documented embyr_rate_limit_requests_total{project_id,outcome} contract for legitimate,
    authenticated projects (obs04's own doc comment) remains accurate

Scenario: Full regression suite passes
  Given the complete pre-existing workspace test suite
  When this feature's changes are applied
  Then no previously-passing test regresses
```

#### Acceptance Criteria
- [ ] AC-RLV-01: sending N requests with N distinct arbitrary/malformed/nonexistent `project_id`
      strings before authentication must not cause N new, unique Prometheus label values to be
      recorded for `embyr_rate_limit_requests_total` (or any other rate-limiter-recorded metric
      family) — a test can send N requests with N different garbage project_ids and confirm the
      metric family's own label-combination count does not grow by N. The exact bounding mechanism is
      DESIGN's decision.
- [ ] AC-RLV-02 (regression guard): rate limiting itself continues to function correctly for real,
      authenticated requests — the allow/reject decision logic and per-project token-bucket
      enforcement (JOB-11's own fair-multitenancy guarantee) are unaffected by this fix; legitimate
      traffic is still correctly rate-limited per real project.
- [ ] AC-RLV-03: the existing `tests/observability/` acceptance test suite (obs01-05, ADR-016 —
      currently 5 files of `#[ignore]`d/`todo!()` scaffolding, not yet implemented assertions per
      this DISCUSS's own reading — see § Reading Confirmation) is not modified by this feature, and
      the documented per-project-label contract for legitimate traffic (`obs04`'s own doc comment)
      remains accurate.
- [ ] AC-RLV-04: no other currently-passing test in the full workspace suite regresses.
- [ ] AC-RLV-05: a single malformed (non-empty, non-conforming) `project_id` does not cause a panic —
      the request still receives a normal rate-limit decision.

#### Outcome KPIs
- **Who**: Sam Chen operating production embyr deployments exposed to unauthenticated internet
  traffic, relying on JOB-12's `/metrics` surface.
- **Does what**: is structurally prevented from ever having the `/metrics` endpoint's own memory
  footprint grow unboundedly in response to unauthenticated, attacker-chosen `project_id` input;
  legitimate, authenticated per-project metrics remain accurate and unaffected.
- **By how much**: from unbounded (today, N distinct attacker-chosen project_id strings produce N
  distinct permanent Prometheus label combinations, per the audit's own confirmed finding and this
  DISCUSS's own direct reading of `rate_limit.rs:143-148`) to a small, bounded constant regardless of
  N; 0% regression on real, authenticated project traffic's rate-limit decisions or metrics.
- **Measured by**: an integration test sending N (e.g. 10,000) requests with distinct garbage
  project_ids and asserting the metric family's label-combination count does not grow by N
  (AC-RLV-01); the existing `obs04_rate_limit_metrics.rs`-style real-project scenario asserting
  unchanged accurate labeling (AC-RLV-02/AC-RLV-03); full regression suite (AC-RLV-04).
- **Baseline**: unbounded growth (0% bounded) — confirmed directly by this DISCUSS's own reading of
  `rate_limit.rs:143-148` (unconditional `project_id.to_owned()` as a label with no validation) and
  `observability.rs:39-47` (no eviction/TTL/cardinality cap on the installed recorder), matching the
  audit's own finding #2 evidence exactly.

#### Technical Notes
- The exact bounding mechanism (validate-then-label with sentinel fallback; no project_id label
  pre-authentication; or another approach) is DESIGN's own investigation — this DISCUSS names the
  three candidates from the task framing as DESIGN's starting point but does not choose among them.
- Whether re-ordering the pipeline so `authenticate()` runs before the rate limiter is ever considered
  is explicitly NOT this feature's scope — see § Out of Scope. The rate limiter's own documented
  purpose ("skip Argon2id on requests that would be rate-limited anyway") depends on running pre-auth;
  only the metric LABEL for unvalidated input is bounded here.
- Depends on nothing new — `RateLimiter::check()`, `extract_project_id()`, and the Prometheus recorder
  (`observability.rs`) all already exist (`distributed-rate-limiting`, `observability` features).
- `tests/observability/acceptance/obs04_rate_limit_metrics.rs` and its 4 siblings are pre-existing,
  unimplemented (`#[ignore]`/`todo!()`) scaffolding — DESIGN/DISTILL may choose to implement real
  assertions there to prove AC-RLV-01/02 directly (the file's own structure already anticipates
  exactly this kind of scenario), but implementing them for their own sake (independent of this
  feature's needs) is out of scope.

## Wave: DISCUSS / [REF] Definition of Done

1. AC-RLV-01 through AC-RLV-05 all pass, proven against a real running server instance (real gRPC
   requests, real `GET :9090/metrics` scrape) — mirrors `stripe-webhook-secret-required`'s own real,
   not mocked/unit-only, proof standard.
2. The regression guard (AC-RLV-02) is proven identical to pre-feature behavior for real, authenticated
   traffic, not merely "still works."
3. Full regression suite clean (pre-existing flakes excepted, triaged not assumed, per this session's
   own established `feedback_triage_before_dismissing_as_flaky` practice).
4. Mutation testing runs after DELIVER, per this repo's own `per-feature` strategy (root `CLAUDE.md`)
   — 100% effective kill rate on the new/changed cardinality-bounding logic.
5. Evolution doc written; `docs/product/production-readiness-audit-2026-09-08.md` row 2 updated to
   CLOSED at FINALIZE (this DISCUSS only updates it to IN PROGRESS — see § Next Wave).
6. Memory updated.

## Wave: DISCUSS / [REF] Out of Scope

- **Finding #14 from the same audit** (pre-authentication 3-round-trip amplification against the
  shared system DB) — same root architectural pattern (rate-limiter-runs-before-auth on unvalidated
  input) but a distinct harm mechanism (DB round-trip amplification, not metrics-cardinality memory
  exhaustion) and a separately tracked, still-"Not started" blocker. Not fixed here.
- **Finding #39 from the same audit** (`event_type` becoming an unbounded Prometheus label on the
  Stripe webhook route) — same unbounded-cardinality mechanism, different route, gated behind Stripe
  signature verification, a separate low-severity finding. Not fixed here.
- **Re-ordering the gRPC pipeline so `authenticate()` runs before the rate limiter** — a deeper
  architectural change that would address both #2's and #14's shared root cause at once, but is not
  requested or required by this feature's own narrower requirement (bound the metric label, not
  change when authentication runs). DESIGN may note it as a considered alternative; this DISCUSS does
  not mandate it.
- **The exact bounding mechanism** — explicitly DESIGN's own investigation, not decided or
  investigated in this DISCUSS (three candidates named in § Business Context as its starting point).
- **Implementing `tests/observability/`'s pre-existing `#[ignore]`d/`todo!()` test stubs** for their
  own sake, independent of what this feature's own AC-RLV-01/02 require — a separate backlog item
  belonging to the `observability` feature itself.
- **Any code implementation** — this is DISCUSS only, per explicit task instruction; DESIGN performs
  the actual investigation and implementation planning.

## Wave: DISCUSS / [REF] Walking Skeleton Strategy

**Strategy A** (real, minimal, end-to-end) — every scenario is a real gRPC request against a real
running server instance, with a real `GET :9090/metrics` scrape to observe the resulting label set,
mirroring `stripe-webhook-secret-required`'s own Strategy A precedent for this exact
composition-root/middleware layer. This feature IS the walking skeleton — single story, no further
slicing.

## Wave: DISCUSS / [REF] Driving Ports

The existing gRPC :8080 listener (any RPC handler that calls `extract_project_id` +
`RateLimiter::check`, e.g. `GetDocument`) and the existing Admin :9090 `GET /metrics` endpoint
(already exists, `observability` feature). Zero new RPC/HTTP endpoint — this feature changes only
what label value is recorded for unvalidated pre-auth input.

## Wave: DISCUSS / [REF] Pre-requisites

- None beyond what already exists. `RateLimiter::check()`, `extract_project_id()`, and the Prometheus
  recorder (`get_or_install_prometheus_handle`) all already exist and are unchanged in their core
  responsibilities by this feature — only the label-value computation for unvalidated input is in
  scope.
- The `observability` feature's own `/metrics` endpoint, histogram buckets, and counter conventions
  (ADR-016) are the established pattern this feature must remain consistent with.

## Wave: DISCUSS / [REF] Definition of Ready Validation

### DoR Checklist (9-item hard gate)
1. [x] Story traces to a job_id (JOB-12) — reused, not new, with reasoning against two nearer
   alternatives (JOB-11, JOB-13) explicitly documented in § Persona & Job.
2. [x] Story has a complete Elevator Pitch (Before / After / Decision enabled).
3. [x] Every AC is testable without ambiguity (5 ACs, each a real gRPC-request + metrics-scrape
   assertion against a real running server, or a real full-suite regression run).
4. [x] Walking Skeleton identified (US-01 is the whole feature's walking skeleton).
5. [x] Scope Assessment passed.
6. [x] Story is not `@infrastructure`-only with no user-visible value — Decision 4 = Yes (full JTBD
   path); it directly enables Sam Chen's own trust decision (Elevator Pitch "Decision enabled") that
   the `/metrics` surface JOB-12 established is itself safe to expose in production.
7. [x] Out of Scope explicitly named (6 items, each reasoned, including the two related findings
   #14/#39 and the deeper pipeline-reordering alternative).
8. [x] Outcome KPIs have a numeric framing (unbounded → bounded constant) and measurement methods.
9. [x] Prior-wave artifacts read and reconciled (the audit's own finding #2 and #14, JOB-12's existing
   job story and its own D-OBS-7 assumption documented in `obs04`'s test file, and
   `stripe-webhook-secret-required`'s own precedent all directly informed this feature's shape; no
   contradiction found with any existing decision).

### DoR Status: **PASSED**

## Wave: DISCUSS / [REF] Wave Decisions Summary

### Key Decisions
- [D1] Job reused: JOB-12 (`observability`), not JOB-11 (`fair-multitenancy` — wrong concern, the
  rate-limiting decision itself is unaffected) or JOB-13 (`production-deployment` — wrong shape, this
  is not a startup-config gap).
- [D2] Scope is narrowly the metrics-cardinality side-channel, not the rate-limiting decision logic
  and not the pre-auth/post-auth pipeline ordering — both explicitly named as related but out of
  scope, mirroring finding #14's own separate tracking in the audit.
- [D3] The exact bounding mechanism is explicitly DESIGN's own investigation; three candidates named
  as its starting point, none decided here.
- [D4] `tests/observability/`'s existing stub test files are confirmed, by direct reading, to be
  unimplemented (`#[ignore]`/`todo!()`) scaffolding today — this DISCUSS states the "must continue to
  pass" requirement honestly (AC-RLV-03) rather than implying they currently exercise real behavior.

### Requirements Summary
- Primary need: unauthenticated, attacker-chosen `project_id` input cannot cause unbounded Prometheus
  label growth in the rate limiter's own metrics; real, authenticated project traffic and the actual
  rate-limiting decision logic remain completely unaffected.
- Walking skeleton scope: US-01, the entire feature — single story, 6 UAT scenarios.
- Feature type: Security fix.

### Constraints Established
- Zero behavior change to the rate-limiting allow/reject decision for real, authenticated traffic.
- Zero behavior change to when the rate limiter runs relative to `authenticate()` (pre-auth ordering
  is preserved; only the label computed for unvalidated input is bounded).
- No new bounded context, no new domain type, no new RPC/HTTP endpoint.
- No modification to the pre-existing `tests/observability/` test files as part of this feature.

### Upstream Changes
None — this DISCUSS extends JOB-12's existing scope (same job, same persona), consistent with this
session's own repeated "make it real / close a gap in an already-covered surface" pattern (e.g.
`customer-db-transaction-sweeper` → JOB-12).

## Wave: DISCUSS / [REF] Next Wave

**Handoff To**: nw-solution-architect (DESIGN wave)
**Deliverables**: this feature-delta.md, 4 locked Decisions (D1-D4), 1-story walking-skeleton plan, 5
ACs (AC-RLV-01 through AC-RLV-05) to design executable scenarios against. DESIGN's own investigation
scope: (1) the precise bounding mechanism (validate-then-label with sentinel fallback; no
project_id label pre-authentication; or another approach found more appropriate after reading the
code), (2) whether/how to implement real assertions in `tests/observability/`'s existing stub files
to prove AC-RLV-01/02, (3) confirming no other RPC handler beyond `handle_get_document` needs
special-case treatment (the audit's own citation names `handler.rs:99-107,1267,1272` as the
representative pattern — DESIGN should confirm via its own blast-radius grep how many handlers share
this exact `extract_project_id` → `rate_limiter.check` call shape).

---

## Wave: DESIGN / [REF] Reading Confirmation

✓ `crates/embyr-server/src/grpc/handler.rs` read in full around `extract_project_id` (lines 96-107,
confirmed byte-for-byte as DISCUSS described: `name.splitn(5, '/')`, accepts any non-empty `pid`
substring, zero format/existence check) and `handle_get_document` (lines 1258-1296+, confirmed exact
ordering: `extract_project_id` (1264) → `rate_limiter.check` (1267, pre-auth) → `authenticate` (1272)).
✓ **Blast-radius grep performed independently** (`rg '\.rate_limiter\.check\(' crates/embyr-server/src/grpc/handler.rs --count`
— exact count, not estimated) — confirmed **15 call sites** in `handler.rs` (lines 1267, 1570, 1809,
2038, 2264, 2455, 2522, 2694, 2734, 2823, 2957, 3005, 3293, 3534, 3675), **not the ~18 DISCUSS's own
reading estimated** — this DESIGN corrects that number via direct count rather than repeating it,
per the task's own instruction to confirm independently rather than trust the prior finding. Plus
**1 call site** in `rate_limit.rs:349` (`rest_rate_limit_middleware`, the REST/`accounts:<verb>`
identity-bridge gate, confirmed sharing the SAME `Arc<RateLimiter>` instance per `lib.rs:404-415`'s
own comment). **Every single one of these 16 call sites routes through the one public method
`RateLimiter::check()` (`rate_limit.rs:140-150`) — this is the ONLY place
`metrics::counter!("embyr_rate_limit_requests_total"...)` is ever invoked** (confirmed via
`rg 'metrics::counter!|increment_counter!' crates/` — the only other project_id-adjacent counter in the
codebase, `embyr_rate_limit_pg_timeout_total` at `rate_limit.rs:178`, carries no `project_id` label and
is unaffected by this feature). **This means the fix has exactly ONE edit point, not 16** — none of the
15 `handler.rs` call sites or the 1 REST middleware call site need
any change; `check()`'s public signature (`Result<RateLimitInfo, RateLimitInfo>`) is unchanged.
✓ `crates/embyr-server/src/middleware/rate_limit.rs` read in full again at DESIGN depth. Confirmed
`check_inner`/`check_pg`/`check_in_process` are all `check()`'s own private helpers (not called from
anywhere else in the workspace — confirmed no `#[cfg(test)]` module in this file calls them directly).
Traced exactly what `project_id` is used for on each path:
  - **Rate-limit bucket KEY** (legitimate, must stay unchanged per AC-RLV-02): `check_pg`'s `UPDATE
    rate_buckets ... WHERE project_id = $3` (line 208 today) and `check_in_process`'s
    `buckets.entry(project_id.to_string())` (line 292 today, in-process `HashMap<String, TokenBucket>`
    key). Both are internal, non-Prometheus-facing state — an internal Postgres row key and an
    in-process map key respectively, not a public-facing unbounded surface.
  - **Prometheus METRIC LABEL** (the vulnerable part): `check()`'s own `metrics::counter!(...,
    "project_id" => project_id.to_owned(), ...)` (lines 143-148 today) — entirely independent code,
    downstream of the bucket-key usage, sharing only the same string value.
✓ `crates/embyr-core/src/domain/project.rs` read in full. `ProjectId::new()` / `is_valid_project_id`
(lines 7-22, 102-112) already implements a cheap format check (`^[a-z][a-z0-9-]{0,62}$`) — this
project's own established "reuse, don't invent" candidate for a format gate. **Investigated and
rejected as the metrics-label mechanism** — see § Mechanism Decision below; the format-valid string
space is still astronomically large and fully attacker-controlled, so gating the label on format
validity alone does not bound cardinality against a targeted attacker, only against accidental/typo
garbage. (`validate_field_path` in `crates/embyr-core/src/domain/query.rs:16-24` was also read as the
task's suggested reference pattern for "a similar cheap-format-gate used elsewhere" — same conclusion:
a format gate answers "syntactically plausible," never "bounded in count.")
✓ `crates/embyr-server/src/admin/handlers/provision.rs` read (lines 120-160+). Found the load-bearing
fact this design turns on: `insert_rate_bucket_in_tx` (lines 135-157) inserts a `rate_buckets` row in
the **same transaction** as every project's own `projects` INSERT, in all four backend-mode
provisioning branches (grep-confirmed: 4 call sites at lines 224, 276, 335, 389, one per backend mode).
Every genuinely-provisioned project therefore has a `rate_buckets` row from the moment of creation —
before any request, gRPC or REST, ever reaches it.
✓ `tests/observability/acceptance/obs04_rate_limit_metrics.rs` and `tests/observability/common/mod.rs`
re-read at DESIGN depth. Confirmed no contradiction with the chosen mechanism, and surfaced one
important constraint the mechanism had to satisfy: every one of obs04's 3 stub scenarios is a
**freshly-provisioned project's first-ever request** in a fresh test process (`ObsTestContext::start()`
+ `provision_project()` then exactly one `make_grpc_call()`). A naive "confirm real via a set populated
on `authenticate()` success" design (literal reading of DISCUSS candidate (b)) would mislabel every one
of these scenarios, because `rate_limiter.check()` runs strictly before `authenticate()` in the same
request (see above) — the confirmation write would always arrive one request too late for a
single-request test scenario. This directly shaped the mechanism selected below (see § Mechanism
Decision, "Why naive (b) is rejected"). `common/mod.rs:15`'s own doc comment
(`make_grpc_call(): LIVE — tonic GetDocument (unauthenticated; counter fires regardless)`) confirms the
counter firing pre-auth is the existing, intended contract — this design does not change *when* the
counter fires, only what label value it uses.
✓ Existing regression-guard test files enumerated via grep (`rg 'RateLimiter|rate_limiter' tests/`):
`tests/acceptance/us_14_rate_limiting.rs` (4 real, non-`#[ignore]` tests), `tests/rest_rate_limiting/acceptance/rest_calls_are_rate_limited.rs`
(2 real tests), `tests/distributed_rate_limiting/acceptance/b12_postgres_rate_limit.rs` (1 real
walking-skeleton test + 3 `#[ignore]`d), `tests/distributed_rate_limiting/acceptance/b13_fallback_on_pg_failure.rs`
(3 `#[ignore]`d, 0 real), `tests/production_readiness/acceptance/pr01_config_from_env.rs` (constructs a
`RateLimiter` from config, not behavior-relevant to this feature), plus `tests/observability/acceptance/obs04_rate_limit_metrics.rs`
itself (3 `#[ignore]`d stubs, per DISCUSS's own reading, unchanged). See § Handoff Package for which of
these DISTILL/DELIVER must run as regression guards.

## Wave: DESIGN / [REF] Mechanism Decision

**Decision: reuse the rate limiter's own pre-existing existence check as the metrics-label gate.**
Full reasoning, alternatives, and consequences are recorded in
`docs/product/architecture/adr-069-rate-limit-metric-label-cardinality-bounding.md` (new ADR) —
summarized here for the walking-skeleton record:

- **Neither DISCUSS candidate, taken literally, is correct.** (a) Format-validate-then-sentinel does
  not bound cardinality (an attacker can generate unlimited format-valid strings matching `ProjectId`'s
  own `^[a-z][a-z0-9-]{0,62}$` regex). (b) Post-auth-confirmed labeling, implemented literally as "move
  the metric to after `authenticate()` succeeds" or "gate on a new set populated by `authenticate()`",
  breaks first-request labeling for every real project (see Reading Confirmation above) and would
  require editing all 18 `handler.rs` call sites instead of one function.
- **The actual mechanism is a corrected, zero-new-infrastructure realization of (b)'s intent**: instead
  of inventing a NEW "is this project confirmed real" signal populated by `authenticate()`, reuse the
  signal `check_pg()`/`check_in_process()` **already compute for themselves** to make the rate-limiting
  decision — Postgres row existence in `rate_buckets` (Postgres-backed path) or map-key existence in the
  in-process `HashMap` (in-process fallback path). Because `provision.rs::insert_rate_bucket_in_tx`
  atomically inserts a `rate_buckets` row for every real project at creation time, this signal is true
  for real projects from their very first request — no cold-start gap, no new DB round trip, no new data
  structure, no change to the actual rate-limit bucket key (AC-RLV-02 preserved exactly).
- **The label**: `known_existing` (the reused boolean) → the real `project_id`; otherwise → a constant
  sentinel, `"unconfirmed"`. This bounds the metric family to at most one new permanent label value
  (`"unconfirmed"` × 2 outcomes = 2 time series) for the entire life of the process, regardless of how
  many distinct garbage `project_id` strings an attacker sends — satisfying AC-RLV-01 exactly.
- Format validation (`ProjectId::new`) is **not** used as part of this mechanism — it adds no bounding
  value the existence check doesn't already provide, and introducing it would be unrequested complexity
  (ponytail: don't add a gate that does no work). No parsing is added to the hot path, so AC-RLV-05
  (malformed input must not panic) is satisfied by construction — the design adds no new
  `unwrap()`/regex/parsing calls, only reuses a boolean two call sites already compute.

### Code sketch (illustrative — DELIVER owns exact naming/decomposition)

```rust
// crates/embyr-server/src/middleware/rate_limit.rs

/// Sentinel label for `project_id` values not confirmed to belong to a
/// provisioned project at the time of this rate-limit check (ADR-069).
/// Bounds `embyr_rate_limit_requests_total` cardinality: an unauthenticated
/// attacker sending N distinct, never-provisioned project_id strings
/// contributes at most this ONE new label value, never N.
const UNCONFIRMED_PROJECT_LABEL: &str = "unconfirmed";

pub async fn check(&self, project_id: &str) -> Result<RateLimitInfo, RateLimitInfo> {
    let (result, known_existing) = self.check_inner(project_id).await;
    let outcome = if result.is_ok() { "allowed" } else { "rejected" };
    let label = if known_existing {
        project_id.to_owned()
    } else {
        UNCONFIRMED_PROJECT_LABEL.to_owned()
    };
    metrics::counter!(
        "embyr_rate_limit_requests_total",
        "project_id" => label,
        "outcome" => outcome
    )
    .increment(1);
    result
}

/// Returns the rate-limit decision AND whether `project_id` was already
/// known to this rate limiter's backing store *before* this call (ADR-069).
async fn check_inner(&self, project_id: &str) -> (Result<RateLimitInfo, RateLimitInfo>, bool) {
    if !self.enabled {
        return (Ok(RateLimitInfo { remaining: self.capacity, limit: self.capacity, reset_ms: 0 }), false);
    }
    if let Some(pool) = &self.pg_pool {
        match tokio::time::timeout(..., self.check_pg(project_id, pool)).await {
            Ok(result_and_existed) => return result_and_existed,
            Err(_timeout) => { /* unchanged pg-timeout counter + warn! */ }
        }
    }
    self.check_in_process(project_id)
}

// check_pg: the `Some(remaining)` branch (RETURNING succeeded) => existed = true.
// The `None` branch already computes `row_exists` today to disambiguate
// "rejected" from "row absent" — surface that same boolean instead of
// discarding it; the `!row_exists` sub-branch (today's finding-#14 insert)
// existed = false.

// check_in_process: capture `buckets.lock()...contains_key(project_id)`
// BEFORE the `.entry(...).or_insert_with(...)` call, return it alongside
// the existing Ok/Err decision.
```

`RateLimitInfo` (`crates/embyr-core/src/rate_limit.rs`) is **not modified** — the `known_existing`
signal is internal to `rate_limit.rs` and never crosses the `embyr-core` boundary or reaches callers.

## Wave: DESIGN / [REF] Residual Risks Carried Forward (documented, not fixed here)

- Finding #14 (DB amplification via `check_pg`'s auto-insert-on-absent-row) remains unfixed. A
  secondary, bounded consequence for THIS feature: if an attacker reuses the same fake `project_id`
  many times, #14 will insert a row for it, and from the second occurrence onward this feature's
  mechanism will treat it as "existing" and label it with the attacker's chosen string. This does not
  reopen unbounded cardinality (bounded by however many distinct IDs the attacker chooses to repeat,
  not by request count) and is an inherited, documented consequence of #14 being out of scope, not a
  new gap. See ADR-069 § Consequences for full reasoning.
- In-process fallback mode (`check_in_process`, used when no `pg_pool` is configured or during a
  Postgres timeout) has a per-instance, not-pre-seeded version of the existence signal — a real
  project's first request landing on a given instance before that instance has seen it before is
  labeled `"unconfirmed"` once, self-correcting from the next request onward. Accepted (ADR-069 §
  Consequences) — mirrors ADR-015's already-documented fail-open trade-off for this fallback path.

## Wave: DESIGN / Handoff Package

**Files requiring a change:**
1. `crates/embyr-server/src/middleware/rate_limit.rs` — the ONLY production-code file requiring a
   change. `check()` (lines 140-150), `check_inner()` (154-185), `check_pg()` (193-280),
   `check_in_process()` (287-311) — signatures change to thread a `known_existing: bool` alongside the
   existing `Result<RateLimitInfo, RateLimitInfo>`; add the `UNCONFIRMED_PROJECT_LABEL` constant and the
   label-selection branch in `check()`. No other function in this file changes.

**Files confirmed to need NO change (blast radius, grep-verified — exact count, corrects DISCUSS's own
"~18" estimate):**
- `crates/embyr-server/src/grpc/handler.rs` — all 15 `rate_limiter.check(...)` call sites (lines 1267,
  1570, 1809, 2038, 2264, 2455, 2522, 2694, 2734, 2823, 2957, 3005, 3293, 3534, 3675) consume
  only `check()`'s unchanged public `Result<RateLimitInfo, RateLimitInfo>` return type.
- `crates/embyr-server/src/middleware/rate_limit.rs:337-353` (`rest_rate_limit_middleware`) — same
  shared `check()` call, same unchanged signature.
- `crates/embyr-server/src/observability.rs` — no eviction/cap infrastructure needed; the fix is
  entirely at the label-value computation, not the recorder.
- `crates/embyr-core/*` — `ProjectId`, `RateLimitInfo` both investigated, both unmodified.
- `crates/embyr-server/src/admin/handlers/provision.rs` — its existing `rate_buckets` insert behavior
  is read as a signal, not modified.

**Documentation changes made this wave:**
- `docs/product/architecture/adr-069-rate-limit-metric-label-cardinality-bounding.md` (new).
- `docs/product/architecture/adr-016-prometheus-metrics.md` — one amendment note added above the
  existing "HIGH CARDINALITY" section, pointing to ADR-069 (ADR left otherwise unmodified per
  immutable-ADR convention).

**Regression guards DISTILL/DELIVER must run (grep-verified to exist and be real, non-`#[ignore]`d
tests unless noted):**
- `tests/acceptance/us_14_rate_limiting.rs` — 4 real tests, proves the allow/reject DECISION logic
  (AC-RLV-02's core regression guard).
- `tests/rest_rate_limiting/acceptance/rest_calls_are_rate_limited.rs` — 2 real tests, proves the REST
  `accounts:<verb>` gate (shares `check()`) is unaffected.
- `tests/distributed_rate_limiting/acceptance/b12_postgres_rate_limit.rs` — 1 real walking-skeleton
  test (`distributed_rate_limit_rejects_when_bucket_exhausted`) exercises `check_pg`'s exact
  Postgres-backed decision path this feature's signature change touches most directly; its 3
  `#[ignore]`d siblings are NOT currently exercised (informational only, not a required regression gate).
- `tests/observability/acceptance/obs04_rate_limit_metrics.rs` — currently 3 `#[ignore]`d `todo!()`
  stubs (AC-RLV-03 requires this file stay unmodified by this feature; implementing these stubs is
  explicitly out of scope per DISCUSS, but DISTILL should confirm its own new AC-RLV-01/02 acceptance
  tests are written as NEW files, not edits to this one).
- Full workspace `cargo test` (AC-RLV-04) — run once, at the pre-commit gate, per this repo's own
  root `CLAUDE.md` test-run token-discipline rule (scoped/foreground runs during the inner loop,
  full-suite run once before commit).

**New test scenarios DISTILL must design (not existing today):** an integration-level test proving
AC-RLV-01 directly — send N (e.g. 10,000) `GetDocument` requests with N distinct, never-provisioned
`project_id` strings against a real running server, scrape `GET :9090/metrics`, assert the
`embyr_rate_limit_requests_total` family's distinct `project_id` label count attributable to this
traffic is O(1) (specifically: exactly the `"unconfirmed"` value), not O(N). A second scenario proving
a freshly-provisioned project's first-ever request gets its own accurate label immediately (the
cold-start guarantee ADR-069 relies on `provision.rs` for) — mirrors, but does not modify,
`obs04_rate_limit_metrics.rs`'s own documented scenario shape.
