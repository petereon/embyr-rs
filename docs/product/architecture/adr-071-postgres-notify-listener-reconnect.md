# ADR-071: Postgres NOTIFY Listener Reconnect — Reuse sqlx's Built-In Auto-Reconnect, Add Only an Outer Backoff Loop

## Status

Accepted

## Context

`docs/product/production-readiness-audit-2026-09-08.md` finding #4 (Blocker): `PostgresNotifyListener`'s
background task (`crates/embyr-server/src/adapters/postgres_notify_listener.rs:57-72`) ends permanently
— `break`, no retry — the first time `pg_listener.recv().await` returns `Err`. `handle_listen`'s own
listener-provisioning guard (`crates/embyr-server/src/grpc/handler.rs:3544-3567`) only ever checks
`active_listeners.contains_key(&project_id)`; because nothing in the workspace ever calls `.remove()`
against that map (grep-confirmed), a project's real-time delivery dies silently and permanently after
one transient Postgres blip, recoverable today only by restarting the entire multi-tenant
`embyr-server` process.

DISCUSS (`docs/feature/realtime-listener-reconnect/feature-delta.md`) named two candidate shapes and
opened with a recommendation for (a) internal reconnect-with-backoff over (b) die-clean +
external-detection-and-recreate, reasoning that (a) preserves `PostgresNotifyListener`'s own implicit
"struct alive ⇒ task alive" invariant with zero change to `handle_listen`'s guard. DISCUSS's own
reasoning assumed a dropped connection requires creating a **new** `PgListener` and re-issuing `LISTEN`
on reconnect, since Postgres session-scoped `LISTEN` state does not survive a reconnect at the server
level.

### The load-bearing correction

Reading `sqlx-postgres`'s own source (`sqlx-postgres-0.8.6/src/listener.rs`, the version pinned in this
workspace's `Cargo.lock`) shows `PgListener` already implements exactly the reconnect DISCUSS assumed
would need to be built: its own doc comment states *"This listener will auto-reconnect. If the active
connection being used ever dies, this listener will detect that event, create a new connection, will
re-subscribe to all of the originally specified channels, and will resume operations as normal."*
Concretely, `try_recv()` catches four specific IO-error kinds (`ConnectionAborted`, `UnexpectedEof`,
`TimedOut`, `BrokenPipe`), drops the dead connection, and — because `eager_reconnect` defaults to `true`
— immediately re-`pool.acquire()`s a connection and re-issues `LISTEN` for every channel already in its
own internally tracked `channels: Vec<String>` list.

What `sqlx` does **not** do: retry an eager-reconnect attempt that itself fails, or apply any backoff.
If Postgres is still unreachable at the exact instant `connect_if_needed()`'s single `pool.acquire()`
call fires, that error propagates out of `recv()` as `Err` — this, and only this, is the actual gap:
a socket-level blip Postgres itself already survived is already invisible today (never reaches this
codebase's own `Err` arm at all); only an outage that outlasts sqlx's own single eager-reconnect attempt
does.

## Decision

**Keep DISCUSS's approach (a) — internal reconnect-with-backoff — but implement it as a strictly smaller
change than DISCUSS's own reasoning anticipated.** The background task's loop, on `Err`, does not
`break`. It applies a capped exponential backoff sleep, then calls `.recv()` again on the **same**
`pg_listener` binding already captured in the spawned task's closure. No new `PgListener::connect()`, no
new `.listen()` call, no new pool, no new connection is ever created for a reconnect cycle — `sqlx`'s own
`try_recv()` transparently reconnects and re-subscribes on the very next call, using state it already
maintains internally.

```rust
fn reconnect_backoff(consecutive_failures: u32) -> std::time::Duration {
    let shift = consecutive_failures.saturating_sub(1).min(5); // 2^5s = 32s, capped at 30s below
    std::time::Duration::from_secs(1u64 << shift).min(std::time::Duration::from_secs(30))
}
```

1s → 30s exponential doubling, matching JOB-08's own documented (but, confirmed by grep, never actually
implemented anywhere in this workspace) reference precedent for the *other* reconnect mechanism in this
codebase (SaaS↔agent Subscribe-stream reconnect). Written fresh — nothing exists to reuse — but kept
numerically consistent with that precedent rather than inventing an unrelated curve.

Consecutive-failure count also drives operator visibility (AC-RLR-04): a `tracing::error!` + a
`metrics::gauge!("embyr_pg_notify_listener_reconnecting", "project_id" => ...)` fire once
`consecutive_failures` reaches a fixed alert threshold (5, ≈15s of sustained failure), reusing the
already-installed `metrics`/`metrics-exporter-prometheus` surface (ADR-016) and the existing
`project_id`-labeled-counter precedent (`rate_limit.rs`'s `embyr_rate_limit_requests_total`). No new
dependency, no new HTTP endpoint — both new metrics are scraped by the existing `/metrics` handler.
Unlike that rate-limiter precedent, no cardinality-bounding sentinel is needed: this code path is only
ever reached after `authenticate()` has already succeeded, so every `project_id` value is already a
confirmed, provisioned project.

`PostgresNotifyListener`'s struct definition, its `Drop` impl, `start()`'s public signature, and
`handle_listen`'s `contains_key` guard are all **unchanged**. Because the spawned task never ends on a
`recv()` error of any kind, `contains_key(&project_id) == true` remains a permanently valid invariant —
the dead-listener eviction problem named in the audit's own finding #4 does not need a fix under this
design, because it does not exist under this design (confirmed, not assumed away: see Consequences for
what *does* remain unresolved).

## Consequences

### Positive

- Exactly one file changes: `crates/embyr-server/src/adapters/postgres_notify_listener.rs`, and only
  the body of one closure inside it. Zero changes to `handler.rs`, `listen_registry.rs`,
  `listen_handler.rs`, or any public type/signature in this feature's blast radius.
- No new external dependency (backoff and jitter-avoidance both implemented with `std`/`tokio`
  primitives already in use in this file).
- A transient blip Postgres itself already recovers from before this feature's own outer loop is ever
  consulted (the common case) has zero behavior change — `sqlx`'s own eager reconnect already handled
  it, silently, before this feature existed.
- No resource accumulation across any number of reconnect cycles: one task, one dedicated connection
  (via `pg_listener`'s own internal single-connection pool), for the life of the project's
  `active_listeners` entry, regardless of failure count.

### Negative / accepted residuals

- **No jitter.** Each project's `PgListener` targets a dedicated, per-tenant Postgres instance (AD-08),
  so one project's retry schedule cannot correlate with another's — a thundering-herd concern does not
  apply under this architecture's current constraint. If a future `backend_mode` ever shares one
  Postgres instance across multiple projects' listeners, jitter should be added at that time.
- **Project suspension/deletion during an active backoff loop is not addressed.** Neither
  `suspend_project` nor `delete_project` (`crates/embyr-server/src/admin/handlers/lifecycle.rs`)
  references `active_listeners` in any way. A deleted project's listener will retry forever (capped at
  one attempt per ≤30s), never reclaimed for the remaining life of the process. This is a real,
  separate resource-lifecycle gap, explicitly out of scope per DISCUSS's own locked scope (no AC of this
  feature requires it) — recommended follow-up: wire the already-`Arc`-shared `active_listeners` map
  into the admin lifecycle handlers and call `.remove(&project_id)` there, letting the struct's existing
  `Drop`/`abort()` reclaim the task. Not designed further here.
- **`fetch_event`'s fallback-to-`Removed` on query failure** (line 165-168 of the same file) is
  unrelated to and unfixed by this ADR: if the *document fetch* (a separate `sqlx::PgPool` from
  `pg_listener`) fails during the same kind of transient outage this feature targets, a false-delete
  event is synthesized and fanned out, rather than skipped or retried. Pre-existing, not introduced or
  worsened here, not covered by any AC — flagged for a future, narrower fix.
- **`backend_mode=agent`'s own `handle_listen` path is separately broken** (confirmed: `authenticate()`
  returns an empty-string DSN for agent-mode projects, and `handle_listen`'s pool-connect call against
  that empty DSN fails immediately, before `PostgresNotifyListener::start()` is ever reached) — a
  pre-existing, unrelated defect, unaffected by this ADR either way, since this feature's change lives
  entirely inside a closure agent-mode traffic never reaches.
- **`crates/embyr-agent/src/notify_bridge.rs`'s `AgentNotifyBridge::subscribe()`** has the textually
  identical unfixed bug (no retry, no backoff, on `recv()` error) on the agent-side half of JOB-08's own
  mechanism. Out of scope for this ADR; flagged as a natural follow-up that could reuse
  `reconnect_backoff()` once it is promoted to a shared location.

## Alternatives Considered

1. **(b) Die clean + external detection/recreate** (DISCUSS's own non-preferred candidate) — rejected,
   for the reasons DISCUSS itself gave: requires widening `handle_listen`'s `contains_key` guard into a
   liveness check (`JoinHandle::is_finished()`), introduces a query-then-act race between the liveness
   check and use that approach (a) does not have, and requires explicit teardown-then-rebuild machinery
   (new pool, new `PgListener::connect`, new `.listen()`) on every recreate cycle that approach (a)
   never needs.
2. **Build a transient-vs-permanent failure classifier at the `sqlx::Error` layer** — rejected, per
   DISCUSS's own § Business Context: `sqlx::Error` surfaces an identical `Err` variant for a momentary
   network blip, a Postgres restart, and a permanently invalid DSN at this call site; an unreliable
   heuristic risks falsely declaring a real, eventually-recoverable project's listener dead, a worse
   failure mode than the one being fixed.
3. **Re-create a fresh `PgListener` + re-`.listen()` on every reconnect** (the literal reading of both
   the task's own suggested framing and DISCUSS's opening recommendation's stated reasoning) — rejected
   once `sqlx-postgres`'s own source was read directly (§ Context): unnecessary, since `sqlx` already
   performs this internally against the existing `pg_listener` binding. Re-creating it would duplicate
   work `sqlx` already does and would discard the channel-list bookkeeping `sqlx` maintains for exactly
   this purpose.
4. **Log-only visibility (no metric)** — rejected: this repo's own established pattern
   (`rate_limit.rs`'s `rate_limit_pg_timeout` warn! + counter pairing, ADR-016) pairs a log line with a
   scrapable metric for every operationally significant degraded-mode condition; a log-only signal is
   not alertable and fails AC-RLR-04's "distinguishable from a healthy, occasionally-blipping listener"
   requirement at a glance.

## Enforcement

No new automated enforcement mechanism beyond the regression guards named in the feature-delta.md DESIGN
handoff (`tests/acceptance/us_05_listen_realtime.rs`, the `tests/security_rules_realtime/**` suite, and
the recommended new `tests/production_readiness/acceptance/pr08_realtime_listener_reconnect.rs`) — this
is a bounded, single-file, private-closure-body change with no new port/adapter boundary, no new public
signature, and no new external dependency.
