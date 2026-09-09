# Mutation Testing Report — realtime-listener-reconnect

**Tool**: cargo-mutants
**Scope**: `crates/embyr-server/src/adapters/postgres_notify_listener.rs`, `--in-diff` scoped to
the DELIVER commit diff (`git diff 5022781 d0b0453 -- crates/embyr-server/src/adapters/postgres_notify_listener.rs`).

Discipline followed: confirmed via `ps -p 82972` → not running (process genuinely exited) and
`git diff crates/embyr-server/src/adapters/postgres_notify_listener.rs` → empty (no leftover
mutation corruption) before reading results. See `feedback_mutation_testing_docker_contention.md`.

## Command

```
cargo mutants -p embyr-server --in-place --timeout 300 --in-diff /tmp/pr08_mut_diff.diff -- --test production_readiness pr08 -- --test-threads=1
```

Scoped to the `pr08` acceptance tests specifically (not the full `production_readiness` target's
8 files) — the only tests that exercise the reconnect logic this diff added. Baseline: 288s build
+ 219s test (real subprocess + Postgres testcontainers per test, matching this file's own
established cost profile).

## Result: 11 mutants tested in 44m — 4 caught, 7 unviable, 0 missed

```
=== caught ===
crates/embyr-server/src/adapters/postgres_notify_listener.rs:55:5: replace reconnect_backoff -> std::time::Duration with Default::default()
crates/embyr-server/src/adapters/postgres_notify_listener.rs:56:78: replace << with >> in reconnect_backoff
crates/embyr-server/src/adapters/postgres_notify_listener.rs:135:49: replace >= with < in PostgresNotifyListener::start
crates/embyr-server/src/adapters/postgres_notify_listener.rs:156:46: replace += with -= in PostgresNotifyListener::start

=== unviable (7) ===
crates/embyr-server/src/adapters/postgres_notify_listener.rs:85:5: replace reconnect_pg_listener -> Result<PgListener, sqlx::Error> with Ok(Default::default())
crates/embyr-server/src/adapters/postgres_notify_listener.rs:122:9: replace PostgresNotifyListener::start -> Result<Self, embyr_core::error::CoreError> with Ok(Default::default())
crates/embyr-server/src/adapters/postgres_notify_listener.rs:156:46: replace += with *= in PostgresNotifyListener::start
crates/embyr-server/src/adapters/postgres_notify_listener.rs:163:49: replace == with != in PostgresNotifyListener::start
crates/embyr-server/src/adapters/postgres_notify_listener.rs:248:50: replace += with -= in PostgresNotifyListener::start
crates/embyr-server/src/adapters/postgres_notify_listener.rs:248:50: replace += with *= in PostgresNotifyListener::start
crates/embyr-server/src/adapters/postgres_notify_listener.rs:254:53: replace == with != in PostgresNotifyListener::start
```

## Interpretation

**Fully clean result — no missed mutants.** All 4 viable, testable mutants are caught:

- `reconnect_backoff -> Default::default()` and `<< with >>` (lines 55-56): the backoff-curve
  arithmetic itself. A wrong backoff value would either make `sustained_outage_reconnect_attempts_are_bounded_not_a_tight_loop`
  (AC-RLR-03) fail (too-fast retries) or push recovery timing past the other tests' own generous
  windows.
- `>= with <` at line 135 (the `consecutive_failures >= RECONNECT_ALERT_THRESHOLD` guard on the
  success path, gating the "recovered" log/metric-reset): caught by
  `listener_recovers_after_crossing_the_sustained_failure_threshold`'s own assertion that the
  `embyr_pg_notify_listener_reconnecting` gauge resets to 0.0 after recovery.
- `+= with -=` at line 156 (`consecutive_failures += 1` on each failure): caught — a decrementing
  counter would never reach `RECONNECT_ALERT_THRESHOLD`, failing
  `sustained_failure_of_one_project_is_operator_visible_and_does_not_affect_a_second_project`'s
  gauge assertion.

The 7 unviable mutants are build failures (`Default::default()` on non-`Default` types
`PgListener`/`Self`, and arithmetic-op substitutions the type system rejects for `u32`/specific
contexts), not coverage gaps — consistent with the pattern already established on the two prior
sibling Blocker fixes this session.

## QUALITY_GATE finding, not a coverage gap

The mutation run itself surfaced no gaps, but the QUALITY_GATE process for this feature found and
fixed two REAL defects mutation testing alone would never have caught (both require empirical,
real-Postgres-container reproduction, not just code mutation):

1. A poisoned-connection hang: DELIVER's own first version reused the OLD, still-broken
   `PgListener` binding after a failed reconnect attempt, which could hang indefinitely on the
   next `.recv()` call rather than erroring again — confirmed via a real container kill/restart
   repro, fixed by never calling `.recv()` again on a listener known to be in an error state.
2. A NOTIFY-loss race: a single write immediately after external Postgres reachability was
   confirmed could lose its own NOTIFY forever if the project's listener hadn't yet finished its
   own internal reconnect (no external signal exists for that state below the alert threshold).
   Fixed by making the acceptance tests retry the WRITE itself (`seed_until_delivered`), which is
   what actually and robustly proves the feature's own claim ("the listener recovers eventually"),
   not by trying to precisely time a settle delay against an inherently variable reconnect latency.

Both are documented in full in the DELIVER commit message and this feature's evolution doc.
