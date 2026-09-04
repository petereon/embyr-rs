# Mutation Testing Report — composite-index-requirement-rules

**Tool**: cargo-mutants 27.0.0
**Date**: 2026-09-05
**Scope**: `--in-diff` against this feature's own full diff of BOTH touched files —
`crates/embyr-server/src/grpc/handler.rs` (all 3 slices: `requires_composite_index`'s widened
control flow, `collect_filter_fields`'s new signature, `missing_index_fields`) and
`crates/embyr-core/src/domain/query.rs` (`FilterOp`'s new `Copy` derive) — `b1f7e95..HEAD`. Test
harness scoped to `-- --lib composite_index_requirement_tests::`, the fast, Docker-free unit tests
in `handler.rs`'s own `mod tests` (mirrors this session's own established Docker-contention
discipline).

## Pass 1 — `embyr-server`, first and only run

`cargo mutants -p embyr-server --in-place --timeout 30 --in-diff <feature-diff> -- --lib composite_index_requirement_tests::`

**Result**: 26 mutants — **2 caught, 24 unviable, 0 missed, 0 timeouts.**

**100% effective kill rate on the FIRST pass — no gap-closing fix needed this time**, unlike every
prior QUALITY_GATE this session (4c/4d/4e/`firestore-composite-indexes-admin-api` all needed at
least one follow-up pass). The difference: unit tests for every new branch (both new rules in
Slice 01/02, plus `missing_index_fields`'s own 3 trigger-shape outputs in Slice 03) were written
DURING each slice's own DELIVER, covering not just the happy path per rule but the specific
boundary that distinguishes each rule from its neighbors (same-field-vs-different-field for the
`IN`+range rule; every-`orderBy`-field-already-filtered for the multi-`orderBy` rule) — closing
exactly the class of gap the prior 4 features' own QUALITY_GATEs each had to discover reactively.

The 24 "unviable" mutants are cargo-mutants' own term for a mutation that fails to COMPILE (not a
missed correctness gap) — expected and common in this diff given how much of it is precise
`FilterOp` enum matching and tightly-typed tuple/struct construction, where many syntactic mutations
(e.g., swapping a match arm's own pattern) simply don't type-check. Not a scoping artifact: the
`--in-diff` patch correctly targeted every line this feature actually changed.

## `embyr-core` — no separate pass needed

`FilterOp`'s new `#[derive(Copy)]` (line 89) is a marker-trait addition with zero executable logic
of its own — cargo-mutants correctly found no mutable surface there; the ONE consumer of that
`Copy`-ness (`collect_filter_fields`'s new `Vec<(&str, FilterOp)>` return type) lives in
`embyr-server` and was already covered by the pass above.

## `embyr-pg-storage` — not applicable, out of scope

The severe, PRE-EXISTING gap discovered during this feature's own Slice 02 DELIVER
(`append_field_filter` panics on `In`/`NotIn`/`ArrayContains`/`ArrayContainsAny`,
`crates/embyr-pg-storage/src/encoding/query.rs`) is explicitly out of this feature's own scope
(feature-delta.md § Discovered Gap) — this feature never modifies that file, so it carries no
mutation-testing obligation here.

## Overall verdict

**PASS.** 100% effective kill rate on the first pass, zero gap-closing work needed. Full
`embyr-server` regression clean (441 passed, 1 pre-existing unrelated `distributed_rate_limiting`
flake, the same failure mode already documented in this session's own prior evolution docs).

Proceeding to FINALIZE.
