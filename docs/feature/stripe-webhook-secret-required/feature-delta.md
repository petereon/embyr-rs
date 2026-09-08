# Feature Delta: stripe-webhook-secret-required

## Wave: DISCUSS / [REF] Prior Wave Consultation — Reading Confirmation

✓ `docs/product/production-readiness-audit-2026-09-08.md` — read in full (all 41 findings + bloat
list + notes). Finding #1's exact wording confirmed: "Stripe webhook signing secret defaults to
empty string when unset; webhook route stays mounted, HMAC becomes forgeable (empty key is a
valid HMAC key). Unauthenticated attacker can set any tenant's subscription status/plan."
Location cited by the audit: `crates/embyr-server/src/main.rs:216` (`.unwrap_or_default()`);
`crates/embyr-server/src/admin/router.rs:400-406` (unconditional mount);
`crates/embyr-server/src/admin/handlers/webhooks_stripe.rs:129,174,231,264`. Severity: **Blocker**
— the audit's own overall framing (§ Notes) places this in the "features that silently no-op when
an env var is unset" cluster, alongside composite indexes/purge sweeper/agent field-path guard,
none of which the audit says require a redesign — "most fixes are single-function."
✓ `docs/product/jobs.yaml` — read in full (all 20 jobs). Searched explicitly for a Stripe/billing/
webhook-specific job before considering a new one. Two candidates found: **JOB-14**
(`manage-subscription`, persona P5 Chris Account Admin) — self-service billing UI/UX (view plan,
add card, upgrade/downgrade); **JOB-13** (`production-deployment`, persona P2 Sam Chen Service
Operator) — safe, fail-fast production configuration (`docker run` + env vars, "server exits
non-zero with named missing var," "no cryptic failures"). No dedicated
"webhook signature validation" or "Stripe security" job exists. JOB-14 is the wrong fit: this
finding is not about Chris's self-service billing UX (plan cards, usage bars, upgrade buttons) —
it is an unauthenticated-attacker exploit against production configuration, discovered by an
operator-facing production-readiness scan, with zero end-user (Chris) interaction. JOB-13 is the
right fit — its own functional dimension already names exactly this shape: "server exits non-zero
with named missing var." `firestore-tls-support` already established the precedent of extending
JOB-13 for a new pair of fail-fast config vars on a security-relevant surface (TLS cert/key).
This finding extends that same precedent to a third pair of vars (Stripe billing config),
confirming — not merely assuming — the same fail-fast discipline applies here too.
✓ `docs/feature/firestore-tls-support/feature-delta.md` — read in full to confirm this project's
own established single-file, Tier-1-only `feature-delta.md` convention (`## Wave: DISCUSS / [REF]
{Section}` heading format, no standalone `acceptance-criteria.md`/`outcome-kpis.md`/`story-map.md`
files) and its own precedent for extending JOB-13 with a new fail-fast config-validation pair
(`EMBYR_TLS_CERT_PATH`/`EMBYR_TLS_KEY_PATH`, both-or-neither, named error, before any port binds).
This DISCUSS mirrors that convention and that precedent's shape directly.
✓ Light context read (NOT a DESIGN-wave code investigation — reading exactly enough to state the
business context accurately and avoid contradicting an existing, documented decision):
`crates/embyr-server/src/main.rs:150-230` and `crates/embyr-server/src/admin/router.rs:380-414`.
Confirmed the audit's own file:line citations are accurate and confirmed the fact this DISCUSS was
explicitly told to reconcile with, not contradict: `cfg.stripe_secret_key` is already `Option`
(optional in V1 — absent means billing is unwired), and `StripeGateway::probe()` at startup is
already a documented soft-failure/WARN, not a startup refusal ("billing not on critical path,"
ADR-020/021, `card-payments-backend`). Also confirmed the webhook sub-router
(`/admin/v1/webhooks/stripe`) is unconditionally `.merge()`d into the admin router regardless of
whether `stripe_secret_key` is configured at all — the unconditional-mount half of the audit's own
finding, confirmed directly rather than assumed. No further code investigation performed; the
precise mechanism for "how do we know Stripe billing is enabled at all" is explicitly left to
DESIGN, per this task's own scope limit.
✓ Confirmed `docs/product/known-gaps.md` is NOT touched by this DISCUSS (that list is closed,
this is a separate audit, per this task's own explicit instruction).

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

- Feature type: **Security fix / Infrastructure** (production configuration safety, not an
  SDK-developer-facing feature).
- JTBD: **reuse JOB-13** (`production-deployment`, P2 Sam Chen) — extended, not replaced. Not a
  new job; not JOB-14 (wrong persona, wrong concern — see § Reading Confirmation).
- Decision 4 (full JTBD path vs. infrastructure-only): **Yes — full JTBD path.** This is a real
  security vulnerability an unauthenticated attacker can exploit against a real production
  deployment to forge subscription-status changes for a real tenant. It is not infrastructure-only
  scaffolding; it directly serves JOB-13's own "feel in control — startup errors are named and
  specific, no cryptic failures" emotional dimension and its "no traffic is accepted until
  migrations and DB probe succeed" functional discipline, extended to a new attack surface.
- Walking Skeleton: **Yes** — single story, no further slicing (mirrors `firestore-tls-support`'s
  own single-story precedent for this exact "extend the fail-fast config gate" shape).
- UX Research Depth: **Lightweight** — an operator-facing configuration-safety fix, not an
  end-user journey; no ASCII TUI mockups or emotional-arc journey YAML warranted (mirrors
  `firestore-tls-support`'s own Lightweight designation for the identical reason).
- **The fail-open-vs-fail-closed tradeoff is this DISCUSS's own call, decided below in
  § Business Context and § User Stories, not deferred to DESIGN.**

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: **P2 Sam Chen (Service Operator / Platform Engineer)** — unchanged from JOB-13's
existing profile. Not Chris (P5, JOB-14) — this finding was surfaced by a production-readiness
security scan, not a billing self-service journey, and its remediation is a deployment-time
configuration discipline Sam Chen already owns for every other required/optional env var in this
same composition root.

**Job**: **JOB-13 `production-deployment`**, reused, EXTENDED (not replaced) to also cover: when
Stripe billing is being enabled for a deployment (`STRIPE_SECRET_KEY` set), the webhook signing
secret (`STRIPE_WEBHOOK_SIGNING_SECRET`) must ALSO be present, or startup refuses to proceed with
a named error — the exact same all-or-none, fail-fast discipline JOB-13's own functional dimension
already established for `DATABASE_URL`/`EMBYR_ADMIN_KEY`/`EMBYR_ENCRYPTION_KEY`, and which
`firestore-tls-support` already extended once to a second config pair
(`EMBYR_TLS_CERT_PATH`/`EMBYR_TLS_KEY_PATH`). Deployments with NO Stripe billing configured at all
(a legitimate, already-documented V1 shape per ADR-020/021 — `stripe_secret_key` is optional, and
`StripeGateway::probe()` is a soft-failure, not a startup refusal) remain completely unaffected —
this feature does not make Stripe billing itself mandatory; it makes the specific
"billing-half-enabled" state impossible.

## Wave: DISCUSS / [REF] Business Context

Today, `STRIPE_WEBHOOK_SIGNING_SECRET` defaults to an empty string via
`.unwrap_or_default()` (`main.rs:216`) whenever the operator has not set it — silently, with no
startup warning and no failure. The webhook route (`POST /admin/v1/webhooks/stripe`) stays
mounted unconditionally (`admin/router.rs:400-406`) regardless of whether Stripe is configured at
all. HMAC-SHA256 (the Stripe signature scheme) treats an empty string as a completely valid key —
so any unauthenticated attacker who knows (or guesses) that a target deployment left this variable
unset can compute a valid `Stripe-Signature` header themselves, using the same empty key, over a
payload they construct. The webhook handler (`webhooks_stripe.rs:129,174,231,264`) then executes
real `UPDATE subscriptions SET status = ...` mutations against real tenant subscription rows based
on that attacker-supplied, unauthenticated input — an attacker can set any tenant's subscription
status or plan (e.g. force a paying tenant into a suspended state, or force a free tenant into a
paid-plan's higher caps) with no credential of any kind.

**The tradeoff this DISCUSS decides** (not deferred to DESIGN): should the server refuse to start
entirely whenever the webhook secret is missing, or only when Stripe billing is otherwise being
enabled? Stripe billing is confirmed, by direct reading of `main.rs:150-190`, to already be an
OPTIONAL feature of this server — `cfg.stripe_secret_key` is `Option<String>`, and its absence is
an already-documented, already-shipped V1 shape (ADR-020/021, `card-payments-backend`): a
self-hosted/BYOC deployment can run today with zero billing integration, and `StripeGateway::
probe()` already treats Stripe unreachability as a soft WARN, not a hard startup refusal, because
billing is explicitly "not on the Firestore protocol-serving critical path." Making the WHOLE
server refuse to start merely because `STRIPE_WEBHOOK_SIGNING_SECRET` is unset would therefore be
WRONG — it would break every zero-billing deployment that works correctly today, contradicting the
server's own already-established Stripe-is-optional decision.

**Decided shape**: the fail-fast condition is narrower and precise — it fires only when Stripe
billing is being enabled (signaled by `STRIPE_SECRET_KEY` being present) but the webhook secret
specifically is absent. This mirrors this project's own already-established "all or none" pattern,
used identically for the 3 port binds (D-PR-6) and the 2 TLS vars
(`firestore-tls-support`'s own D1/D5/D6) — a config pair where EITHER both are present or the
dependent one is treated as effectively missing. When Stripe billing is entirely unconfigured
(`STRIPE_SECRET_KEY` absent), this feature requires the webhook route be safe by construction — it
must not be reachable/exploitable — without requiring the server to refuse to start. The EXACT
mechanism (not mounting the route at all, vs. mounting it but having it immediately and
unconditionally reject every request) is explicitly DESIGN's own call, not decided here — this
DISCUSS requires only the OUTCOME: an unconfigured-Stripe deployment is unaffected and safe.

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

**Signals checked**: >10 user stories? No (1). >3 bounded contexts/modules? No — entirely confined
to `embyr-server`'s own composition root and admin sub-router (the same layer
`firestore-tls-support` and `production-readiness` both operated in): startup config validation
(`main.rs`/`config.rs`) plus the webhook route-mounting decision (`admin/router.rs`). Walking
skeleton >5 integration points? No (4: startup-config-validation path, route-mount/reject path,
real correctly-signed webhook request, real forged/empty-key-signed webhook request — all against
the SAME single running server instance). Estimated effort >2 weeks? No — the fail-fast pattern
being extended is proven, already-shipped precedent (D-PR-1/D-PR-6, and `firestore-tls-support`'s
own direct extension of it); no new domain concept, no new adapter trait, no new bounded context.
Multiple independent user outcomes? No — "a misconfigured Stripe-webhook-secret deployment can
never run" is a single outcome; the unconfigured-vs-partially-configured-vs-fully-configured
states are 3 verification points of that ONE outcome, not 3 separable stories.

**Scope Assessment: PASS** — 1 user story, 1 bounded context (`embyr-server` composition root +
admin sub-router), estimated ≤2 days, 5 UAT scenarios (within the 3-7 right-sized range).

## Wave: DISCUSS / [REF] System Constraints

- `STRIPE_SECRET_KEY` present (billing enabled) but `STRIPE_WEBHOOK_SIGNING_SECRET` absent is a
  hard startup-refusal state — server exits non-zero with a named error before serving any
  traffic, mirroring D-PR-6/D-PR-1 and `firestore-tls-support`'s own D5/D6 partial-config
  discipline exactly.
- `STRIPE_SECRET_KEY` absent (billing fully unconfigured) is unaffected by this feature — the
  server starts exactly as it does today, and the webhook route must be safe (unreachable or
  unconditionally rejecting) rather than silently forgeable.
- `STRIPE_SECRET_KEY` and `STRIPE_WEBHOOK_SIGNING_SECRET` both present (correctly configured) must
  behave EXACTLY as today's real (non-empty-key) signature verification already does — this
  feature closes a silent-misconfiguration hole, it does not touch the real verification logic
  path for deployments that were already configured correctly.
- No new external dependency, no new RPC/HTTP endpoint — the webhook route already exists; this
  feature changes only WHEN it is reachable and WHETHER startup permits the vulnerable state to
  exist at all.
- Out of scope for this feature (see § Out of Scope): findings #2/#3/#39 from the same audit
  (unbounded label cardinality, unbounded body buffering, `event_type` cardinality on this same
  route) — related, same route, separate blockers, not fixed here.

## Wave: DISCUSS / [REF] User Stories

### US-01: A Misconfigured Stripe Webhook Secret Can Never Be Running In Production

**job_id**: JOB-13

#### Elevator Pitch
**Before**: Sam Chen enables Stripe billing for a production deployment (Meridian Health's
account) by setting `STRIPE_SECRET_KEY`, but forgets — or is never prompted — to also set
`STRIPE_WEBHOOK_SIGNING_SECRET`. The server starts successfully, with no warning of any kind. The
webhook route stays mounted, and because an empty string is a cryptographically valid HMAC key, an
unauthenticated attacker who knows or guesses this can forge a signature and set Meridian Health's
subscription status or plan to anything they choose — Sam has no way to know this is possible
until it is exploited.
**After**: The moment Sam sets `STRIPE_SECRET_KEY` without also setting
`STRIPE_WEBHOOK_SIGNING_SECRET`, the server refuses to start, exits non-zero, and names
`STRIPE_WEBHOOK_SIGNING_SECRET` specifically as the missing variable required because billing is
enabled. A deployment with no Stripe billing at all (neither variable set) is completely
unaffected and starts exactly as before.
**Decision enabled**: Sam can trust that if his deployment is running with Stripe billing enabled,
the webhook signature check is real and enforced — he never has to separately verify this by
reading source code or waiting for an incident; the same "did it start?" signal he already relies
on for every other required-config decision in this deployment now covers this one too.

#### Who
- Sam Chen (P2) | Service operator deploying embyr-server with Stripe billing enabled for one or
  more tenants | Needs the same fail-fast, named-error startup discipline JOB-13's own
  `production-readiness` feature already established for `DATABASE_URL`/`EMBYR_ADMIN_KEY`/
  `EMBYR_ENCRYPTION_KEY`, and `firestore-tls-support` already extended once, applied to this
  Stripe-webhook-secret config pair.

#### Solution
When `STRIPE_SECRET_KEY` is present (Stripe billing enabled) but `STRIPE_WEBHOOK_SIGNING_SECRET`
is absent, startup fails fast with a named error identifying the missing variable, before any port
binds — mirroring the existing all-or-none pattern already used for the 3 port binds and the 2 TLS
vars. When Stripe billing is entirely unconfigured (`STRIPE_SECRET_KEY` absent), the server starts
normally and the webhook route is safe by construction (unreachable or unconditionally rejecting —
DESIGN decides the exact mechanism). When both variables are correctly configured, existing
correct-signature verification behavior is unchanged.

#### Domain Examples

**Example 1 (Happy Path — Stripe fully unconfigured, regression guard)**: Sam Chen deploys
embyr-server for a self-hosted BYOC customer with zero Stripe integration — neither
`STRIPE_SECRET_KEY` nor `STRIPE_WEBHOOK_SIGNING_SECRET` is set, exactly as configured before this
feature shipped. The server starts normally, binds all 3 ports, and the webhook route is not a
usable attack surface — no request to it can mutate any subscription state.

**Example 2 (Edge Case — billing enabled, secret forgotten)**: Sam Chen configures Meridian
Health's production deployment with `STRIPE_SECRET_KEY=sk_live_...` (billing enabled) but forgets
`STRIPE_WEBHOOK_SIGNING_SECRET` — a copy-paste mistake deploying from a checklist, the same failure
shape `firestore-tls-support`'s own Example 2 already named for its TLS var pair. The server
refuses to start, exits non-zero, and logs an error naming `STRIPE_WEBHOOK_SIGNING_SECRET`
specifically as required because `STRIPE_SECRET_KEY` is set — not a generic "billing misconfigured"
message.

**Example 3 (Error/Boundary — the exploit this feature closes)**: "Marcus Webb," an unauthenticated
attacker with no credential of any kind, discovers a target embyr deployment (Meridian Health's)
has Stripe billing enabled but no webhook secret configured. Before this feature: Marcus computes
an HMAC-SHA256 signature using an empty string as the key over a forged `customer.subscription.
updated` payload, sends it to `POST /admin/v1/webhooks/stripe`, and the signature verifies —
Meridian Health's subscription status is silently changed to whatever Marcus specified. After this
feature: this configuration state can never be running in production, because startup already
refused to boot the moment `STRIPE_SECRET_KEY` was set without `STRIPE_WEBHOOK_SIGNING_SECRET`.

**Example 4 (Regression Guard — correctly configured)**: Sam Chen sets both
`STRIPE_SECRET_KEY` and `STRIPE_WEBHOOK_SIGNING_SECRET` correctly for Meridian Health's
deployment. A real Stripe-issued webhook event (`customer.subscription.updated`, correctly signed
by Stripe using the real shared secret) updates Meridian Health's subscription status exactly as
it does today. Marcus Webb's forged, empty-key-signed request is rejected — the real signature
check, unchanged by this feature, still rejects a signature that does not match the real secret.

#### UAT Scenarios (BDD)

```gherkin
Scenario: A deployment with Stripe billing fully unconfigured starts normally and is unaffected
  Given STRIPE_SECRET_KEY and STRIPE_WEBHOOK_SIGNING_SECRET are both unset
  When Sam Chen starts embyr-server
  Then the server starts successfully and binds all 3 listeners exactly as before this feature
  And no request to the Stripe webhook route can mutate any tenant's subscription state

Scenario: Startup fails fast when Stripe billing is enabled but the webhook secret is missing
  Given STRIPE_SECRET_KEY is set to a live Stripe secret key
  And STRIPE_WEBHOOK_SIGNING_SECRET is unset
  When Sam Chen starts embyr-server
  Then the process exits with a non-zero code before any port is bound
  And stderr names STRIPE_WEBHOOK_SIGNING_SECRET specifically as required because billing is enabled

Scenario: Correctly configured webhook signature verification is unchanged
  Given STRIPE_SECRET_KEY and STRIPE_WEBHOOK_SIGNING_SECRET are both set to matching real values
  When Sam Chen starts embyr-server
  And Stripe sends a correctly-signed customer.subscription.updated webhook event for Meridian Health
  Then Meridian Health's subscription status is updated exactly as it was before this feature

Scenario: A forged signature computed with an empty key is rejected once the secret is configured
  Given STRIPE_SECRET_KEY and STRIPE_WEBHOOK_SIGNING_SECRET are both correctly configured
  When an attacker sends a webhook request signed with an empty-string HMAC key
  Then the request is rejected
  And Meridian Health's subscription state is not modified

Scenario: Existing webhook and billing tests continue to pass unmodified
  Given the full existing test suite for webhooks_stripe and billing behavior
  When this feature's changes are applied
  Then every previously-passing test in that suite still passes with no behavior change
```

#### Acceptance Criteria
- [ ] AC-WHS-01: with `STRIPE_SECRET_KEY` unset (Stripe billing fully unconfigured), the server
      starts normally and the webhook route cannot be used to mutate any tenant's subscription
      state — unconfigured-Stripe deployments are unaffected and safe (mechanism — unmount vs.
      unconditional-reject — is DESIGN's own decision).
- [ ] AC-WHS-02: with `STRIPE_SECRET_KEY` set and `STRIPE_WEBHOOK_SIGNING_SECRET` unset, startup
      exits non-zero before any port binds, with a named error identifying
      `STRIPE_WEBHOOK_SIGNING_SECRET` specifically as required because billing is enabled — never a
      silent empty-string default.
- [ ] AC-WHS-03 (regression guard): with both variables correctly configured, real Stripe webhook
      signature verification and subscription-state updates behave identically to today's correct
      behavior — no change to the real verification path.
- [ ] AC-WHS-04 (regression guard): with both variables correctly configured, a request signed with
      an empty-string HMAC key is rejected — this already works today via the real HMAC check; this
      AC proves the fix does not accidentally weaken it.
- [ ] AC-WHS-05: the full pre-existing test suite (webhook handling, billing, and unrelated
      surfaces) passes with zero regressions.

#### Outcome KPIs
- **Who**: Sam Chen operating production embyr deployments with Stripe billing enabled.
- **Does what**: is structurally prevented from ever running a deployment in the specific
  "billing enabled, webhook secret missing" state that makes the webhook route forgeable;
  deployments with no Stripe billing at all remain completely unaffected.
- **By how much**: from 0% (today, 100% of "billing enabled, secret missing" configurations start
  successfully and serve a forgeable webhook route, per the audit's own confirmed
  `.unwrap_or_default()` finding) to 100% of such misconfigurations failing startup with a named
  error; 0% regression on both the fully-unconfigured and the correctly-configured deployment
  shapes.
- **Measured by**: an integration test asserting non-zero exit + named
  `STRIPE_WEBHOOK_SIGNING_SECRET` error when `STRIPE_SECRET_KEY` is set alone (AC-WHS-02); an
  integration test asserting unaffected startup/behavior when Stripe is fully unconfigured
  (AC-WHS-01); the existing correct-signature and forged-empty-key-signature webhook tests
  remaining green (AC-WHS-03/04); full regression suite (AC-WHS-05).
- **Baseline**: 0% — confirmed directly by this DISCUSS's own reading of `main.rs:216`
  (`.unwrap_or_default()`) and `admin/router.rs:400-406` (unconditional mount), matching the
  audit's own finding #1 evidence exactly.

#### Technical Notes
- The exact signal for "is Stripe billing being enabled" (`STRIPE_SECRET_KEY` presence, or some
  other/additional condition) is DESIGN's own investigation — this DISCUSS names
  `STRIPE_SECRET_KEY` as the primary signal per the audit's own citations and this DISCUSS's own
  light confirmation that it is already `Option<String>` in `ServerConfig`, but does not lock the
  precise mechanism.
- The exact mechanism for making the webhook route safe when Stripe is fully unconfigured
  (unmounting the route entirely vs. mounting it but having it unconditionally reject) is
  explicitly DESIGN's own decision, not decided here.
- Mirrors `firestore-tls-support`'s own established shape for a fail-fast, both-or-effectively-
  required config pair (D5/D6) — reuse that precedent's validation-ordering approach
  (`ServerConfig::from_env()` runs before any port bind) rather than inventing a new mechanism.
- Depends on nothing new — `STRIPE_SECRET_KEY`, `STRIPE_WEBHOOK_SIGNING_SECRET`, the webhook route,
  and the signature-verification middleware all already exist (`card-payments-backend`,
  ADR-020/021).

## Wave: DISCUSS / [REF] Definition of Done

1. AC-WHS-01 through AC-WHS-05 all pass, proven against a real running server instance (real
   startup, real subprocess exit code/stderr, real HTTP requests to the webhook route) — mirrors
   `firestore-tls-support`'s own Strategy A precedent, not a mocked/unit-only proof.
2. The regression guards (AC-WHS-01, AC-WHS-03, AC-WHS-04) are proven identical to pre-feature
   behavior, not merely "still works."
3. The fail-fast path (AC-WHS-02) exits non-zero before any port binds, with a named, specific
   error — never a panic, never a silent empty-key fallback.
4. Full regression suite clean (pre-existing flakes excepted, triaged not assumed, per this
   session's own established `feedback_triage_before_dismissing_as_flaky` practice).
5. Mutation testing runs after DELIVER, per this repo's own `per-feature` strategy
   (root `CLAUDE.md`) — 100% effective kill rate on the new/changed startup-validation and
   route-mount/reject logic.
6. Evolution doc written; `docs/product/production-readiness-audit-2026-09-08.md` row 1 updated to
   CLOSED at FINALIZE (this DISCUSS only updates it to IN PROGRESS — see § Next Wave).
7. Memory updated.

## Wave: DISCUSS / [REF] Out of Scope

- **Findings #2, #3, #39 from the same audit** (unbounded Prometheus label cardinality from
  unauthenticated `project_id`; unbounded request-body buffering on this same webhook route before
  signature check; `event_type` becoming an unbounded Prometheus label on this same route) — same
  route, related, but separate blockers with separate remediations. Not fixed here; named as
  related, explicitly deferred, not silently dropped.
- **The exact code mechanism** for detecting "is Stripe configured at all" and for making the
  webhook route safe when unconfigured — both are explicitly DESIGN's own investigation, not
  decided or investigated in this DISCUSS.
- **Rotating or hot-reloading the webhook secret at runtime** — matches the read-once-at-startup
  pattern already established for every other secret in this config (`EMBYR_ADMIN_KEY`,
  `EMBYR_ENCRYPTION_KEY`, and `firestore-tls-support`'s own TLS cert/key).
- **The `async-stripe` pre-1.0 release-candidate pin** (finding #38 in the same audit) — unrelated
  dependency-risk concern, not touched by this feature.
- **Any code implementation** — this is DISCUSS only, per explicit task instruction; DESIGN
  performs the actual investigation and implementation planning.

## Wave: DISCUSS / [REF] Walking Skeleton Strategy

**Strategy A** (real, minimal, end-to-end) — every scenario is a real server startup/subprocess
exit-code-and-stderr check, or a real HTTP request (correctly-signed or forged-empty-key-signed)
against a real running server instance, mirroring `firestore-tls-support`'s own Strategy A
precedent for this exact composition-root layer. This feature IS the walking skeleton — single
story, no further slicing.

## Wave: DISCUSS / [REF] Driving Ports

The existing admin HTTP route `POST /admin/v1/webhooks/stripe` (already exists, no new endpoint),
gated by the existing `stripe_signature_middleware`; and `embyr-server`'s existing startup
config-validation entry point (`ServerConfig::from_env()`). Zero new RPC/HTTP endpoint — this
feature changes only the validation and mounting behavior around ports that already exist.

## Wave: DISCUSS / [REF] Pre-requisites

- None beyond what already exists. The fail-fast/named-error startup discipline this feature
  extends is already established by `production-readiness` (D-PR-1/D-PR-6/D-PR-7) and already
  extended once by `firestore-tls-support` for a different config pair — this feature is a third,
  same-shape application of that pattern, not new architecture.
- The webhook route, its signature-verification middleware, and the `STRIPE_SECRET_KEY`-optional /
  soft-failure-probe precedent for Stripe billing itself all already exist
  (`card-payments-backend`, ADR-020/021) — confirmed by direct reading, not assumed.

## Wave: DISCUSS / [REF] Definition of Ready Validation

### DoR Checklist (9-item hard gate)
1. [x] Story traces to a job_id (JOB-13) — reused, not new, with reasoning against the nearest
   alternative (JOB-14) explicitly documented in § Reading Confirmation.
2. [x] Story has a complete Elevator Pitch (Before / After / Decision enabled).
3. [x] Every AC is testable without ambiguity (5 ACs, each a real startup exit-code/stderr
   assertion or a real HTTP request against a real running server).
4. [x] Walking Skeleton identified (US-01 is the whole feature's walking skeleton).
5. [x] Scope Assessment passed.
6. [x] Story is not `@infrastructure`-only with no user-visible value — Decision 4 = Yes (full
   JTBD path); it directly enables Sam Chen's own trust decision (Elevator Pitch "Decision
   enabled") that a running deployment's webhook signature check is real.
7. [x] Out of Scope explicitly named (5 items, each reasoned).
8. [x] Outcome KPIs have numeric targets (0% → 100%) and measurement methods.
9. [x] Prior-wave artifacts read and reconciled (the audit's own finding #1, JOB-13's existing job
   story, and `firestore-tls-support`'s own precedent all directly informed this feature's shape;
   the Stripe-is-optional/soft-failure fact from `card-payments-backend`/ADR-020/021 was
   confirmed, not contradicted).

### DoR Status: **PASSED**

## Wave: DISCUSS / [REF] Wave Decisions Summary

### Key Decisions
- [D1] Fail-fast condition is narrow, not global: `STRIPE_SECRET_KEY` present +
  `STRIPE_WEBHOOK_SIGNING_SECRET` absent → hard startup refusal. `STRIPE_SECRET_KEY` absent (Stripe
  fully unconfigured) → server starts normally, unaffected.
- [D2] Deciding this narrow-vs-global shape was this DISCUSS's own call (per explicit task
  instruction), grounded in the already-confirmed fact that Stripe billing is itself optional in
  this codebase (`Option<String>` `stripe_secret_key`, soft-failure `StripeGateway::probe()`,
  ADR-020/021) — making the whole server refuse to start on a missing webhook secret would
  contradict that existing, already-shipped decision.
- [D3] The exact mechanism for "how do we detect Stripe is enabled" and "how do we make the
  webhook route safe when unconfigured" are both explicitly DESIGN's own investigation, not
  decided here.
- [D4] Reuses JOB-13, not JOB-14 — persona is Sam Chen (operator/config-safety concern), not Chris
  (billing self-service UX concern).

### Requirements Summary
- Primary need: a deployment cannot silently run with a forgeable Stripe webhook signature check;
  deployments without Stripe billing at all remain fully unaffected.
- Walking skeleton scope: US-01, the entire feature — single story, 5 UAT scenarios.
- Feature type: Security fix / Infrastructure.

### Constraints Established
- Zero behavior change for deployments with Stripe billing fully unconfigured.
- Zero silent empty-key fallback when billing is enabled but the secret is missing — always a
  named, fail-fast startup error.
- Zero change to real (correctly-configured) signature verification behavior.
- No new bounded context, no new domain type, no new RPC/HTTP endpoint.

### Upstream Changes
None — this DISCUSS extends JOB-13's existing scope (same job, same persona), consistent with
`firestore-tls-support`'s own precedent for extending JOB-13 with a new fail-fast config pair.

## Wave: DISCUSS / [REF] Next Wave

**Handoff To**: nw-solution-architect (DESIGN wave)
**Deliverables**: this feature-delta.md, 4 locked Decisions (D1-D4), 1-story walking-skeleton plan,
5 ACs (AC-WHS-01 through AC-WHS-05) to design executable scenarios against. DESIGN's own
investigation scope: (1) the precise mechanism for detecting "is Stripe billing enabled," (2) the
precise mechanism for making the webhook route safe when Stripe is unconfigured (unmount vs.
unconditional-reject), (3) the exact `ConfigError`/validation wiring shape, mirroring
`firestore-tls-support`'s own DESIGN-wave precedent for a fail-fast config-pair extension.

## Wave: DESIGN / [REF] Reading Confirmation

✓ `crates/embyr-server/src/config.rs` read in full. Confirmed the exact TLS-pair mechanism
(`EMBYR_TLS_CERT_PATH`/`EMBYR_TLS_KEY_PATH`, lines 254-270) this feature must mirror: two local
`Option<String>` reads with `.filter(|v| !v.is_empty())`, a `match` that pushes the partner's name
onto the SAME `missing: Vec<String>` accumulator already used for `DATABASE_URL`/`EMBYR_ADMIN_KEY`/
`EMBYR_ENCRYPTION_KEY`, then a single `if !missing.is_empty() { return Err(ConfigError::MissingVars(missing)) }`
gate — no new `ConfigError` variant for TLS. `ConfigError::MissingVars(Vec<String>)`'s `Display`
formats each entry as `"missing required environment variable: {v}"` — nothing more.
✓ `crates/embyr-server/src/main.rs` lines 1-240 read in full. Confirmed: `ServerConfig::from_env()`
is Step 1 (line 67), before tracing, before any TCP bind (Step 8, lines 150-153) — a `missing`-vars
failure there already exits the process before any port opens, for free, for ANY var added to the
existing `missing` accumulator inside `from_env()`. Confirmed the existing Stripe wiring at Step 10
(lines 178-219): `StripeGateway` is always constructed with a placeholder key when
`stripe_secret_key` is absent; `stripe_gateway.probe()` is a soft WARN (ADR-020/021); and line 216
is the exact vulnerable line the audit cited: `cfg.stripe_webhook_signing_secret.clone().unwrap_or_default()`
passed straight into `build_admin_router`.
✓ `crates/embyr-server/src/admin/router.rs` read in full (`build_admin_router`, lines 96-414, plus
the 4 backward-compatible test wrappers, lines 425-493). Confirmed the `webhook_router` (lines
400-406) is a `Router::<WebhookState>::new()` built and `.merge()`d unconditionally alongside
`operator_router`/`dual_auth_router`/`public_router`/`session_router` (line 412) — a plain
Rust `Router` value, not a macro-generated route table; wrapping its construction/merge in an
`Option`/`if let` is ordinary, idiomatic axum, not awkward.
✓ Blast radius grepped: `build_admin_router` callers (`crates/embyr-server/src/admin/router.rs`
lines 439-492: 4 backward-compatible test wrappers; `tests/card_payments_backend/common/mod.rs`
lines 239-254 and 331-346: 2 direct constructors). `stripe_webhook`/`stripe_signature`/
`webhooks/stripe` occurrences across `tests/`: `tests/card_payments_backend/acceptance/
cpb03_webhook_ingestion.rs` and `cpb04_dunning_suspension_recovery.rs` are the ONLY tests that
issue real HTTP requests to `POST /admin/v1/webhooks/stripe`; both go through
`CpbTestContext::with_webhook_secret`/`new_with_stripe_key`, both of which always pass a real,
non-empty `webhook_signing_secret` string — these are this feature's regression guards for
AC-WHS-03/04, not new tests to write.
✓ **Critical finding, not named by DISCUSS — CI-wide blast radius via a shared test harness**:
`.github/workflows/ci.yml` sets `STRIPE_SECRET_KEY: ${{ secrets.STRIPE_SECRET_KEY }}` at **job**
level (lines 30-41), not step-scoped — it is inherited by every process the `cargo test --workspace`
step spawns, including every subprocess `tests/production_readiness/common/mod.rs::ServerProcess::start()`
launches (that function does NOT call `.env_clear()`; only its sibling `start_env_only()` does).
Grepped every `ServerProcess::start(` call site: 22 call sites across 6 files —
`tests/production_readiness/acceptance/pr01_config_from_env.rs` (4 call sites, including the
walking-skeleton `server_starts_with_all_required_env_vars_set`), `pr04_graceful_shutdown.rs` (3),
`tests/customer_db_onboarding/common/mod.rs` (1), `tests/secrets_management/acceptance/
sm01_admin_key_secrets_manager.rs` (1), `sm02_encryption_key_secrets_manager.rs` (1),
`sm03_encryption_key_rotation.rs` (4), `sm04_admin_key_rotation.rs` (7) — **none of them set
`STRIPE_WEBHOOK_SIGNING_SECRET`**. On the main branch, where the `STRIPE_SECRET_KEY` repo secret
already exists (card-payments-backend's own 4 real-Stripe tests depend on it being set), every one
of these 22 subprocess-spawned servers would silently inherit a non-empty `STRIPE_SECRET_KEY` from
the CI job environment. Without a fix, this feature's own fail-fast check would make every one of
them refuse to start in CI — a sweeping, self-inflicted regression across 6 unrelated feature test
suites, not a real misconfiguration. This is addressed below (§ Decision 3) and is the single
highest-risk item in this feature's entire blast radius.

## Wave: DESIGN / [REF] Decision 1 — Validation Mechanism (config.rs)

**Reuse `ConfigError::MissingVars`'s existing `missing: Vec<String>` accumulator directly — no new
`ConfigError` variant.** This is the smallest possible diff, and it is the mechanism DISCUSS's own
Technical Notes flagged as "the laziest, most consistent fix" — confirmed correct by reading, not
merely assumed.

Two NEW, fully isolated local bindings added to `ServerConfig::from_env()`, immediately after the
existing TLS `match` block (after line 261) and before the existing `if !missing.is_empty()` gate
(line 263) — same position in the function TLS's own check occupies, so the "runs before any port
bind" property is inherited for free, exactly as it is for TLS:

```rust
// ── Stripe webhook secret (stripe-webhook-secret-required, D1) ─────────────
// Independent, throwaway reads — deliberately NOT reusing the
// `stripe_secret_key`/`stripe_webhook_signing_secret` fields populated later
// in this function (lines 309-310 today, unchanged): those use plain `.ok()`
// with no empty-string filter, because GitHub Actions sets a configured-but-
// absent repo secret to `""`, not "unset" (confirmed in ci.yml's own
// STRIPE_SECRET_KEY comment) — an empty string must NOT count as "billing
// enabled" here, or every fork/PR without the secret would fail-fast
// startup. `.filter(|v| !v.is_empty())` mirrors the exact TLS pattern two
// blocks above.
let stripe_secret_key_present = std::env::var("STRIPE_SECRET_KEY")
    .ok()
    .filter(|v| !v.is_empty())
    .is_some();
let stripe_webhook_signing_secret_present = std::env::var("STRIPE_WEBHOOK_SIGNING_SECRET")
    .ok()
    .filter(|v| !v.is_empty())
    .is_some();
if stripe_secret_key_present && !stripe_webhook_signing_secret_present {
    missing.push(
        "STRIPE_WEBHOOK_SIGNING_SECRET (required because STRIPE_SECRET_KEY is set)".to_string(),
    );
}
```

Resulting error text (via the existing, unmodified `Display` impl):
`"missing required environment variable: STRIPE_WEBHOOK_SIGNING_SECRET (required because STRIPE_SECRET_KEY is set)"`
— satisfies AC-WHS-02 exactly: names `STRIPE_WEBHOOK_SIGNING_SECRET` specifically (the literal
substring is present for any `stderr.contains("STRIPE_WEBHOOK_SIGNING_SECRET")` assertion) AND
states the causal reason inline ("required because STRIPE_SECRET_KEY is set") — not a bare var name
and not a generic "billing misconfigured" message.

**Existing lines 309-311 (the real `stripe_secret_key`/`stripe_webhook_signing_secret`/
`stripe_publishable_key` field population) are left completely untouched** — zero risk of changing
what `cfg.stripe_secret_key`/`cfg.stripe_webhook_signing_secret` resolve to for any other consumer.
This is a pure addition, not a modification, of existing config-parsing code.

**Rejected alternative 1**: a new `ConfigError::StripeConfigIncomplete { .. }` variant, mirroring
`TlsFileNotFound`/`TlsInvalidPem`'s shape. Rejected — it would require a new `Display` arm and
duplicate exactly what `MissingVars` already does (name a missing var), for zero behavioral gain;
the `missing`-accumulator mechanism the codebase already uses for 5 other required vars (D-PR-1,
D-PR-6, and this feature's own TLS precedent) already produces a message containing the variable
name, and AC-WHS-02's "because billing is enabled" requirement is satisfiable by formatting the
pushed string, not by adding a type.

**Rejected alternative 2**: computing `stripe_secret_key_present`/`stripe_webhook_signing_secret_present`
by reusing the `Option<String>` fields populated later in the function (moved up, deduplicating the
two `std::env::var` reads). Rejected — it would require either (a) filtering those fields for
empty-string too, which is an unrelated, unscoped behavior change to `cfg.stripe_secret_key`'s
existing semantics (main.rs's placeholder-key fallback at Step 10 currently tolerates `Some("")`
silently; changing that is out of this feature's scope), or (b) leaving them unfiltered and getting
the fork/CI false-positive described above. Two independent, throwaway, filtered local reads is a
6-line addition with zero coupling to existing field semantics — the smaller, safer diff.

## Wave: DESIGN / [REF] Decision 2 — Webhook Route Safety Mechanism (router.rs / main.rs)

**Chosen: (a) do not mount the webhook route at all when Stripe billing is not enabled** — router-
level conditional, computed once at the composition root, using a signal already available there.
Confirmed against the real code shape (not assumed): `webhook_router` is an ordinary `Router` value
built in `build_admin_router` and `.merge()`d once (line 412) — wrapping that in `if let Some(r) =
webhook_router { router = router.merge(r); }` is trivial, idiomatic axum; nothing about the router's
construction fights conditional mounting.

**Why (a) over (b) (mount-but-reject-in-handler)**, weighed against the real code, not in the
abstract:
- **Smaller diff.** (a) needs zero changes to `stripe_webhook_handler` or `stripe_signature_middleware`
  — both already exist, both are untouched. (b) would require threading an "is configured" flag into
  `WebhookState`, adding a new early-return branch inside the middleware or handler, and a new
  response shape (404/501) with its own test coverage — strictly more surface for the identical
  outcome DISCUSS asked for ("the route cannot be used to mutate any tenant's subscription state").
- **Root-cause fix, not a per-caller guard** (this project's own ponytail/minimal-diff philosophy,
  §4 of the DESIGN task instructions). The single unmount decision lives at the ONE composition-root
  call site (`main.rs` Step 10) that already holds `cfg.stripe_secret_key`; every other path to this
  route (there is only one — `POST /admin/v1/webhooks/stripe`) is closed by construction, not by an
  in-handler `if`.
- **No route-enumeration leak.** An unmounted route returns axum's default 404 identically to any
  other nonexistent path — an attacker probing for `/admin/v1/webhooks/stripe` on an
  unconfigured-Stripe deployment learns nothing. A mount-but-reject design would still return a
  distinguishable "this route exists but is disabled" response shape unless deliberately designed
  to look identical to a generic 404 — solvable, but strictly more design surface for no benefit.
- **Matches DISCUSS's own framing directly**: "the feature doesn't exist" (§ System Constraints) —
  (a) makes that literally true at the router level, not simulated by a handler-level check.

**Same empty-string hazard as Decision 1 applies to the mount signal.** `cfg.stripe_secret_key` is
NOT empty-filtered (Decision 1's Rejected Alternative 2 above); the mount decision in `main.rs` must
apply its own `.filter(|v| !v.is_empty())`, independently, exactly mirroring the fail-fast check's
own filter — otherwise a fork/CI run with `STRIPE_SECRET_KEY=""` would incorrectly mount the webhook
route (with an `.expect()` panic once the code below also expects a non-empty webhook secret that
was never fail-fast-checked for that empty-string case). Both checks must agree on what "present"
means, or the invariant `build_admin_router` depends on (secret_key present ⇒ webhook secret
present) silently breaks for the one input shape (`Some("")`) that matters most in CI/forks.

### Code sketch — `crates/embyr-server/src/main.rs` (replaces line 216 and its surrounding call)

```rust
// stripe-webhook-secret-required (D1/D2): mount the webhook sub-router only
// when Stripe billing is genuinely enabled (non-empty STRIPE_SECRET_KEY).
// ServerConfig::from_env() (Step 1, already run) already refused to start
// the process if STRIPE_SECRET_KEY was set without STRIPE_WEBHOOK_SIGNING_SECRET
// — the `.expect()` below documents that invariant, it never fires in a
// process that reached this line.
let stripe_billing_enabled = cfg
    .stripe_secret_key
    .as_deref()
    .is_some_and(|v| !v.is_empty());
let webhook_signing_secret: Option<String> = if stripe_billing_enabled {
    Some(
        cfg.stripe_webhook_signing_secret
            .clone()
            .filter(|v| !v.is_empty())
            .expect(
                "invariant violated: STRIPE_SECRET_KEY is set but \
                 STRIPE_WEBHOOK_SIGNING_SECRET is absent/empty — \
                 ServerConfig::from_env() must already have refused startup",
            ),
    )
} else {
    None
};
```

`build_admin_router(...)`'s call site then passes `webhook_signing_secret` (the `Option<String>`
above) in place of today's `cfg.stripe_webhook_signing_secret.clone().unwrap_or_default()`.

### Code sketch — `crates/embyr-server/src/admin/router.rs`

Signature change (mirrors the codebase's own existing idiom of `Option<T>` meaning "feature off" —
`stripe_secret_key: Option<String>`, `tls: Option<TlsMaterial>` in `config.rs` already use this
exact shape):

```rust
pub fn build_admin_router(
    // ...unchanged params...
    stripe_gateway: Arc<StripeGateway>,
    // card-payments-backend (US-203) → stripe-webhook-secret-required (D1/D2):
    // `None` = Stripe billing not enabled for this deployment — the webhook
    // sub-router is not mounted at all (AC-WHS-01). `Some(secret)` = mount it,
    // gated by `stripe_signature_middleware` exactly as today.
    webhook_signing_secret: Option<String>,
    cap_status_cache: Arc<CapStatusCache>,
) -> Router {
    // ...operator_state / user_state construction unchanged...

    // Webhook sub-router: built only when Stripe billing is enabled.
    let webhook_router: Option<Router> = webhook_signing_secret.map(|secret| {
        let webhook_state = WebhookState {
            system_db: system_db.clone(),
            stripe_gateway: stripe_gateway.clone(),
            webhook_signing_secret: secret,
            credential_cache: credential_cache.clone(),
        };
        Router::<WebhookState>::new()
            .route("/admin/v1/webhooks/stripe", post(stripe_webhook_handler))
            .route_layer(axum::middleware::from_fn_with_state(
                webhook_state.clone(),
                stripe_signature_middleware,
            ))
            .with_state(webhook_state)
    });

    let mut router = Router::new()
        .merge(operator_router)
        .merge(dual_auth_router)
        .merge(public_router)
        .merge(session_router);
    if let Some(webhook_router) = webhook_router {
        router = router.merge(webhook_router);
    }
    router
}
```

The standalone `let webhook_state = WebhookState { .. };` construction currently at lines 132-137
(built unconditionally alongside `operator_state`/`user_state`) moves inside the `.map()` closure
above — it is now built only when a secret is actually supplied.

## Wave: DESIGN / [REF] Decision 3 — CI/Test-Harness Blast-Radius Fix (critical, DISCUSS did not surface this)

**Root cause**: `tests/production_readiness/common/mod.rs::ServerProcess::start()` spawns the real
`embyr-server` binary WITHOUT `.env_clear()`, so it inherits the entire parent process environment
— including CI's job-level `STRIPE_SECRET_KEY`. 22 call sites across 6 unrelated feature test files
(§ Reading Confirmation above) never set `STRIPE_WEBHOOK_SIGNING_SECRET`. Once Decision 1 ships,
every one of those 22 subprocess-spawned servers would refuse to start in any CI run where the
`STRIPE_SECRET_KEY` repo secret is configured (the main branch, per ci.yml's own comment) — not
because of a real misconfiguration, only because of environment leakage through a shared test
helper this feature does not otherwise touch.

**Fix (root cause, one shared function, not 22 call sites)**: `ServerProcess::start()` explicitly
removes both Stripe vars from the child's environment before applying `extra_env`, so no caller
silently inherits them; any test that legitimately wants Stripe configured can still opt in via its
own `extra_env` (removed, then re-added by the existing `for (key, val) in extra_env { cmd.env(key, val); }`
loop, which runs after the removal below — explicit opt-in still works, nothing is permanently
blocked).

### Code sketch — `tests/production_readiness/common/mod.rs::ServerProcess::start()`

```rust
pub fn start(db_url: &str, extra_env: &[(&str, &str)]) -> Self {
    // ...unchanged: port allocation...
    let bin = embyr_server_binary();
    let mut cmd = Command::new(&bin);
    cmd.env("DATABASE_URL", db_url)
        .env("GRPC_PORT", grpc_port.to_string())
        .env("REST_PORT", rest_port.to_string())
        .env("ADMIN_PORT", admin_port.to_string())
        .env("RUST_LOG", "info")
        // stripe-webhook-secret-required: this harness does not clear the
        // environment (unlike its sibling `start_env_only()`), so it would
        // otherwise silently inherit CI's job-level STRIPE_SECRET_KEY —
        // making every one of this helper's 22 unrelated call sites
        // fail-fast in CI for no real misconfiguration reason. Remove both;
        // `extra_env` below can still re-add either explicitly.
        .env_remove("STRIPE_SECRET_KEY")
        .env_remove("STRIPE_WEBHOOK_SIGNING_SECRET")
        .stderr(Stdio::piped())
        .stdout(Stdio::piped());

    for (key, val) in extra_env {
        cmd.env(key, val);
    }
    // ...unchanged: spawn + drain...
}
```

`start_env_only()` needs NO change — it already `.env_clear()`s, so it never inherits anything not
explicitly listed.

## Wave: DESIGN / [REF] Blast Radius — Call Sites Requiring Updates

Changing `build_admin_router`'s `webhook_signing_secret: String` parameter to `Option<String>`
requires updating every existing caller (grepped, all 6 found above are accounted for):

| Call site | Current | New | Why safe |
|---|---|---|---|
| `crates/embyr-server/src/main.rs` (Step 10) | `cfg.stripe_webhook_signing_secret.clone().unwrap_or_default()` | `webhook_signing_secret` (computed per Decision 2 sketch) | This IS the fix — production behavior changes exactly as intended. |
| `crates/embyr-server/src/admin/router.rs::build_with_secret_fetchers` (line 489) | `String::new()` | `None` | These 4 backward-compatible test wrappers back ~15 unrelated test suites (security_rules, client_auth, oauth_providers, anonymous_sessions, admin_api_v2, etc. — none exercise the webhook route). `None` unmounts it — a strict hardening improvement (today these test servers mount a forgeable empty-key webhook route too, just never reachable from any existing test), zero behavior change for any assertion those suites make. |
| `tests/card_payments_backend/common/mod.rs::CpbTestContext::with_webhook_secret` (line 252) | `webhook_signing_secret.to_string()` | `Some(webhook_signing_secret.to_string())` | Always called with a real, non-empty secret (`stripe_secret_key()`'s real test-mode key path or an explicit `whsec_...` value) — route stays mounted, zero behavior change. |
| `tests/card_payments_backend/common/mod.rs::CpbTestContext::new_with_stripe_key` (line 344) | `webhook_signing_secret.to_string()` | `Some(webhook_signing_secret.to_string())` | Same — always a real placeholder secret, route stays mounted. |

No other `build_admin_router` callers exist (grep confirmed: `crates/embyr-server/src/admin/router.rs`
and `tests/card_payments_backend/common/mod.rs` are the only two files that call it).

## Wave: DESIGN / [REF] Handoff Package

**Files requiring changes** (DESIGN specifies; DISTILL designs/adjusts tests; DELIVER implements):

1. `crates/embyr-server/src/config.rs` — ADD the 2-local-binding fail-fast check to `from_env()`
   (Decision 1). No new `ConfigError` variant. No changes to existing field-population lines.
2. `crates/embyr-server/src/main.rs` — REPLACE line 216's `.unwrap_or_default()` call with the
   `stripe_billing_enabled`/`webhook_signing_secret` computation (Decision 2 sketch); pass the
   resulting `Option<String>` into `build_admin_router`.
3. `crates/embyr-server/src/admin/router.rs` — CHANGE `build_admin_router`'s
   `webhook_signing_secret: String` parameter to `Option<String>`; move `WebhookState` construction
   and the `webhook_router` build inside a `.map()` closure; conditionally `.merge()` it (Decision 2
   sketch). UPDATE `build_with_secret_fetchers` (line 489) to pass `None`.
4. `tests/card_payments_backend/common/mod.rs` — UPDATE 2 call sites (lines 252, 344) to wrap the
   existing string argument in `Some(...)`.
5. `tests/production_readiness/common/mod.rs::ServerProcess::start()` — ADD `.env_remove("STRIPE_SECRET_KEY")`
   / `.env_remove("STRIPE_WEBHOOK_SIGNING_SECRET")` before the `extra_env` loop (Decision 3) — the
   single highest-risk item in this feature's blast radius; without it, 22 unrelated subprocess-test
   call sites across 6 files break in main-branch CI.

**No handler/middleware changes**: `crates/embyr-server/src/admin/handlers/webhooks_stripe.rs` and
`crates/embyr-server/src/admin/middleware/stripe_signature.rs` are untouched — Decision 2 (unmount,
not reject-in-handler) means the real signature-verification code path this feature must NOT change
(D3/AC-WHS-03/04) is, by construction, not modified at all.

**Regression-guard tests DISTILL/DELIVER should point at, not re-write**:
- `tests/card_payments_backend/acceptance/cpb03_webhook_ingestion.rs` — real correctly-signed and
  wrong-signature webhook requests against a server built with a real, non-empty secret (AC-WHS-03/04).
- `tests/card_payments_backend/acceptance/cpb04_dunning_suspension_recovery.rs` — same route, dunning
  event types.
- `tests/production_readiness/acceptance/pr01_config_from_env.rs::server_starts_with_all_required_env_vars_set`
  (walking skeleton, AC-WHS-01's "unconfigured Stripe deployment starts exactly as before" claim) and
  its 3 sibling `ServerProcess::start()` call sites in the same file.
- `tests/production_readiness/acceptance/pr04_graceful_shutdown.rs` (3 call sites),
  `tests/customer_db_onboarding/common/mod.rs` (1), `tests/secrets_management/acceptance/
  sm01_admin_key_secrets_manager.rs` / `sm02_encryption_key_secrets_manager.rs` /
  `sm03_encryption_key_rotation.rs` / `sm04_admin_key_rotation.rs` (13 combined) — all depend on
  Decision 3's harness fix landing in the SAME change as Decision 1, not as a follow-up.
- `tests/production_readiness/acceptance/pr05_tls_support.rs` — structural precedent this feature's
  own new AC-WHS-02 subprocess test(s) should mirror exactly (`ServerProcess::start_env_only`,
  `#[ignore]`, `stderr.contains(...)`, `!ServerProcess::port_is_bound(...)`) — not a regression guard
  itself (TLS and Stripe vars are independent), but the shape DISTILL should reuse.

**External integration note for platform-architect (DEVOPS wave)**: this feature touches the
Stripe-webhook integration boundary but adds no new external call — `stripe_signature_middleware`'s
HMAC verification is already local/deterministic (no network). No new contract-testing surface is
introduced beyond what `card-payments-backend` already established.

**Quality gate self-check**: requirements traced to components (Decisions 1-3 above, each tied to
AC-WHS-01/02/03/04/05) — dependency-inversion unaffected (no new port/adapter, pure composition-root
+ config change) — simplest-solution check passed (rejected 2 alternatives per decision, chose the
smaller diff both times) — C4 diagrams: not applicable, this feature adds no new
container/component/external system to the existing architecture (confirmed — no new boundary,
purely a validation + conditional-mount change inside `embyr-server`'s existing composition root)
— OSS preference: n/a, zero new dependencies — AC are behavioral (exit code, stderr content, HTTP
reachability), never implementation-coupled — external integrations: none new (see above) —
enforcement tooling: n/a, no new architectural layering rule introduced.
