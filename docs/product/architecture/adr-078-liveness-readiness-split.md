# ADR-078: Liveness/Readiness Split — `/healthz` Redefined as Readiness, New `/livez` for Liveness

## Status

Accepted

## Context

`production-readiness-audit-2026-09-08.md` finding #15 (High, DevOps/SRE): `/healthz`
(`crates/embyr-server/src/grpc/healthz.rs`) is a hardcoded `200 OK`, mounted identically on the
:8081 (REST/gRPC-Web) and :9090 (Admin) HTTP surfaces. An orchestrator has no way to distinguish
"this process is wedged, restart it" from "this process is fine but cannot currently reach
Postgres, stop routing traffic to it" — both conditions look identical (always 200) today.

An already-tested dependency-check primitive already exists: `SystemDb::probe()`
(`crates/embyr-server/src/adapters/system_db.rs:310-330` — `SELECT 1` + `information_schema`
schema check), used exactly once today, at startup, per ADR-017's own production-startup gate.
This feature's core gap is not "invent a health check" — it is "make an already-correct check
observable on an ongoing basis."

The naive fix — add the Postgres check to the SAME endpoint an orchestrator's liveness probe
polls — creates a restart-storm anti-pattern: a Postgres outage (which a process restart cannot
fix) would cause every pod in the fleet to be killed and restarted simultaneously, destroying
in-flight work and adding a reconnection storm on top of an already-degraded database. This is
the single most commonly cited Kubernetes health-check anti-pattern.

`/healthz` is not a green-field endpoint. Three existing regression tests
(`tests/production_readiness/acceptance/pr01_config_from_env.rs::server_starts_with_all_required_env_vars_set`,
`::non_default_ports_respected`, `pr04_graceful_shutdown.rs`'s own precondition) already poll
`GET :{admin_port}/healthz` expecting HTTP 200 as "the server finished startup and is ready,"
always against a real, reachable Postgres (testcontainers). None of the three assert on response
body content — only the status code.

This decision establishes a repo-wide operational contract that finding #19 (Kubernetes
manifests/Helm chart — separately tracked, out of scope here) will depend on to wire orchestrator
probe configuration correctly. Getting the liveness/readiness assignment wrong at the point a
manifest is written would silently reopen the exact restart-storm anti-pattern this feature exists
to close — the naming/semantics decision needs to be durable, discoverable record, not buried in a
feature-delta doc only. That is why this gets a new ADR rather than a plain extension note (the
recent pattern for smaller findings, e.g. #14 `preauth-db-amplification`, which touched no new
architectural contract and got none).

## Decision

**`/healthz` is redefined to mean readiness** (backed by a real `SystemDb::probe()` call against
the shared system Postgres pool). **A new `/livez` endpoint is added for liveness** (zero I/O,
returns 200 unconditionally whenever the process can handle an HTTP request at all — literally
today's old hardcoded handler body, unchanged).

| Endpoint | Meaning | Depends on Postgres | Orchestrator remediation on failure |
|---|---|---|---|
| `/healthz` (redefined) | Readiness — can this pod usefully serve traffic right now | Yes — `SystemDb::probe()`, shared system pool only | Stop routing traffic; do not restart |
| `/livez` (new) | Liveness — is the process itself alive/responsive | No — zero external calls | Restart the pod |

Both endpoints are mounted on **both** existing HTTP surfaces (:8081 REST/gRPC-Web, :9090 Admin),
mirroring `/healthz`'s own existing dual-mount today — no new listener, no new port.

### Alternatives considered

1. **Conventional K8s-doc naming: `/healthz` = liveness (unchanged), new `/readyz` = readiness.**
   Rejected as the primary choice because it silently breaks the *meaning* three existing
   regression tests already assume of `/healthz` (a startup-readiness gate) even though it would
   not break their *assertions* (status-code-only). Preserving `/healthz`'s already-established,
   3-tests-deep in-repo meaning is the smaller, lower-risk change, and both options satisfy this
   feature's ACs identically since the ACs are written at the observable-outcome level, not the
   URL path (DISCUSS OQ-HDC-01). Documented here specifically so a future K8s manifest author
   (finding #19) does not default to the "obvious" convention and get it backwards.
2. **Single endpoint returning a tri-state body (`ok` / `degraded` / `down`), one orchestrator
   probe parses the body.** Rejected: most orchestrators (Kubernetes included) make binary
   restart/no-restart and route/no-route decisions off HTTP status code alone, not response body
   parsing; a single endpoint also structurally reopens the exact conflation this ADR exists to
   prevent — liveness and readiness would share one failure signal again.
3. **Do nothing to `/healthz`'s own meaning; add both `/livez` and `/readyz` as new endpoints,
   leaving `/healthz` as a third, now-redundant hardcoded `200 OK`.** Rejected: three permanently
   redundant endpoints (one dead) is needless surface area for zero benefit, and leaves the
   already-tested `/healthz`-as-readiness expectation encoded in three regression tests
   permanently disconnected from a real check — the opposite of this feature's purpose.

### Probe reuse, caching, and timeout (OQ-HDC-02/03)

- **No caching.** `/healthz` calls `SystemDb::probe()` fresh on every request. `probe()`'s own
  cost is one indexed `SELECT 1` plus one small `information_schema.tables` lookup — negligible
  against a shared pool already sized for `auth`/`admin`/rate-limiting traffic. Caching was
  rejected: a cache TTL long enough to matter would risk delaying detection of BOTH failure
  (AC-HDC-07's "unhealthy within one probe interval") and recovery (AC-HDC-07's "healthy within
  one probe interval of Postgres becoming reachable again") past the bound the story requires,
  for a cost saving that does not exist at the orchestrator's typical polling cadence
  (`periodSeconds: 10` in Kubernetes' own default).
- **Explicit 3-second timeout at the call site**, via `tokio::time::timeout` (already an
  established pattern in this codebase — `middleware/rate_limit.rs`,
  `middleware/signin_rate_limit.rs`, `adapters/postgres_notify_listener.rs`). Applied by the
  `/healthz` handler wrapping its call to `system_db.probe()`, **not** inside `SystemDb::probe()`
  itself — `probe()` is also called once at startup (ADR-017's gate) with its own already-tested,
  unrelated failure semantics (bounded by the pool's own 5s `acquire_timeout`, set in
  `SystemDb::new()`); changing `probe()`'s own behavior risks that unrelated caller. A hung (not
  merely down) Postgres connection past the pool-acquire step has no query-level timeout today —
  the 3s call-site timeout ensures `/healthz` reports unhealthy promptly rather than hanging past
  whatever timeout the orchestrator itself enforces. `docs/feature/healthz-dependency-checks/`'s
  own DESIGN section documents that an eventual K8s manifest (finding #19) should set
  `timeoutSeconds` comfortably above 3s for the readiness probe.
- Timeout elapsing is treated identically to `probe()` returning `Err` — readiness reports
  unhealthy (503), the raw error/timeout detail is never included in the HTTP response body (see
  Consequences — this is a new leak surface distinct from, and not already covered by, ADR-075).

## Consequences

**Positive:**
- Closes finding #15 with zero new Cargo dependency, zero schema change, zero `embyr-core` edit —
  reuses an already-implemented, already-tested primitive exactly per the Earned Trust principle of
  preferring a real, empirically-proven check over inventing a new one.
- Restart-storm anti-pattern is structurally prevented, not just avoided by convention: `/livez`'s
  handler has no code path that can reach Postgres, so a future regression cannot silently
  reintroduce the coupling without an explicit, reviewable code change to that handler.
- Three existing regression tests continue to pass unchanged (status-code-only assertions, always
  against real reachable Postgres).
- Establishes the exact endpoint contract finding #19's eventual K8s manifest must wire
  (`livenessProbe` → `/livez`, `readinessProbe` → `/healthz`), removing ambiguity for that future
  work.

**Negative / accepted trade-offs:**
- `/healthz`'s URL no longer matches the conventional Kubernetes-docs pairing
  (`/healthz`=liveness, `/readyz`=readiness) — anyone wiring a manifest from muscle memory must
  consult this ADR or the feature's own docs rather than assume the common convention. Mitigated
  by explicit documentation here and in the feature-delta handoff to platform-architect.
- The readiness handler is a **new, distinct leak-surface boundary** for `CoreError::BackendUnavailable`'s
  `Display` text (raw driver/schema error strings) — this is an HTTP/axum conversion boundary, not
  a `tonic::Status` conversion boundary, so it was NOT covered by ADR-075's 26-site sanitization
  sweep (that sweep was `Status::internal(` sites only). The handler must return a fixed, generic
  unhealthy body (e.g. `{"status":"unhealthy"}`) with no embedded error text — same fixed-string
  principle as ADR-075, applied at a new boundary.

## Enforcement

- The 3 existing regression tests (`pr01_config_from_env.rs` x2, `pr04_graceful_shutdown.rs`)
  remain the regression guard for "readiness still reports healthy when Postgres is genuinely
  reachable" — no new test infrastructure needed for that guard.
- A new walking-skeleton integration test (DISTILL wave) using the already-established
  testcontainers `stop()`/`start()` outage mechanism (`pr08_realtime_listener_reconnect.rs`) proves
  the split empirically: `/livez` stays healthy and `/healthz` turns unhealthy-then-recovers across
  a real Postgres outage, polled on both :8081 and :9090.
- Recommend a `#[test]` (or clippy/grep-based lint, DISTILL's call) asserting `/livez`'s handler
  module has zero imports of `SystemDb`/`sqlx`/any adapter — a cheap structural guard against a
  future PR silently wiring a dependency into liveness (mirrors this codebase's existing
  `dependency-cruiser`-style intent for `embyr-core`, applied here at the function level since Rust
  has no workspace-wide import-linter equivalent already installed).
