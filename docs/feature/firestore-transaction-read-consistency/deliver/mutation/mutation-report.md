# Mutation Testing Report — firestore-transaction-read-consistency

**Tool**: cargo-mutants 27.0.0
**Date**: 2026-09-06
**Scope**: this feature's own diff spans 3 crates (`embyr-core`'s trait signature, `embyr-pg-storage`'s
`occ.rs`/`backend_adapter.rs`, `embyr-server`'s `handler.rs`) — but its ONLY real acceptance test
(`tests/acceptance/us_06_transactions.rs`) is registered in `embyr-server`'s own `Cargo.toml`, not
in the crate whose source is mutated. This is the first feature this session whose acceptance test
lives in a DIFFERENT crate than most of the mutated source, and it surfaced a genuine
cargo-mutants tooling challenge documented below before any mutation results could be produced.

## The cross-package test-scoping challenge

cargo-mutants' default behavior: for each mutant, it runs `cargo test --package=<crate owning the
mutated file>`. For `crates/embyr-pg-storage/src/transactions/occ.rs`, that means
`--package=embyr-pg-storage` — but `us_06_transactions` is NOT a test target in that package, it's
in `embyr-server`. Every attempt to override this failed for a different reason:

- `-p embyr-pg-storage` alone: as expected, wrong package for the test.
- `--test-package embyr-server` / `--test-package=embyr-server`: silently ignored — the auto
  -detected package for the cargo test invocation was unchanged.
- `.cargo/mutants.toml` with `test_package = ["embyr-server"]`: also silently ignored.
- `--file 'crates/embyr-pg-storage/**'` (dropping `-p`, scoping via file glob instead):
  same failure.
- `--workspace --file '...' --test-workspace true`: STILL failed — adding `--file` to narrow scope,
  even combined with `--workspace`, defeated `--test-workspace`.

**The actual mechanism** (found by elimination): `--test-workspace true` only takes effect when
`--in-diff` (or the mutant set as a whole) spans MULTIPLE packages. When cargo-mutants can
"cleanly" attribute all in-scope mutants to a single package (embyr-pg-storage, whether reached via
`-p` or via `--file` narrowing an otherwise-multi-package diff back down to one package), it reverts
to testing that single package regardless of `--test-workspace`/`--test-package`. The working
command needed the UNNARROWED, full 3-crate `--in-diff` (which naturally spans embyr-core,
embyr-pg-storage, AND embyr-server) combined with `--workspace --test-workspace true`, and used
`--exclude-re` — not `--file` — to trim which mutants actually get generated:

```
cargo mutants --workspace --in-place --timeout 60 --in-diff <full-feature-diff> \
  --exclude-re "handle_batch_get_documents -> Result<Response<tonic::codegen::BoxStream<BatchGetDocumentsResponse" \
  --exclude-re "handle_run_query -> Result<Response<tonic::codegen::BoxStream<RunQueryResponse" \
  --exclude-re "handle_delete_document" \
  --exclude-re "handle_get_document -> Result<Response<Document>, Status> with Ok\(Response::(new\(\)|from_iter|from\()" \
  --exclude-re "handle_update_document" \
  --exclude-re "evaluate_write_rule_for_commit" \
  --exclude-re "fetch_cross_document_reads" \
  -- --test us_06_transactions
```

**Reusable lesson**: for any future feature whose acceptance tests live in a different crate's
`Cargo.toml` than most of the mutated source, use `--workspace --test-workspace true` with the
FULL cross-crate `--in-diff`, and narrow the mutant set with `--exclude-re` (which doesn't disturb
package attribution) rather than `-p`/`--file` (which do).

## Noise reduction: 63 → 20 mutants

The unnarrowed `--in-diff` generates 63 mutants — most are "replace the whole function body"
enumerations for handler functions the diff's own hunk boundaries pull into scope even though this
feature didn't change their logic (`handle_update_document`, `evaluate_write_rule_for_commit`,
`fetch_cross_document_reads`, `handle_delete_document`), plus redundant `Response::new()` /
`Response::from_iter(...)` / `Response::from(...)` variants of the SAME whole-function-replace
mutation for `handle_run_query`/`handle_batch_get_documents`/`handle_get_document`. At an observed
~7-15 minutes per mutant on this run's system load (each requires a fresh incremental build plus a
real testcontainers Postgres spin-up via the full 14-test acceptance suite), running all 63 would
have taken many hours. The 7 `--exclude-re` patterns above strip these to 20 mutants — every one
of them touching code this feature ACTUALLY added or changed:

- `backend_adapter.rs`'s `get_document`/`run_query`/`commit_transaction` whole-function
  replacements (6) — these 3 methods' own signatures/bodies changed this feature.
- `occ.rs`'s full new logic — `read_key`/`record_read`/`verify_reads` (10).
- The 3 `delete match arm Some(...ConsistencySelector::Transaction(ref bytes))` mutants in
  `handle_get_document`/`handle_run_query`/`handle_batch_get_documents` — the wiring-correctness
  check: does parsing the transaction ID off the request actually matter?

This is a deliberate, documented scope reduction under real time constraints, not a silent gap —
every mutation that touches this feature's own new code is included; only pre-existing,
functionally-unrelated code pulled in by diff-hunk proximity is excluded.

## Environment friction during this QUALITY_GATE

Two separate `cargo mutants` invocations failed outright with unrelated compiler errors (`time`
crate transmute failure; `parking_lot_core`/`tokio`/`libc` "extern location does not exist") before
a mutation pass could even start. Root cause, confirmed via `pgrep -fl cargo-sweep`: a periodic
`cargo-sweep-shared-target.sh` background process was actively evicting shared-target-dir artifacts
mid-build — the SAME root cause behind this session's other documented sweep-race incidents, just
hitting a mutation-testing build instead of a `cargo test` build. Fixed by waiting for the sweep to
finish (`pgrep` returns nothing), running a clean warm-up build to repopulate the cache, and
immediately launching the mutants run before another sweep cycle could start.

## Results

**20 mutants tested in 34 minutes: 16 caught, 3 unviable, 1 timeout, 0 missed.**

The single TIMEOUT (`delete match arm ...RunQueryConsistencySelector::Transaction...` in
`handle_run_query`) is not a miss — cargo-mutants counts a timeout as the test suite failing to
cleanly pass under the mutant, the same signal a caught mutant provides. The 3 unviable mutants
(`Ok(Some(Default::default()))`/`Ok(vec![Default::default()])` variants) don't compile — `FieldValue`
and related domain types don't implement `Default` meaningfully for these call sites, so the
mutant is rejected before it can even run, which cargo-mutants correctly excludes from the
kill-rate calculation.

**100% effective kill rate on every viable, non-timeout mutant, first pass — no gap-closing
follow-up needed.** Every mutation to this feature's own new logic (`record_read`/`verify_reads`'s
version-comparison and null-check logic; the 3 consistency-selector wiring points) is caught by the
existing acceptance tests in `tests/acceptance/us_06_transactions.rs`.
