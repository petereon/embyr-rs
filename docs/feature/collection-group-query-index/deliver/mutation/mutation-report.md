# Mutation Testing Report — collection-group-query-index

**Tool**: cargo-mutants 27.0.0
**Scope**: `--in-diff` against `git diff ba8fce2..b32ebfb -- crates/embyr-pg-storage/src/encoding/query.rs
crates/embyr-pg-storage/src/backend_adapter.rs` (`ba8fce2` = pool-sizing-and-limits' own FINALIZE
commit, confirmed the direct parent of `b32ebfb` via `git log --oneline b32ebfb~3..b32ebfb`).
Production logic only — `crates/embyr-pg-storage/src/encoding/query.rs`
(`push_all_descendants_predicate`, `is_probe_stale`) and `crates/embyr-pg-storage/src/backend_adapter.rs`
(`backfill_collection_id`, `ensure_collection_group_indexes`, `build_index_concurrently_if_not_exists`,
`schema_capability`, the cache-carrying constructors, and the 4 `run_query`/`run_aggregation_query`
call sites' wiring to `push_all_descendants_predicate`). Excludes the 4 new test files, the
migration SQL, and `tests/common/mod.rs` — cargo-mutants only mutates Rust production code, and
`--in-diff` naturally restricts to the diff's touched spans.

Closes finding #17 (High, Database) from `docs/product/production-readiness-audit-2026-09-08.md`.
DELIVER commit: `b32ebfb`.

**RAM constraint discipline followed** (8GB machine, Docker VM reserves 4GB fixed):

- `--in-place` throughout — never `--test-workspace`, never `--workspace` on any cargo or
  cargo-mutants invocation. Every run used `-p embyr-pg-storage` only.
- `--in-place` refuses `--jobs` outright (the flag itself makes `-j >1` impossible to pass by
  accident) — confirmed by a real `error: the argument '--in-place' cannot be used with '--jobs
  <JOBS>'` on the first launch attempt, corrected by dropping the flag (cargo-mutants' own default
  is already 1 concurrent mutant).
- `--test-threads=1` on every underlying test invocation.
- `docker ps -a` checked clean (0 containers) before the first run and after the final run.
- Only 4 test binaries exist in `embyr-pg-storage/tests/` (`cgi_backfill`, `cgi_index_usage`,
  `cgi_schema_skew`, `cgi_trigger`) — no `-C`/`--cargo-arg` build-scoping needed (unlike
  pool-sizing-and-limits' `embyr-server`, which had ~150 `[[test]]` targets); a full
  `-p embyr-pg-storage` build already builds only what's relevant.

## Incident 1: CLI arg parsing — libtest args need a second `--`

First launch used `-- --test-threads=1`, which cargo-mutants forwarded straight onto `cargo test`'s
own arg list instead of past `cargo test`'s own `--`, producing `error: unexpected argument
'--test-threads' found` and failing the unmutated-baseline check before any mutant ran. Fixed by
double-dashing: `-- -- --test-threads=1` (first `--` ends cargo-mutants' own args / starts the test
*name* filter, which we leave empty; second `--` starts real libtest args).

## Incident 2: `--in-diff` requires exact source/diff line match

After the QUALITY_GATE fix below (extracting `index_build_succeeded`, which shifted line numbers
in `backend_adapter.rs`), the ORIGINAL `ba8fce2..b32ebfb` diff no longer matched the working tree
verbatim at line 387 (`WARN Diff content doesn't match source file` / `ERROR ... The diff might be
out of date with this source tree`) — `--in-diff` matches by literal line content, not just line
number. Fixed by regenerating the diff as `git diff ba8fce2 -- <same 2 files>` (baseline commit vs.
current working tree, capturing the fix in scope) before the final confirmation run.

## Command (final, properly-scoped run)

```
cargo mutants -p embyr-pg-storage --in-place \
  --in-diff cgqi_v2.diff \
  --timeout 200 --build-timeout 90 \
  -- -- --test-threads=1
```

`--timeout 200` was set from a real measurement, run by hand first: `cargo test -p embyr-pg-storage
--test cgi_backfill --test cgi_index_usage --test cgi_schema_skew --test cgi_trigger --
--test-threads=1` (all 4 real-Postgres-testcontainers integration targets, single-threaded) —
**24.07s total** (`cgi_backfill` 6.95s, `cgi_index_usage` 8.33s, `cgi_schema_skew` 6.50s,
`cgi_trigger` 1.27s). cargo-mutants' own unmutated-baseline check independently measured **5s
build + 24-28s test** across the 3 runs, confirming the hand measurement. 200s is a ~8.3x margin
over that baseline — inside the task's 5-10x guidance, and comfortably above every individual
mutant's observed 25-30s test-phase duration. No mutant hit the timeout ceiling (0 timeouts across
both the initial and final runs) — none of this feature's mutations (counter drift, boolean-logic
inversions, cache-guard bypass) produce an infinite loop: the one candidate that could have
(`backfill_collection_id`'s own `rows_affected == 0` loop-exit condition, mutated to `!=`) instead
exits the loop *early* (after the first non-empty batch) rather than never exiting, so it fails
fast via a wrong row count, not a hang.

## Result (final run): 26 mutants — 21 caught, 0 missed, 5 unviable, 0 timeout

```
Found 26 mutants to test
ok       Unmutated baseline in 5s build + 28s test
26 mutants tested in 5m: 21 caught, 5 unviable
```

(The first run, before the QUALITY_GATE fix below, found 24 mutants — 16 caught, 3 missed, 5
unviable; the fix's own pure-function extraction added 2 more candidate mutants — whole-function
stub `true`/`false` on the new `index_build_succeeded` — both caught in the final run.)

### Unviable (5) — confirmed via each mutant's own compile log, not assumed

| File:line | Compile error |
|---|---|
| `backend_adapter.rs:78:9` — `with_pool_config` whole-fn stub → `Ok(Default::default())` | `E0277`: `PostgresBackendAdapter: Default` not satisfied (holds a bare `PgPool` + `tokio::sync::RwLock<CachedSchemaCapability>`, neither `Default`-derivable through `sqlx`'s `Pool`) |
| `backend_adapter.rs:89:9` — `new_from_pool` whole-fn stub → `Default::default()` | same `E0277`, same struct |
| `backend_adapter.rs:427:9` — `schema_capability` whole-fn stub → `Default::default()` | `E0277`: `SchemaCapability` has no `Default` impl (deliberately — `Available`/`Unavailable` is a meaningful 2-variant enum, not a "zero value" type) |
| `backend_adapter.rs:844:9` — `run_query` whole-fn stub → `Ok(vec![Default::default()])` | `E0277`: `FirestoreDocument: Default` not satisfied |
| `backend_adapter.rs:1056:9` — `run_aggregation_query` whole-fn stub → `Ok(Default::default())` | `E0277`: `AggregateValue` has no `Default` impl (a tagged union of `Count`/`Sum`/`Avg`, correctly not "defaultable") |

All 5 are cargo-mutants' generic "stub the whole function body with `Default::default()`" candidate
generator guessing wrong against types this codebase deliberately keeps non-`Default` — the same
pattern documented in `pool-sizing-and-limits`' and `healthz-dependency-checks`' own reports.
Candidate-generation noise, not test-suite weakness.

### Caught (21) — every genuinely viable mutant in scope

The full list (`mutants.out/caught.txt`) spans every new decision point this feature added:
`backfill_collection_id`'s loop-exit condition and both summary counters, `migrate`/
`ensure_collection_group_indexes`/`build_index_concurrently_if_not_exists`'s whole-fn stubs,
`index_build_succeeded`'s boolean logic (both operands and both arms — see QUALITY_GATE fix
below), `schema_capability`'s cache-hit match arm and its TTL-freshness guard (3 separate mutants:
guard→`true`, guard→`false`, `!`-deletion — all 3 caught after the fix), `run_query`'s whole-fn
stub, and `push_all_descendants_predicate`/`is_probe_stale`'s own logic in `query.rs`.

## QUALITY_GATE fix — 3 real gaps found and closed, 0 production-code changes

The first run surfaced 3 missed mutants, each investigated against real test coverage before
deciding real-gap vs. accepted-noise (this session's established discipline). All 3 were **real
gaps**, not noise — closed with test-only changes (one pure-function extraction for testability, no
behavior change) and re-verified via the final run above.

### 1. `backend_adapter.rs:317:33` — `summary.batches_run += 1` → `*=` — MISSED

`BackfillSummary::batches_run` starts at `0` (`#[derive(Default)]`); `0 *= 1` is a silent no-op —
the mutant leaves `batches_run` permanently `0` regardless of how many real batches ran. No test in
`cgi_backfill.rs` asserted on `summary.batches_run` at all — every test only checked
`rows_backfilled`. **Real gap**: a caller relying on `batches_run` (e.g. `embyr-db-prep`'s own
progress reporting, per ADR-080 Decision B) would silently see `0` forever.

**Fix**: added one assertion to the existing
`backfilling_pre_existing_documents_does_not_block_concurrent_writes` test —
`assert_eq!(summary.batches_run, 6, ...)` (30 pre-existing rows ÷ batch_size 5 = exactly 6 batches;
deterministic because the concurrent writes in that test target *new* documents with `collection_id`
already set by the trigger, never contending with the backfill's `WHERE collection_id IS NULL`
row-selection). No new test function — the existing test already runs the exact scenario.

### 2. `backend_adapter.rs:387:34` — `create_result.is_ok() && valid` → `||` — MISSED

This is ADR-072's own "authoritative `indisvalid` re-check" pattern (`composite_index_builder.rs`),
reused here for the collection-group indexes — but reused as an **inlined boolean expression**
rather than ADR-072's own extracted-and-unit-tested `build_succeeded(create_ok, indisvalid) -> bool`
pure function. With `||`, a leftover *invalid* index from an earlier interrupted `CREATE INDEX
CONCURRENTLY` build (`indisvalid = false` in `pg_index`, but a subsequent idempotent `IF NOT EXISTS`
re-run reports `create_result.is_ok() == true` since the statement itself is a no-op success) would
be silently accepted as "built successfully" — never dropped, never retried — leaving a genuinely
broken index in place that the planner will never use. This is exactly the failure mode ADR-072's
own doc comment calls out ("a stale leftover index under the same deterministic name could read
`indisvalid=true` after a failed create" — the *converse* case, `create_ok=true` with
`indisvalid=false`, is equally dangerous and was the one this mutant exposed as uncovered). **Real
gap**, and directly relevant to the task's flagged highest-risk area (index/connection-scoping
correctness around `ensure_collection_group_indexes`).

**Fix (root cause, not the symptom)**: extracted the exact same pure function ADR-072 already
established, `index_build_succeeded(create_ok: bool, indisvalid: bool) -> bool`, right next to
`build_index_concurrently_if_not_exists` in `backend_adapter.rs`, plus a direct 4-case truth-table
unit test (`index_build_succeeded_tests::succeeds_only_when_create_ok_and_indisvalid_both_hold`),
mirroring `composite_index_builder.rs`'s own `build_succeeded`/`ready_requires_both_create_ok_and_indisvalid`
byte-for-byte in shape. This is the 4th instance this session of "extract pure decision fn closes a
mutation gap that an inlined boolean expression can't be unit-tested against without a database"
(`soft_delete_purge_sweeper`, `occ_precondition_validation`, `agent_field_path_validation` were the
prior three). No production behavior changed — `create_ok && indisvalid` is still exactly what
runs; it is now just independently testable and directly tested.

### 3. `backend_adapter.rs:417:24` (later `432:24`) — TTL-freshness match guard → `false` — MISSED

`schema_capability`'s `Unavailable { checked_at } if !is_probe_stale(checked_at, unavailable_ttl)`
guard is what makes a cached-`Unavailable` read skip re-probing `information_schema` within the
TTL window. Mutated to a hardcoded `false`, the guard never matches — every single call, cached or
not, falls through to the wildcard arm and re-probes the catalog. **This is exactly the kind of
gap the task asked to take seriously**: a broken cache short-circuit here doesn't produce a wrong
*answer* (an un-migrated database is genuinely `Unavailable` either way, so
`schema_capability_picks_up_a_migration_landing_mid_session_within_the_ttl` still passed), it
produces a wrong *cost* — silently re-issuing an `information_schema` catalog query on every
collection-group query's hot path, defeating the entire point of ADR-080 Decision C's cache. The
existing AC-CGI-13 performance-proxy test (`cached_schema_capability_reads_are_negligible_versus_uncached_catalog_probes`)
only warmed the cache to `Available` (the *other* match arm, unaffected by this guard) — the
`Unavailable`-branch cache path had zero cost-based coverage.

**Fix**: added a second, mirrored performance-proxy test,
`cached_unavailable_reads_are_negligible_versus_uncached_catalog_probes`, in `cgi_schema_skew.rs` —
identical `p99_of`-based structure to the existing `Available`-branch test, but against
`customer_db_missing_latest_migration()` so the cache warms to `Unavailable` instead. This directly
proves the TTL-freshness guard's cache short-circuit engages for *both* branches, not just one.

## Final state

3 real gaps found by the first run, all closed with test-only changes (1 pure-function extraction
for direct unit-testability + 1 new assertion + 1 new performance-proxy test — zero production
behavior changes, confirmed by `cargo check -p embyr-pg-storage --tests` and by every existing test
still passing unmodified). Final confirmation run: **26 mutants, 21 caught (100% of viable
mutants), 0 missed, 5 unviable (100% compiler-rejected, source-verified above), 0 timeout**.
Docker: 0 containers before the first run and after the final run (`docker ps -a` clean both
times, checked directly, no leaks across 3 cargo-mutants invocations + 2 incidents).

**Disposition summary**: 21 caught, 0 missed, 5 unviable, 0 timeout. Finding #17 (High, Database)
closed with no residual mutation-testing gap.
