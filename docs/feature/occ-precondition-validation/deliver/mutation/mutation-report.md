# Mutation Testing Report — occ-precondition-validation

**Tool**: cargo-mutants
**Scope**: `crates/embyr-pg-storage/src/backend_adapter.rs`, `--in-diff` spanning the DELIVER fix
commit plus the orchestrator's own added `to_datetime_tests` unit-test module.

## A real cargo-mutants tool limitation hit, and how it was resolved

This feature's acceptance tests live in `embyr-server`'s and `embyr-agent`'s own `[[test]]`
targets, but the mutated code (`to_datetime`) lives in `embyr-pg-storage`. cargo-mutants 27.0.0
auto-detects `--package=embyr-pg-storage@0.1.0` for its own internal `cargo test` invocation based
on which package OWNS the mutated file, and neither `--test-workspace true` nor `--test-package
embyr-server` (both explicitly tried) override this — confirmed empirically (both fail with
`error: no test target named "us_02_write_document" in "embyr-pg-storage@0.1.0" package`), matching
this session's own prior `firestore-transaction-read-consistency` finding exactly, and refining it:
the earlier finding's own claimed workaround (a genuinely multi-package `--in-diff`) did NOT
reproduce here either — also tried and confirmed still auto-detecting the single package that
actually owns the mutable content.

**Resolution**: rather than continuing to fight the tool, added a small, direct `#[cfg(test)]`
module (`to_datetime_tests`, 4 tests) calling `to_datetime` directly in the SAME file. This gives
genuine, single-package mutation coverage with zero workaround needed — `cargo mutants -p
embyr-pg-storage ... -- --lib` now has a real, fast (unit-test-speed) target to run against. This
is also good practice independent of the mutation-testing need: a function with real branching
logic (two distinct validation failure modes) benefits from a direct unit test regardless.

## Command

```
cargo mutants -p embyr-pg-storage --in-place --timeout 300 --in-diff /tmp/occ_mut_diff_final.diff -- --lib -- --test-threads=1
```

## Result: 8 mutants tested in 7s — 1 caught, 6 unviable, 1 missed

```
MISSED   crates/embyr-pg-storage/src/backend_adapter.rs:1006:9: replace <impl BackendAdapter for PostgresBackendAdapter>::commit_transaction -> Result<Vec<WriteResult>, CoreError> with Ok(vec![])
```

## Interpretation

**The single miss is a scoping artifact, not a gap in this feature's own logic.** This is a
whole-function-body-replacement mutant on `commit_transaction` itself (the async trait-impl
method containing one of the 2 `to_datetime` call sites this feature touches) — not a mutation of
`to_datetime`'s own validation logic. `commit_transaction` requires a real Postgres connection and
transactional state; it can never be meaningfully exercised by a `--lib`-only unit test scope by
construction — this mutant would exist regardless of whether this feature ever touched the
function. Real coverage exists at the integration level: `tests/acceptance/us_06_transactions.rs`
(confirmed 15/15 passing, independently re-verified twice by both DELIVER and the orchestrator)
asserts on the actual `WriteResult` contents returned by a real transaction commit — an `Ok(vec![])`
stub would fail those assertions immediately. Outside this run's own `--lib`-only scope by design,
not a genuine gap.

**The 1 caught mutant and 6 unviable mutants** are the actual `to_datetime` validation logic this
feature added — all directly exercised by the new `to_datetime_tests` unit tests. Unviable mutants
are the usual `Default::default()`-on-non-`Default`-type build failures for this codebase's
established `Result<DateTime<Utc>, CoreError>` pattern.

## Final state

New unit tests: 4/4 pass (`valid_seconds_and_nanos_succeed`,
`negative_nanos_is_rejected_before_the_seconds_check`, `nanos_at_or_above_one_billion_is_rejected`,
`out_of_range_seconds_is_rejected`). Integration regression: `us_02_write_document` (7/7),
`us_06_transactions` (15/15), `embyr_agent`'s own `us_a02_write_operations` (11/11) — all
independently reconfirmed by the orchestrator, not just DELIVER's own report.
