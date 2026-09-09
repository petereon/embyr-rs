# Feature Delta: stripe-webhook-body-limit

## Wave: DISCUSS / [REF] Prior Wave Consultation — Reading Confirmation

✓ `docs/product/production-readiness-audit-2026-09-08.md` finding #3 confirmed by direct reading of
`crates/embyr-server/src/admin/middleware/stripe_signature.rs` (55 lines, read in full). The exact
vulnerable line is present unchanged: `axum::body::to_bytes(body, usize::MAX)` (line 44), called
BEFORE the `Stripe-Signature` HMAC check (line 50) — the signature header presence is checked first
(line 41), but the body is fully buffered into memory before verification even starts, and
`usize::MAX` places no ceiling on that buffer. An unauthenticated attacker (no credential of any
kind, same threat class as findings #1/#2 in this same audit) can send an arbitrarily large request
body to `POST /admin/v1/webhooks/stripe` and force the process to allocate memory proportional to
whatever they choose to send, with zero authentication.
✓ Grepped `crates/embyr-server/src` for `RequestBodyLimitLayer`, `DefaultBodyLimit`, `body_limit`,
`MAX_BODY` (case-insensitive) — zero matches. No body-size-limit layer of any kind exists anywhere
in this crate today, confirming the audit's own claim directly rather than assuming it.
✓ `docs/feature/stripe-webhook-secret-required/feature-delta.md` (finding #1, same route) read in
full. Confirmed the CURRENT real state of the route (not just the design sketch): the webhook
sub-router is now conditionally built and mounted — `crates/embyr-server/src/admin/router.rs:143`
(`let webhook_router: Option<Router> = webhook_signing_secret.map(|secret| { ... })`) and merged at
line 424-425 (`if let Some(webhook_router) = webhook_router { router = router.merge(webhook_router); }`)
— only when `STRIPE_SECRET_KEY` is genuinely non-empty. Confirmed directly against the real file
(`crates/embyr-server/src/admin/router.rs`, grepped lines 77-151 and 420-426), not merely the
feature-delta's own design narrative — that feature has actually shipped, not just been designed.
This feature's fix must layer onto that existing conditional mount, not re-touch it: whether Stripe
billing is enabled or not, if the webhook route IS mounted and reachable, the body-buffering-before-
signature-check ordering inside `stripe_signature_middleware` is unconditionally vulnerable — the
two findings are orthogonal (finding #1 controls WHETHER the route is reachable at all; finding #3
controls how much memory an already-reachable route can be forced to allocate per request).
✓ `docs/feature/rate-limiter-project-id-validation/feature-delta.md` (finding #2, different route)
read in full to confirm this project's own established single-file, Tier-1-only `feature-delta.md`
convention (`## Wave: DISCUSS / [REF] {Section}` headings, no standalone `acceptance-criteria.md`/
`outcome-kpis.md`/`story-map.md` files) and its own precedent for reasoning explicitly against the
nearest-alternative job before committing to a reuse decision, and for reusing the same attacker
persona ("Marcus Webb") across sibling audit-derived security fixes for narrative continuity. This
DISCUSS mirrors that convention and reuses that same persona.
✓ `docs/product/jobs.yaml` read in full (all 20 jobs, JOB-01 through JOB-20). Searched explicitly for
a dedicated "webhook body limit," "resource exhaustion," or "DoS hardening" job before reusing an
existing one — none exists. Three candidates evaluated (see § Persona & Job below); **JOB-13
`production-deployment`** (P2 Sam Chen) is the correct fit, confirmed by direct comparison against
JOB-11 (`fair-multitenancy`) and JOB-12 (`observability`), not merely assumed from the task's own
suggestion.
✓ `crates/embyr-server/src/admin/router.rs` (lines 1-160, 380-430) read in full to confirm the exact
current shape of `build_admin_router` and the webhook sub-router's construction, so this DISCUSS's
System Constraints do not contradict the already-shipped conditional-mount behavior.

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

- Feature type: **Security fix / Infrastructure** (production-deployment resource-exhaustion
  hardening on an existing, already-partially-hardened route), not an SDK-developer-facing feature.
- JTBD: **reuse JOB-13** (`production-deployment`, P2 Sam Chen) — see § Persona & Job for reasoning
  against JOB-11 and JOB-12, the two nearer-by-file-location candidates.
- Decision 4 (full JTBD path vs. infrastructure-only): **Yes — full JTBD path.** This is a real,
  unauthenticated, remotely exploitable resource-exhaustion vulnerability against a real production
  deployment, not infrastructure-only scaffolding.
- Walking Skeleton: **Yes** — single story, no further slicing (mirrors both `stripe-webhook-secret-
  required` and `rate-limiter-project-id-validation`'s own single-story precedent for a confined,
  single-mechanism security fix against the same audit).
- UX Research Depth: **Lightweight** — an operator-facing production-hardening fix, not an end-user
  journey; no ASCII TUI mockups or emotional-arc journey YAML warranted.
- **Scope lock (narrow vs. global body-limit layer) is this DISCUSS's own call, decided below in
  § Business Context, not deferred to DESIGN.**
- **The exact mechanism** (a body-size-limit layer applied inside `stripe_signature.rs` itself — e.g.
  a manual `Content-Length` pre-check plus a bounded `to_bytes` call — vs. an axum
  `RequestBodyLimitLayer`/`DefaultBodyLimit` applied at the router-construction site for the webhook
  sub-router) is explicitly **DESIGN's own call**, flagged as an open question below, not locked here.

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: **P2 Sam Chen (Service Operator / Platform Engineer)** — unchanged from JOB-11/12/13's
existing profile. Not an SDK developer (P1 Alex), not the billing self-service persona (P5 Chris,
JOB-14) — this finding was surfaced by the same production-readiness security scan that produced
findings #1 and #2, against operational infrastructure Sam Chen alone deploys and operates.

**Job**: **JOB-13 `production-deployment`**, reused, EXTENDED (not replaced). Candidates considered
and rejected:
- **JOB-11 `fair-multitenancy`** (rate limiting) — wrong fit. This finding is not about per-project
  request-rate fairness across a cluster; it is about a single unauthenticated request being able to
  force unbounded memory allocation before any rate-limit or auth check ever runs. The rate limiter
  itself is untouched by this finding.
- **JOB-12 `observability`** (metrics) — wrong fit, and a deliberately different shape from finding
  #2 (which WAS a JOB-12 extension). Finding #2's harm mechanism was an unbounded Prometheus label
  side-channel; this finding's harm mechanism is unbounded memory buffering in a request-handling
  path, with no metrics/observability dimension at all. Reusing JOB-12 here would conflate two
  structurally different vulnerability classes that happen to share an audit.
- **JOB-13 `production-deployment`** (the correct fit) — this job's own functional dimension already
  established the exact discipline this finding needs: safe, fail-fast, resource-bounded production
  configuration ("server exits non-zero with named missing var," "no cryptic failures"). JOB-13 has
  already been extended twice for exactly this kind of "close a resource-safety gap on an existing
  production-deployment surface" pattern — once for TLS termination (`firestore-tls-support`) and
  once for this SAME webhook route's config-safety gap (`stripe-webhook-secret-required`, finding
  #1). This finding is a third, same-shape extension: an unauthenticated request against an
  already-existing production endpoint must not be able to exhaust server memory, mirroring JOB-13's
  own "feel in control — startup errors are named and specific, no cryptic failures" emotional
  dimension, extended from startup-time configuration safety to request-time resource safety on the
  identical route family #1 already hardened.

## Wave: DISCUSS / [REF] Business Context

Today, `stripe_signature_middleware` (`crates/embyr-server/src/admin/middleware/stripe_signature.rs`)
calls `axum::body::to_bytes(body, usize::MAX)` (line 44) to buffer the raw request body for HMAC
verification — necessary because Stripe's signature is computed over the exact raw bytes, so no
streaming/re-serialization is possible. The `Stripe-Signature` header's mere PRESENCE is checked
first (line 41), but its cryptographic validity is not checked until AFTER the entire body has
already been read into memory (line 50, `verify_webhook_signature`). Because `usize::MAX` places no
ceiling on this buffer, an unauthenticated attacker who merely knows the route exists (or discovers
it by probing, if Stripe billing happens to be enabled per finding #1's resolved state) can send a
request with an arbitrarily large body — gigabytes, if the attacker chooses — and the ENTIRE payload
is allocated into memory before the signature check has any chance to reject it. Repeated or
concurrent requests of this shape are a straightforward unauthenticated remote memory-exhaustion
(OOM/DoS) vector against the whole `embyr-server` process, taking down every real tenant's traffic on
that instance — the same blast-radius shape as findings #1 and #2 in this same audit.

Real Stripe webhook payloads are small: Stripe's own documented event payloads are typically well
under 1MB, and Stripe does not send legitimate webhook bodies anywhere near multi-megabyte scale.
This DISCUSS does not pick the exact byte ceiling — that is a DESIGN decision, informed by Stripe's
own real-world payload sizes with generous headroom — but locks the non-negotiable outcome: whatever
limit DESIGN picks, **a legitimate, normally-sized Stripe webhook must still succeed end-to-end,
unconditionally** (this must never break real Stripe traffic; see AC-WBL-02).

**The scope question this DISCUSS decides** (not deferred to DESIGN): should the body-size limit be
a narrow, root-cause fix confined to the Stripe webhook route, or a global body-limit layer applied
to every admin route?

**Decided: narrow, root-cause fix — this route only, not a global layer.** Reasoning:
- The vulnerability is specific to this ONE route's specific shape: it is the only unauthenticated
  (pre-auth) route in the admin router that manually buffers a full request body via
  `axum::body::to_bytes(..., usize::MAX)` before any credential check. Every other admin route sits
  behind `operator_auth_middleware` or `session_auth_middleware` (both already reject before any
  handler-level body reads, per `stripe_signature.rs`'s own doc comment describing itself as
  mirroring that SHAPE), so an unauthenticated attacker cannot reach a large-body-buffering code path
  anywhere else in this router today. A global layer would fix a problem that does not exist on any
  other route while adding blanket behavior to routes this DISCUSS has not investigated (JSON body
  sizes for admin CRUD payloads, OIDC provider config, etc. — none of which this DISCUSS confirmed
  are safe under a single global ceiling).
- Matches this project's own established root-cause-fix-at-the-smallest-correct-scope pattern: both
  sibling audit fixes (#1, #2) scoped their fix to the specific mechanism identified, not a broader
  hardening sweep, and explicitly named broader related concerns as Out of Scope rather than folding
  them in.
- A global default body-limit layer for ALL admin routes may well be independently worth doing later
  (defense-in-depth is generally good practice), but that is a separate, unevidenced hardening
  proposal this DISCUSS did not investigate (no audit finding names it, no other route's body-size
  behavior was read) — bundling it in here would be scope creep beyond what the audit's finding #3
  actually requires.
- DESIGN retains the choice of WHERE the limit is implemented (inside `stripe_signature.rs` itself,
  replacing the `usize::MAX` call directly; or as an axum `RequestBodyLimitLayer`/`DefaultBodyLimit`
  applied only to the webhook sub-router at its construction site in `router.rs`) — both keep the fix
  scoped to this one route; this DISCUSS flags both as open candidates for DESIGN, locking only that
  the SCOPE stays this route, not all admin routes.

Axum's `RequestBodyLimitLayer` returns `413 Payload Too Large` by default when the configured limit is
exceeded (confirmed against the audit's own suggestion and this DISCUSS's own general axum knowledge)
— the idiomatic, correct status code for this rejection shape. This DISCUSS locks 413 as the expected
rejection behavior's status-code family; DESIGN confirms and wires the exact mechanism.

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

**Signals checked**: >10 user stories? No (1). >3 bounded contexts/modules? No — entirely confined to
`crates/embyr-server/src/admin/middleware/stripe_signature.rs` (the fix itself) and, depending on
DESIGN's chosen mechanism, possibly `crates/embyr-server/src/admin/router.rs` (if a layer is applied
at the webhook sub-router's construction site instead) — both already-touched files from finding #1,
same layer this project's own prior two audit fixes operated in. Walking skeleton >5 integration
points? No (3: a real oversized-body request rejected before full buffering; a real normally-sized,
correctly-signed Stripe webhook succeeding end-to-end; a real correctly-signed webhook at or near the
chosen size ceiling still succeeding — all against the same running server instance). Estimated
effort >2 weeks? No — this is a single-function-scale fix per the audit's own overall framing ("most
fixes are single-function"); no new domain concept, no new adapter trait, no new bounded context.
Multiple independent user outcomes? No — "an oversized body cannot be fully buffered before signature
verification" is a single outcome; the legitimate-traffic-unaffected case is a regression-guard
verification point of that ONE outcome, not a separate story.

**Scope Assessment: PASS** — 1 user story, 1 bounded context (`embyr-server` admin middleware,
possibly touching the already-modified `router.rs` conditional-mount site), estimated ≤1 day, 5 UAT
scenarios (within the 3-7 right-sized range).

## Wave: DISCUSS / [REF] System Constraints

- The fix is scoped to the Stripe webhook route only (`POST /admin/v1/webhooks/stripe`) — a narrow,
  root-cause fix, not a global body-limit layer applied to all admin routes (see § Business Context
  for the full reasoning behind this locked scope decision).
- This feature must layer onto, not conflict with or re-touch, `stripe-webhook-secret-required`'s
  already-shipped conditional-mount logic (`admin/router.rs:143`, `Option<Router>` gated on
  `STRIPE_SECRET_KEY`). Whether the route is mounted at all is unaffected by this feature; this
  feature only changes how much of the body an already-reachable route will buffer.
- A legitimate, normally-sized Stripe webhook event must succeed end-to-end, unconditionally — this
  is a non-negotiable regression guard (AC-WBL-02), not a nice-to-have.
- The rejection behavior for an oversized body must occur BEFORE the full body is buffered into
  memory — a fix that still fully buffers the oversized body and only rejects afterward (e.g. a
  post-hoc length check after `to_bytes` already completed) does not close the vulnerability and does
  not satisfy AC-WBL-01.
- The exact byte ceiling is DESIGN's own decision, informed by Stripe's own real-world payload sizes
  (documented as well under 1MB) with generous headroom (audit's own suggestion names ~5MB-ish as
  reasonable) — not locked here.
- The exact mechanism (manual `Content-Length`/bounded-`to_bytes` check inside `stripe_signature.rs`,
  vs. an axum `RequestBodyLimitLayer`/`DefaultBodyLimit` applied at the webhook sub-router's
  construction site in `router.rs`) is DESIGN's own decision — both candidates are named as open
  questions for DESIGN, not decided here.
- No new external dependency, no new RPC/HTTP endpoint — the webhook route already exists; this
  feature changes only how much of an incoming request body it will read before rejecting.
- Out of scope for this feature (see § Out of Scope): findings #1 (already closed,
  `stripe-webhook-secret-required`) and #2 (already closed, `rate-limiter-project-id-validation`),
  and finding #39 (`event_type` Prometheus label cardinality on this same route) — related, same
  route or same audit, separate blockers, not fixed here.

## Wave: DISCUSS / [REF] User Stories

### US-01: An Oversized Stripe Webhook Body Cannot Exhaust Server Memory Before Signature Verification

**job_id**: JOB-13

#### Elevator Pitch
**Before**: Sam Chen operates a production embyr deployment with Stripe billing enabled. "Marcus
Webb" — the same unauthenticated attacker persona this session's other two audit-derived fixes
already established — discovers `POST /admin/v1/webhooks/stripe` and sends a single request with a
multi-gigabyte body. `stripe_signature_middleware` calls `axum::body::to_bytes(body, usize::MAX)` and
attempts to buffer the entire body into memory BEFORE checking whether the `Stripe-Signature` header
is even cryptographically valid. A handful of concurrent requests like this exhaust the process's
memory and crash `embyr-server`, taking down every real tenant's traffic on that instance — with zero
authentication and zero valid Stripe credential required.
**After**: A request body larger than the configured limit is rejected with `413 Payload Too Large`
before the middleware finishes buffering it into memory — Marcus's multi-gigabyte payload never fully
lands in the process's memory, regardless of how many times he repeats or parallelizes the attempt.
Sam's real, normally-sized Stripe webhook traffic (a few KB to a few hundred KB per event, per
Stripe's own documented payload sizes) is completely unaffected and continues to succeed exactly as
before.
**Decision enabled**: Sam Chen can trust that the webhook route he enabled for billing is not itself a
memory-exhaustion weapon reachable by anyone on the internet — he does not need to add a separate
reverse-proxy body-size guard in front of embyr-server, and he does not need to worry that an
attacker with zero credentials can crash his production deployment by sending one oversized request.

#### Who
- Sam Chen (P2) | Service operator running embyr-server in production with Stripe billing enabled,
  exposed to unauthenticated internet traffic on the admin HTTP listener | Needs the webhook route to
  be resource-safe against arbitrary unauthenticated input, not merely correctly signature-gated.

#### Solution
Reject a request body larger than a configured size ceiling BEFORE it is fully buffered into memory,
returning `413 Payload Too Large`, while leaving legitimate, normally-sized Stripe webhook requests
completely unaffected. The exact byte ceiling and the exact implementation mechanism (in-middleware
bounded read vs. a router-level body-limit layer) are DESIGN's own decisions, not fixed here.

#### Domain Examples

**Example 1 (Happy Path — a real, normally-sized Stripe webhook succeeds)**: Stripe sends a real
`customer.subscription.updated` event for Meridian Health's account — a JSON payload of a few
kilobytes, correctly signed with the real shared secret. The request succeeds end-to-end exactly as
it does today: signature verifies, Meridian Health's subscription status updates correctly.

**Example 2 (Edge Case — a request right at the boundary)**: A real Stripe event with an unusually
large payload (e.g. an `invoice.finalized` event with many line items, still well under 1MB per
Stripe's own documented typical payload sizes) arrives. The request succeeds — the chosen ceiling has
generous headroom above any realistic Stripe payload, so legitimate traffic near the upper end of
normal is never at risk of false rejection.

**Example 3 (Error/Boundary — the exploit this feature closes)**: Marcus Webb, an unauthenticated
attacker with no credential of any kind, sends a `POST /admin/v1/webhooks/stripe` request with a
2GB body and no valid `Stripe-Signature` header value. Before this feature: the middleware attempts
to buffer all 2GB into memory before ever checking the signature, and repeating this a handful of
times exhausts server memory. After this feature: the request is rejected with `413 Payload Too
Large` before the body is fully buffered — the attacker's payload size no longer determines how much
memory the server allocates per request.

#### UAT Scenarios (BDD)

```gherkin
Scenario: A legitimate, normally-sized Stripe webhook succeeds end-to-end
  Given STRIPE_SECRET_KEY and STRIPE_WEBHOOK_SIGNING_SECRET are both correctly configured
  When Stripe sends a correctly-signed customer.subscription.updated event for Meridian Health,
    with a body size typical of real Stripe webhook payloads
  Then the request succeeds
  And Meridian Health's subscription status is updated exactly as it is today

Scenario: An oversized request body is rejected before being fully buffered into memory
  Given STRIPE_SECRET_KEY and STRIPE_WEBHOOK_SIGNING_SECRET are both correctly configured
  When an unauthenticated request to POST /admin/v1/webhooks/stripe carries a body larger than
    the configured size limit
  Then the request is rejected with 413 Payload Too Large
  And the server's memory usage for handling this request does not scale with the oversized body's
    actual size
  And no attempt is made to verify a Stripe-Signature header against a body that was never fully read

Scenario: A correctly-signed webhook at or near the configured ceiling still succeeds
  Given STRIPE_SECRET_KEY and STRIPE_WEBHOOK_SIGNING_SECRET are both correctly configured
  When Stripe sends a correctly-signed webhook event with a body size comfortably within the
    configured limit (representative of the largest realistic Stripe payload)
  Then the request succeeds and the subscription state updates correctly

Scenario: An oversized, unsigned request never reaches signature verification or the database
  Given no valid Stripe-Signature header is presented
  When an oversized request body is sent to POST /admin/v1/webhooks/stripe
  Then the request is rejected before signature verification is attempted
  And no tenant's subscription state is modified

Scenario: Existing webhook and billing tests continue to pass unmodified
  Given the full existing test suite for webhooks_stripe and billing behavior
  When this feature's changes are applied
  Then every previously-passing test in that suite still passes with no behavior change
```

#### Acceptance Criteria
- [ ] AC-WBL-01: a request body larger than the configured size limit is rejected with `413 Payload
      Too Large` BEFORE the full body is buffered into memory — proven by sending two requests, one
      just under the limit and one an order of magnitude over it, and asserting the process's
      resident memory (RSS) growth for the oversized request does not scale with its actual body
      size (bounded growth regardless of how large the attacker's payload claims to be); DESIGN/
      DISTILL choose the exact RSS-delta threshold and measurement tooling.
- [ ] AC-WBL-02 (non-negotiable regression guard): a legitimate, normally-sized, correctly-signed
      Stripe webhook event must succeed end-to-end exactly as it does today — this must never break
      real Stripe traffic, regardless of the exact ceiling DESIGN chooses.
- [ ] AC-WBL-03: the rejection response for an oversized body is `413 Payload Too Large` (or an
      equally sane, idiomatic HTTP status if DESIGN's chosen axum mechanism's default differs —
      DESIGN confirms axum's actual default behavior before locking this).
- [ ] AC-WBL-04 (regression guard): with a correctly configured deployment, real signature
      verification and subscription-state updates for normally-sized requests are unchanged from
      today's behavior.
- [ ] AC-WBL-05: the full pre-existing test suite (webhook handling, billing, and unrelated surfaces)
      passes with zero regressions.

#### Outcome KPIs
- **Who**: Sam Chen operating production embyr deployments with Stripe billing enabled, exposed to
  unauthenticated internet traffic on the admin HTTP listener.
- **Does what**: is structurally prevented from ever running a deployment where an unauthenticated
  attacker can force unbounded memory allocation via an oversized webhook request body; legitimate,
  normally-sized Stripe webhook traffic remains completely unaffected.
- **By how much**: from unbounded (today, `usize::MAX` places no ceiling on the buffered body, per
  this DISCUSS's own confirmed reading of `stripe_signature.rs:44`) to a small, fixed, generous
  ceiling informed by Stripe's own real-world payload sizes; 0% regression on legitimate Stripe
  webhook traffic of any realistic size.
- **Measured by**: an integration test sending an oversized body and asserting rejection before full
  buffering (AC-WBL-01); an integration test sending a real, normally-sized, correctly-signed webhook
  and asserting success (AC-WBL-02); the existing correct-signature webhook tests remaining green
  (AC-WBL-04); full regression suite (AC-WBL-05).
- **Baseline**: 0% bounded (unbounded) — confirmed directly by this DISCUSS's own reading of
  `stripe_signature.rs:44` (`axum::body::to_bytes(body, usize::MAX)`), matching the audit's own
  finding #3 evidence exactly.

#### Technical Notes
- **Open question for DESIGN (mechanism)**: implement the limit as (a) a manual bounded read inside
  `stripe_signature_middleware` itself (e.g. check `Content-Length` up front and/or replace
  `usize::MAX` with a concrete constant passed to `axum::body::to_bytes`), or (b) an axum
  `RequestBodyLimitLayer`/`DefaultBodyLimit` applied at the webhook sub-router's construction site in
  `admin/router.rs` (the same site `stripe-webhook-secret-required` already modified to conditionally
  mount this route). Both keep the fix scoped to this one route (see § Business Context's locked
  narrow-scope decision) — DESIGN chooses based on which is more idiomatic/smallest-diff against the
  current real code shape.
- **Open question for DESIGN (exact ceiling)**: the precise byte limit is not picked here. Stripe's
  own real-world webhook payloads are typically well under 1MB; the audit's own suggestion names a
  figure in the low single-digit megabytes as generous headroom. DESIGN should confirm Stripe's own
  documented maximum event payload size (if any) before locking a number.
- **Open question for DESIGN (status code confirmation)**: this DISCUSS names `413 Payload Too Large`
  as the expected/idiomatic rejection status based on axum's `RequestBodyLimitLayer`'s documented
  default behavior — DESIGN should confirm this against the actual axum version pinned in this
  workspace before relying on it, and confirm the same status code applies if mechanism (a) (manual
  check) is chosen instead.
- Must layer onto `stripe-webhook-secret-required`'s already-shipped conditional mount
  (`admin/router.rs:143`, `Option<Router>`) without re-touching that mounting logic — this feature
  only changes body-buffering behavior inside the already-conditionally-mounted route.
- Depends on nothing new — `stripe_signature_middleware`, the webhook route, and axum's body-handling
  primitives all already exist in this workspace's already-pinned axum dependency.
- **NFR guidance for DESIGN**: the rejection must be synchronous within the existing request-handling
  path (no queuing/async deferral), must not log or persist the oversized body's content, and must
  not introduce a new unbounded-cardinality metric label (mirrors the discipline `rate-limiter-
  project-id-validation` established for this same audit's finding #2). No numeric latency target is
  set here — DESIGN picks one only if the chosen mechanism risks adding a measurable hot-path cost.
- **Assumption DESIGN should confirm, not re-litigate**: this feature's narrow-scope decision (§
  Business Context) assumes every OTHER admin route already rejects unauthenticated requests via
  `operator_auth_middleware`/`session_auth_middleware` before any handler-level body read, so no
  other route shares this vulnerability shape. This DISCUSS did not audit every route's own body-read
  ordering line-by-line — DESIGN should do a quick confirming grep before treating the narrow scope
  as final.

## Wave: DISCUSS / [REF] Definition of Done

1. AC-WBL-01 through AC-WBL-05 all pass, proven against a real running server instance (real HTTP
   requests with real oversized and real normally-sized, correctly-signed bodies) — mirrors both
   sibling audit fixes' own real, not mocked/unit-only, proof standard.
2. The regression guard (AC-WBL-02/AC-WBL-04) is proven identical to pre-feature behavior for
   legitimate Stripe traffic, not merely "still works."
3. The oversized-body rejection (AC-WBL-01) is proven to reject before full buffering, not merely to
   eventually return an error after the whole body was already read.
4. Full regression suite clean (pre-existing flakes excepted, triaged not assumed, per this session's
   own established `feedback_triage_before_dismissing_as_flaky` practice).
5. Mutation testing runs after DELIVER, per this repo's own `per-feature` strategy (root `CLAUDE.md`)
   — 100% effective kill rate on the new/changed body-limit logic.
6. Evolution doc written; `docs/product/production-readiness-audit-2026-09-08.md` row 3 updated to
   CLOSED at FINALIZE (this DISCUSS only updates it to IN PROGRESS — see § Next Wave).
7. Memory updated.

## Wave: DISCUSS / [REF] Out of Scope

- **Finding #1 from the same audit** (`stripe-webhook-secret-required`, already CLOSED) — the
  config-safety/conditional-mount fix this feature layers onto, not re-touched here.
- **Finding #2 from the same audit** (`rate-limiter-project-id-validation`, already CLOSED, different
  route) — unrelated route, unrelated mechanism, not touched here.
- **Finding #39 from the same audit** (`event_type` becoming an unbounded Prometheus label on this
  same webhook route) — same route, related, but a separate cardinality concern with a separate
  remediation shape (mirrors finding #2's own bounding-mechanism pattern, not a body-size concern).
  Not fixed here.
- **A global body-limit layer for all admin routes** — explicitly considered and rejected in favor of
  a narrow, root-cause fix confined to this one route (see § Business Context for full reasoning).
  May be independently worth proposing later as its own defense-in-depth feature, but is not
  evidenced or investigated by this DISCUSS.
- **The exact byte ceiling and exact implementation mechanism** — both explicitly DESIGN's own
  decision, flagged as open questions above, not decided or investigated in this DISCUSS.
- **Any code implementation** — this is DISCUSS only, per explicit task instruction; DESIGN performs
  the actual investigation and implementation planning.

## Wave: DISCUSS / [REF] Walking Skeleton Strategy

**Strategy A** (real, minimal, end-to-end) — every scenario is a real HTTP request (oversized or
normally-sized, correctly-signed or unsigned) against a real running server instance, mirroring both
`stripe-webhook-secret-required`'s and `rate-limiter-project-id-validation`'s own Strategy A
precedent for this exact composition-root/middleware layer. This feature IS the walking skeleton —
single story, no further slicing.

## Wave: DISCUSS / [REF] Driving Ports

The existing admin HTTP route `POST /admin/v1/webhooks/stripe` (already exists, no new endpoint),
gated by the existing `stripe_signature_middleware`. Zero new RPC/HTTP endpoint — this feature
changes only how much of an incoming request body the middleware will read before rejecting.

## Wave: DISCUSS / [REF] Pre-requisites

- None beyond what already exists. `stripe_signature_middleware`, the webhook route, and
  `stripe-webhook-secret-required`'s already-shipped conditional-mount logic all already exist and
  are confirmed by direct reading, not assumed.

## Wave: DISCUSS / [REF] Definition of Ready Validation

### DoR Checklist (9-item hard gate)
1. [x] Story traces to a job_id (JOB-13) — reused, not new, with reasoning against two nearer
   alternatives (JOB-11, JOB-12) explicitly documented in § Persona & Job.
2. [x] Story has a complete Elevator Pitch (Before / After / Decision enabled).
3. [x] Every AC is testable without ambiguity (5 ACs, each a real HTTP request against a real running
   server, or a real full-suite regression run).
4. [x] Walking Skeleton identified (US-01 is the whole feature's walking skeleton).
5. [x] Scope Assessment passed.
6. [x] Story is not `@infrastructure`-only with no user-visible value — Decision 4 = Yes (full JTBD
   path); it directly enables Sam Chen's own trust decision (Elevator Pitch "Decision enabled") that
   the webhook route he enabled is resource-safe against unauthenticated input.
7. [x] Out of Scope explicitly named (6 items, each reasoned).
8. [x] Outcome KPIs have numeric framing (unbounded → fixed generous ceiling) and measurement
   methods.
9. [x] Prior-wave artifacts read and reconciled (the audit's own finding #3, JOB-13's existing job
   story, `stripe-webhook-secret-required`'s already-shipped conditional-mount state, and
   `rate-limiter-project-id-validation`'s own convention all directly informed this feature's shape;
   no contradiction found with any existing decision).

### DoR Status: **PASSED**

## Wave: DISCUSS / [REF] Wave Decisions Summary

### Key Decisions
- [D1] Job reused: JOB-13 (`production-deployment`), not JOB-11 (`fair-multitenancy` — wrong
  concern, rate-limiting decision logic is untouched) or JOB-12 (`observability` — wrong concern, no
  metrics/cardinality dimension in this finding, unlike sibling finding #2).
- [D2] Scope is a narrow, root-cause fix confined to the Stripe webhook route only — explicitly NOT a
  global body-limit layer for all admin routes. Reasoning: this is the only unauthenticated,
  full-body-buffering route in the admin router; every other route sits behind auth middleware that
  rejects before any handler-level body read; a global layer would apply blanket, uninvestigated
  behavior to routes this DISCUSS did not analyze.
- [D3] The exact byte ceiling and exact implementation mechanism (manual bounded read vs. axum
  `RequestBodyLimitLayer`/`DefaultBodyLimit`) are both explicitly DESIGN's own investigation, not
  decided here — two candidate approaches are named as DESIGN's own starting point.
- [D4] `413 Payload Too Large` is locked as the expected rejection status-code family, based on
  axum's documented `RequestBodyLimitLayer` default behavior — DESIGN confirms this against the
  actual pinned axum version before finalizing.
- [D5] "A legitimate Stripe webhook must still succeed" is locked as a non-negotiable regression
  guard (AC-WBL-02) — this must never break real Stripe traffic, regardless of which mechanism or
  exact ceiling DESIGN chooses.

### Requirements Summary
- Primary need: an oversized, unauthenticated request body sent to the Stripe webhook route cannot be
  fully buffered into memory before signature verification; legitimate, normally-sized Stripe
  webhook traffic remains completely unaffected.
- Walking skeleton scope: US-01, the entire feature — single story, 5 UAT scenarios.
- Feature type: Security fix / Infrastructure.

### Constraints Established
- Zero behavior change for legitimate, normally-sized, correctly-signed Stripe webhook requests.
- Zero change to the real signature-verification logic path itself — only how much body is read
  before that logic runs.
- Fix confined to the Stripe webhook route; no global body-limit layer added to other admin routes.
- No new bounded context, no new domain type, no new RPC/HTTP endpoint.

### Upstream Changes
None — this DISCUSS extends JOB-13's existing scope (same job, same persona), consistent with
`stripe-webhook-secret-required`'s and `firestore-tls-support`'s own precedent for extending JOB-13
with a new production-hardening concern on an already-existing surface.

## Wave: DISCUSS / [REF] Next Wave

**Handoff To**: nw-solution-architect (DESIGN wave)
**Deliverables**: this feature-delta.md, 5 locked Decisions (D1-D5), 1-story walking-skeleton plan, 5
ACs (AC-WBL-01 through AC-WBL-05) to design executable scenarios against. DESIGN's own investigation
scope: (1) the precise mechanism (manual bounded read inside `stripe_signature.rs` vs. an axum
`RequestBodyLimitLayer`/`DefaultBodyLimit` applied at the webhook sub-router's construction site in
`router.rs`), (2) the precise byte ceiling (informed by Stripe's own real-world payload sizes, with
generous headroom), (3) confirming axum's actual default rejection status code for the chosen
mechanism against this workspace's pinned axum version.

## Wave: DESIGN / [REF] Reading Confirmation

✓ `crates/embyr-server/src/admin/middleware/stripe_signature.rs` (55 lines) re-read in full at DESIGN
depth. Confirmed exact vulnerable line (`44`: `axum::body::to_bytes(body, usize::MAX)`), exact ordering
(header-presence check at `41` → full buffer at `44-46` → HMAC verification at `48-51`), and that this
function is the ONLY place in this file (or, per the grep below, this crate's `admin/` tree) that reads
the request body.
✓ **Confirming grep performed independently** (DISCUSS's own "Assumption DESIGN should confirm, not
re-litigate" instruction): `rg 'to_bytes\(' crates/embyr-server/src/admin` — **exactly ONE match**,
`stripe_signature.rs:44` itself. No other admin-route handler or middleware manually buffers a body
anywhere in this crate. This directly confirms DISCUSS's own narrow-scope assumption (every other admin
route sits behind `operator_auth_middleware`/`session_auth_middleware`, which reject before any
handler-level body read) — **no additional target found**, the narrow scope from DISCUSS § Business
Context is confirmed correct, not merely assumed. No new finding to flag for the audit doc.
✓ `crates/embyr-server/Cargo.toml` (full file) and the workspace root `Cargo.toml` (full file) read.
Confirmed: (a) `tower-http` is **not a dependency anywhere in this workspace**, direct or transitive —
`rg 'tower.http|RequestBodyLimitLayer|DefaultBodyLimit|body_limit|MAX_BODY' crates/embyr-server`
(case-insensitive) returns zero matches, independently reconfirming DISCUSS's own identical grep result;
(b) `axum = "0.7"` is the pinned workspace version, confirmed as `0.7.9` exactly via `Cargo.lock`
(`rg 'name = "axum"' -A2 Cargo.lock`); (c) `http-body-util.workspace = true` is already a **direct**
`embyr-server` dependency (`Cargo.toml:25`), version `0.1.3` per `Cargo.lock` — load-bearing for the
Mechanism Decision below, since it means the type needed to discriminate a size-limit-exceeded error
(`http_body_util::LengthLimitError`) requires zero new dependency to name.
✓ `docs/feature/stripe-webhook-secret-required/feature-delta.md` DESIGN section (`router.rs:143-157`,
`424-426`) re-read to confirm the conditional-mount shape this feature must not re-touch. Confirmed this
feature's chosen mechanism (below) requires **zero changes to `router.rs`** — the webhook sub-router's
construction, its `Option<Router>` gate, and its `route_layer(stripe_signature_middleware)` wiring are
all left completely untouched.
✓ `crates/embyr-server/src/admin/state.rs` (`WebhookState`) and
`crates/embyr-server/src/admin/handlers/webhooks_stripe.rs` read. Confirmed `stripe_webhook_handler`
(the downstream handler `stripe_signature_middleware` forwards to on success) never itself reads the
raw body — it consumes the already-verified `WebhookEvent` stashed in request extensions (line 54 of
the middleware). No second body-read site exists downstream either.
✓ Blast radius for `stripe_signature_middleware` itself confirmed via `rg 'stripe_signature_middleware'`
across the whole repo: exactly 2 non-test matches — its own definition
(`middleware/stripe_signature.rs`) and its ONE registration site
(`admin/router.rs:81` import, `:152-155` `route_layer` call). No other call site, no other wiring. This
feature's change is fully contained to the middleware function body; no call-shape elsewhere depends on
its internals.

## Wave: DESIGN / [REF] Mechanism Decision

**Decision: replace `usize::MAX` with a concrete byte constant in the SAME `axum::body::to_bytes` call
already at `stripe_signature.rs:44`** — DISCUSS candidate (a), in its simplest form (no `Content-Length`
pre-check needed; no router-level layer). Full reasoning, alternatives, and consequences are recorded in
`docs/product/architecture/adr-070-stripe-webhook-body-size-ceiling.md` (new ADR) — summarized here:

- **Root cause, confirmed precisely**: `axum::body::to_bytes(body, limit)`'s `limit` parameter is not a
  cosmetic afterthought — it is a *streaming* buffer ceiling, checked after each polled frame is
  appended, that aborts (without continuing to buffer further frames) once the accumulated length
  exceeds `limit`. The vulnerable line is not "missing a guard the function doesn't have" — it is the
  ONE call in this codebase that passes `usize::MAX` to a parameter whose entire purpose is to prevent
  exactly this. This makes mechanism (a) the *literal*, root-cause fix, not a workaround.
- **Mechanism (b) (`DefaultBodyLimit`/`RequestBodyLimitLayer` at `router.rs`'s webhook sub-router) is
  rejected**: `tower-http` is not a workspace dependency (grep-confirmed above and independently by
  DISCUSS) — adding it purely for `RequestBodyLimitLayer` violates the locked "no new external
  dependency" constraint. Axum's own native `DefaultBodyLimit` would add no new dependency, but operates
  by wrapping the body for consumption through axum's extractor mechanism — it does not naturally compose
  with this middleware's existing manual `into_parts()`/`to_bytes()` pattern without restructuring
  already-correct-shaped code, and it would touch `router.rs`, widening blast radius beyond the single
  file this fix needs. Mechanism (a) requires zero changes to `router.rs`.
- **Byte ceiling: 5 MiB (`5 * 1024 * 1024` = 5,242,880 bytes)**. Stripe publishes no single documented
  hard maximum webhook payload size; real-world events are typically a few KB, and even large
  `invoice.finalized`-shaped events with many line items stay well under 1 MB. 5 MiB is a deliberately
  generous, round number — 10-50x any realistic real payload — chosen to make AC-WBL-02 regression risk
  effectively zero while still bounding an attacker's forced per-request allocation to a small, fixed
  number. Not made configurable (no env var/admin knob): this is a security invariant informed by a
  third party's own payload-size behavior, not an operator-tunable trade-off (ponytail: no config for a
  value that never changes).
- **Status code: `413 Payload Too Large`, discriminated from the pre-existing `401 Unauthorized`
  mapping.** `axum::body::to_bytes`'s error type (`axum::Error`) does not distinguish
  size-limit-exceeded from any other body-read failure at the type level — both are folded into the same
  boxed error. The middleware must downcast the boxed inner error to `http_body_util::LengthLimitError`
  (already a direct dependency, confirmed above, zero new dependency) to tell the two apart:
  size-limit-exceeded → 413 (AC-WBL-01/03); every other `to_bytes` error cause (network disconnects,
  malformed transfer-encoding) → 401, **exactly the pre-existing behavior, unchanged** — zero regression
  on that pre-existing error path.

### Code sketch (illustrative — DELIVER owns exact naming/decomposition and confirms the downcast
compiles and behaves as designed against the real pinned `axum 0.7.9`/`http-body-util 0.1.3`, via a real
HTTP request in an acceptance test, not design-time assertion alone)

```rust
// crates/embyr-server/src/admin/middleware/stripe_signature.rs

/// Stripe's own real-world webhook payloads are typically a few KB, rarely
/// approaching the low hundreds of KB even for large `invoice.*` events with
/// many line items. 5 MiB is a deliberately generous ceiling chosen to make
/// AC-WBL-02 regression risk effectively zero while still bounding an
/// attacker's forced per-request allocation (ADR-070).
const MAX_STRIPE_WEBHOOK_BODY_BYTES: usize = 5 * 1024 * 1024; // 5 MiB

let (parts, body) = req.into_parts();
let bytes = axum::body::to_bytes(body, MAX_STRIPE_WEBHOOK_BODY_BYTES)
    .await
    .map_err(|err| {
        if err
            .into_inner()
            .downcast_ref::<http_body_util::LengthLimitError>()
            .is_some()
        {
            StatusCode::PAYLOAD_TOO_LARGE
        } else {
            StatusCode::UNAUTHORIZED
        }
    })?;
```

No change to `verify_webhook_signature`, `WebhookState`, `router.rs`, or any downstream handler.

## Wave: DESIGN / [REF] Blast Radius — Confirmed, Not Re-Touched

- `crates/embyr-server/src/admin/router.rs` — **zero changes**. The webhook sub-router's `Option<Router>`
  conditional mount (`stripe-webhook-secret-required`'s own shipped logic, lines 143-157) and its
  `route_layer(stripe_signature_middleware)` wiring are unaffected; this feature changes only what
  happens *inside* the already-wired middleware function.
- `crates/embyr-server/src/admin/state.rs` (`WebhookState`) — **zero changes**. No new field needed; the
  byte ceiling is a `const`, not configuration threaded through state.
- `crates/embyr-server/src/admin/handlers/webhooks_stripe.rs` (`stripe_webhook_handler`) — **zero
  changes**. Confirmed it never re-reads the raw body.
- Every other admin route/handler — **zero changes, confirmed by grep** (only one `to_bytes(` call site
  in the entire `admin/` tree, per Reading Confirmation above).

## Wave: DESIGN / [REF] External Integration Annotation (carried forward for platform-architect)

Stripe (webhook delivery + signature verification via `async-stripe-webhook`) remains the external
integration on this route, unchanged by this feature. Restating the annotation already established by
`stripe-webhook-secret-required`/`card-payments-backend` for continuity: **Contract tests recommended
for Stripe's webhook delivery shape** — consumer-driven contracts (e.g., Pact) or, at minimum, Stripe's
own published webhook event fixtures, to detect any future change in typical/maximum event payload size
that could erode this feature's 5 MiB headroom assumption before it reaches production.

## Wave: DESIGN / Handoff Package

**Files requiring a change:**
1. `crates/embyr-server/src/admin/middleware/stripe_signature.rs` — the ONLY production-code file
   requiring a change. Add `MAX_STRIPE_WEBHOOK_BODY_BYTES` constant; replace `usize::MAX` with it at the
   `to_bytes` call (line 44 today); add the `LengthLimitError`-discriminating `map_err` branch (413 vs.
   401). No other function, no other file, changes.

**Files confirmed to need NO change (blast radius, grep-verified):**
- `crates/embyr-server/src/admin/router.rs` — conditional mount and `route_layer` wiring untouched.
- `crates/embyr-server/src/admin/state.rs` — `WebhookState` untouched.
- `crates/embyr-server/src/admin/handlers/webhooks_stripe.rs` — untouched.
- Every other admin route/handler/middleware — confirmed via grep, no other `to_bytes(` call site exists.
- `Cargo.toml` (workspace and `embyr-server`) — **no new dependency**; `http-body-util` is already
  present.

**Documentation changes made this wave:**
- `docs/product/architecture/adr-070-stripe-webhook-body-size-ceiling.md` (new).

**Regression guards DISTILL/DELIVER must prove, against a real running server instance (mirrors both
sibling audit fixes' own proof standard, per DoD item 1):**
- A request with a body larger than 5 MiB is rejected with `413 Payload Too Large` before the middleware
  finishes buffering it — proven via bounded RSS growth (AC-WBL-01), not merely a returned status code.
- A correctly-signed, normally-sized (few-KB) Stripe webhook event succeeds end-to-end exactly as today
  (AC-WBL-02, non-negotiable).
- A correctly-signed webhook comfortably within the 5 MiB ceiling still succeeds (AC-WBL-03's boundary
  case).
- An oversized, unsigned request is rejected before signature verification is attempted and before any
  DB write (AC-WBL-01/AC-WBL-04 composition — mirrors the existing `AC-203-02` zero-DB-write-on-rejection
  guarantee this middleware's own doc comment already claims).
- Full workspace `cargo test` (AC-WBL-05) — run once, at the pre-commit gate, per this repo's own root
  `CLAUDE.md` test-run token-discipline rule.
- Mutation testing on the new/changed logic (the byte-constant comparison and the 413/401
  discrimination branch) after DELIVER, per this repo's `per-feature` strategy.

**Open items explicitly NOT carried to DISTILL as ambiguous** (DESIGN resolved both open questions
DISCUSS handed off):
- Mechanism: **locked** — in-place `to_bytes` limit, no router-level layer.
- Byte ceiling: **locked** — 5 MiB (`5 * 1024 * 1024`).
- Status code: **locked** — 413 for size-limit-exceeded, 401 (unchanged) for every other `to_bytes`
  error cause.
