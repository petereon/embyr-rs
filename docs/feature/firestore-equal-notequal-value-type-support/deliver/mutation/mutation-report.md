# Mutation Testing Report — firestore-equal-notequal-value-type-support

**Tool**: cargo-mutants 27.0.0
**Date**: 2026-09-05
**Scope**: `--in-diff` against this feature's own full diff of
`crates/embyr-pg-storage/src/encoding/query.rs` — this feature's ENTIRE production-code footprint
(a 2-line dispatch change: `FilterOp::Equal`/`FilterOp::NotEqual` now call `push_value_equality`
instead of `push_scalar_comparison`, plus doc-comment-only edits). Test harness scoped to
`-- --lib encoding::query::tests::`, Docker-free.

## Pass 1 — `embyr-pg-storage`, only pass needed

`cargo mutants -p embyr-pg-storage --in-place --timeout 30 --in-diff <feature-diff> -- --lib encoding::query::tests::`

**Result**: 4 mutants — **4 caught, 0 missed, 0 unviable, 0 timeouts.**

100% effective kill rate on the first pass — no fix needed. Expected: this feature's own diff is
the smallest of any built this session (2 dispatch-target changes, zero new function, zero new
match arm), and DELIVER already added direct unit coverage for the exact behavior change
(`equal_compares_the_whole_field_value_after_...`, `not_equal_compares_the_whole_field_value`,
`equal_does_not_panic_on_previously_crashing_value_types`, `equal_on_cross_numeric_types_binds_
distinct_json_values`) plus updated the 2 pre-existing tests whose own assertions depended on the
old dispatch target.

## `embyr-core`/`embyr-server` — not applicable

Zero lines touched outside this one function's own 2 match arms — confirmed by construction.

## Overall verdict

**PASS.** 100% effective kill rate, zero gap-closing work needed — reusing an already-hardened
helper (`push_value_equality`, itself already proven at 100% mutation coverage during
`firestore-query-filter-operator-support`'s own QUALITY_GATE) inherits that helper's own
correctness rather than introducing new surface to test. Full `embyr-server` regression clean.

Proceeding to FINALIZE.
