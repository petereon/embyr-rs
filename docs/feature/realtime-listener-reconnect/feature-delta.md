# Feature Delta: realtime-listener-reconnect

## Wave: DISCUSS / [REF] Prior Wave Consultation — Reading Confirmation

✓ `docs/product/production-readiness-audit-2026-09-08.md` row 4 confirmed by direct reading, exact
wording: "Real-time LISTEN/NOTIFY has no reconnect logic and the dead-listener map entry is never
evicted — one transient Postgres blip permanently and silently kills realtime delivery for that
project for the life of the process, while its dedicated connection pool leaks forever. Confirmed
independently by 2 agents (SRE + database)." Location cited:
`crates/embyr-server/src/adapters/postgres_notify_listener.rs:57-72`;
`crates/embyr-server/src/grpc/handler.rs:3547-3565`. Severity: **Blocker**. Status: **Not started**
(this is the first DISCUSS against it).
✓ `crates/embyr-server/src/adapters/postgres_notify_listener.rs` read in full (169 lines). Confirmed
exactly: `PostgresNotifyListener::start()` (lines 40-75) opens one dedicated `PgListener::connect(dsn)`
connection (line 49, NOT pooled — matches AD-08's own "dedicated Postgres connection per project for
NOTIFY" decision, see § Business Context), calls `.listen(&channel)`, then spawns a task
(`tokio::spawn`, lines 57-74) that loops on `pg_listener.recv().await`. On `Ok`, it fetches the
changed document and calls `registry.fan_out(...)` (lines 60-65). On `Err(e)` (line 66-69): logs
`tracing::warn!` and `break`s — the loop, and the task, end there. No retry, no reconnect attempt, no
backoff. `Drop for PostgresNotifyListener` (lines 26-30) aborts `_task` via `JoinHandle::abort()` — but
this only ever runs if the **struct itself** is dropped, which requires removal from
`active_listeners`.
✓ `crates/embyr-server/src/grpc/handler.rs` read in full around `handle_listen` (lines 3514-3620+).
Confirmed exactly: the listener-provisioning guard (lines 3544-3567) takes `self.active_listeners.lock()`,
checks `!listeners.contains_key(&project_id)`, and ONLY THEN builds a fresh
`sqlx::postgres::PgPoolOptions::new().max_connections(2).connect(&dsn)` pool and calls
`PostgresNotifyListener::start(...)`, inserting the result via `listeners.insert(project_id.clone(),
listener)` (line 3565). There is no `else` branch, no liveness check on the existing entry — `contains_key
== true` is treated as "a working listener already exists," unconditionally.
✓ **Grep-confirmed independently** (`rg 'active_listeners' crates/`, `rg '\.remove\(' crates/embyr-server/src/grpc/handler.rs crates/embyr-server/src/adapters/postgres_notify_listener.rs`):
zero `.remove()` call sites against `active_listeners` anywhere in the workspace. The map is
write-only-by-insert for the entire life of the process — the audit's own "never evicted" claim is
confirmed precisely, not assumed.
✓ **`ListenRegistry::fan_out` read in full** (`crates/embyr-server/src/realtime/listen_registry.rs:91-103`):
if a channel has no registered subscribers (or, by extension of this bug, if nothing is fanning
anything out to that channel at all because the listener task died), `fan_out` is simply never called
for that project again — there is no error path, no exception, nothing surfaces. This confirms the
"silently" half of the audit's claim: a dead listener produces zero client-visible signal of any kind.
✓ **Independent verification / correction to the task's own framing** (§2 of the pool-leak claim): the
task's summary states the connection pool "leaks forever... since `PostgresNotifyListener`'s own
`Drop` impl... never runs." Direct reading shows this needs one refinement: `PostgresNotifyListener`
itself holds only the `JoinHandle` (line 23); the dedicated `PgListener` connection and the 2-connection
`backend_pool` are both moved (`async move`) into the **spawned task's own closure** (lines 57-74), not
held by the struct. When the task's loop hits `break` and the async block ends *normally* (not via
`abort()`), ordinary Rust/Tokio drop semantics DO run on that closure's captured state at that point —
so the specific claim "the pool leaks because `Drop` never runs" is not quite the precise mechanism;
whether sqlx's `PgPool`/`PgListener` connection teardown on ordinary drop is prompt or eventually-consistent
is an implementation detail this DISCUSS did not need to resolve to establish the requirement (see next
bullet). What IS confirmed, unambiguous, and does not depend on that detail: because the map entry is
never evicted, **no new pool or listener is EVER created again for that project**, for the remaining
life of the process — this is the actual, structurally-guaranteed permanent-death mechanism, and it
alone is sufficient justification for "a dead listener's resources must be recoverable," independent of
the exact timing of the old pool's own connection teardown. This refinement is flagged for DESIGN, not
resolved here — DESIGN should confirm sqlx's actual drop-time connection-close behavior only if the
chosen mechanism's design depends on it.
✓ `docs/product/architecture/brief.md` read (Quality Attributes table and Architecture Decisions table).
Confirmed AD-08: "Dedicated Postgres connection per project for NOTIFY — Prevents pool contention;
LISTEN requires a dedicated connection lifecycle" — this feature must preserve that decision (still one
dedicated connection per project), not fold NOTIFY into the shared connection pool. Confirmed Quality
Attribute #4: "Real-time latency — KPI: write → `onSnapshot` callback p99 ≤ 2s (US-05, AC-05c). Drives
the LISTEN/NOTIFY architecture choice over polling" — the non-failure-path latency contract this
feature must not regress.
✓ `docs/product/jobs.yaml` read in full (JOB-01 through JOB-20). See § Persona & Job for the reasoning
between the candidates evaluated (JOB-03, JOB-08, JOB-11, JOB-12, JOB-13). JOB-08's own dimension text
already documents a reconnect precedent for a *different* code path (agent-mode Subscribe stream):
"embyr SaaS fans out to Listen targets; overflow triggers RESET; SaaS reconnects with exponential
backoff 1s→30s" — named below as a reference precedent, not this feature's own mechanism.
✓ `docs/evolution/2026-05-27-embyr-rs.md` read (Architecture Decisions and Feature Backlog tables).
Confirmed step 05-02 ("Postgres LISTEN/NOTIFY fan-out for live updates") is the original walking-skeleton
slice that shipped `PostgresNotifyListener`/`active_listeners` as designed — this feature closes a
reliability gap in that original design, it does not redesign it.
✓ Searched `docs/evolution/` for "realtime"/"LISTEN"/"NOTIFY"/"listener" (case-insensitive) — no
evolution doc dedicated to real-time reconnect/lifecycle exists; the closest prior architectural context
is `docs/product/architecture/adr-033-listen-compliance-composition-and-collection-scoping.md`
(referenced directly in `postgres_notify_listener.rs`'s own doc comments for `fetch_event`, but scoped
to access-control/compliance semantics for delete events, not reconnect/lifecycle — read, confirmed
orthogonal, not re-litigated here).
✓ `docs/feature/rate-limiter-project-id-validation/feature-delta.md` and
`docs/feature/stripe-webhook-body-limit/feature-delta.md` read in full to confirm this project's own
established single-file, Tier-1-only `feature-delta.md` convention (`## Wave: DISCUSS / [REF] {Section}`
headings, no standalone `acceptance-criteria.md`/`outcome-kpis.md`/`story-map.md` files) and the
"attacker persona 'Marcus Webb' for narrative continuity" pattern for audit-derived findings whose harm
requires an attacker. **This feature does not reuse Marcus Webb** — findings #1-#3 were unauthenticated
security exploits; this finding (#4) requires no attacker at all, only ordinary infrastructure flakiness
(a routine Postgres restart, a brief network blip) — a materially different threat shape. This DISCUSS
uses Sam Chen and a named customer project (Trailmark, `trailmark-prod`) instead, consistent with this
project's own established use of "Trailmark" as a `direct_pg`-mode customer elsewhere in `jobs.yaml`
(JOB-16's own NOTE).

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

- Feature type: **Reliability fix** (an ordinary, non-adversarial infrastructure-recovery gap), not a
  security exploit and not an SDK-developer-facing new capability.
- JTBD: **reuse JOB-03** (`live-sync`, P1 Alex) as the job whose functional promise is broken, with P2
  Sam Chen named as the story's own primary "Who" (the operational persona who detects, is harmed by,
  and must recover from this specific failure mode) — see § Persona & Job for the full reasoning,
  including why this is NOT a JOB-11/12/13 extension despite the audit-finding shape matching those
  three siblings.
- Decision 4 (full JTBD path vs. infrastructure-only): **Yes — full JTBD path.** This directly breaks a
  named, load-bearing functional promise (JOB-03's own "onSnapshot fires for every committed write") for
  real, paying customers — not infrastructure-only scaffolding.
- Walking Skeleton: **single story**, sized at the upper edge of "right-sized" (7 UAT scenarios) rather
  than split — see § Scope Assessment for why the four requirement facets (recover / clean up / bound
  retries / surface persistent failure) are sub-parts of one outcome, not four independent ones.
- UX Research Depth: **Lightweight** — an operator-facing backend reliability fix for a long-running
  background task, not an end-user journey; no ASCII TUI mockups or emotional-arc journey YAML
  warranted (matches both sibling audit fixes' own precedent).
- **The exact reconnect mechanism is this DISCUSS's own central open question, explicitly flagged as an
  opening recommendation for DESIGN to finalize** — per the task's own framing, DISCUSS locks the
  user-facing OUTCOME only (see § Central Design Question).

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: **P2 Sam Chen (Service Operator / Platform Engineer)** — the story's primary "Who." This
finding was surfaced by the same class of production-readiness scan as findings #1-#3 ("Confirmed
independently by 2 agents (SRE + database)" — an SRE and a DBA, both squarely Sam Chen's own
worldview), against operational infrastructure Sam Chen alone deploys, monitors, and would otherwise
have to manually recover (today, only by restarting the entire multi-tenant `embyr-server` process).

**Job**: **JOB-03 `live-sync`** (persona P1 Alex per its own job record), reused and EXTENDED to cover
continuous, failure-resilient delivery — not merely initial-connection delivery. Candidates considered
and rejected:
- **JOB-11 `fair-multitenancy`** — wrong fit. This finding has nothing to do with per-project
  rate-limit fairness across a horizontally-scaled cluster; the rate limiter and its token-bucket logic
  are entirely untouched by this bug.
- **JOB-12 `observability`** — wrong fit as the PRIMARY job, despite genuine overlap (this feature's own
  AC-4 below requires an operator-visible signal for persistent failure, which will likely reuse JOB-12's
  existing `/metrics`/log surface as its *mechanism*). JOB-12's own functional dimension is specifically
  about the Prometheus `/metrics` endpoint's own counters/histograms; it does not name real-time
  delivery continuity at all. Reusing JOB-12 as the primary job would misrepresent the core broken
  promise (JOB-03's own, not JOB-12's).
- **JOB-13 `production-deployment`** — the task's own suggested "production system must recover from
  transient infra blips" framing is intuitive but, on direct comparison against JOB-13's own job story
  and all three of its existing extensions (`stripe-webhook-secret-required`, `firestore-tls-support`,
  `stripe-webhook-body-limit`), every one of those is a **startup-time or request-admission-time**
  config-safety/resource-safety gap ("server exits non-zero with named missing var," "no cryptic
  failures" at boot, a request rejected before being fully buffered). This finding is neither: it is a
  **long-running background task's own mid-life failure-recovery gap**, a different shape than anything
  JOB-13 has been extended to cover so far. Forcing it into JOB-13 would blur that job's own established
  "safe to start, safe to admit a request" boundary into "safe to keep running indefinitely," diluting
  its own functional dimension rather than genuinely fitting it.
- **JOB-08 `agent-livesync`** — wrong fit, and explicitly NOT the same code path. JOB-08's own
  functional dimension ("Agent maintains Postgres LISTEN connection; pushes DocChange over Subscribe
  stream... SaaS reconnects with exponential backoff 1s→30s") is about `backend_mode=agent`'s own
  Subscribe RPC forwarding mechanism — a structurally different transport (agent-to-SaaS gRPC stream)
  from this feature's target (`PostgresNotifyListener`, SaaS-to-customer-Postgres LISTEN, used by
  `direct_pg`/`aws_secret`/`gcp_secret` modes only). JOB-08's own "SaaS reconnects... 1s→30s" text is
  cited below purely as a **reference precedent** for backoff shape, not as the job this feature belongs
  to.
- **JOB-03 `live-sync`** (the correct fit) — this job's own functional dimension names the EXACT broken
  promise verbatim: `"onSnapshot fires for every committed write; resume tokens allow reconnect."` A
  listener that permanently and silently dies after one transient blip means `onSnapshot` stops firing
  for every committed write for that project, for the rest of the process's life — a direct, unambiguous
  violation of JOB-03's own already-documented functional dimension, exactly as literal a textual match
  as the one `rate-limiter-project-id-validation` used to justify reusing JOB-12 for its own metric name.
  JOB-03's own emotional dimension ("Feel confident that users see a consistent view") and social
  dimension ("Deliver a responsive collaborative UX that rivals Google Firestore") are both directly
  undermined for Alex's end users when this silently breaks — real Firestore does not permanently die on
  a transient database blip.
  This is JOB-03's **first** documented extension; it is also its first extension whose primary
  operational stakeholder (Sam Chen, P2) differs from the job's own recorded primary persona (Alex, P1)
  — mirroring the established `persona`/`secondary_persona` dual-stakeholder pattern already used
  elsewhere in `jobs.yaml` (e.g. JOB-08 itself: `persona: P1, secondary_persona: P4`). Recording a
  `secondary_persona: P2` NOTE against JOB-03 is recommended at FINALIZE (see § Wave Decisions Summary,
  Upstream Changes) — not applied to `jobs.yaml` in this DISCUSS, since this feature-delta.md is this
  wave's only requested deliverable.

## Wave: DISCUSS / [REF] Business Context

Today, `PostgresNotifyListener::start()` (`postgres_notify_listener.rs:40-75`) opens one dedicated
`PgListener` connection per project (AD-08's own architecture decision — never pooled, since `LISTEN`
requires a persistent, single-connection session) and spawns a background task that loops on
`pg_listener.recv().await`. `sqlx::Error` does not distinguish a transient network blip, a Postgres
restart, or a permanently invalid DSN at this call site — all three surface as the same `Err` variant.
On ANY error, the task logs one `tracing::warn!` and `break`s, ending itself. `handle_listen`
(`handler.rs:3544-3567`) guards listener creation with `contains_key(&project_id)` only — once a project
has ever had a listener, no code path ever re-examines whether that listener's background task is still
alive. Because no `.remove()` call exists anywhere against `active_listeners` (grep-confirmed), this is
permanent: **one transient blip converts a project's real-time delivery into a silent, permanent outage
for the remaining life of the `embyr-server` process**, recoverable today only by restarting the entire
process — an action that also drops every OTHER tenant's active connections, including their own
perfectly healthy listeners.

This is a materially different, larger-surface-area problem than the three prior audit fixes this
session closed (findings #1-#3, each a narrow input-validation/resource-limit fix in a single,
short-lived request-handling path). This finding is about the **failure-recovery lifecycle of a
long-running background task** — a different design category requiring an actual reconnect/backoff
mechanism and lifecycle-management decision, not a bounds check. This DISCUSS does not force it into the
narrower shape of its predecessors, but it also does not expand scope to redesigning the wider real-time
subsystem (`ListenRegistry`, resume-token/RESET semantics, the Listen RPC's own streaming shape are all
unaffected and untouched — see § Out of Scope).

### Central Design Question — Reconnect Architecture (Opening Recommendation for DESIGN)

The task names this as the CENTRAL design question and is explicit that DISCUSS should lock only the
user-facing outcome (invisible to the AC which approach is chosen), while flagging an opening
recommendation. Two candidate shapes:

- **(a) Internal reconnect** — the background task itself never truly dies on a transient error; its
  loop body wraps the existing `PgListener::connect` + `.listen()` + `recv()` sequence in a
  reconnect-with-backoff retry, so the task keeps running (and the struct in `active_listeners` remains
  a valid, live invariant) across any number of transient blips. The task only ever ends via explicit
  external cancellation (i.e., the existing `Drop`/`abort()` path, today unreachable because nothing
  removes the map entry — this feature would need to add a real removal trigger, e.g. project suspension
  or deletion, as a SEPARATE concern from reconnect itself).
- **(b) Die clean + external detection** — the task still ends via `break` on error (as today), but
  something else — a periodic health check, or `handle_listen`'s own guard — detects that the stored
  `PostgresNotifyListener`'s task has finished (e.g. via `JoinHandle::is_finished()`) and evicts +
  recreates it, either proactively or lazily on the next `handle_listen` call for that project.

**Opening recommendation: (a).** Reasoning, offered as DESIGN's starting point, not a locked decision:
- `PostgresNotifyListener`'s own `Drop` impl already encodes an implicit invariant: *"this struct's
  presence in `active_listeners` means its task is alive and forwarding."* That invariant is exactly
  what today's bug violates (the task can die while the struct silently lives on). Approach (a) restores
  the invariant directly — as long as the struct exists, its task is alive, retrying internally through
  any number of blips — with **zero change** to `handle_listen`'s own `contains_key` guard or to the
  `Drop` impl itself. Approach (b) requires WIDENING that guard everywhere it's read (today: 1 call
  site) to also check liveness, and introduces a query-then-act race between the liveness check and use
  that (a) does not have.
- Approach (a) is a strictly smaller, more contained diff: it touches only the loop body already inside
  `PostgresNotifyListener::start`'s spawned task (lines 57-72) — the exact lines the audit's own citation
  names. Approach (b) touches both that file AND the shared `handle_listen` guard.
- Approach (a) naturally satisfies the pool-cleanup requirement without inventing new cleanup machinery:
  since the task never dies on a transient failure, the SAME dedicated connection and pool are reused
  across reconnects — there is no "old" pool to leak, clean up, or replace, because nothing is ever torn
  down and rebuilt for a merely-transient failure. Approach (b) would need explicit teardown-then-rebuild
  logic (new pool, new `PgListener::connect`) on every recreate cycle.
- This recommendation does not resolve backoff shape, the persistent-failure signal, or whether a
  never-provisioned/deleted project's listener should ever be evicted (a genuinely separate lifecycle
  question from reconnect) — all three remain open for DESIGN regardless of which shape is chosen.

### Bounding Reconnect Attempts (Backoff)

Sustained Postgres unavailability must not turn "keep trying" into "hammer Postgres in a tight loop."
JOB-08's own already-shipped dimension text for the Subscribe-stream reconnect case ("SaaS reconnects
with exponential backoff 1s→30s") is named here as a reference precedent for consistency across this
codebase's two independent reconnect mechanisms — **not** mandated verbatim for this feature; DESIGN
may choose a different curve if warranted, but should explain any material divergence from this
codebase's own existing precedent rather than inventing an unrelated shape from nothing.

### Permanent vs. Transient Failure

`sqlx::Error` cannot reliably distinguish "Postgres will be back in 5 seconds" from "this DSN is
permanently invalid because the project was deleted" at this call site (confirmed fact, not assumed —
both surface as the identical `Err` variant today). Given that, this DISCUSS locks a narrower,
achievable outcome rather than inventing a fragile transient-vs-permanent classifier:

- The system is **not required** to ever fully stop retrying and declare a listener permanently dead —
  building an unreliable heuristic to do so risks a worse failure mode (falsely declaring a real,
  eventually-recoverable project dead) than the one being fixed.
- What IS required: sustained failure beyond a bounded, ordinary transient-blip window must become
  **operator-visible** and **distinguishable from a healthy, occasionally-blipping listener** — Sam Chen
  must never be in the position of "the metric/log looks identical whether this project's real-time
  delivery is fine or has been down for six hours." The exact mechanism (an elevated log level after N
  consecutive failures, a JOB-12-style `/metrics` counter/gauge, or another approach) is DESIGN's own
  choice, informed by JOB-12's existing observability surface.
- This directly satisfies the task's own requirement ("must not silently swallow a PERMANENT failure...
  into an infinite retry loop that never surfaces anything") without requiring the system to correctly
  classify permanent vs. transient, which this DISCUSS explicitly could not evidence a reliable mechanism
  for at this layer.

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

**Signals checked**: >10 user stories? No (1). >3 bounded contexts/modules? No — confined to
`embyr-server`'s own realtime adapter (`postgres_notify_listener.rs`) and its one call site in
`handler.rs`'s `handle_listen`; `ListenRegistry`/`fan_out` is read for context (confirmed unaffected) but
not modified. Walking skeleton >5 integration points? Borderline but no (5): (1) a real transient
connection drop against a real Postgres instance, confirming automatic recovery; (2) a real repeated-blip
sequence, confirming no task/pool accumulation; (3) a real sustained-outage window, confirming capped
backoff (no tight-loop hammering); (4) confirming an operator-visible signal appears after sustained
failure; (5) a real, never-failing listener, confirming zero behavior change on the happy path — all
against the same running server instance and a real customer project (`trailmark-prod`). Estimated
effort >2 weeks? No, but larger than this session's prior 3 audit fixes — this requires genuine
reconnect/backoff logic and a lifecycle-semantics decision (not a bounds check), estimated 2-3 days, not
1. Multiple independent user outcomes? No — "a transient failure does not permanently and silently kill
real-time delivery, and does not leak resources or storm Postgres while recovering" is ONE outcome; the
four requirement facets (recover / clean up / bound retries / surface persistent failure) are all
necessary conditions of that single outcome holding, not separable, independently shippable outcomes —
splitting them would produce stories that are each individually meaningless without the others (e.g.
"backoff without recovery" delivers nothing demonstrable).

**Scope Assessment: PASS** — 1 user story, 1 bounded context (`embyr-server` realtime adapter +
its one `handle_listen` call site), estimated 2-3 days, 7 UAT scenarios (at the upper edge of, but
within, the 3-7 right-sized range).

## Wave: DISCUSS / [REF] System Constraints

- AD-08 (dedicated Postgres connection per project for NOTIFY) is preserved — the fix must not fold
  LISTEN onto a shared/pooled connection; each project's listener remains a dedicated connection.
- The real-time latency KPI (write → `onSnapshot` p99 ≤ 2s, US-05/AC-05c) must be unaffected for a
  listener that never experiences a failure — this feature changes only the failure-recovery path, not
  the steady-state fan-out path (`fan_out`/`ListenRegistry` are untouched).
- `PostgresNotifyListener`'s public `start()` signature and its `Drop`-aborts-task lifecycle contract
  should not be broken for existing callers unless DESIGN's chosen mechanism requires it — confirmed only
  one call site exists (`handler.rs:3557-3564`).
- No behavior change to resume-token/RESET semantics, initial-snapshot delivery, or the Listen RPC's own
  streaming shape (`listen_handler.rs`) — this feature is scoped to the LISTEN/NOTIFY background task's
  own failure-recovery lifecycle, not the client-facing streaming protocol.
- `backend_mode=agent` is out of scope — its own real-time forwarding uses the Subscribe-stream mechanism
  (JOB-08), a structurally different code path from `PostgresNotifyListener`. DESIGN should confirm via a
  quick read whether `handle_listen`'s own listener-provisioning block is even reached for
  `backend_mode=agent` projects (unconfirmed by this DISCUSS — the `dsn` value's shape for agent-mode
  projects was not traced end-to-end).
- No new external dependency — reconnect/backoff logic can be built from `tokio::time::sleep` and the
  already-present `sqlx`/`tracing` primitives already used in this same file.

## Wave: DISCUSS / [REF] User Stories

### US-01: A Transient Postgres Blip No Longer Permanently and Silently Kills a Project's Real-Time Delivery

**job_id**: JOB-03

#### Elevator Pitch
**Before**: Sam Chen operates a production embyr deployment serving multiple customers, including
Trailmark (`trailmark-prod`, `backend_mode=direct_pg`), whose app relies on `onSnapshot` for a live
dashboard. A routine, several-second Postgres blip — a minor-version patch, a brief network hiccup,
nothing catastrophic — causes `PostgresNotifyListener`'s background task to hit `pg_listener.recv()`'s
error arm, log one easy-to-miss `tracing::warn!` line, and silently end forever. The `active_listeners`
map entry for `trailmark-prod` is never removed (confirmed: no `.remove()` call exists anywhere in the
workspace), so every subsequent `Listen` RPC for `trailmark-prod` sees `contains_key == true` and skips
creating a new listener. Trailmark's dashboard silently stops updating in real time — no error reaches
the SDK, nothing distinguishes this from healthy operation beyond one line in a log Sam Chen isn't
watching every second of — and this persists for the remaining life of the process. Sam Chen only learns
about it from a Trailmark support ticket, days later, and the only fix available today is restarting the
entire `embyr-server` process — taking down every OTHER tenant's active connections just to recover one
project's dead listener.
**After**: The same brief Postgres blip against `trailmark-prod` causes, at most, a bounded, short pause
in real-time delivery, followed by automatic resumption — zero client action, zero operator action, zero
process restart. If Postgres is down for an extended period, reconnect attempts back off instead of
hammering it, and if the outage persists well beyond a normal transient blip, Sam Chen gets an
operator-visible signal distinguishing "still retrying" from "silently, invisibly dead" — he is never
blind to a project stuck in a bad state, and he never needs to restart the whole process to recover one
tenant.
**Decision enabled**: Sam Chen can trust that a single transient Postgres hiccup against any one customer
project never permanently kills that project's real-time delivery or leaves its connection resources
unrecoverable for the life of the process — he does not need to proactively poll every project's listener
health himself, and he does not need to restart the entire multi-tenant `embyr-server` process (impacting
every other tenant) just to recover one project's dead listener.

#### Who
- Sam Chen (P2) | Service operator running `embyr-server` in production, serving multiple customer
  projects (e.g., Trailmark on `backend_mode=direct_pg`) over LISTEN/NOTIFY-backed real-time delivery |
  Relies on JOB-03's own promised "onSnapshot fires for every committed write" guarantee holding
  continuously, not merely at first connection | Needs a single tenant's transient infrastructure blip to
  never become a silent, permanent, whole-process-restart-requiring outage.

#### Solution
Make a project's real-time listener recover automatically from a transient connection failure, ensure a
listener that is no longer serving traffic does not permanently block a fresh one from being established
(closing the resource-leak/permanent-death gap), bound the rate of reconnect attempts during a sustained
outage, and surface a distinguishable operator signal when a listener has been failing to reconnect
beyond a normal transient window — without changing behavior for a listener that never fails. The exact
mechanism (internal reconnect-with-backoff vs. external death-detection-and-recreate; see § Central
Design Question) is DESIGN's decision, not fixed here.

#### Domain Examples

**Example 1 (Happy Path — a real customer's listener self-heals)**: Trailmark's project (`trailmark-prod`,
`backend_mode=direct_pg`) has an active `onSnapshot` listener behind Alex's live dashboard. Postgres
briefly restarts for a routine minor-version patch (a few seconds). Before this feature: Trailmark's
real-time updates die permanently until Sam Chen notices — typically from a support ticket — and restarts
`embyr-server`, an action that also drops every OTHER tenant's connections. After this feature:
Trailmark's listener automatically reconnects within a bounded window and `DocumentChange` delivery
resumes with no client action and no process restart.

**Example 2 (Edge Case — repeated blips do not accumulate dangling resources)**: A flapping network link
causes `trailmark-prod`'s Postgres connection to drop and recover 20 times over the course of an hour.
Before this feature: only the FIRST blip matters — the listener dies for good on drop #1 and every
subsequent drop is moot because there is nothing left to drop. After this feature: each of the 20
reconnect cycles leaves exactly one active background task and one active dedicated connection/pool for
`trailmark-prod` at any given time — no accumulation of 20 dangling tasks or pools from the 20 blips.

**Example 3 (Error/Boundary — sustained outage, bounded retries, and visibility)**: Trailmark's own
Postgres instance is down for 45 minutes for a planned resize. Before this feature: this is moot too —
the listener already died permanently on the very first failure with zero further attempts, so there is
no "storm" today, only total silence for the rest of the process's life. After this feature: the listener
retries with capped backoff throughout the 45-minute window without hammering Postgres, and once the
outage has continued well beyond a normal transient blip, Sam Chen sees an operator-visible signal
distinguishing this from a healthy listener — he can proactively investigate rather than waiting for a
customer complaint. When Postgres comes back, the listener reconnects and delivery resumes automatically
with zero manual steps.

#### UAT Scenarios (BDD)

```gherkin
Scenario: A transient Postgres blip does not permanently end real-time delivery
  Given Trailmark's project "trailmark-prod" has an active onSnapshot listener receiving live updates
  When the underlying Postgres connection for that listener drops for a few seconds and then recovers
  Then the listener automatically resumes delivering DocumentChange events for "trailmark-prod"
  And no new Listen RPC call, client action, or operator action was required to resume delivery

Scenario: Repeated transient blips do not accumulate dangling connections or tasks
  Given Trailmark's project "trailmark-prod" experiences 20 short connection drops over one hour
  When each drop resolves and the listener recovers
  Then at most one active background task and one active dedicated connection/pool exist for
    "trailmark-prod" at any point in time
  And no accumulation of orphaned tasks or pools is observable after the 20 cycles

Scenario: A sustained outage does not cause unbounded reconnect attempts against Postgres
  Given Trailmark's Postgres instance for "trailmark-prod" is unavailable for an extended period
  When the listener repeatedly attempts to reconnect during that outage
  Then reconnect attempts are spaced by a capped backoff, not a tight loop
  And the rate of connection attempts against Postgres does not scale unboundedly with outage duration

Scenario: Sustained listener failure becomes visible to the operator
  Given "trailmark-prod"'s listener has been failing to reconnect for longer than an ordinary transient
    blip window
  When Sam Chen inspects the operator-visible signal for this project's real-time delivery health
  Then the persistent-failure state is distinguishable from a healthy, occasionally-blipping listener
  And Sam Chen is not left with the same signal for "silently dead for hours" as for "healthy"

Scenario: A project whose listener previously died is not permanently blocked from real-time delivery
  Given a project's listener has previously failed and the project's active_listeners entry still exists
  When the underlying connectivity issue is resolved
  Then that project's clients eventually receive live DocumentChange events again without requiring a
    full embyr-server process restart

Scenario: A listener that never fails behaves exactly as it does today
  Given a project's listener has experienced no connection failures
  When writes are committed to that project's documents
  Then onSnapshot delivery continues to meet the existing p99 write-to-callback latency expectation
  And no change in behavior, latency, or resource usage is observable on this non-failure path

Scenario: Full regression suite passes
  Given the complete pre-existing workspace test suite, including real-time Listen acceptance tests
  When this feature's changes are applied
  Then no previously-passing test regresses
```

#### Acceptance Criteria
- [ ] AC-RLR-01: given a transient connection failure (simulated against a real Postgres instance —
      e.g. terminating and restoring the backing connection), the listener resumes delivering
      `DocumentChange` events for that project within a bounded, defined recovery window, without a new
      `Listen` RPC, client action, or operator action.
- [ ] AC-RLR-02: after N repeated transient failures for the same project, no more than the current
      single active generation of background task and dedicated connection/pool exists for that
      project — no accumulation across reconnect cycles.
- [ ] AC-RLR-03: during a sustained outage, reconnect attempts are demonstrably rate-limited (capped
      backoff) — an integration test can assert the attempt rate does not scale linearly/unbounded with
      outage duration.
- [ ] AC-RLR-04: after a listener has been failing to reconnect for longer than a defined threshold, an
      operator-visible signal (log level and/or metric, DESIGN's own choice of mechanism) distinguishes
      this state from a healthy listener.
- [ ] AC-RLR-05 (regression guard): a project's real-time delivery is not permanently and unrecoverably
      blocked once its underlying connectivity issue is resolved — no full-process restart is required
      to recover a single project's real-time delivery.
- [ ] AC-RLR-06 (regression guard): a listener that experiences no failures shows zero change in
      behavior, latency (p99 write→onSnapshot ≤ 2s, US-05/AC-05c unaffected), or resource usage compared
      to today.
- [ ] AC-RLR-07: no other currently-passing test in the full workspace suite regresses, including
      existing real-time Listen acceptance tests (US-05 family).

#### Outcome KPIs
- **Who**: Sam Chen operating production embyr deployments with `direct_pg`/`aws_secret`/`gcp_secret`-backed
  customer projects using real-time Listen (e.g., Trailmark).
- **Does what**: no longer needs to restart the entire multi-tenant `embyr-server` process to recover one
  project's dead real-time listener; a transient infrastructure blip self-heals automatically instead of
  permanently and silently killing that project's real-time delivery.
- **By how much**: from 0% of transient blips self-healing today (100% require either a full-process
  restart or remain silently dead forever, per this DISCUSS's own confirmed reading) to 100% of
  transient blips self-healing within the bounded recovery window, with 0 dangling listener
  tasks/pools accumulating across repeated blips and reconnect attempts bounded (not unbounded) during
  sustained outages.
- **Measured by**: an integration test simulating a transient connection drop against a real Postgres
  instance and a real `embyr-server`, asserting (a) delivery resumes within the bounded window without
  an RPC/process restart (AC-RLR-01), (b) no orphaned tasks/pools accumulate across repeated blips
  (AC-RLR-02), (c) reconnect attempt rate is capped during a sustained outage (AC-RLR-03), (d) an
  operator-visible signal appears after sustained failure (AC-RLR-04); full regression suite (AC-RLR-07).
- **Baseline**: 0% recoverable without a full-process restart — confirmed directly by this DISCUSS's own
  reading of `postgres_notify_listener.rs:57-72` (recv-error → `break`, no retry) and `handler.rs:3544-3567`
  (`contains_key`-only guard, no `.remove()` anywhere in the workspace), matching the audit's own finding
  #4 evidence exactly.

#### Technical Notes
- The exact reconnect mechanism (internal reconnect-with-backoff vs. external death-detection-and-recreate)
  is DESIGN's own investigation — § Central Design Question above names an opening recommendation
  (internal reconnect, approach (a)) and the reasoning behind it, but does not lock it.
- Backoff curve: JOB-08's own already-shipped "exponential backoff 1s→30s" (Subscribe-stream reconnect)
  is named as a reference precedent for consistency across this codebase's two reconnect mechanisms —
  not mandated verbatim.
- The persistent-failure operator-visible signal's exact mechanism (log level escalation, a new JOB-12-style
  `/metrics` counter/gauge, or another approach) is DESIGN's own choice, informed by JOB-12's existing
  observability surface (`observability.rs`, already-installed Prometheus recorder) as a reuse candidate.
- This DISCUSS explicitly does NOT require the system to ever declare a listener permanently, irrecoverably
  dead and stop retrying — `sqlx::Error` cannot reliably distinguish transient from permanent failure at
  this call site (confirmed fact). See § Business Context, "Permanent vs. Transient Failure," for the full
  reasoning behind this scoping decision.
- `backend_mode=agent` is out of scope — confirm during DESIGN whether `handle_listen`'s own listener-
  provisioning block is even reachable for agent-mode projects (unconfirmed by this DISCUSS).
- Depends on nothing new — `PostgresNotifyListener`, `active_listeners`, `tokio::time::sleep`, and the
  already-pinned `sqlx`/`tracing` crates all already exist in this workspace.
- No modification to `ListenRegistry`, resume-token/RESET semantics, or the Listen RPC's own streaming
  shape (`listen_handler.rs`) — confirmed out of scope, see § System Constraints.

## Wave: DISCUSS / [REF] Definition of Done

1. AC-RLR-01 through AC-RLR-07 all pass, proven against a real running server instance and a real
   Postgres instance whose connectivity is genuinely interrupted and restored (not mocked) — mirrors
   this session's own established real, not mocked/unit-only, proof standard for audit-derived fixes.
2. The non-failure-path regression guard (AC-RLR-06) is proven identical to pre-feature behavior, not
   merely "still works."
3. The resource-accumulation guard (AC-RLR-02) is proven across multiple (not just one) reconnect
   cycles.
4. Full regression suite clean (pre-existing flakes excepted, triaged not assumed, per this session's own
   established `feedback_triage_before_dismissing_as_flaky` practice).
5. Mutation testing runs after DELIVER, per this repo's own `per-feature` strategy (root `CLAUDE.md`) —
   100% effective kill rate on the new/changed reconnect/backoff/cleanup logic.
6. Evolution doc written; `docs/product/production-readiness-audit-2026-09-08.md` row 4 updated to CLOSED
   at FINALIZE (this DISCUSS only leaves it "Not started" — DESIGN/DELIVER move it to IN PROGRESS/CLOSED).
7. `docs/product/jobs.yaml`'s JOB-03 record gets a NOTE recording this extension and the recommended
   `secondary_persona: P2` annotation (see § Wave Decisions Summary, Upstream Changes) — applied at
   FINALIZE, per this session's own established pattern of some NOTEs landing at FINALIZE rather than
   DISCUSS.
8. Memory updated.

## Wave: DISCUSS / [REF] Out of Scope

- **`backend_mode=agent`'s own real-time forwarding** — a structurally different mechanism (JOB-08's own
  Subscribe-stream reconnect, already designed with backoff) — not touched here. DESIGN should confirm
  (not assumed) whether `handle_listen`'s own `PostgresNotifyListener`-provisioning block is even reached
  for agent-mode projects.
- **`ListenRegistry`, resume-token/RESET semantics, and the Listen RPC's own client-facing streaming
  shape** (`listen_handler.rs`) — all confirmed unaffected by this feature; only the LISTEN/NOTIFY
  background task's own failure-recovery lifecycle changes.
- **Explicit listener teardown for legitimate lifecycle events** (e.g., a project being suspended or
  deleted while its listener is active) — a genuinely separate lifecycle concern from transient-failure
  recovery. Not investigated or evidenced by this DISCUSS; DESIGN may note it as a related follow-up if
  its chosen mechanism happens to expose a natural teardown hook, but it is not required by this
  feature's own ACs.
- **Building a transient-vs-permanent failure classifier** — explicitly rejected as an unevidenced,
  unreliable mechanism at this layer (see § Business Context). The system is required to make sustained
  failure VISIBLE, not to correctly classify or ever fully give up.
- **The exact reconnect mechanism, backoff curve, and persistent-failure signal mechanism** — all
  explicitly DESIGN's own investigation; an opening recommendation is offered (§ Central Design
  Question) but none of these are locked here.
- **Any code implementation** — this is DISCUSS only, per explicit task instruction; DESIGN performs the
  actual investigation and implementation planning.

## Wave: DISCUSS / [REF] Walking Skeleton Strategy

**Strategy A** (real, minimal, end-to-end) — every scenario is a real connection-failure simulation
against a real running server instance and a real Postgres instance, mirroring this session's own
Strategy A precedent for audit-derived fixes in this same server layer. This feature IS the walking
skeleton — single story, no further slicing (see § Scope Assessment for why the four requirement facets
are one outcome, not four).

## Wave: DISCUSS / [REF] Driving Ports

The existing gRPC :8080 `Listen` streaming RPC (`handle_listen`, already exists, no new endpoint) and its
existing internal collaborator `PostgresNotifyListener` (already exists). Zero new RPC/HTTP endpoint —
this feature changes only the failure-recovery behavior of an existing background task and, depending on
DESIGN's chosen mechanism, potentially the persistent-failure signal surfaced via the existing JOB-12
`/metrics`/logging surface.

## Wave: DISCUSS / [REF] Pre-requisites

- None beyond what already exists. `PostgresNotifyListener`, `active_listeners`, `ListenRegistry`, and
  the already-pinned `sqlx`/`tokio`/`tracing` crates all already exist and are unchanged in their core
  responsibilities by this feature — only the background task's own failure-recovery behavior is in
  scope.
- JOB-08's own already-shipped "exponential backoff 1s→30s" precedent and JOB-12's own already-installed
  observability surface are the established patterns this feature's chosen mechanism should stay
  consistent with, per DESIGN's own investigation.

## Wave: DISCUSS / [REF] Definition of Ready Validation

### DoR Checklist (9-item hard gate)
1. [x] Story traces to a job_id (JOB-03) — reused, not new, with reasoning against four nearer-seeming
   alternatives (JOB-08, JOB-11, JOB-12, JOB-13) explicitly documented in § Persona & Job.
2. [x] Story has a complete Elevator Pitch (Before / After / Decision enabled).
3. [x] Every AC is testable without ambiguity (7 ACs, each a real connection-failure simulation or
   full-suite regression run against a real running server and real Postgres instance).
4. [x] Walking Skeleton identified (US-01 is the whole feature's walking skeleton).
5. [x] Scope Assessment passed (at the upper edge of right-sized — 7 scenarios, 2-3 days — explicitly
   reasoned, not force-fit into a smaller shape it doesn't hold).
6. [x] Story is not `@infrastructure`-only with no user-visible value — Decision 4 = Yes (full JTBD
   path); it directly enables Sam Chen's own trust decision (Elevator Pitch "Decision enabled") that a
   transient blip against any one tenant never becomes a silent, permanent, whole-process-restart-requiring
   outage.
7. [x] Out of Scope explicitly named (6 items, each reasoned, including the two genuinely separate
   lifecycle/classification questions this feature does not attempt to solve).
8. [x] Outcome KPIs have a numeric framing (0% self-healing → 100% self-healing within a bounded window)
   and measurement methods.
9. [x] Prior-wave artifacts read and reconciled (the audit's own finding #4, JOB-03's existing job story,
   JOB-08's own backoff precedent, JOB-12's own observability surface, AD-08's dedicated-connection
   decision, and the original 05-02 walking-skeleton evolution doc all directly informed this feature's
   shape; no contradiction found with any existing decision; one refinement to the task's own pool-leak
   framing flagged for DESIGN in § Reading Confirmation).

### DoR Status: **PASSED**

## Wave: DISCUSS / [REF] Wave Decisions Summary

### Key Decisions
- [D1] Job reused: JOB-03 (`live-sync`) — the job whose functional dimension names the exact broken
  promise verbatim ("onSnapshot fires for every committed write"). Not JOB-08 (different code path,
  agent-mode only), not JOB-11 (rate-limiting decision untouched), not JOB-12 (metrics surface is a
  likely mechanism reuse, not the broken promise itself), not JOB-13 (a startup/request-admission-time
  shape, not this bug's mid-life background-task-recovery shape).
- [D2] Story persona: P2 Sam Chen (operational stakeholder, matches this finding's own SRE+DBA discovery
  origin), distinct from JOB-03's own recorded primary persona (P1 Alex) — mirrors the established
  `persona`/`secondary_persona` dual-stakeholder pattern (e.g. JOB-08).
- [D3] The exact reconnect mechanism is DESIGN's own investigation; opening recommendation offered
  (internal reconnect-with-backoff, approach (a)) with explicit reasoning, not locked.
- [D4] The system is not required to ever declare a listener permanently, irrecoverably dead — only to
  make sustained failure operator-visible. No transient-vs-permanent classifier is built.
- [D5] Single story, not split — the four requirement facets (recover / clean up / bound retries /
  surface persistent failure) are necessary conditions of one outcome, not independently shippable ones.
- [D6] A refinement to the task's own "pool leaks forever" framing is flagged for DESIGN: the confirmed,
  mechanism-independent permanent-harm fact is that no new listener/pool can EVER be created again for a
  project once its entry is dead (the `contains_key`-only guard), not necessarily that the OLD pool's
  connections are never closed at the OS level by ordinary Rust drop semantics when the task ends via
  `break`.

### Requirements Summary
- Primary need: a transient Postgres connection failure against a project's real-time listener does not
  permanently and silently end that project's real-time delivery, does not leak or accumulate connection
  resources, does not cause unbounded reconnect attempts against Postgres, and — if failure is sustained
  well beyond a normal transient blip — becomes visible to the operator.
- Walking skeleton scope: US-01, the entire feature — single story, 7 UAT scenarios.
- Feature type: Reliability fix.

### Constraints Established
- AD-08 (dedicated Postgres connection per project) preserved.
- Zero behavior change to the non-failure-path real-time latency contract (p99 write→onSnapshot ≤ 2s).
- Zero behavior change to `ListenRegistry`, resume-token/RESET semantics, or the Listen RPC's own
  streaming shape.
- No new external dependency.
- `backend_mode=agent` out of scope (separate, already-existing reconnect mechanism, JOB-08).

### Upstream Changes
Recommended for FINALIZE (not applied in this DISCUSS): add a NOTE to `docs/product/jobs.yaml`'s JOB-03
record documenting this extension (continuous, failure-resilient delivery, not merely initial-connection
delivery) and recording a `secondary_persona: P2` annotation, mirroring JOB-08's own existing
`persona`/`secondary_persona` structure.

## Wave: DISCUSS / [REF] Next Wave

**Handoff To**: nw-solution-architect (DESIGN wave)
**Deliverables**: this feature-delta.md, 6 locked Decisions (D1-D6), 1-story walking-skeleton plan, 7 ACs
(AC-RLR-01 through AC-RLR-07) to design executable scenarios against. DESIGN's own investigation scope:
(1) the precise reconnect mechanism (internal reconnect-with-backoff vs. external death-detection-and-
recreate — opening recommendation: internal, see § Central Design Question), (2) the exact backoff curve
(JOB-08's own "1s→30s exponential" named as reference precedent), (3) the exact persistent-failure
operator-visible signal mechanism (log escalation vs. a new JOB-12-style metric), (4) confirming whether
`handle_listen`'s own listener-provisioning block is reachable for `backend_mode=agent` projects at all
(unconfirmed by this DISCUSS), (5) confirming sqlx's actual connection-teardown timing on ordinary task
completion (via `break`) only if the chosen mechanism's design depends on that detail (see § Reading
Confirmation's flagged refinement).

## Wave: DESIGN / [REF] Reading Confirmation

✓ `crates/embyr-server/src/adapters/postgres_notify_listener.rs` re-read in full at DESIGN depth
(169 lines, unchanged since DISCUSS). Confirms DISCUSS's citation exactly: the spawned task's loop
(lines 57-72) `break`s on any `Err` from `pg_listener.recv().await`.

✓ **`sqlx-postgres` source read directly** (`~/.cargo/registry/src/.../sqlx-postgres-0.8.6/src/listener.rs`,
version confirmed pinned via workspace `Cargo.lock`) — this is the single most consequential finding
of this DESIGN and **corrects both the task's own framing and DISCUSS's opening recommendation's
stated reasoning**, though not its conclusion:

- `sqlx::postgres::PgListener`'s own doc comment (line 22-27): *"This listener will auto-reconnect. If
  the active connection being used ever dies, this listener will detect that event, create a new
  connection, will re-subscribe to all of the originally specified channels, and will resume operations
  as normal."* This is not aspirational — `try_recv()` (lines 256-327) implements it: on an IO error
  matching `ConnectionAborted | UnexpectedEof | TimedOut | BrokenPipe` (lines 282-290), it drops the
  dead connection, and — because `eager_reconnect` defaults to `true` (line 74) — immediately calls
  `self.connect_if_needed()` (line 299), which re-`pool.acquire()`s a fresh connection and **re-issues
  `LISTEN` for every channel in `self.channels`** (line 179's `build_listen_all_query(&self.channels)`,
  populated by the original `.listen(&channel)` call at construction and never cleared). `recv()`
  (lines 222-228) is a thin loop over `try_recv()`.
- **Consequence: creating a fresh `PgListener` and re-calling `.listen()` on reconnect — the task's own
  suggested framing, and the literal reading of DISCUSS's "a dropped connection means a fresh
  `PgListener` must be created and must re-issue `LISTEN`" — is unnecessary and wrong.** The *existing*
  `pg_listener` binding already moved into the spawned task's closure (line 57's `async move`) is the
  correct thing to keep calling `.recv()` on, indefinitely, across any number of reconnects. Postgres
  session-scoped `LISTEN` state does not survive a reconnect at the *server* level, but `sqlx` already
  re-establishes it at the *client* level, transparently, using its own internally-tracked channel list
  — this codebase does not need to reimplement that.
- **What `sqlx` does NOT do**: retry with backoff, or survive a reconnect attempt that itself fails.
  `connect_if_needed()`'s `pool.acquire().await?` is a single, unguarded attempt — if Postgres is still
  unreachable at that instant (a sustained outage, not a momentary TCP drop), the `?` propagates the
  acquire error out of `try_recv()` → out of `recv()` as `Err`. This is the *actual*, narrower shape of
  today's bug: a "the TCP socket dropped but Postgres itself is already back up" blip is already
  self-healing today (silently, via `sqlx`, before this feature exists) and never reaches our own
  `Err` arm at all; only a blip that outlasts one immediate, eager reconnect attempt reaches `break`.
  This does not change the requirement (AC-RLR-01/03 still name real, multi-second-plus outages), but
  it does change the fix: **the only thing missing is retrying `pg_listener.recv().await` itself, with
  backoff, instead of ending the task on its first `Err`.** No new `PgListener`, no re-`.listen()`, no
  new pool, no new connection — the existing `pg_listener` and `backend_pool` bindings, already captured
  in the closure, are reused across every reconnect cycle, for the life of the task.

✓ **`crates/embyr-server/src/grpc/handler.rs` re-read in full around `handle_listen` and `authenticate`**
(lines 199-364, 3514-3567) at DESIGN depth, specifically to answer the task's open question (4):
confirmed `authenticate()`'s `agent`-mode branch (lines 299-334) returns `(shared, String::new())` —
**the DSN is the empty string for every `backend_mode = "agent"` project.** `handle_listen`'s
listener-provisioning block (lines 3544-3567) has **no `backend_mode` branch at all** — it
unconditionally attempts `sqlx::postgres::PgPoolOptions::new().max_connections(2).connect(&dsn)` (line
3552-3556) using that same empty string, for every project, agent-mode included. Answering DISCUSS's
open question directly: **yes, the block is reached for agent-mode projects, and the pool-connect call
against an empty DSN fails immediately** (`sqlx` cannot parse `""` as a connection string), turning
every agent-mode `Listen` RPC into `Status::internal("notify listener pool: ...")` today, before
`PostgresNotifyListener::start()` is ever called. This is a **pre-existing, separate defect**, unrelated
to audit finding #4 and outside every one of this feature's 7 ACs (no acceptance test in
`tests/production_readiness/` or `tests/acceptance/us_05_listen_realtime.rs` exercises agent-mode
Listen — none exists in the `[[test]]` registry in `crates/embyr-server/Cargo.toml`). Flagged in
§ Residual Risks below, not fixed here — this feature's own change lives entirely inside
`PostgresNotifyListener::start()`'s spawned-task closure, a point agent-mode traffic never reaches
(it fails one step earlier, at the pool-connect call). Blast radius to `backend_mode=agent` is zero
either way.

✓ **Blast-radius grep performed independently**: `rg 'PostgresNotifyListener::start\(' crates/` →
exactly **one call site**, `handler.rs:3557`, unchanged 4-argument shape
(`&dsn, &project_id, Arc::clone(&self.listen_registry), pool`). `rg 'active_listeners' crates/` →
3 files (`main.rs`, `lib.rs`, `handler.rs`), all Arc-cloning/wiring, zero `.remove()` call sites
anywhere in the workspace (DISCUSS's own finding, re-confirmed, still true — no new call was added
since DISCUSS). This feature's design changes **neither** `handle_listen` **nor** `start()`'s public
signature — only the body of the closure already inside `start()`'s `tokio::spawn` — so both of the
task's named blast-radius touchpoints are provably unaffected in shape.

✓ **A second, textually-identical instance of this exact bug found and confirmed out of scope**:
`crates/embyr-agent/src/notify_bridge.rs`'s `AgentNotifyBridge::subscribe()` (lines 66-94) — the
agent-side half of JOB-08's own Subscribe-stream mechanism (customer-VPC agent LISTENs on the
*customer's* Postgres, translates NOTIFY into `DocChange`, forwards over the Subscribe gRPC stream to
SaaS) — has the identical shape: `pg_listener.recv().await`'s `Err` arm logs `tracing::error!` and
`break`s, ending the bridge's background task forever, with the identical "no reconnect, no backoff"
gap this feature closes for the SaaS-side `direct_pg`/`aws_secret`/`gcp_secret` listener. **This is not
touched by this feature** (out of scope per DISCUSS: `backend_mode=agent`'s own real-time forwarding is
a structurally different mechanism/crate) but is flagged as a natural, low-risk follow-up candidate
that could reuse this feature's exact `reconnect_backoff()` function (see § Mechanism Decision) once
promoted to a shared location, since it needs the identical fix. Not designed here — no ADR, no AC
covers it; named only so it is not rediscovered as a "new" finding later.

✓ **A second, unrelated `PostgresNotifyListener` struct found and confirmed to be dead code**:
`crates/embyr-pg-storage/src/notify_listener.rs` (lines 20-67) defines its own `PostgresNotifyListener`
struct/`start()` method — a generic, registry-free variant sending raw payload strings over an `mpsc`
channel. `rg 'notify_listener::PostgresNotifyListener' crates/` → the only two matches
(`main.rs:31`, `lib.rs:23`) both resolve to `crate::adapters::postgres_notify_listener::PostgresNotifyListener`
(this feature's own target struct, same name, different crate/module path) via `use` aliasing — **zero
call sites of `embyr_pg_storage::notify_listener::PostgresNotifyListener::start()` exist anywhere in the
workspace.** Confirmed dead code, unrelated to this feature (only its sibling free function
`notify_channel()` is reused, by both `embyr-server` and `embyr-agent`). Not touched — zero blast
radius, mentioned for completeness only.

## Wave: DESIGN / [REF] Central Design Question — Resolved

**Decision: Approach (a), internal reconnect-with-backoff — confirmed, with DISCUSS's own reasoning
corrected per § Reading Confirmation above.** Full reasoning, alternatives, and consequences recorded in
`docs/product/architecture/adr-071-postgres-notify-listener-reconnect.md` (new ADR); summarized here:

- The background task's `loop { match pg_listener.recv().await { ... } }` never `break`s on a
  `recv()` error. It logs, applies a capped exponential backoff sleep, and calls `.recv()` again on the
  **same** `pg_listener` — which `sqlx` itself will transparently attempt to reconnect and
  re-`LISTEN` on, per § Reading Confirmation. The task ends only via the pre-existing external
  `Drop`/`abort()` path (unchanged, and — per § Deprovisioning below — still not wired to any
  project-lifecycle event; a separate, out-of-scope concern).
- **This is a smaller diff than DISCUSS's own opening recommendation anticipated**: no new `PgListener`,
  no re-`.listen()`, no new pool, no new connection is ever created for a reconnect cycle — only the
  existing `pg_listener`/`backend_pool` bindings already captured in the closure are reused. The
  `PostgresNotifyListener` struct, `start()`'s signature, its `Drop` impl, and `handle_listen`'s
  `contains_key` guard are **all unchanged** (see § Handoff Package for the exact, single-file diff
  surface).
- Satisfies every one of the 7 ACs:
  - **AC-RLR-01** (bounded recovery window): a socket-level blip Postgres itself already survived is
    already invisible today (sqlx's own eager reconnect, § Reading Confirmation) — zero change in
    behavior for that case. A blip that outlasts sqlx's single eager-reconnect attempt is retried by
    this feature's own outer loop; worst-case added latency after Postgres becomes reachable again is
    bounded by the current backoff interval (≤30s, and typically ≤4s for the "few seconds" blip named
    in US-01 Example 1, since only 1-2 outer retries are needed before Postgres answers again).
  - **AC-RLR-02** (no accumulation across reconnect cycles): trivially true — one task, one
    `PostgresNotifyListener` struct, one dedicated connection (via `pg_listener`'s own internal
    single-connection pool) exist for the life of the project's entry in `active_listeners`, regardless
    of how many times `recv()` returns `Err`. Nothing new is ever allocated per reconnect.
  - **AC-RLR-03** (capped backoff, not a tight loop): the outer loop's `tokio::time::sleep` between
    retries, per § Backoff below.
  - **AC-RLR-04** (operator-visible signal after sustained failure): `tracing::error!` + Prometheus
    gauge once a consecutive-failure threshold is crossed, per § Observability below.
  - **AC-RLR-05** (no permanent block once connectivity resolves): trivially true — the *same* live
    task resumes normal delivery the instant `pg_listener.recv()` next succeeds; no process restart, no
    new `Listen` RPC, no `handle_listen` change of any kind is involved.
  - **AC-RLR-06** (zero behavior change on the non-failure path): the `Ok(notification)` arm is
    unchanged except for resetting a local `consecutive_failures` counter to `0` — a single, unconditional
    integer assignment, no new allocation, no new I/O, no measurable cost against the p99 ≤ 2s KPI.
  - **AC-RLR-07** (full regression suite): no signature, no public type, no call site changes — see
    § Handoff Package's blast-radius confirmation.

## Wave: DESIGN / [REF] Backoff

**1s → 30s exponential doubling** (JOB-08's own documented reference precedent, § Business Context),
implemented as a new, minimal, private pure function — **not reused from existing code**, because none
exists to reuse: `rg 'backoff|exponential' crates/` returns zero matches anywhere in this workspace.
JOB-08's own "SaaS reconnects with exponential backoff 1s→30s" text (`docs/product/jobs.yaml`) describes
an *aspirational* functional dimension, not shipped code — confirmed by reading the actual agent-mode
Subscribe-stream call path (`crates/embyr-server/src/adapters/agent_backend.rs`: zero matches for
`backoff|sleep|Duration|retry`). There is nothing to reuse; this feature's curve is written fresh, but
kept numerically consistent with JOB-08's own documented shape rather than inventing a different one, per
DISCUSS's own instruction.

```rust
const RECONNECT_INITIAL_BACKOFF_SECS: u32 = 1;
const RECONNECT_MAX_BACKOFF: std::time::Duration = std::time::Duration::from_secs(30);

/// Exponential backoff for `PostgresNotifyListener`'s reconnect loop: 1s, 2s,
/// 4s, 8s, 16s, then capped at 30s. `consecutive_failures` is 1-indexed (the
/// value immediately after the Nth consecutive `recv()` error).
///
/// ponytail: no jitter — each project's `PgListener` targets a dedicated,
/// per-tenant Postgres instance (AD-08), so one project's retry schedule is
/// never correlated with another's. Add per-project jitter only if a future
/// backend_mode ever shares one Postgres instance across multiple projects'
/// listeners.
fn reconnect_backoff(consecutive_failures: u32) -> std::time::Duration {
    let shift = consecutive_failures.saturating_sub(1).min(5); // 2^5s = 32s, capped at 30s below
    std::time::Duration::from_secs(u64::from(RECONNECT_INITIAL_BACKOFF_SECS) << shift)
        .min(RECONNECT_MAX_BACKOFF)
}
```

## Wave: DESIGN / [REF] Observability (AC-RLR-04)

Reuses this repo's own already-installed `metrics`/`metrics-exporter-prometheus` surface (ADR-016,
`crates/embyr-server/src/observability.rs`) and its own established `project_id`-labeled-counter
precedent (`crates/embyr-server/src/middleware/rate_limit.rs:162-167`,
`embyr_rate_limit_requests_total{project_id, outcome}`) — no new dependency, no new HTTP
endpoint/admin route; both new metrics are scraped by the existing `/metrics` handler
(`crates/embyr-server/src/admin/handlers/prometheus_metrics.rs`).

**Cardinality note — stronger guarantee than ADR-069's rate-limiter case**: this code path is only ever
reached *after* `self.authenticate(&project_id, &api_key).await?` has already succeeded
(`handler.rs:3539`, strictly before the listener-provisioning block at `:3544`) — unlike the
pre-authentication rate-limiter counter ADR-069 had to bound with a sentinel fallback, every
`project_id` value reaching this metric is already a confirmed-real, provisioned project. No
`"unconfirmed"`-style sentinel gate is needed here.

- `embyr_pg_notify_listener_reconnect_attempts_total{project_id}` (counter) — incremented once per
  failed `recv()` (i.e., once per outer retry), from the very first failure.
- `embyr_pg_notify_listener_reconnecting{project_id}` (gauge, 0 or 1) — set to `1` once
  `consecutive_failures` reaches `RECONNECT_ALERT_THRESHOLD`; reset to `0` on the next successful
  `recv()`. This is the AC-RLR-04 signal: `0` (or a counter incrementing only rarely, self-healing
  within a request or two) reads as "healthy, occasionally blipping"; `1` reads as "sustained failure,"
  distinguishable at a glance in Grafana/alerting — directly answering "Sam Chen must never see the same
  signal for silently-dead-for-hours as for healthy" (US-01 Example 3).
- `tracing::warn!` per attempt below threshold (matches today's existing log level for a transient
  condition); escalates to `tracing::error!` once the threshold is crossed, and again on eventual
  recovery logs at `tracing::info!` ("recovered after N attempts") — mirroring this repo's own existing
  level-escalation precedent (`rate_limit.rs`'s `rate_limit_pg_timeout` warn! + counter pairing, ADR-016).

```rust
const RECONNECT_ALERT_THRESHOLD: u32 = 5; // ~15s of sustained failure (1+2+4+8s of backoff already
                                           // elapsed before the 5th failure) — well past US-01's own
                                           // "a few seconds" transient-blip example.
```

## Wave: DESIGN / [REF] Mechanism Decision — Reconnect Loop (illustrative code sketch)

DELIVER owns exact naming/decomposition; this sketch fixes only the WHAT — the loop never `break`s on
error, backoff is applied between retries, the observability calls above fire at the stated points.

```rust
// crates/embyr-server/src/adapters/postgres_notify_listener.rs — start()'s spawned task, replacing
// today's lines 57-72. `pg_listener` and `backend_pool` are the SAME bindings already moved into this
// closure today — nothing about their construction changes.

let task = tokio::spawn(async move {
    let mut consecutive_failures: u32 = 0;
    loop {
        match pg_listener.recv().await {
            Ok(notification) => {
                if consecutive_failures >= RECONNECT_ALERT_THRESHOLD {
                    tracing::info!(project_id = %project_id, channel = %channel,
                        attempts = consecutive_failures, "postgres_notify_listener_recovered");
                    metrics::gauge!("embyr_pg_notify_listener_reconnecting",
                        "project_id" => project_id.clone()).set(0.0);
                }
                consecutive_failures = 0;

                let payload = notification.payload().to_string();
                let event = fetch_event(&backend_pool, &project_id, &payload).await;
                registry.fan_out(&channel, event).await;
            }
            Err(e) => {
                consecutive_failures += 1;
                metrics::counter!("embyr_pg_notify_listener_reconnect_attempts_total",
                    "project_id" => project_id.clone()).increment(1);

                if consecutive_failures == RECONNECT_ALERT_THRESHOLD {
                    tracing::error!(project_id = %project_id, channel = %channel,
                        attempts = consecutive_failures, error = %e,
                        "postgres_notify_listener_sustained_failure");
                    metrics::gauge!("embyr_pg_notify_listener_reconnecting",
                        "project_id" => project_id.clone()).set(1.0);
                } else {
                    tracing::warn!(project_id = %project_id, channel = %channel,
                        attempt = consecutive_failures, error = %e,
                        "postgres_notify_listener_recv_error");
                }

                tokio::time::sleep(reconnect_backoff(consecutive_failures)).await;
                // Loop back to recv() — sqlx's own PgListener transparently
                // reconnects and re-LISTENs on the next call (§ Reading
                // Confirmation). No new PgListener/pool/connection is created.
            }
        }
    }
});
```

## Wave: DESIGN / [REF] Active-Listeners Map / Eviction — Resolved

**Confirmed, not assumed: with approach (a), the "dead-listener eviction" problem named in the audit's
own finding #4 no longer exists, for the reason DISCUSS's own opening recommendation predicted.** The
spawned task never ends on a `recv()` error of any kind (transient or sustained) — only via the
pre-existing external `Drop`/`abort()` path. `handle_listen`'s `contains_key(&project_id)` guard
(`handler.rs:3548`) therefore remains a permanently valid invariant: *"an entry exists" ⇒ "a live task is
running and will eventually deliver, including after any number of Postgres blips."* Zero change to
`handle_listen` is required or made.

**What is NOT resolved, confirmed explicitly rather than assumed away (the task's own second half of
this question)**: a project that is genuinely **suspended or deleted** while its listener is mid-backoff
retains a live, forever-retrying background task. Confirmed by reading
`crates/embyr-server/src/admin/handlers/lifecycle.rs`'s `suspend_project` (lines 57-83) and
`delete_project` (lines 165-182+) in full — **neither references `active_listeners` in any way**
(grep-confirmed: only `main.rs`, `lib.rs`, `handler.rs` do). A suspended/deleted project's listener will
retry forever, capped at one connection attempt per ≤30s, for the remaining life of the `embyr-server`
process, contributing one idle `tokio::task` + one dedicated (failed-to-establish) connection attempt
per cycle — cheap per-instance but **never reclaimed**, a real, separate, currently-unaddressed
resource-lifecycle gap.

**This is out of scope for this feature**, exactly as DISCUSS locked (§ Out of Scope: "Explicit listener
teardown for legitimate lifecycle events... not required by this feature's own ACs"). None of AC-RLR-01
through 07 test project suspension/deletion. Recorded here as a **Recommended Follow-Up**, not designed
or implemented:

- The natural hook is `active_listeners: Arc<Mutex<HashMap<String, PostgresNotifyListener>>>` itself —
  already an `Arc`, already cloned into every gRPC service struct in `lib.rs` (lines 556-959). Wiring the
  same `Arc` into `AdminState` (or passing it explicitly into `suspend_project`/`delete_project`) and
  calling `.remove(&project_id)` there would let `PostgresNotifyListener`'s own pre-existing `Drop` impl
  (`abort()`) reclaim the task — the FIRST `.remove()` call this codebase would ever make against this
  map. Not designed further here (no AC requires it); flagged so it is not rediscovered as a "surprise"
  later.

## Wave: DESIGN / [REF] Residual Risks Carried Forward (documented, not fixed here)

- **`fetch_event`'s fallback-to-`Removed` on query failure** (`postgres_notify_listener.rs:165-168`):
  `fetch_event` queries `backend_pool` (a separate `sqlx::PgPool` from `pg_listener`) after every
  successful `recv()`. If that *query* fails — including for the same class of transient Postgres
  unavailability this feature targets — the `_ => ListenEvent::Removed { ... }` catch-all (today's
  comment: "Row genuinely absent — defensive, should not occur") synthesizes a false-delete event and
  fans it out to subscribers, rather than skipping/retrying. This is a **pre-existing, separate bug**,
  not introduced or worsened by this feature (this feature does not touch `fetch_event`), and not
  covered by any of the 7 ACs. Flagged for a future, narrowly-scoped fix (distinguish "row absent" from
  "query errored" in `fetch_event`'s `match row_opt`), not designed here.
- **`backend_mode=agent`'s empty-DSN `handle_listen` failure** (§ Reading Confirmation above) — a
  pre-existing, separate defect making every agent-mode `Listen` RPC fail today, unrelated to and
  unaffected by this feature.
- **`AgentNotifyBridge::subscribe()`'s identical unfixed reconnect gap** (§ Reading Confirmation
  above) — out of scope, flagged as a natural follow-up.
- **Project suspension/deletion during an active backoff retry loop** (§ Active-Listeners Map above) —
  out of scope, flagged with a concrete recommended hook.

## Wave: DESIGN / Handoff Package

**Files requiring a change:**
1. `crates/embyr-server/src/adapters/postgres_notify_listener.rs` — the ONLY production-code file
   requiring a change. `start()`'s spawned-task loop body (today's lines 57-72) is replaced per
   § Mechanism Decision; two small private additions (`reconnect_backoff()`, the two `RECONNECT_*`
   constants). `start()`'s own signature (`pub async fn start(dsn: &str, project_id: &str, registry:
   Arc<ListenRegistry>, backend_pool: PgPool) -> Result<Self, CoreError>`), the `PostgresNotifyListener`
   struct definition, and its `Drop` impl are all **unchanged**.

**Files confirmed to need NO change (blast radius, grep-verified):**
- `crates/embyr-server/src/grpc/handler.rs` — `handle_listen`'s listener-provisioning block
  (lines 3544-3567), including the `contains_key` guard, is unchanged; the single call site at line
  3557 passes the same 4 arguments in the same shape.
- `crates/embyr-server/src/realtime/listen_registry.rs`, `listen_handler.rs` — untouched, per DISCUSS's
  own System Constraints.
- `crates/embyr-pg-storage/*` — both `notify_channel()` (reused, unmodified) and the dead-code
  `notify_listener::PostgresNotifyListener` (untouched, zero callers).
- `crates/embyr-agent/*` — `AgentNotifyBridge` shares no code with the file being changed; its own
  identical bug is flagged, not fixed, this feature.

**Documentation changes made this wave:**
- `docs/product/architecture/adr-071-postgres-notify-listener-reconnect.md` (new).

**Regression guards DISTILL/DELIVER should run:**
- `tests/acceptance/us_05_listen_realtime.rs` — the existing non-failure-path Listen regression guard
  (AC-RLR-06/07).
- `tests/security_rules_realtime/**` (8 slices) — exercises `fetch_event`/`fan_out` on the identical
  code path; must stay green (AC-RLR-07).
- New acceptance test file recommended (DISTILL's own naming call): this is finding #4 of
  `docs/product/production-readiness-audit-2026-09-08.md`, whose findings #1-#3 already established the
  `tests/production_readiness/acceptance/prNN_*.rs` sequential-numbering convention (pr06 = finding #1,
  pr07 = finding #3's body-limit sibling) — `pr08_realtime_listener_reconnect.rs` would continue that
  sequence and is the natural home for AC-RLR-01 through 07's own real-Postgres-kill/restore scenarios
  (mirrors this session's own "real, not mocked" proof standard, per DoR item 1).
- Full workspace `cargo test`, once, at the pre-commit gate — per this repo's own root `CLAUDE.md`
  test-run token-discipline rule.

## Wave: DESIGN / [REF] Peer Review

Reviewed by solution-architect-reviewer (iteration 1). **Result: conditionally_approved, 0 critical,
1 high, 1 medium, 1 low.** All resolved without a second iteration:

- **High** — reviewer could not independently locate/read the pinned `sqlx-postgres-0.8.6` source
  under its own sandboxed `~/.cargo/registry` to verify the auto-reconnect claim in § Reading
  Confirmation, and asked for empirical (not just source-read) proof. This DESIGN already read that
  exact source file directly (cited path and line numbers in § Reading Confirmation/ADR-071 § Context)
  — the claim stands. The reviewer's own suggested closure is, independently, already required:
  AC-RLR-01's own acceptance test (a real Postgres connection genuinely terminated and restored, per
  DoR item 1's "not mocked" standard) will empirically exercise `sqlx`'s internal reconnect path as a
  direct byproduct of proving AC-RLR-01 — no new test is added solely for this; DISTILL/DELIVER should
  note in that test's own doc comment that it also serves as the empirical confirmation of this ADR's
  central technical claim.
- **Medium** — project-suspension/deletion during an active backoff loop should be "surfaced alongside
  this feature's own landing, not just left in an ADR." Actioned: recorded as an explicit FINALIZE-time
  action item below, not only inside ADR-071's Consequences.
- **Low** — `reconnect_backoff()`'s comment ("2^5s = 32s already exceeds the cap") was misleading (32s
  does not exceed 30s, it is simply capped by `.min()`). Fixed in both this file and ADR-071 to
  "2^5s = 32s, capped at 30s below."

**FINALIZE action item (carried forward, not applied this wave)**: when this feature's evolution doc is
written, include a named follow-up — "project suspension/deletion does not evict `active_listeners`;
add `.remove(&project_id)` calls to `suspend_project`/`delete_project`
(`crates/embyr-server/src/admin/handlers/lifecycle.rs`) as a small, separate future fix" — so it is
tracked rather than only discoverable inside ADR-071's own Consequences section.

## Wave: DISTILL / [REF] Prior Wave Consultation — Reading Confirmation

✓ `docs/feature/realtime-listener-reconnect/feature-delta.md` (this file, DISCUSS+DESIGN sections)
read in full — 1 story (US-01), 7 UAT scenarios, AC-RLR-01 through AC-RLR-07, DESIGN's locked
mechanism (ADR-071: reuse `sqlx`'s own `PgListener` auto-reconnect, add only a capped-backoff outer
retry loop that never `break`s on `recv()` error), the exact Handoff Package (single production file:
`crates/embyr-server/src/adapters/postgres_notify_listener.rs`), and the DESIGN peer-review's
already-resolved High/Medium/Low findings.
✓ `docs/product/architecture/adr-071-postgres-notify-listener-reconnect.md` read in full — Decision,
backoff curve (`reconnect_backoff`: 1s→30s exponential doubling, no jitter, `RECONNECT_ALERT_THRESHOLD
= 5`), observability (`embyr_pg_notify_listener_reconnect_attempts_total{project_id}` counter,
`embyr_pg_notify_listener_reconnecting{project_id}` gauge), Consequences (residual risks — project
suspension/deletion during backoff, `fetch_event`'s pre-existing fallback-to-Removed bug,
`backend_mode=agent`'s separate empty-DSN defect, `AgentNotifyBridge`'s identical unfixed gap — all
out of scope, none require new acceptance coverage here), and Alternatives Considered.
✓ `crates/embyr-server/src/adapters/postgres_notify_listener.rs` re-read at DISTILL depth (unchanged
since DESIGN) — confirmed today's spawned task still `break`s on any `recv()` `Err` (lines 57-72);
this is the exact behavior the new scenarios below must observe as RED.
✓ Existing test suites inventoried before writing anything new (reuse-before-write, per Mandate 6 /
Architecture of Reference): `tests/acceptance/us_05_listen_realtime.rs` (existing non-failure-path
Listen regression guard — AC-RLR-06 reuses this AS-IS, no new test written for it) and
`tests/distributed_rate_limiting/acceptance/b13_fallback_on_pg_failure.rs` (checked for an existing
"simulate Postgres unavailability" mechanism to reuse — found it is itself still an unimplemented
`todo!()` scaffold with zero working mechanism; no precedent existed to reuse, so DISTILL wrote one
fresh: real `ContainerAsync::stop()`/`::start()` against a FIXED host-port-mapped testcontainer —
empirically verified during this DISTILL pass that Docker's default EPHEMERAL port mapping is
reassigned to a new random host port on every `start()` after a `stop()`, which would have produced a
false SETUP_FAILURE red; fixed by mapping a `find_free_port()`-allocated FIXED host port via
`ImageExt::with_mapped_port`, confirmed stable across stop/start).
✓ `tests/distributed_rate_limiting/acceptance/b15_project_id_metric_label_cardinality.rs` and its
`tests/distributed_rate_limiting/common/mod.rs` (`get_metrics`, `ADMIN_KEY` = `"test-admin-key-secret"`)
read for the established `/metrics` scrape convention — reused verbatim (same admin Bearer key,
confirmed identical across every `embyr_server::start_test_server_with_*` variant via
`crates/embyr-server/src/lib.rs` grep).
✓ `tests/production_readiness/mod.rs` and `tests/production_readiness/common/mod.rs` read in full —
confirmed the `pr0N` sequential-numbering convention (DESIGN's own Handoff Package recommendation),
the `ServerProcess` subprocess harness (used by pr01/pr02/pr04/pr06/pr07 for process-boundary
concerns — env vars, TLS, Docker, graceful shutdown, webhook body limits) and `find_free_port()`
(reused directly here via `crate::common::find_free_port`). Confirmed `crates/embyr-server/Cargo.toml`
registers `production_readiness` as ONE `[[test]]` binary including all `pr0N` modules via
`tests/production_readiness/mod.rs` — no new `[[test]]` entry required, only a new `mod pr08_...;`
line.
✓ `#[ignore]` convention confirmed by direct inspection of `pr06_stripe_webhook_secret_required.rs`
(all 4 tests `#[ignore]`, single-story feature, DELIVER unskips one at a time) vs.
`pr07_stripe_webhook_body_limit.rs` (comment states the same "all stay `#[ignore]`" intent but the
FINALIZED file today shows zero `#[ignore]` remaining — confirms DELIVER progressively removes
`#[ignore]` as each scenario is unskipped and landed, and a finalized sibling naturally ends with none
left). Applied pr06's stricter convention here: the walking skeleton is the only non-`#[ignore]`
test; the remaining 4 stay `#[ignore]` for DELIVER to unskip one at a time.

## Wave: DISTILL / [REF] Scenario List

Tier A only (single-story feature, config-shaped input space — no Tier B state-machine PBT per
Mandate 10: the journey is 1 walking skeleton + 4 focused scenarios, not a domain-rich generative
input space; skip condition "journey has 1-2 scenarios per capability, only observable is
recover-or-not" applies).

| # | Scenario | AC | Tags |
|---|---|---|---|
| 1 | A transient Postgres blip does not permanently end real-time delivery | AC-RLR-01 | `@walking_skeleton @driving_port @real-io @US-01` — NOT `#[ignore]` |
| 2 | Repeated transient blips do not duplicate delivered changes | AC-RLR-02 | `@driving_port @real-io @US-01` — `#[ignore]` |
| 3 | Sustained outage: reconnect attempts are bounded, not a tight loop | AC-RLR-03 | `@driving_port @real-io @US-01` — `#[ignore]` |
| 4 | Sustained failure is operator-visible AND a second project is unaffected | AC-RLR-04 | `@driving_port @real-io @US-01` — `#[ignore]` |
| 5 | Listener recovers even after crossing the sustained-failure threshold | AC-RLR-05 | `@driving_port @real-io @US-01` — `#[ignore]` |
| — | (regression guard, no new test) | AC-RLR-06 | covered by existing `tests/acceptance/us_05_listen_realtime.rs` |
| — | (regression guard, no new test) | AC-RLR-07 | full workspace `cargo test`, once, at the pre-commit gate |

Scenario 4 also covers the orchestrator's own explicitly-requested per-project-isolation proof
(AD-08: a second, unrelated project's listener is unaffected by the first project's blip) — folded
into the same test rather than a separate one, since both assertions share the identical two-project
setup cost.

## Wave: DISTILL / [REF] Walking Skeleton Strategy

Architecture of Reference applied: driving port (gRPC `Listen` streaming RPC + admin `/metrics`
endpoint) = real adapter, in-process `embyr_server::TestServer` (mirrors `us_05_listen_realtime.rs`,
not this suite's own subprocess `ServerProcess` — the OS process boundary is not what this feature's
own fix touches). Driven internal (customer Postgres via `PostgresNotifyListener`) = real adapter,
real `testcontainers-rs` Postgres 15-alpine per project (dedicated instance per project, proving AD-08
literally rather than merely by convention). No driven-external/non-deterministic ports in scope.

Walking skeleton litmus test: title describes the user-observable goal ("a transient blip does not
permanently end real-time delivery"), Given/When/Then describe Trailmark's own onSnapshot listener and
a Postgres connection dropping/recovering (business-observable), Then describes a DocumentChange event
arriving on the still-open stream (client-observable outcome) — a non-technical stakeholder (Sam Chen)
can confirm "yes, that is what operators need."

## Wave: DISTILL / [REF] Adapter Coverage Table

| Adapter | `@real-io` scenario | Covered by |
|---|---|---|
| `PostgresNotifyListener` (LISTEN/NOTIFY, dedicated connection per project) | YES | Scenario 1 (WS) — real Postgres container genuinely stopped/restarted |
| `ListenRegistry::fan_out` (existing, unmodified — exercised as a byproduct) | YES | Scenario 1, Scenario 2 (duplicate-delivery proxy) |
| Admin `/metrics` Prometheus scrape (existing endpoint, two NEW metrics) | YES | Scenario 3, Scenario 4, Scenario 5 |
| `handle_listen`'s `active_listeners` map / `contains_key` guard (existing, unmodified) | YES (indirectly — same open stream never re-provisions) | Scenario 1, Scenario 5 |

No new driven adapter is introduced by this feature (ADR-071's own Handoff Package: one file changes,
zero new ports/adapters) — the table above covers the adapters this feature's fix actually touches or
whose behavior the acceptance scenarios depend on, not a hypothetical new-adapter inventory.

## Wave: DISTILL / [REF] Scaffolds

None. Per Mandate 7, scaffolding is only required when a test imports a production module that does
not yet exist. This feature's entire fix lives inside `postgres_notify_listener.rs`'s already-existing,
already-compiling `PostgresNotifyListener::start()` — every symbol the new tests import
(`PostgresNotifyListener`, `notify_channel`, `TestServer`, `start_test_server_with_keepalive`,
`SystemDb`, the `Listen`/`CreateDocument` gRPC surface) already exists and already compiles. The two
new Prometheus metric names (`embyr_pg_notify_listener_reconnect_attempts_total`,
`embyr_pg_notify_listener_reconnecting`) do not require scaffolding either — they are read via a
generic `/metrics` text-body parser (`metric_value_for_label`) that returns `None`/defaults to `0.0`
when a metric has not fired yet; this is precisely what makes scenarios 3-5 RED for the right reason
today (MISSING_FUNCTIONALITY: the metric doesn't exist because the reconnect loop that would emit it
doesn't exist) rather than a compile error.

## Wave: DISTILL / [REF] Test Placement

`tests/production_readiness/acceptance/pr08_realtime_listener_reconnect.rs` — continues this
directory's own established `pr0N` sequential-numbering convention (pr06 = finding #1 sibling, pr07 =
finding #3 sibling), per DESIGN's own Handoff Package naming recommendation. Registered via
`tests/production_readiness/mod.rs`'s `mod pr08_realtime_listener_reconnect;` inside the existing
`production_readiness` `[[test]]` binary — no new `[[test]]` entry in `crates/embyr-server/Cargo.toml`
was needed (the whole directory is one binary).

## Wave: DISTILL / [REF] Driving Adapter Coverage

The existing gRPC `Listen` streaming RPC (`handle_listen`) is exercised via a real `tonic` client
against the real in-process `TestServer` in every one of the 5 scenarios — no new driving adapter is
introduced by this feature (DISCUSS's own Driving Ports section: "Zero new RPC/HTTP endpoint"). The
existing admin `/metrics` HTTP endpoint is exercised via a real `reqwest` GET with the operator Bearer
key in scenarios 3-5.

## Wave: DISTILL / [REF] Pre-requisites

- Real Docker daemon (testcontainers-rs, Postgres 15-alpine) — same pre-requisite this suite's other
  `pr0N` files and `us_05_listen_realtime.rs` already carry.
- No new environment variable, no new port, no new external dependency (matches DISCUSS's own System
  Constraints and ADR-071's own Positive consequences).

## Wave: DISTILL / [REF] RED-State Verification

Ran against TODAY's unfixed code (`git status` confirms `postgres_notify_listener.rs` untouched by
this DISTILL pass):

- **Scenario 1 (walking skeleton, not `#[ignore]`)** — `cargo test --test production_readiness -p
  embyr-server transient_postgres_blip_does_not_permanently_end_realtime_delivery -- --nocapture`.
  Result: FAILED after 18.48s. Panic: `"expected the SAME Listen stream to deliver a DocumentChange
  event for a write committed after the Postgres blip resolved..."` — an `assert!` firing because
  `next_document_change` timed out waiting for delivery. Classification: **MISSING_FUNCTIONALITY**
  (the listener task dies permanently on the container's first connection drop and never resumes,
  exactly matching today's known `break`-on-`Err` bug) — not a setup/fixture/import error.
- **Scenario 4 (`#[ignore]`, run explicitly via `--ignored`)** —
  `sustained_failure_of_one_project_is_operator_visible_and_does_not_affect_a_second_project`. Result:
  FAILED after 25.89s. Panic: `assertion left == right failed ... left: 0.0, right: 1.0` — the
  `embyr_pg_notify_listener_reconnecting` gauge is absent (defaults to `0.0` in the test's own parser)
  because the metric does not exist in production code yet. Classification: **MISSING_FUNCTIONALITY**.
  This run also empirically validated the two-dedicated-Postgres-container fixture setup and the
  `/metrics` scrape path used by scenarios 3-5.
- **Scenarios 2, 3, 5 (`#[ignore]`)** — not individually executed this pass (token/wall-clock
  discipline: each is a multi-container, multi-second real-Postgres test; scenarios 1 and 4 already
  exercised every distinct helper function these three reuse — `open_listen_stream_and_wait_for_current`,
  `seed_document`, the stop/start cycle, `get_metrics`/`metric_value_for_label` — with zero new
  mechanism introduced). Compile-checked clean (`cargo check --test production_readiness -p
  embyr-server`, zero errors). DELIVER's own RED phase entry gate (ADR-025) re-verifies each
  individually immediately before implementing it, per the one-scenario-at-a-time cycle.
- An earlier draft of Scenario 1 was caught and fixed BEFORE this RED-state confirmation: it used
  `testcontainers-rs`'s default ephemeral port mapping, which Docker reassigns to a new random host
  port on every `start()` after a `stop()` — this produced a false **SETUP_FAILURE** (`"customer
  Postgres container did not become reachable again within 15s"`, because the test kept polling the
  STALE original port). Fixed by mapping a `find_free_port()`-allocated FIXED host port via
  `ImageExt::with_mapped_port` before starting each customer container — confirmed via a standalone
  `docker stop`/`docker start` experiment that a fixed mapping (unlike an ephemeral one) survives
  restart. Re-ran after the fix: genuine MISSING_FUNCTIONALITY red, recorded above.

Docker cleanup verified after every run (`docker ps -a` empty of leftover Postgres containers) — per
this repo's own established `feedback_container_cleanup` practice.

## Wave: DISTILL / [REF] Mandate Compliance Evidence

- **CM-A** (Mandate 1, hexagonal boundary): every scenario imports only `embyr_proto::firestore::*`
  (the gRPC driving port), `embyr_server::{TestServer, start_test_server_with_keepalive,
  adapters::system_db::SystemDb}` (composition-root-level test entry points, same imports
  `us_05_listen_realtime.rs` already uses), and `reqwest` against the admin `/metrics` port. Zero
  imports of `crate::realtime::listen_registry` internals, zero direct construction of
  `PostgresNotifyListener` in test code — the struct is only ever reached indirectly via
  `handle_listen`'s own real provisioning path.
- **CM-B** (Mandate 2, business language): scenario doc-comments and assertion messages use Trailmark/
  Sam Chen/operator-visible/reconnects/delivers-a-DocumentChange language throughout; zero occurrences
  of "HTTP status", "JSON schema", "database row" in Given/When/Then prose (technical terms appear
  only in file-level implementation-note comments, per Mandate 2's Layer 1/Layer 2 split).
- **CM-C** (Mandate 3, user journey completeness): every scenario has a User trigger (an active
  onSnapshot listener), Business logic (a real Postgres connection failure/recovery), and an
  Observable outcome tied to Business value (delivery resumes / stays bounded / becomes visible /
  isolates one project from another) — matches the Correct Example shape in `nw-bdd-methodology`.
- **CM-D** (Mandate 4, pure function extraction): not applicable at the AT layer here — this feature's
  own fix (`reconnect_backoff`) is ALREADY a pure function per ADR-071's own design; DISTILL's
  scenarios exercise it only indirectly through the real adapter's timing/metric behavior (WS/`
  @wiring_e2e` layer, per Layered Test Discipline — traditional assertions, not `assert_state_delta`,
  are correct at this layer). DELIVER's own inner-loop unit tests own direct `reconnect_backoff()`
  PBT coverage (layer 1 — full PBT per Mandate 9).
- **CM-E/F/H** (Mandates 8/9/11): all 5 scenarios run at the WS/`@wiring_e2e` layer (real Postgres
  container, real in-process server, ~seconds each) — per the Layered Test Discipline table this layer
  uses traditional assertions (not `assert_state_delta`) and is example-only (no PBT machinery
  imported anywhere in this file) — correctly matching Mandate 9/11's own layer constraints, not a
  violation of Mandate 8 (Mandate 8 is scoped to layers 1-3; this is layer 4+... actually WS is its own
  named row, "traditional" assertion is explicitly permitted there).
- **CM-G** (Mandate 10): Tier B correctly ABSENT — single-story feature, 5 scenarios total, input space
  is not domain-rich (a connection either fails or doesn't; no generative free-text/date/payload
  space to explore), matching the documented "Skip Tier B" conditions exactly.

## Wave: DISTILL / [REF] Next Wave

**Handoff To**: nw-software-crafter (DELIVER wave).
**Deliverables**: `tests/production_readiness/acceptance/pr08_realtime_listener_reconnect.rs` (5
scenarios: 1 walking skeleton + 4 `#[ignore]`), `tests/production_readiness/mod.rs` updated with the
new module registration, RED-state verification above (2 scenarios empirically confirmed
MISSING_FUNCTIONALITY, 3 compile-checked and structurally identical to the confirmed two). DELIVER
implements ADR-071's Handoff Package (the single-file change to
`postgres_notify_listener.rs`'s spawned-task closure) one scenario at a time, per ADR-025's 3-phase
RED→GREEN→COMMIT cycle, starting from the walking skeleton (already RED) and unskipping scenarios
2 through 5 in order.
