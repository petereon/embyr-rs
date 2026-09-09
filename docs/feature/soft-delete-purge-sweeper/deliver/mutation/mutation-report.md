# Mutation Testing Report — soft-delete-purge-sweeper

**Tool**: cargo-mutants
**Scope**: `crates/embyr-server/src/sweepers/soft_delete_purge_sweeper.rs`, `--in-diff` scoped to
the DELIVER commit diff (`git diff 7dbc788 580b37c -- crates/embyr-server/src/sweepers/soft_delete_purge_sweeper.rs`).

Discipline followed: confirmed via `ps -p 43254` → not running (process genuinely exited) and
`git diff --stat` on the file → empty (no leftover mutation corruption) before reading results.
See `feedback_mutation_testing_docker_contention.md`.

Also confirmed and worked around during this QUALITY_GATE: a background `cargo-sweep` process
escalated to a full `cargo clean` (125GB, 413K files) mid-way through the preceding full-workspace
regression run, corrupting in-flight build artifacts (`could not execute process ... never
executed`, then `failed to open object file`). Confirmed via `cargo-sweep.log` this was external
environmental interference, not a code regression, and re-ran the full regression cleanly from
scratch once the sweep genuinely finished.

## Command

```
cargo mutants -p embyr-server --in-place --timeout 300 --in-diff /tmp/sdp_mut_diff.diff -- \
  --test soft_delete_purge_sweeper_us01_purge_credentials_after_grace_window --lib -- --test-threads=1
```

## Result: 11 mutants tested in 35m — 4 caught, 5 unviable, 2 missed (1 fixed, 1 confirmed cosmetic)

```
MISSED   soft_delete_purge_sweeper.rs:57:23: replace != with == in spawn
MISSED   soft_delete_purge_sweeper.rs:106:23: replace > with >= in run_cycle
```

## Interpretation

**Miss #1 (`locked != Some(true)` → `==`, line 57) — a genuine gap, investigated empirically,
fixed.** This is the core advisory-lock serialization guard: only a genuinely-acquired lock
(`Some(true)`) should let this instance proceed to `run_cycle`. Rather than assume this was a
scoping artifact, the mutation was manually applied to the file and run against the real
acceptance test with temporary debug tracing added. Result: the test still passed, but for an
accidental reason, not a real guarantee — tracing showed:

```
tick 1: locked = Some(true)  → (mutant) skip, WITHOUT calling pg_advisory_unlock
tick 2: locked = Some(false) → (mutant) since Some(false) != Some(true), falls through and runs
```

Skipping the lock's own `pg_advisory_unlock` call on tick 1 (via the mutated `continue`) leaves
the lock held by tick 1's connection, which is then returned to the pool still holding it. Tick
2's `system_db.pool().acquire()` happened to return a DIFFERENT physical connection, which then
failed to acquire the (still-held) lock — and the *inverted* condition happens to treat "failed to
acquire" as "proceed," stumbling into `run_cycle` anyway. This is an artifact of this specific
test's small connection-pool churn, not a real correctness guarantee — in production, with a
larger, busier connection pool less likely to reuse the exact same physical connection across
ticks in the same pattern, the inverted mutant could plausibly leave the sweeper never running at
all. **Fixed**: extracted the decision into a pure `should_run_cycle(locked: Option<bool>) -> bool`
function with a direct 3-case unit test (`Some(true)`/`Some(false)`/`None`), giving complete,
deterministic coverage independent of connection-pool timing.

**Miss #2 (`purged > 0` → `>= 0`, line 106) — investigated, confirmed cosmetic, not fixed.**
`purged: u64` (from `rows_affected()`) can never be negative, so `purged >= 0` is unconditionally
true. The only behavioral difference: `metrics::counter!(...).increment(purged)` is called with
`purged = 0` on every empty cycle instead of being skipped — but incrementing a counter by 0 is a
value-identical no-op (the counter's own numeric value is unaffected either way), so no test
asserting on the counter's value could ever distinguish the two. The sole observable difference is
an extra, harmless `tracing::info!("...purged...")` log line firing on cycles that purged nothing
— log noise, not a correctness or security issue. Not fixed; documented as an accepted, low-value,
effectively-equivalent mutant (cargo-mutants has no `unviable`-style classification for semantic
equivalence, only build-failure).

## Final state

7/7 acceptance scenarios green after the fix. Full `cargo test -p embyr-server --lib`: 75/75 (74
pre-existing + the new `should_run_cycle` unit test).
