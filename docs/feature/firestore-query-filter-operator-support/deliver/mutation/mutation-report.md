# Mutation Testing Report — firestore-query-filter-operator-support

**Tool**: cargo-mutants 27.0.0
**Date**: 2026-09-05
**Scope**: `--in-diff` against this feature's own full diff of
`crates/embyr-pg-storage/src/encoding/query.rs` (`ec10e56..HEAD`) — this feature's ENTIRE
production-code footprint. Test harness scoped to `-- --lib encoding::query::tests::`, the fast,
Docker-free unit tests added in this file's own `mod tests` (this feature's own SQL-generation unit
tests use `QueryBuilder::sql()` to inspect the generated SQL text directly, without needing a live
DB connection — real EXECUTION correctness is proven separately by the 4 Docker-backed acceptance
tests, `qfo01`-`qfo04`).

## Pass 1 — `embyr-pg-storage`, initial run

`cargo mutants -p embyr-pg-storage --in-place --timeout 30 --in-diff <feature-diff> -- --lib encoding::query::tests::`

**Result**: 27 mutants — **12 caught, 15 missed, 0 unviable, 0 timeouts.**

### The 15 misses — two distinct root causes, both closed

**Root cause 1 — join-boundary mutants (5 misses)**: `replace > with ==`/`replace > with >=` on the
`if i > 0` separator-insertion check inside the `ArrayContainsAny`/`In`/`NotIn` loops. The EXISTING
2-target tests (`array_contains_any_ors_one_containment_check_per_target`,
`in_ors_one_equality_check_per_target`) only asserted `sql.contains(" OR ")` — a loose presence
check that stays TRUE even when the mutant moves the separator to the WRONG position (e.g. before
the first item instead of between items) or changes its total occurrence count, because a
2-target/1-separator case can't distinguish "1 separator, correctly placed" from "1 separator,
misplaced" using presence alone. **Fix**: strengthened all 3 tests to use 3 targets and assert the
EXACT separator count (`sql.matches(" OR ").count() == 2` for 3 targets), which any of the 3
boundary-shifting mutants breaks.

**Root cause 2 — pre-existing, previously-untested operator/type combinations (10 misses)**:
`push_scalar_comparison`'s own 6 comparison operators (`LessThan` through `NotEqual`) and its own
`Integer`/`Double`/`Boolean` match arms — ALL pre-existing behavior, unchanged by this feature's own
refactor (confirmed by the identical `equal_still_generates_the_same_shape_as_before_the_refactor`
regression-guard test passing throughout). These lines appeared in THIS feature's own `--in-diff`
scope only because the refactor (extracting `push_scalar_comparison` from an inline match) moved
them, not because this feature changed their behavior. Before this feature, NO unit test in this
file exercised ANY operator besides `Equal`, and NO value type besides `String` — a genuine,
pre-existing test-coverage gap this feature's own diff happened to expose, not one this feature
introduced. **Fix**: one new parameterized unit test
(`push_scalar_comparison_covers_every_operator_and_value_type`) exercising all 5 previously
-untested operators against `Integer`/`Double`/`Boolean` targets, asserting both the operator symbol
and the type-specific SQL cast appear in the generated SQL.

## Pass 2 — final confirmation

**Result**: 27 mutants — **27 caught, 0 missed, 0 unviable, 0 timeouts.**

**Effective kill rate: 100%, no residual gaps of any kind** — the cleanest QUALITY_GATE result of
any feature this session (even `composite-index-requirement-rules`'s own 100%-first-pass result
had unviable/timeout mutants; this pass has none).

## `embyr-core`/`embyr-server` — not applicable

This feature touches zero lines outside `crates/embyr-pg-storage/src/encoding/query.rs` — confirmed
by construction (ADR-level design decision: no new domain type, no new proto-translation logic).

## A note on scope discipline

The 10 pre-existing-operator misses (root cause 2) were a genuine, real test-coverage gap in this
codebase — but NOT one introduced by this feature, and arguably outside this feature's own DISCUSS
-locked scope (widening `In`/`NotIn`/`ArrayContains`/`ArrayContainsAny` support, not auditing every
pre-existing operator's own test coverage). Closed anyway here rather than deferred, since: (a) the
fix was a single, small, cheap unit test (not a design change), (b) leaving a QUALITY_GATE-detected
gap unclosed simply because it predates the feature would be a worse precedent than fixing it, and
(c) `push_scalar_comparison`'s own extraction (this feature's own Slice 03 work) is precisely what
made this gap CHEAP to close (a standalone function, directly callable from a unit test, versus the
original inline match block).

## Overall verdict

**PASS.** 100% effective kill rate after 2 targeted fixes (join-boundary test strengthening;
pre-existing operator/type coverage). Full `embyr-server` regression clean.

Proceeding to FINALIZE.
