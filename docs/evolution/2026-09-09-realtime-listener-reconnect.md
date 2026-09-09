# Evolution: realtime-listener-reconnect

**Date:** 2026-09-09
**Feature:** Postgres LISTEN/NOTIFY real-time listener now recovers automatically from a
transient connection failure instead of dying silently and permanently for the life of the
process.
**Job:** JOB-03 (`live-sync`) — reused, secondary persona P2 Sam Chen noted.
**ADRs:** ADR-071 (new, amended during DELIVER) — internal reconnect-with-backoff mechanism.

## This closes finding #4 from `docs/product/production-readiness-audit-2026-09-08.md`

## Business Context

`PostgresNotifyListener`'s background task `break`d out of its loop on ANY `recv()` error —
network blip, Postgres restart, connection drop — silently ending real-time delivery for that
project forever. Worse, `handler.rs`'s `handle_listen` used a `contains_key`-only guard on
`active_listeners` with zero `.remove()` calls anywhere in the workspace, so no future `Listen`
RPC for the same project would ever create a replacement listener — the only recovery path was a
full process restart. Confirmed independently by two audit agents (SRE + database).

## Key Decisions

| Decision | Verdict |
|---|---|
| D1 | Internal reconnect-with-backoff (the task never dies) over external death-detection-and-recreate — matches the existing `Drop`-based lifecycle invariant, zero change to `handler.rs`'s `contains_key` guard |
| D2 (amended during DELIVER) | Never trust `sqlx::PgListener`'s own internal auto-reconnect — it only self-heals for 4 specific IO error kinds, and empirically a real container kill surfaces `ConnectionReset`, outside that set. Always build a fresh `PgListener`/pool on any error via `reconnect_pg_listener()` |
| D3 | 1s→30s capped exponential backoff, no jitter (dedicated per-project connections mean retries never correlate across tenants) |
| D4 | Reuse the existing Prometheus/`metrics` registry pattern (ADR-016, `rate_limit.rs` precedent) for operator visibility — new counter + gauge, escalate to `tracing::error!` after 5 consecutive failures |
| D5 (found during DELIVER, orchestrator fix) | Never call `.recv()` again on a `PgListener` known to be in an error state — a failed reconnect attempt must retry `reconnect_pg_listener()` directly in its own loop, not fall back to the outer `.recv()` loop on the old, poisoned listener |

## Steps Completed

Full nWave subagent pipeline (DISCUSS=`nw-product-owner`, DESIGN=`nw-solution-architect` with peer
review, DISTILL=`nw-acceptance-designer`, DELIVER=`nw-software-crafter`), followed by an unusually
deep orchestrator-led QUALITY_GATE investigation:

1. **DISCUSS**: reused JOB-03 (`live-sync`), corrected the audit finding's own "pool leaks
   forever" framing (drop semantics likely already free the resources; the real, unambiguous harm
   is that no NEW listener can ever be created again for that project). Recommended internal
   reconnect-with-backoff as the opening design direction.
2. **DESIGN**: peer-reviewed (0 critical, 1 high, 1 medium, 1 low, all resolved in one pass).
   Read `sqlx-postgres`'s own source and found `PgListener` already internally reconnects for
   common error kinds — initially concluded the fix could be a ~15-line change reusing that
   internal mechanism. Picked the backoff curve, metrics, and confirmed the `active_listeners`
   eviction problem no longer applies once the task never dies.
3. **DISTILL**: wrote 5 acceptance tests covering AC-RLR-01 through 05. Confirmed correct RED.
4. **DELIVER**: implemented DESIGN's mechanism, then empirically discovered (via real container
   kill/restart testing) that `PgListener`'s auto-reconnect only covers 4 of many possible IO
   error kinds — `ConnectionReset` (what a real container kill reliably produces) isn't one of
   them. Amended the design to always force a fresh connection via `reconnect_pg_listener()`,
   never trusting the old binding. Reported 5/5 reliable passes.
5. **Orchestrator's full-workspace regression**: clean (1 unrelated `PortNotExposed` flake,
   confirmed transient via isolated rerun — the same already-documented Docker-contention class
   hit 6+ times this session).
6. **Orchestrator's own empirical QUALITY_GATE investigation** (the most eventful part of this
   feature): while verifying DELIVER's own reported reliability, found the walking-skeleton test
   failing non-deterministically. Root-caused via real debug tracing (not guessing) to a genuine,
   deeper bug: DELIVER's own retry logic reused the OLD, still-poisoned `PgListener` after a
   failed reconnect attempt, looping back to `.recv()` on it — which `sqlx`'s own source confirms
   never clears its dead connection handle for `ConnectionReset`, and was empirically observed to
   sometimes hang indefinitely rather than error again. Fixed directly: failed reconnect attempts
   now retry `reconnect_pg_listener()` in their own dedicated loop, never falling back to `.recv()`
   on a known-poisoned listener. Verified via repeated isolated reruns (3/3, then more) before and
   after the fix.
7. **A second, distinct empirical finding**: even after the hang fix, tests remained
   intermittently flaky. Traced (again via real debug tracing, not assumption) to a structural
   test-design gap, not a production bug: a write committed immediately after external Postgres
   reachability is confirmed can lose its own NOTIFY forever if this project's listener hasn't
   yet finished its own internal reconnect — no external signal exists for that state below the
   alert threshold. Fixed by rewriting the acceptance tests to retry the WRITE itself
   (`seed_until_delivered`), not just the wait — the only way to robustly test "the listener
   recovers eventually" without being sensitive to the listener's own variable reconnect latency.
8. Verified 5/5 pr08 tests pass reliably across 2 full combined runs plus many isolated reruns;
   existing regression guards (`us_05_listen_realtime`, `drl_b15`) unmodified and green.
9. **QUALITY_GATE**: scoped mutation run, 4/4 caught, 7 unviable, 0 missed — fully clean.

## Lessons Learned

1. **A subagent's own "N/N reliable" empirical claim still needs independent re-verification
   before trusting it as QUALITY_GATE evidence — especially for timing-sensitive, real-I/O
   background-task recovery logic.** DELIVER's own testing (5/5) did not surface either of the
   two real bugs the orchestrator's own re-verification found; both required actual debug tracing
   against a real container kill/restart, not just re-running the same test suite. This doesn't
   mean subagent reports are unreliable in general — it means recovery/retry logic touching real
   external state (network, containers, timing) deserves the orchestrator's own hands-on
   verification pass specifically, not just a scoped-test rerun, before accepting it as done.
2. **A library's own documented "auto-reconnect" claim needs verification against its actual
   source for the SPECIFIC error class your own failure scenario produces, not accepted at face
   value.** `sqlx::PgListener`'s doc comment says it "automatically reconnects" — true only for 4
   of many IO error kinds. A real container kill (the exact scenario this feature exists to fix)
   produces `ConnectionReset`, outside that set. This is the same "verify a design snippet against
   real library behavior, don't trust the docs verbatim" lesson from the sibling
   `stripe-webhook-body-limit` feature, now confirmed for a second, structurally different library
   behavior in the same session.
3. **When testing eventually-consistent recovery, retry the ACTION under test, not just the
   WAIT.** A single write-then-wait test for "does the system recover" is inherently flaky when
   the system's own recovery latency is variable and the specific action can be silently and
   permanently lost in a narrow race window (NOTIFY has no replay). Retrying the write itself
   (`seed_until_delivered`) tests the actual claim ("eventually recovers") robustly, without either
   weakening the assertion or chasing ever-larger fixed timeout constants that still occasionally
   miss.
4. **`PortNotExposed`-class Docker-contention flakes have now hit 6+ distinct tests this session**
   — always triaged via isolated rerun before dismissing, per `feedback_triage_before_dismissing_as_flaky.md`.

## Key Files

- `crates/embyr-server/src/adapters/postgres_notify_listener.rs` — the only production file
  changed; reconnect loop, `reconnect_backoff()`, `reconnect_pg_listener()`, new metrics.
- `docs/product/architecture/adr-071-postgres-notify-listener-reconnect.md` — includes the DELIVER
  amendment documenting the `sqlx` auto-reconnect correction.
- `tests/production_readiness/acceptance/pr08_realtime_listener_reconnect.rs` (new) — 5 tests:
  `transient_postgres_blip_does_not_permanently_end_realtime_delivery`,
  `repeated_transient_blips_do_not_duplicate_delivered_changes`,
  `sustained_outage_reconnect_attempts_are_bounded_not_a_tight_loop`,
  `sustained_failure_of_one_project_is_operator_visible_and_does_not_affect_a_second_project`,
  `listener_recovers_after_crossing_the_sustained_failure_threshold`. Includes the
  `seed_until_delivered` retry helper.
- `docs/feature/realtime-listener-reconnect/deliver/mutation/mutation-report.md` — full account.

## Follow-Up Work (found but explicitly out of scope for this feature)

- `backend_mode=agent`'s own `handle_listen` path is separately broken today (empty-string DSN) —
  not touched.
- `crates/embyr-agent/src/notify_bridge.rs`'s `AgentNotifyBridge::subscribe()` has the textually
  identical unfixed reconnect bug this feature just fixed on the server side — a real, separate
  finding worth its own future item; could reuse this feature's own `reconnect_backoff()` shape.
- `fetch_event`'s fallback synthesizes a false-delete on a transient query failure (pre-existing,
  flagged by DESIGN's own review, not touched).
- `embyr-pg-storage`'s own duplicate `PostgresNotifyListener` struct is dead code (flagged by
  DESIGN's review, not touched — a candidate for the bloat-list follow-up).
- Findings #5-#8 from the same audit remain: fake composite-index creation; missing 168h
  soft-delete purge sweeper; hardcoded-None aws/gcp secret fetchers; no build/release path for
  embyr-agent.
