# Feature Delta: firestore-tls-support

## Wave: DISCUSS / [REF] Prior Wave Consultation — Reading Confirmation

✓ `docs/product/jobs.yaml`, JOB-13 (`production-deployment`, persona P2 Sam Chen) — read in
full. Its own functional dimension already names `docker run` + env-var configuration + named
startup errors as the shape of "deploy embyr to production correctly" — TLS termination is a
direct extension of that same shape (a new pair of optional env vars, same fail-fast-with-a-
named-error startup discipline as `DATABASE_URL`/`EMBYR_ADMIN_KEY`/`EMBYR_ENCRYPTION_KEY`
already established), not a new concern.
✓ `docs/product/known-gaps.md` #5 — exact current wording confirmed: "No TLS/mTLS on any of
the 3 listeners (:8080 gRPC, :8081 REST/gRPC-Web, :9090 Admin)... grep across
`embyr-server/src`: zero `rustls`/`TlsAcceptor` hits... Depends on deploy topology (fine if a
TLS-terminating LB sits in front)... Blocks production unless mitigated at the LB... Status:
Not started" — this DISCUSS does not re-run that grep or otherwise inspect
`crates/embyr-server/src/lib.rs`; the codebase-level "how" is DESIGN's own investigation.
✓ `docs/product/personas/` — only `chris-account-admin.yaml` (P5) exists as a dedicated
persona file. Sam Chen (P2, Service Operator / Platform Engineer) has no dedicated persona
YAML yet — same situation as every other JOB-13/JOB-11/JOB-12 feature this session
(`production-readiness`, `distributed-rate-limiting`, `observability`), all of which reused
Sam Chen's `jobs.yaml`-embedded profile directly without a separate persona file. Not a gap
introduced by this feature; not resolved here.
✓ `docs/feature/firestore-or-filter-support/feature-delta.md`,
`docs/feature/firestore-is-null-filter-support/feature-delta.md`,
`docs/feature/production-readiness/feature-delta.md` — read in full to confirm this project's
own established single-file, Tier-1-only `feature-delta.md` convention (`## Wave: DISCUSS /
[REF] {Section}` heading format, no standalone `acceptance-criteria.md`/`outcome-kpis.md`/
`story-map.md` files). This DISCUSS mirrors that convention exactly.
✓ Known-gaps.md #6 (graceful shutdown) — noted as already resolved ("stale finding", closed
2026-09-08, direct inspection found it already shipped in `production-readiness`) prior to this
DISCUSS's own work. Referenced for context only; out of scope for this feature, not
re-investigated.

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

- Feature type: **Infrastructure**.
- JTBD: **reuse JOB-13** (`production-deployment`, P2 Sam Chen) — TLS termination is a natural
  extension of "run embyr in production correctly," the same theme `production-readiness`
  already serves under this exact job. Not a new job.
- Walking Skeleton: **Yes** — plain server-side TLS (**no mTLS**) on all 3 listeners, opt-in via
  config.
- UX Research Depth: **Lightweight**.
- **mTLS (client certificate verification) is explicitly OUT OF SCOPE.** The gap's own severity
  reasoning is entirely about plain server-side TLS ("blocks production unless mitigated at the
  LB") — mTLS is named in the gap's TITLE but never appears in its actual severity argument. No
  evidenced customer need for client-cert verification exists. Named exclusion, not a silent
  drop (see § Out of Scope).
- Also out of scope: cert rotation/hot-reload (cert/key read once at startup, matching every
  other secret in this config — `EMBYR_ADMIN_KEY`/`EMBYR_ENCRYPTION_KEY` are also read-once);
  per-listener distinct TLS config (one cert/key pair serves all 3 listeners); ACME/Let's
  Encrypt automation (operator provides pre-issued PEM files).

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: **P2 Sam Chen (Service Operator / Platform Engineer)** — unchanged from JOB-13's
existing profile.

**Job**: **JOB-13 `production-deployment`**, reused, EXTENDED (not replaced) to also cover: two
new optional env vars (`EMBYR_TLS_CERT_PATH`, `EMBYR_TLS_KEY_PATH`) make all 3 listeners
terminate TLS in-process, for the common self-hosted/bare-metal deployment shape where no
TLS-terminating load balancer sits in front of embyr. Leaving both unset preserves today's
plaintext behavior byte-for-byte, so the LB-fronted deployment shape JOB-13's own walking
skeleton already validated (`production-readiness`, `healthz` over plain HTTP) is completely
unaffected.

## Wave: DISCUSS / [REF] Business Context

Gap #5 from `docs/product/known-gaps.md`: none of the 3 TCP listeners (gRPC :8080, REST/
gRPC-Web :8081, Admin :9090) offer any in-process TLS option. Today, a Sam-Chen-shaped operator
running embyr WITHOUT a TLS-terminating LB in front (a common self-hosted/bare-metal deployment
shape — e.g. a single VPS, an on-prem box behind a corporate firewall but not behind a managed
LB) has client SDK traffic (carrying the project's `api_key`), admin API traffic (carrying
`EMBYR_ADMIN_KEY`), and REST/gRPC-Web browser traffic all traveling in plaintext, with zero
mitigation available inside embyr itself. LB-fronted deployments (today's assumed default) are
unaffected by the gap and must remain unaffected by the fix — this is why the regression guard
(AC-TLS-01) is the single highest-priority acceptance criterion in this feature.

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

**Signals checked**: >10 user stories? No (1). >3 bounded contexts/modules? No — entirely
confined to `embyr-server`'s own composition root (the 3 listener bind sites), mirroring
`production-readiness`'s own single-composition-root scope, which passed this same gate.
Walking skeleton >5 integration points? No (4: 3 real TLS handshakes + 1 regression check on the
unset-vars path — all against the SAME single running server instance, not 4 separate systems).
Estimated effort >2 weeks? No — one cert/key pair, read once at startup, wrapping 3 already-
existing `TcpListener::bind` call sites; no new domain concept, no new adapter trait, no new
bounded context. Multiple independent user outcomes? No — "encrypt all 3 listeners with one
opt-in config" is a single outcome; the 3 listeners are 3 verification points of that ONE
outcome, not 3 separable stories (a partial rollout — TLS on gRPC but not Admin — is explicitly
rejected by the single-cert-pair-for-all-3-listeners decision above, so splitting by listener
would fragment an atomically-decided design, not deliver independent value).

**Scope Assessment: PASS** — 1 user story, 1 bounded context (`embyr-server` composition root),
estimated ≤2 days, 7 UAT scenarios (at the upper bound of right-sized, not over it).

## Wave: DISCUSS / [REF] Journey — Sam's "No LB, No Problem" Arc

### Mental model

Sam Chen deploys embyr-server on a bare-metal box or a single VPS with no TLS-terminating LB in
front — the common shape for a small self-hosted deployment, or a customer whose network
topology doesn't include a managed LB in the request path. Today Sam has no in-process option:
either accept plaintext traffic carrying API keys and the admin key, or go build a reverse-proxy
layer himself before he can call the deployment production-ready. Sam expects the SAME
fail-fast, named-error startup discipline this job's own `production-readiness` feature already
established for `DATABASE_URL`/`EMBYR_ADMIN_KEY`/`EMBYR_ENCRYPTION_KEY` — a misconfigured TLS
setup must refuse to start loudly, never silently serve plaintext while claiming to be secure.

### Failure modes (feeds DELIVER test design)

- Both TLS vars unset (today's implicit default, LB-fronted deployments): server must start and
  behave exactly as it does today — zero behavior change.
- Exactly one of the two vars set: a config mistake (e.g. copy-pasted only half the pair) —
  server must refuse to start with a named error identifying which variable is missing.
- Both vars set, but one file path doesn't exist (typo, wrong mount path in a container): server
  must refuse to start with a named "file not found" error — never fall back to plaintext
  silently, since that would be a worse outcome than refusing to start at all.
- Both vars set, files exist, but content isn't valid PEM (wrong file swapped in, corrupted
  copy): server must refuse to start with a named parse error — same reasoning as the missing-
  file case.
- Both vars set correctly: all 3 listeners must terminate TLS, each independently verified by a
  real handshake — a working gRPC listener does not imply the Admin or REST/gRPC-Web listener is
  also correctly wired, since each is built on a different underlying server framework
  (`tonic` for gRPC, `axum` for REST/gRPC-Web and Admin).

## Wave: DISCUSS / [REF] Story Map & Walking Skeleton

### Backbone

Sam sets 2 env vars → **this feature**: server startup reads `EMBYR_TLS_CERT_PATH`/
`EMBYR_TLS_KEY_PATH`, validates the pair (both-or-neither, files exist, PEM parses) → on
success, all 3 `TcpListener::bind` sites (gRPC :8080, REST/gRPC-Web :8081, Admin :9090) wrap
their accepted connections in a TLS handshake using the same cert/key pair → on any validation
failure, server exits non-zero with a named error before binding any port, mirroring
`production-readiness`'s own D-PR-6 "all three ports or none" discipline.

### Walking Skeleton

This feature IS the walking skeleton (single story, no further slicing) — proven end-to-end
against: (1) the regression path (both vars unset, plaintext unaffected), (2) all 3 listeners
independently TLS-verified when configured, (3) both fail-fast error paths (partial config,
bad file/PEM).

## Wave: DISCUSS / [REF] Elephant Carpaccio Slices

| # | Story | Release | Estimate | Learning Hypothesis | Reference Class |
|---|---|---|---|---|---|
| 01 | US-01 (TLS-encrypted listeners via 2 config vars, Walking Skeleton) | 1 | ≤2 days | Disproves: a single opt-in cert/key pair, read once at startup, is sufficient to TLS-wrap all 3 already-existing listener bind sites without touching per-listener application logic (gRPC service handlers, admin route handlers, REST/gRPC-Web translation are all unaware of the transport-layer change) | Mirrors `production-readiness`'s own US-PR-01 precedent: env-var-driven, fail-fast-with-named-error startup config, validated before any port binds |

## Wave: DISCUSS / [REF] Prioritization

Single story, no sequencing decision required. Within the story, the regression-guard scenario
(AC-TLS-01) and the 2 fail-fast scenarios (AC-TLS-05/06/07) are the highest-priority tests to
prove first in DELIVER — they protect the default (unset) deployment shape from ANY regression,
which matters more than proving the new capability works, exactly as `production-readiness`'s
own D-PR-6 "no partial startup" guard was proven before the happy path in that feature.

## Wave: DISCUSS / [REF] System Constraints

- Two new optional env vars: `EMBYR_TLS_CERT_PATH`, `EMBYR_TLS_KEY_PATH`. Both unset → today's
  plaintext behavior, byte-for-byte unchanged (hard regression guard, not merely "should still
  work"). Exactly one set → startup fails fast with a named error identifying which is missing.
  Both set → both must resolve to existing, readable files containing valid PEM content, or
  startup fails fast with a named error identifying the specific failure (file not found vs.
  unparseable content are named DIFFERENTLY, matching `production-readiness`'s own D-PR-1 "error
  accumulation... operator sees every problem" discipline applied to this narrower 2-var case).
- One cert/key pair serves ALL 3 listeners — no per-listener distinct TLS config in this
  feature's scope.
- Cert/key are read ONCE at startup — no hot-reload, no file-watching, no rotation mechanism.
  Matches `EMBYR_ADMIN_KEY`/`EMBYR_ENCRYPTION_KEY`'s own existing read-once-at-startup pattern
  exactly (per `production-readiness`'s D-PR-1/D-PR-7).
- No client-certificate verification (mTLS) anywhere in this feature's scope — every listener
  remains open to any TLS client presenting no certificate, exactly as before this feature
  except for the encryption itself.
- All 3 listeners or none — if TLS is correctly configured, all 3 bind with TLS; if TLS config
  is invalid, no port binds at all (mirrors D-PR-6's "all three ports or none" exactly, extended
  to cover TLS validation failure as an equally hard startup gate).

## Wave: DISCUSS / [REF] User Stories

### US-01: An Operator Without a TLS-Terminating Load Balancer Can Encrypt All 3 Listeners

**job_id**: JOB-13

#### Elevator Pitch
**Before**: Sam Chen deploys embyr-server on a bare-metal box or VPS with no TLS-terminating
load balancer in front. Every listener — gRPC :8080 (carrying client SDK traffic and project
`api_key`s), Admin :9090 (carrying `EMBYR_ADMIN_KEY`), and REST/gRPC-Web :8081 (carrying browser
traffic) — serves plaintext with zero in-process alternative.
**After**: Sam sets `EMBYR_TLS_CERT_PATH` and `EMBYR_TLS_KEY_PATH` to point at a PEM certificate
and private key. Restarting the server, all 3 listeners now require and correctly terminate
TLS. Leaving both unset leaves every listener exactly as plaintext as before — LB-fronted
deployments (today's default) are unaffected.
**Decision enabled**: Sam can choose, per deployment, whether TLS termination happens at an
external LB or in-process inside embyr itself — a real architectural decision he can now make
with a 2-variable config change instead of being forced to build or bolt on a reverse-proxy
layer before calling a bare-metal deployment production-ready.

#### Who
- Sam Chen (P2) | Service operator deploying embyr WITHOUT a TLS-terminating LB in front (bare-
  metal/self-hosted deployment shape) | Needs in-process TLS termination with the same fail-
  fast, named-error startup discipline `production-readiness` already established for the
  existing required env vars.

#### Solution
Two new optional env vars, `EMBYR_TLS_CERT_PATH` and `EMBYR_TLS_KEY_PATH`, validated together at
startup (both-or-neither; both files must exist and parse as valid PEM). When both are set and
valid, all 3 existing `TcpListener` bind sites (gRPC, REST/gRPC-Web, Admin) wrap accepted
connections in a TLS handshake using the same cert/key pair. When both are unset, startup and
runtime behavior are unchanged from today.

#### Domain Examples

**Example 1 (Happy Path)**: Sam Chen deploys embyr-server for Meridian Health on a bare-metal
box with no LB in front. He sets:
```
EMBYR_TLS_CERT_PATH=/etc/embyr/tls/server.crt
EMBYR_TLS_KEY_PATH=/etc/embyr/tls/server.key
```
pointing at a certificate issued by Meridian's internal CA. He restarts the server. Firebase SDK
clients connecting to :8080 with `grpc.credentials.createSsl()` complete a TLS handshake and
their Firestore calls succeed. `curl --cacert ca.pem https://host:9090/healthz` returns 200 over
HTTPS. Browser clients hitting the REST/gRPC-Web endpoint at :8081 see a valid TLS certificate
in the browser's own connection info panel.

**Example 2 (Edge Case — partial config)**: Sam sets `EMBYR_TLS_CERT_PATH` but forgets
`EMBYR_TLS_KEY_PATH` (a copy-paste mistake deploying from a checklist). The server refuses to
start, exits non-zero, and logs an error naming `EMBYR_TLS_KEY_PATH` specifically as the missing
half of the pair — not a generic "TLS misconfigured" message.

**Example 3 (Error/Boundary — bad file path)**: Sam sets both vars, but
`EMBYR_TLS_KEY_PATH=/etc/embyr/tls/sever.key` (typo, file doesn't exist at that path in the
container's mounted volume). The server refuses to start, exits non-zero, and logs an error
naming the exact path that could not be read — never silently continuing in plaintext mode.

**Example 4 (Regression Guard — today's default)**: Sam Chen's OTHER deployment, for a customer
already running embyr behind an AWS ALB that terminates TLS, has neither `EMBYR_TLS_CERT_PATH`
nor `EMBYR_TLS_KEY_PATH` set — exactly as configured before this feature shipped. The server
starts, binds all 3 ports in plaintext, and behaves identically to every deployment before this
feature existed.

#### UAT Scenarios (BDD)

```gherkin
Scenario: Existing plaintext deployments are unaffected when TLS is not configured
  Given EMBYR_TLS_CERT_PATH and EMBYR_TLS_KEY_PATH are both unset
  When Sam Chen starts embyr-server
  Then all three listeners (gRPC :8080, REST/gRPC-Web :8081, Admin :9090) accept plain TCP connections exactly as before this feature
  And no TLS handshake is attempted or required on any listener

Scenario: The gRPC listener terminates TLS when configured
  Given EMBYR_TLS_CERT_PATH and EMBYR_TLS_KEY_PATH point to a valid PEM certificate and private key
  When Sam Chen starts embyr-server
  And a gRPC client connects to :8080 using TLS credentials
  Then the TLS handshake succeeds
  And the client's Firestore RPCs complete normally over the encrypted connection

Scenario: The Admin listener terminates TLS when configured
  Given EMBYR_TLS_CERT_PATH and EMBYR_TLS_KEY_PATH point to a valid PEM certificate and private key
  When Sam Chen starts embyr-server
  And an HTTPS client connects to :9090 using TLS
  Then the TLS handshake succeeds
  And GET /healthz over that TLS connection returns HTTP 200

Scenario: The REST/gRPC-Web listener terminates TLS when configured
  Given EMBYR_TLS_CERT_PATH and EMBYR_TLS_KEY_PATH point to a valid PEM certificate and private key
  When Sam Chen starts embyr-server
  And a browser-style HTTPS client connects to :8081 using TLS
  Then the TLS handshake succeeds
  And a REST/gRPC-Web request completes normally over the encrypted connection

Scenario: Startup fails fast when only one of the two TLS config vars is set
  Given EMBYR_TLS_CERT_PATH is set and EMBYR_TLS_KEY_PATH is unset
  When Sam Chen starts embyr-server
  Then the process exits with a non-zero code before any port is bound
  And stderr names EMBYR_TLS_KEY_PATH specifically as the missing required variable

Scenario: Startup fails fast when the configured cert or key file is missing
  Given EMBYR_TLS_CERT_PATH and EMBYR_TLS_KEY_PATH are both set
  And the file at EMBYR_TLS_KEY_PATH does not exist on disk
  When Sam Chen starts embyr-server
  Then the process exits with a non-zero code before any port is bound
  And stderr names the specific missing file path
  And no listener falls back to plaintext

Scenario: Startup fails fast when the configured PEM content is unparseable
  Given EMBYR_TLS_CERT_PATH and EMBYR_TLS_KEY_PATH both point to existing, readable files
  And the file at EMBYR_TLS_CERT_PATH does not contain valid PEM certificate data
  When Sam Chen starts embyr-server
  Then the process exits with a non-zero code before any port is bound
  And stderr names the PEM parse failure
  And no listener falls back to plaintext
```

#### Acceptance Criteria
- [ ] AC-TLS-01 (regression guard): with both `EMBYR_TLS_CERT_PATH`/`EMBYR_TLS_KEY_PATH` unset,
      startup and runtime behavior on all 3 listeners is unchanged, byte-for-byte, from today.
- [ ] AC-TLS-02: a real TLS handshake succeeds against the gRPC listener (:8080) when both vars
      are correctly configured, and a Firestore RPC completes over that connection.
- [ ] AC-TLS-03: a real TLS handshake succeeds against the Admin listener (:9090) when both vars
      are correctly configured, and `GET /healthz` returns 200 over that connection.
- [ ] AC-TLS-04: a real TLS handshake succeeds against the REST/gRPC-Web listener (:8081) when
      both vars are correctly configured, and a request completes over that connection.
- [ ] AC-TLS-05: exactly one of the two vars set (not both, not neither) causes startup to exit
      non-zero before any port binds, naming the specific missing variable.
- [ ] AC-TLS-06: both vars set but pointing at a missing file causes startup to exit non-zero
      before any port binds, naming the specific missing file — never a panic, never a silent
      plaintext fallback.
- [ ] AC-TLS-07: both vars set but pointing at unparseable PEM content causes startup to exit
      non-zero before any port binds, naming the parse failure — never a panic, never a silent
      plaintext fallback.

#### Outcome KPIs
- **Who**: Sam Chen operating self-hosted/bare-metal embyr deployments with no TLS-terminating
  load balancer in front.
- **Does what**: Enables in-process TLS encryption on all 3 listeners using a 2-variable config
  change, with no reverse-proxy layer to build or operate.
- **By how much**: from 0% (zero in-process TLS option exists today, per known-gaps.md #5's own
  `rustls`/`TlsAcceptor` grep finding of zero hits) to 100% of correctly-configured deployments
  serving TLS on all 3 listeners; 0% regression on the existing plaintext/LB-fronted deployment
  shape.
- **Measured by**: real TLS handshake success against all 3 listeners in integration tests
  (AC-TLS-02/03/04); zero regressions in the existing plaintext-mode test suite (AC-TLS-01).
- **Baseline**: 0% — confirmed zero `rustls`/`TlsAcceptor` usage anywhere in `embyr-server/src`
  (known-gaps.md #5's own grep finding, not re-verified independently by this DISCUSS per its
  explicit scope limit against deep codebase investigation).

#### Technical Notes
- Both vars must be validated TOGETHER (both-or-neither) — a partial config is a startup error,
  never a degraded-but-running state.
- File-not-found and PEM-parse-failure are named DIFFERENTLY in the startup error, mirroring
  `production-readiness`'s own D-PR-1 error-accumulation discipline extended to this narrower
  2-var case.
- One cert/key pair for all 3 listeners — DESIGN decides the exact mechanism (shared
  `rustls::ServerConfig`, wrapping each of the 3 existing accept loops), not decided here.
- `embyr-agent` already implements its OWN separate mTLS mechanism for its StorageAgent gRPC
  channel to embyr SaaS (JOB-09, `agent-auditproof`) — a different, already-shipped, unrelated
  surface (agent-to-SaaS, not SaaS-to-SDK-client). This DISCUSS did not investigate whether that
  existing mechanism offers reusable primitives; flagged for DESIGN to check, not assumed either
  way, per this DISCUSS's explicit scope limit against implementation-file investigation.
- Depends on nothing new — the 3 listener bind sites already exist and are unchanged in number
  or purpose by this feature.

## Wave: DISCUSS / [REF] Definition of Done

1. Both env vars translate from unset/set-pair/invalid into the correct startup behavior in
   every case named by AC-TLS-01 through AC-TLS-07.
2. All 3 listeners (gRPC, REST/gRPC-Web, Admin) independently prove a real TLS handshake when
   configured — no listener's correctness is inferred from another's.
3. The regression guard (AC-TLS-01) is proven identical to pre-feature behavior, not merely
   "still works" — the existing plaintext test suite passes unmodified.
4. Both fail-fast paths (partial config; bad file/PEM) exit non-zero before any port binds, with
   a named, specific error — never a panic, never a silent plaintext fallback.
5. Full regression suite clean (pre-existing flakes excepted, triaged not assumed, per this
   session's own established `feedback_triage_before_dismissing_as_flaky` practice).
6. Mutation testing: 100% effective kill rate on the new/changed startup-validation and
   listener-wrapping logic.
7. Evolution doc written; `docs/product/known-gaps.md` #5 updated to CLOSED.
8. Memory updated.

## Wave: DISCUSS / [REF] Out of Scope

- **mTLS (client certificate verification)** — the gap's own severity argument is entirely
  about plain server-side TLS ("blocks production unless mitigated at the LB"); mTLS is named
  only in the gap's TITLE, never in its severity reasoning. No evidenced customer need for
  client-cert verification exists today. Named, reasoned exclusion — a candidate follow-up
  feature if a real need surfaces later, not silently dropped.
- **Cert rotation / hot-reload** — cert and key are read once at startup, matching every other
  secret in this config (`EMBYR_ADMIN_KEY`, `EMBYR_ENCRYPTION_KEY`). Rotating a cert requires a
  server restart, exactly as rotating the admin key does today.
- **Per-listener distinct TLS configuration** — one cert/key pair serves all 3 listeners. A
  deployment needing different certificates per listener (e.g. a public-facing gRPC cert vs. an
  internal-only Admin cert) is out of scope; the operator's own network topology (e.g. binding
  Admin to a private interface) is the recommended mitigation, not a per-listener config surface
  in embyr itself.
- **ACME/Let's Encrypt automation** — the operator provides pre-issued PEM files. No in-process
  certificate issuance or renewal automation is in scope.

## Wave: DISCUSS / [REF] Walking Skeleton Strategy

**Strategy A** (real, minimal, end-to-end) — every scenario is a real TLS handshake (or a real
absence of one) against a real running server instance, including the 2 fail-fast startup paths
verified via real subprocess exit-code + stderr inspection, mirroring
`production-readiness`'s own Strategy C precedent (real subprocess + real driven-internal
Postgres) for this exact composition-root layer.

## Wave: DISCUSS / [REF] Driving Ports

All 3 already-existing TCP listeners, zero new RPC/HTTP endpoint: gRPC `:8080` (tonic), REST/
gRPC-Web `:8081` (axum + tonic-web), Admin HTTP `:9090` (axum). TLS is a transport-layer wrap
around each existing accept loop — no application-level route, handler, or RPC method changes.

## Wave: DISCUSS / [REF] Pre-requisites

- None beyond what already exists. The 3 listener bind sites, and the fail-fast/named-error
  startup discipline they must extend, are already established by `production-readiness`
  (D-PR-1, D-PR-6, D-PR-7) — this feature is a direct, same-shape extension of that pattern, not
  new architecture.
- No new external dependency confirmed as required or rejected here — DESIGN's own concern
  (e.g. `rustls`/`tokio-rustls` selection). This DISCUSS names the requirement only: TLS
  termination from a PEM cert/key pair, applied uniformly to 3 existing async accept loops.

## Wave: DISCUSS / [REF] Definition of Ready Validation

### Requirements Completeness Score: **0.95**

### DoR Checklist (9-item hard gate)
1. [x] Story traces to a job_id (JOB-13)
2. [x] Story has a complete Elevator Pitch
3. [x] Every AC is testable without ambiguity (7 ACs, each a real handshake or a real exit-code/
   stderr assertion)
4. [x] Walking Skeleton identified (US-01 is the whole feature's walking skeleton)
5. [x] Scope Assessment passed
6. [x] Story is not `@infrastructure`-only with no user-visible value — it directly enables
   Sam Chen's own deployment-topology decision (Elevator Pitch "Decision enabled")
7. [x] Out of Scope explicitly named (4 items, each reasoned)
8. [x] Outcome KPIs have numeric targets (0% → 100%) and measurement methods
9. [x] Prior-wave artifacts read and reconciled (JOB-13's own job story and `production-
   readiness`'s own D-PR-1/D-PR-6/D-PR-7 directly informed this feature's design)

### DoR Status: **PASSED**

## Wave: DISCUSS / [REF] Open Questions

None carried forward unresolved. The one question a naive reading of the gap's own TITLE might
raise (does "TLS/mTLS" mean both are in scope?) is directly resolved in § Orchestrator Decisions
and § Out of Scope: mTLS is a named, reasoned exclusion, not an oversight.

## Wave: DISCUSS / [REF] Wave Decisions Summary

### Key Decisions
- [D1] Two new optional env vars, `EMBYR_TLS_CERT_PATH`/`EMBYR_TLS_KEY_PATH`, validated
  together (both-or-neither) at startup, before any port binds.
- [D2] One cert/key pair serves all 3 listeners — no per-listener distinct config.
- [D3] Cert/key are read once at startup — no rotation/hot-reload mechanism in this feature.
- [D4] No mTLS (client certificate verification) anywhere in this feature's scope.
- [D5] Missing-file and unparseable-PEM failures are named DIFFERENTLY in the startup error,
  and both are hard startup gates (exit non-zero, no port binds) — never a silent plaintext
  fallback.

### Requirements Summary
- Primary need: operators deploying without a TLS-terminating LB need an in-process option to
  encrypt all 3 listeners, without regressing the existing LB-fronted default deployment shape.
- Walking skeleton scope: US-01, the entire feature — single story, 7 UAT scenarios.
- Feature type: Infrastructure.

### Constraints Established
- Zero behavior change when both vars are unset (hard regression guard).
- Zero silent plaintext fallback on any TLS-config failure — always a named, fail-fast startup
  error.
- No new bounded context, no new domain type, no new RPC/HTTP endpoint.

### Upstream Changes
None — this DISCUSS extends JOB-13's existing scope (same job, same persona), consistent with
`production-readiness`'s own original job story, which already anticipated "no cryptic
failures" and "startup errors are named and specific" as JOB-13's own emotional dimension.

## Wave: DISCUSS / [REF] Next Wave

**Handoff To**: nw-solution-architect (DESIGN wave)
**Deliverables**: this feature-delta.md, 5 locked Decisions, 1-story walking-skeleton plan

---

## Wave: DESIGN / [REF] Prior Wave Consultation — Reading Confirmation

✓ This feature-delta.md's own DISCUSS sections in full, all 5 Decisions (D1-D5).
✓ `crates/embyr-server/Cargo.toml` — confirmed `tonic = { version = "0.12", features = ["tls"] }`
is already enabled at the workspace level (root `Cargo.toml` line 16); confirmed `rustls`/
`tokio-rustls` are already `[dependencies]` of `embyr-server` (used today only by
`adapters/stripe_gateway.rs` for outbound TLS); confirmed `rcgen`/`rustls-pemfile`/`tempfile` are
already `[dev-dependencies]` — present but NOT yet promoted to `[dependencies]`, unlike
`ed25519-dalek`, which this SAME Cargo.toml documents (lines 66-71) as "promoted from
dev-dependencies... enables a REAL... key in PRODUCTION code" for `client-auth-hosted-identity`.
This feature needs the identical promotion for `rustls-pemfile` (production PEM parsing, not just
test fixtures) — a direct, already-proven precedent to reuse, not a novel decision.
✓ `crates/embyr-server/src/main.rs` — full 14-step startup sequence read; confirmed Step 1
(`ServerConfig::from_env()`) runs before Step 8 (`TcpListener::bind` × 3), so any TLS-config
validation failure surfaced from `from_env()` already satisfies AC-TLS-05/06/07's "before any port
binds" requirement with ZERO reordering of `main.rs`'s own existing step sequence.
✓ `crates/embyr-server/src/config.rs` — full file read; confirmed the exact `from_env()` shape
this feature extends: resolve every var into local `Option`s while accumulating missing required
names into a single `missing: Vec<String>` → one `if !missing.is_empty() { return
Err(MissingVars(missing)) }` gate → THEN validate the now-guaranteed-present values (e.g.
`validate_encryption_key_hex`). This exact shape is reused, not reinvented, for the two new TLS
vars (see § Architecture Design below).
✓ `crates/embyr-server/src/lib.rs` — full file read; confirmed `spawn_all_servers` has exactly 9
existing call sites, ALL inside this same file (`main.rs` × 1, plus 8 test-server constructor
functions: `start_test_server_with_keepalive`, `start_test_server_with_email_sender`,
`start_test_server_with_oauth`, `start_test_server_with_aws_fetcher`,
`start_test_server_with_gcp_fetcher`, `start_test_server_with_distributed_rate_limit`,
`start_test_server_with_rate_limit`, plus `start_test_server` which delegates to
`start_test_server_with_keepalive`) — confirmed by direct read, not assumed. Confirmed the Admin
listener (`:9090`) is the ONLY one of the 3 using bare `axum::serve(admin_listener, admin_app)`
with no TLS hook; confirmed the gRPC listener already builds its own
`tonic::transport::Server::builder()...serve_with_incoming_shutdown(...)` future independently,
the correct insertion point for `.tls_config(...)`.
✓ `crates/embyr-server/src/rest/grpc_web.rs` — full file read; confirmed `spawn_hybrid_server`'s
manual accept loop (`listener.accept()` → `TokioIo::new(stream)` →
`hyper_util::server::conn::auto::Builder` → `serve_connection_with_upgrades`) is the exact,
already-proven extension point named in the task brief — confirmed directly, not assumed.
Confirmed `incoming_to_axum_body` is currently a private `fn` — needs `pub(crate)` visibility to
be reused by the Admin listener's new accept loop (§ Architecture Design, Decision D9).
✓ **UNCHECKED LEAD from DISCUSS, now resolved**: `tests/acceptance/embyr_agent/mod.rs::
test_tls_config()` (rcgen-based self-signed CA/server/client cert generation for `embyr-agent`'s
own, separate, already-shipped mTLS surface) — confirmed this is agent-to-SaaS, structurally
unrelated to this feature's SaaS-to-SDK-client listeners; no reusable production code, but its
OWN pattern (`rcgen` for test cert generation, `rustls::crypto::ring::default_provider()
.install_default()` as a required one-time global crypto-provider install) is directly reusable
by DISTILL for this feature's own acceptance tests — recommended, not required, since `rcgen`/
`rustls-pemfile`/`tempfile` are already `[dev-dependencies]` of `embyr-server` itself (not just
`embyr-agent`'s test harness), confirmed by direct read of `crates/embyr-server/Cargo.toml` lines
1064-1067.
✓ **Stronger precedent found, not previously known to DISCUSS**: `tests/acceptance/
us_12_agent_backend.rs` lines 745-767 ALREADY builds a real `rustls` config in THIS crate's own
test suite using the EXACT API shape this design proposes: `rustls::pki_types::CertificateDer`,
`rustls_pemfile::certs(&mut reader).collect::<Result<Vec<CertificateDer<'static>>, _>>()`, and
`rustls::ClientConfig::builder()...with_no_client_auth()`. This is directly load-bearing evidence
that the `rustls`/`rustls-pemfile`/`rustls::pki_types` API combination this design uses for
`ServerConfig::builder()...with_single_cert(...)` (the server-side mirror of that exact
client-side call) already compiles and works against this exact dependency-version set in this
exact codebase — not a novel, unverified API guess.
✓ Confirmed via direct grep (not assumed) — `spawn_all_servers`/`spawn_hybrid_server` blast
radius: 10 files match, of which exactly 3 are real Rust source (`lib.rs`, `main.rs`,
`grpc_web.rs`); the remaining 7 are documentation/JSON artifacts from the unrelated
`production-readiness` feature (which coined the D-PR-6 "all or none" pattern this feature
extends) — zero other crate or test file constructs either function directly, confirming
DISCUSS's own scope claim ("entirely confined to `embyr-server`'s own composition root").
✓ Confirmed via direct grep — `ServerConfig {` struct-literal construction occurs in exactly one
place (`config.rs`'s own `from_env()`, line 272) — no test or other module constructs
`ServerConfig` by hand, so adding a new field cannot silently break an out-of-band construction
site.

## Wave: DESIGN / [REF] Architecture Design

### Overview

One cert/key pair, read once, validated eagerly (file-exists + PEM-parseable + key-matches-cert)
inside `ServerConfig::from_env()` — mirroring `validate_encryption_key_hex`'s own precedent
exactly. The validated result is stored as `Option<TlsMaterial>` (raw PEM bytes for tonic's own
`Identity::from_pem`, PLUS a pre-built `Arc<rustls::ServerConfig>` for the two axum-based
listeners). Two DIFFERENT TLS types are threaded to the 3 listeners because tonic's own TLS
mechanism (`tonic::transport::ServerTlsConfig`) and the manual-accept-loop listeners' mechanism
(`tokio_rustls::TlsAcceptor`) are unrelated types, both built from the SAME underlying PEM bytes —
no double-source-of-truth, no risk of the two listeners disagreeing about which cert is active.

### 1. Dependency change (`crates/embyr-server/Cargo.toml`)

Promote `rustls-pemfile` from `[dev-dependencies]` to `[dependencies]` — mirrors this SAME file's
own documented `ed25519-dalek` promotion (lines 66-71: "promoted from dev-dependencies ...
generates a REAL ... key in PRODUCTION code"). No new crate is added; `rustls::pki_types::
{CertificateDer, PrivateKeyDer}` is used via `rustls`'s own re-export (confirmed available,
`us_12_agent_backend.rs` line 745 already imports it this way) — avoids adding a separate
`rustls-pki-types` dependency the ladder's "already-installed dependency" rung would reject.

```toml
[dependencies]
# ...existing...
rustls-pemfile.workspace = true   # firestore-tls-support: production PEM parsing
                                    # (promoted from dev-deps, mirrors ed25519-dalek's own
                                    # documented promotion above)
```

### 2. `ServerConfig` extension (`crates/embyr-server/src/config.rs`)

New struct (custom `Debug` — never print raw key bytes):

```rust
/// Validated TLS material for firestore-tls-support: one cert/key pair
/// shared by all 3 listeners, read once at startup. `cert_pem`/`key_pem`
/// are the raw PEM bytes (tonic's own `Identity::from_pem` wants raw PEM);
/// `rustls_config` is pre-built once so both axum-based listeners share the
/// IDENTICAL `Arc<rustls::ServerConfig>` instance rather than each
/// re-parsing PEM independently.
#[derive(Clone)]
pub struct TlsMaterial {
    pub cert_pem: Vec<u8>,
    pub key_pem: Vec<u8>,
    pub rustls_config: std::sync::Arc<rustls::ServerConfig>,
}

impl fmt::Debug for TlsMaterial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TlsMaterial")
            .field("cert_pem_len", &self.cert_pem.len())
            .field("key_pem", &"<redacted>")
            .finish()
    }
}
```

`ServerConfig` gains one field:

```rust
/// `EMBYR_TLS_CERT_PATH`/`EMBYR_TLS_KEY_PATH` — optional, both-or-neither.
/// `None` = today's plaintext behavior on all 3 listeners, unchanged
/// (AC-TLS-01, hard regression guard).
pub tls: Option<TlsMaterial>,
```

Two new `ConfigError` variants (mirrors `InvalidEncryptionKey`'s per-field shape — file-not-found
and unparseable-PEM are named DIFFERENTLY per D5):

```rust
/// EMBYR_TLS_CERT_PATH or EMBYR_TLS_KEY_PATH names a path that does not
/// exist or cannot be read. `var` names which of the two (AC-TLS-06).
TlsFileNotFound { var: String, path: String },
/// The file at `var`'s configured path was read, but its content is not
/// valid PEM, or the private key does not match the certificate
/// (AC-TLS-07).
TlsInvalidPem { var: String, reason: String },
```

Partial config (exactly one of the two vars set) is deliberately **not** a new variant — it reuses
the EXISTING `MissingVars` accumulator, since a var whose pair-partner is set is, in that
configuration state, literally a missing required variable (see Decision D6 below).

`from_env()` wiring — inserted in the SAME position as `db_url`/`admin_key`/`encryption_key`
resolution (resolve → accumulate into `missing` → gate → validate present values):

```rust
// ── TLS (firestore-tls-support, D1/D5) ─────────────────────────────────
let cert_path = std::env::var("EMBYR_TLS_CERT_PATH").ok().filter(|v| !v.is_empty());
let key_path = std::env::var("EMBYR_TLS_KEY_PATH").ok().filter(|v| !v.is_empty());
match (&cert_path, &key_path) {
    (Some(_), None) => missing.push("EMBYR_TLS_KEY_PATH".to_string()),
    (None, Some(_)) => missing.push("EMBYR_TLS_CERT_PATH".to_string()),
    _ => {} // both set, or both unset — no missing-var error either way
}

if !missing.is_empty() {
    return Err(ConfigError::MissingVars(missing));
}

// ...existing encryption_key validation...

let tls = match (cert_path, key_path) {
    (Some(cert_path), Some(key_path)) => Some(load_tls_material(&cert_path, &key_path)?),
    _ => None, // AC-TLS-01
};
```

`load_tls_material` — the eager, fail-fast PEM validation (this function IS the Earned Trust
probe for the filesystem dependency; see § Earned Trust below):

```rust
/// Read and parse the cert/key PEM pair. Installs the process-wide rustls
/// crypto provider (idempotent — mirrors `StripeGateway`'s own already-
/// established `rustls::crypto::ring::default_provider().install_default()`
/// call in `adapters/stripe_gateway.rs`). This feature's own call cannot
/// rely on StripeGateway's call happening first — StripeGateway is
/// constructed in `main.rs` Step 10, well AFTER this function runs in Step
/// 1 (`from_env()`) — so TLS material construction installs the provider
/// itself.
fn load_tls_material(cert_path: &str, key_path: &str) -> Result<TlsMaterial, ConfigError> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let cert_pem = std::fs::read(cert_path).map_err(|_| ConfigError::TlsFileNotFound {
        var: "EMBYR_TLS_CERT_PATH".to_string(),
        path: cert_path.to_string(),
    })?;
    let key_pem = std::fs::read(key_path).map_err(|_| ConfigError::TlsFileNotFound {
        var: "EMBYR_TLS_KEY_PATH".to_string(),
        path: key_path.to_string(),
    })?;

    let certs: Vec<rustls::pki_types::CertificateDer<'static>> =
        rustls_pemfile::certs(&mut cert_pem.as_slice())
            .collect::<Result<_, _>>()
            .map_err(|e| ConfigError::TlsInvalidPem {
                var: "EMBYR_TLS_CERT_PATH".to_string(),
                reason: e.to_string(),
            })?;
    if certs.is_empty() {
        return Err(ConfigError::TlsInvalidPem {
            var: "EMBYR_TLS_CERT_PATH".to_string(),
            reason: "no certificate found in PEM file".to_string(),
        });
    }

    let key = rustls_pemfile::private_key(&mut key_pem.as_slice())
        .map_err(|e| ConfigError::TlsInvalidPem {
            var: "EMBYR_TLS_KEY_PATH".to_string(),
            reason: e.to_string(),
        })?
        .ok_or_else(|| ConfigError::TlsInvalidPem {
            var: "EMBYR_TLS_KEY_PATH".to_string(),
            reason: "no private key found in PEM file".to_string(),
        })?;

    // D4: with_no_client_auth() — no mTLS in this feature's scope.
    let rustls_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| ConfigError::TlsInvalidPem {
            var: "EMBYR_TLS_KEY_PATH".to_string(),
            reason: format!("certificate/key mismatch or invalid: {e}"),
        })?;

    Ok(TlsMaterial {
        cert_pem,
        key_pem,
        rustls_config: std::sync::Arc::new(rustls_config),
    })
}
```

### 3. Shared TLS-accept helper (NEW file: `crates/embyr-server/src/adapters/tls.rs`)

Both axum-based listeners (REST/gRPC-Web `:8081`, Admin `:9090`) need the identical
"handshake-if-configured" step. Writing this logic twice would duplicate a security-sensitive
boundary (principle 11's DRY concern, named explicitly in the task brief) — extracted once here,
used by both:

```rust
use tokio::io::{AsyncRead, AsyncWrite};

/// Marker trait unifying a plain `TcpStream` and a `TlsStream<TcpStream>`
/// behind one boxable type, so the accept loop can hold either without a
/// hand-rolled enum. Blanket-implemented for anything already satisfying
/// the bounds hyper's connection builder requires.
pub trait TlsOrPlainStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> TlsOrPlainStream for T {}

/// Complete the TLS handshake on `stream` if `tls_acceptor` is `Some`;
/// otherwise return it unwrapped. The ONE place either listener performs a
/// TLS handshake — shared so this security-sensitive step exists exactly
/// once (principle 11).
pub async fn accept_maybe_tls(
    stream: tokio::net::TcpStream,
    tls_acceptor: Option<&tokio_rustls::TlsAcceptor>,
) -> std::io::Result<Box<dyn TlsOrPlainStream>> {
    match tls_acceptor {
        Some(acceptor) => Ok(Box::new(acceptor.accept(stream).await?)),
        None => Ok(Box::new(stream)),
    }
}
```

`crates/embyr-server/src/adapters/mod.rs` gains `pub mod tls;`.

### 4. gRPC listener (`crates/embyr-server/src/lib.rs::spawn_all_servers`)

```rust
let mut grpc_builder = tonic::transport::Server::builder();
if let Some(tls) = tonic_tls_config {
    // Safe to .expect(): the SAME PEM bytes already passed
    // rustls::ServerConfig::builder()...with_single_cert(...) inside
    // load_tls_material() during from_env() — this cannot fail on content
    // that already-succeeded parse/validate produced.
    grpc_builder = grpc_builder
        .tls_config(tls)
        .expect("tls config already validated in ServerConfig::from_env()");
}
let grpc_fut = grpc_builder
    .add_service(FirestoreServer::new(service))
    .serve_with_incoming_shutdown(grpc_incoming, async {
        let _ = grpc_shutdown_rx.await;
    });
```

### 5. REST/gRPC-Web listener (`crates/embyr-server/src/rest/grpc_web.rs::spawn_hybrid_server`)

`incoming_to_axum_body` visibility widens `fn` → `pub(crate) fn` (reused by the new Admin accept
loop, § 6). `spawn_hybrid_server` gains one parameter and one `.await` between `accept()` and
`TokioIo::new(...)`:

```rust
pub fn spawn_hybrid_server(
    listener: tokio::net::TcpListener,
    grpc_service: FirestoreService,
    axum_app: axum::Router,
    tls_acceptor: Option<tokio_rustls::TlsAcceptor>,
) -> tokio::task::JoinHandle<()> {
    let hybrid = HybridService::new(grpc_service, axum_app);
    tokio::spawn(async move {
        loop {
            let (stream, _peer) = match listener.accept().await {
                Ok(pair) => pair,
                Err(_) => break,
            };
            let svc = hybrid.clone();
            let tls_acceptor = tls_acceptor.clone();
            tokio::spawn(async move {
                let io = match crate::adapters::tls::accept_maybe_tls(stream, tls_acceptor.as_ref()).await {
                    Ok(io) => TokioIo::new(io),
                    Err(_) => return, // handshake failed — drop connection, never fall back plaintext
                };
                let svc = TowerToHyperService::new(svc);
                hyper_util::server::conn::auto::Builder::new(TokioExecutor::new())
                    .serve_connection_with_upgrades(io, svc)
                    .await
                    .ok();
            });
        }
    })
}
```

### 6. Admin listener migration (`crates/embyr-server/src/lib.rs`)

**Decision (resolves the task brief's open design question)**: migrate the Admin listener from
`axum::serve(admin_listener, admin_app)` to the SAME manual-accept-loop shape as
`spawn_hybrid_server`, reusing `accept_maybe_tls` from § 3 — NOT `axum-server` (no new
dependency; `rustls`/`tokio-rustls` already installed and sufficient, per the task brief's own
ladder-rung instruction). New function, single caller (`spawn_all_servers`):

```rust
/// Serve `admin_app` on `listener`, optionally TLS-wrapped. Mirrors
/// `rest::grpc_web::spawn_hybrid_server`'s own accept-loop shape and
/// reuses its EXACT `accept_maybe_tls` handshake step — the Admin listener
/// previously used `axum::serve`, which owns its own accept loop
/// internally and has no TLS hook.
fn spawn_admin_server(
    listener: tokio::net::TcpListener,
    admin_app: axum::Router,
    tls_acceptor: Option<tokio_rustls::TlsAcceptor>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let (stream, _peer) = match listener.accept().await {
                Ok(pair) => pair,
                Err(_) => break,
            };
            let app = admin_app.clone();
            let tls_acceptor = tls_acceptor.clone();
            tokio::spawn(async move {
                let io = match adapters::tls::accept_maybe_tls(stream, tls_acceptor.as_ref()).await {
                    Ok(io) => TokioIo::new(io),
                    Err(_) => return,
                };
                // Reuses grpc_web's own incoming_to_axum_body (now pub(crate))
                // — identical Incoming→axum::body::Body conversion HybridService's
                // non-grpc-web branch already performs.
                let svc = tower::service_fn(move |req: http::Request<hyper::body::Incoming>| {
                    let app = app.clone();
                    async move {
                        Ok::<_, std::convert::Infallible>(
                            tower::ServiceExt::oneshot(
                                app,
                                req.map(rest::grpc_web::incoming_to_axum_body),
                            )
                            .await
                            .unwrap_or_else(|_: std::convert::Infallible| {
                                http::Response::builder().status(500).body(axum::body::Body::empty()).unwrap()
                            }),
                        )
                    }
                });
                hyper_util::server::conn::auto::Builder::new(TokioExecutor::new())
                    .serve_connection_with_upgrades(io, TowerToHyperService::new(svc))
                    .await
                    .ok();
            });
        }
    })
}
```

`spawn_all_servers`'s inner `tokio::select!` arm changes from `_ = admin_fut => {}` to
`_ = spawn_admin_server(admin_listener, admin_app, tls_acceptor.clone()) => {}` (awaiting the
returned `JoinHandle` in place of the old bare `axum::serve` future — same ignored-result shape).

`spawn_all_servers` gains 2 trailing parameters:

```rust
pub fn spawn_all_servers(
    // ...existing 10 params...
    tonic_tls_config: Option<tonic::transport::ServerTlsConfig>,
    tls_acceptor: Option<tokio_rustls::TlsAcceptor>,
) -> tokio::task::JoinHandle<()>
```

All 8 existing test-server constructors' `spawn_all_servers(...)` call sites append `None, None`
(zero behavior change — AC-TLS-01 applied to the test surface, not just production). ONE new
constructor, `start_test_server_with_tls(system_db, tls: TlsMaterial) -> TestServer`, is added
alongside them for DISTILL's own AC-TLS-02/03/04 tests, constructing `tonic_tls_config`/
`tls_acceptor` from the passed `TlsMaterial` the same way `main.rs` does (§ 7) and passing
`Some(...)`.

### 7. `main.rs` wiring

No reordering of the existing 14 steps — Step 1 (`from_env()`) already runs before Step 8 (port
binds), so TLS-config failure already exits before any bind. Immediately before Step 11
(`spawn_all_servers` call), derive the 2 listener-facing types from `cfg.tls`:

```rust
let tonic_tls_config = cfg.tls.as_ref().map(|tls| {
    tonic::transport::ServerTlsConfig::new()
        .identity(tonic::transport::Identity::from_pem(&tls.cert_pem, &tls.key_pem))
});
let tls_acceptor = cfg.tls.as_ref().map(|tls| {
    tokio_rustls::TlsAcceptor::from(std::sync::Arc::clone(&tls.rustls_config))
});
```

...then pass both as the 2 new trailing args to `spawn_all_servers(...)`.

### Earned Trust: the filesystem dependency (principle 12)

The one external dependency this feature introduces is the filesystem (reading operator-provided
cert/key files) — the environment that can lie here is a missing mount, a wrong permission, or
swapped/corrupted file content. `load_tls_material()` (§ 2) IS this dependency's probe: it
performs a REAL, unmocked `std::fs::read` of the REAL configured paths, then feeds the REAL bytes
into `rustls::ServerConfig::builder()...with_single_cert(...)`, which does genuine cryptographic
validation (ASN.1/x509 structural parse AND private-key-matches-certificate check) — not a
syntax-only sniff. This satisfies "wire → probe → use": `from_env()` (wire: read config) →
`load_tls_material()` (probe: prove the filesystem honestly returned parseable, matching PEM
material) → `main.rs` Step 11 (use: thread the already-proven material into all 3 listeners). No
separate `probe()` method is warranted — inventing one would duplicate the exact cryptographic
check `with_single_cert` already performs for real, against real bytes, at real startup time,
matching the project's own established hard-gate-before-bind pattern (`SystemDb::probe()`
precedent in `main.rs` Step 6). A live self-handshake probe (spin up a loopback listener and
complete a real TLS handshake against itself) was considered and rejected as gold-plating beyond
DISCUSS's own ≤2-day estimate: `with_single_cert`'s cryptographic validation already proves
everything a self-handshake would additionally prove for a single-cert, no-client-auth
configuration (no cipher-suite negotiation surface exists that this check wouldn't already have
caught via the same certificate/key material).

### Rejected simpler alternatives (principle 8)

1. **Rely on an external TLS-terminating LB instead of in-process TLS.** Rejected — this is
   precisely the deployment shape (no LB) DISCUSS's own gap targets; not a design alternative,
   the absence of this option IS the gap.
2. **`axum-server` crate for the Admin listener** instead of migrating it to a manual accept
   loop. Rejected per the task brief's explicit ladder-rung instruction: `rustls`/`tokio-rustls`
   are already installed and sufficient; a new dependency for what ~15 lines (reusing
   `accept_maybe_tls`) already does would violate the OSS/dependency-minimalism principle for no
   added correctness.
3. **A single `TlsConfig` enum variant parameterizing `Composite`-style shared state** instead of
   `Option<TlsMaterial>`. Rejected — `Option` already expresses "configured or not" with zero new
   type; introducing an enum for a boolean-shaped decision would be an unrequested abstraction.
4. **Deferred/lazy PEM validation** (only attempted when the first TLS connection arrives)
   instead of eager validation in `from_env()`. Rejected — this is the option AC-TLS-05/06/07
   themselves foreclose: "before any port binds" is only achievable if validation happens at
   config-parse time, matching `validate_encryption_key_hex`'s own established precedent exactly.

## Wave: DESIGN / [REF] Wave Decisions Summary

### Key Decisions
- [D6] Partial TLS config (exactly one of the two vars set) reuses the EXISTING `MissingVars`
  accumulator — no new `ConfigError` variant needed; the absent partner is, in that state,
  literally a missing required variable.
- [D7] File-not-found (`TlsFileNotFound`) and unparseable-PEM (`TlsInvalidPem`) ARE 2 new,
  distinct `ConfigError` variants — D5 (DISCUSS) requires them named differently, and neither
  maps cleanly onto any existing variant's shape.
- [D8] `TlsMaterial` carries BOTH raw PEM bytes (for tonic's `Identity::from_pem`) AND a
  pre-built `Arc<rustls::ServerConfig>` (for the two axum-based listeners' `TlsAcceptor`) — built
  ONCE in `load_tls_material()`, never re-parsed per listener, so all 3 listeners are
  structurally guaranteed to serve the SAME cert/key pair (D2).
- [D9] The Admin listener (`:9090`) migrates from `axum::serve` to a new `spawn_admin_server`
  manual accept loop mirroring `spawn_hybrid_server`'s own shape, sharing the IDENTICAL
  `accept_maybe_tls` handshake helper — avoids duplicating a security-sensitive TLS-handshake
  step across 2 differently-shaped mechanisms (principle 11). `incoming_to_axum_body` widens
  `fn` → `pub(crate) fn` to support this reuse.
- [D10] `rustls-pemfile` promotes from `[dev-dependencies]` to `[dependencies]` — no new crate;
  mirrors this same `Cargo.toml`'s own documented `ed25519-dalek` promotion precedent.
- [D11] The rustls crypto-provider install (`rustls::crypto::ring::default_provider()
  .install_default()`) happens inside `load_tls_material()`, NOT relying on
  `StripeGateway::new()`'s own existing call — that call happens later in `main.rs` (Step 10)
  than TLS material is needed (Step 1/8), so this feature installs it independently. Idempotent
  (`let _ = ...`), matching the existing call's own established shape exactly.

### Constraints Established
- No new external dependency (`rustls-pemfile` promoted, not added; `axum-server` explicitly
  rejected).
- `spawn_all_servers`'s 8 existing test-caller sites pass `None, None` — zero test-suite
  regression from this feature's own signature change.
- No change to any RPC/HTTP route, handler, or application-level type — TLS is a pure
  transport-layer wrap, confirmed by construction (only `config.rs`, `lib.rs`,
  `rest/grpc_web.rs`, and a new `adapters/tls.rs` are touched).
- No ADR file created for this feature — mirrors `firestore-or-filter-support`'s own established
  precedent of embedding architecturally-significant decisions directly in this narrative
  feature-delta.md's own Wave Decisions Summary rather than a separate `adr-*.md`, reserved for
  larger cross-cutting decisions (e.g. ADR-017/018/031).

### External Integration Note
None. This feature has zero external API/vendor SDK surface — no contract-testing annotation
applies (unlike, e.g., the Stripe/AWS/GCP integrations elsewhere in this codebase).

## Wave: DESIGN / Handoff Package

**Every file requiring a code change** (confirmed exhaustive by direct grep — no other file
constructs `ServerConfig {}`, calls `spawn_all_servers`, or calls `spawn_hybrid_server`; see §
Reading Confirmation):

1. `crates/embyr-server/Cargo.toml` — promote `rustls-pemfile` to `[dependencies]`.
2. `crates/embyr-server/src/config.rs` — `TlsMaterial` struct + custom `Debug`, `ServerConfig.tls`
   field, 2 new `ConfigError` variants + their `Display` arms, TLS var resolution in `from_env()`,
   `load_tls_material()`.
3. `crates/embyr-server/src/adapters/tls.rs` (**new file**) — `TlsOrPlainStream` trait +
   blanket impl, `accept_maybe_tls()`.
4. `crates/embyr-server/src/adapters/mod.rs` — add `pub mod tls;`.
5. `crates/embyr-server/src/rest/grpc_web.rs` — `incoming_to_axum_body` visibility widened to
   `pub(crate)`; `spawn_hybrid_server` gains `tls_acceptor` parameter + handshake step.
6. `crates/embyr-server/src/lib.rs` — new `spawn_admin_server` fn; `spawn_all_servers` gains 2
   trailing params + conditional `.tls_config(...)` on the gRPC builder + admin-listener
   migration; all 8 existing test-constructor call sites append `None, None`; 1 new
   `start_test_server_with_tls` constructor added for DISTILL.
7. `crates/embyr-server/src/main.rs` — construct `tonic_tls_config`/`tls_acceptor` from `cfg.tls`
   before Step 11; pass as 2 new trailing args to `spawn_all_servers(...)`.

**Blast-radius confirmation**: grep for `spawn_hybrid_server|spawn_all_servers\(` across the
whole repo returns exactly 10 matches — 3 real Rust source files (all listed above) and 7
documentation/JSON artifacts belonging to the unrelated, already-shipped `production-readiness`
feature (untouched by this feature). Grep for `ServerConfig \{` returns exactly 1 match
(`config.rs`'s own `from_env()`) — no other construction site to break.

**External integrations**: none — no contract-testing annotation required for this handoff.

**Development paradigm** (for DISTILL/DELIVER): functional-where-practical Rust, per this
repo's own root `CLAUDE.md` — `load_tls_material` is a pure-ish `Result`-returning function
(its only side effects are the 2 `std::fs::read` calls and the idempotent crypto-provider
install), `accept_maybe_tls` is the single, explicit, narrowly-scoped IO boundary for the TLS
handshake itself.

## Wave: DESIGN / [REF] Next Wave

**Handoff To**: nw-acceptance-designer (DISTILL wave)
**Deliverables**: this feature-delta.md's DESIGN section (§ Architecture Design, § Wave Decisions
Summary, § Handoff Package); 7 ACs (AC-TLS-01 through 07) to design executable scenarios against;
recommended test-cert-generation approach: reuse the `rcgen` + `rustls::crypto::ring::
default_provider().install_default()` pattern already proven working in `tests/acceptance/
embyr_agent/mod.rs::test_tls_config()` and `tests/acceptance/us_12_agent_backend.rs` (both already
`[dev-dependencies]` of `embyr-server` itself, not just `embyr-agent`'s harness) — no new test
dependency needed.
