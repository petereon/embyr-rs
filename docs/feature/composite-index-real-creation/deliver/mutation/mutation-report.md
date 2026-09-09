# Mutation Testing Report — composite-index-real-creation

**Tool**: cargo-mutants
**Scope**: `--in-diff` scoped to the 4 highest-value new-logic files from the DELIVER diff:
`composite_index_ddl.rs`, `composite_index_builder.rs`, `customer_db_connect.rs`,
`admin/handlers/composite_indexes.rs`. Excluded from scope: `system_db.rs`/`admin/state.rs`/
`admin/router.rs` (thin wiring additions), `transaction_sweeper.rs` (import-only change, its own
existing 11-test suite already covers it unmodified).

Discipline followed: confirmed via `ps -p 73938` → not running (process genuinely exited) and a
`git diff --stat` on all 4 scoped files → empty (no leftover mutation corruption) before reading
results. See `feedback_mutation_testing_docker_contention.md`.

## Command

```
cargo mutants -p embyr-server --in-place --timeout 300 --in-diff /tmp/cxr_mut_diff.diff -- \
  --test composite_index_real_creation_cxr01_real_index_creation_and_usage \
  --test composite_index_real_creation_cxr02_non_blocking_build \
  --test composite_index_real_creation_cxr03_delete_removes_real_index --lib -- --test-threads=1
```

First attempt aborted with "cargo test failed in an unmutated tree" — the baseline itself hit a
`ConnectionReset`/`PoolTimedOut` under real, independently-confirmed system memory/Docker
contention at the time (the harness had just killed a wait-loop shell for low memory). Confirmed
via isolated rerun that the baseline test passes cleanly once Docker was cleaned and load settled,
then relaunched. Second attempt completed cleanly.

## Result: 37 mutants tested in 3h — 22 caught, 8 unviable, 7 missed (1 fixed, 6 pre-existing/scoping)

```
MISSED   composite_index_builder.rs:144:30: replace && with || in run_build
MISSED   customer_db_connect.rs:65:5: replace resolve_aws_secret_dsn -> Option<String> with None
MISSED   customer_db_connect.rs:65:5: replace resolve_aws_secret_dsn -> Option<String> with Some(String::new())
MISSED   customer_db_connect.rs:65:5: replace resolve_aws_secret_dsn -> Option<String> with Some("xyzzy".into())
MISSED   customer_db_connect.rs:87:5: replace resolve_gcp_secret_dsn -> Option<String> with None
MISSED   customer_db_connect.rs:87:5: replace resolve_gcp_secret_dsn -> Option<String> with Some(String::new())
MISSED   customer_db_connect.rs:87:5: replace resolve_gcp_secret_dsn -> Option<String> with Some("xyzzy".into())
```

## Interpretation

**1 genuine, new-logic gap — investigated and fixed.** `composite_index_builder.rs:144`
(`create_result.is_ok() && valid` → `"ready"`) is ADR-072 Decision B's own authoritative
ready/failed signal — the single most correctness-critical line in this feature. The only existing
failure-path test (`a_build_against_an_unreachable_backend_mode_ends_in_a_distinct_failed_status`)
makes BOTH operands `false` (unreachable backend ⇒ `create_result` errors AND the `pg_index` query
also fails, defaulting `valid` to `false` via `.unwrap_or(false)`), so `&&` and `||` produce the
identical result in that one scenario — the mutant survives because no test forces an *asymmetric*
state (one true, one false), which is exactly the mid-build-connection-drop race ADR-072 names as
the reason this re-check exists at all. Rather than attempt to reliably simulate that exact race
against a real Postgres container (a mid-build connection drop that leaves `create_result` `Ok`
but the index `INVALID` is a narrow, timing-dependent Postgres behavior, hard to force
deterministically), the decision was extracted into a pure `build_succeeded(create_ok: bool,
indisvalid: bool) -> bool` function with a direct unit test covering all 4 boolean combinations —
complete, deterministic coverage an integration test could not reliably provide. Fixed in a
follow-up commit; `ready_requires_both_create_ok_and_indisvalid` (1/1 pass) now proves the exact
invariant the `&&`→`||` mutant would violate.

**6 misses on `resolve_aws_secret_dsn`/`resolve_gcp_secret_dsn` — investigated, not fixed, two
different classifications:**

- **`resolve_gcp_secret_dsn` (3 misses): a scoping artifact.** These functions were EXTRACTED
  (behavior-preserving move, not new logic) from `transaction_sweeper.rs`'s own
  `resolve_dsn_without_api_key` per ADR-072 Decision C. Confirmed via grep that
  `tests/customer_db_transaction_sweeper/acceptance/us01_reclaim_orphaned_transactions.rs`'s own
  `gcp_secret_orphaned_transaction_is_reclaimed_without_api_key` test DOES exercise this exact
  function end-to-end with a real `GcpSecretFetcher` — but that test target was OUTSIDE this
  mutation run's own scope (only the 3 `cxr0N` targets + `--lib`). Real coverage exists elsewhere
  in the codebase; not a genuine gap.
- **`resolve_aws_secret_dsn` (3 misses): a genuine, PRE-EXISTING gap, not introduced by this
  feature and out of scope to fix here.** Confirmed via grep across `tests/customer_db_transaction_sweeper/`
  and `tests/secrets_management/` that NO test anywhere in the workspace exercises the aws_secret
  DSN-resolution path end-to-end (`us02_purge_terminal_transactions.rs` only mentions it in a
  comment). This matches DESIGN's own explicitly disclosed, pre-existing limitation (ADR-072,
  citing ADR-054 § D7): "`direct_pg` is the only mode with an end-to-end-wired secret-fetcher path
  today." This function is a straight extraction of already-existing, already-uncovered logic —
  fixing it here would be scope creep beyond finding #5, matching the precedent set by
  `rate-limiter-project-id-validation`'s own "pre-existing gap is out of scope" QUALITY_GATE
  finding. A genuine follow-up candidate for a future aws_secret-mode-specific feature.

## Final state

All 12 acceptance scenarios (3 files) reconfirmed green after the fix (24/24 across repeated
reruns). Full `cargo test -p embyr-server --lib`: 74/74 (73 pre-existing + the 1 new
`build_succeeded` unit test).
