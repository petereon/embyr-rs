# Mutation Testing Report — firestore-malformed-filter-shape-validation

**Tool**: cargo-mutants 27.0.0
**Date**: 2026-09-06
**Scope**: single crate, `crates/embyr-server/src/grpc/handler.rs`'s new `translate_filter`
rejection checks (non-`Array` + `In`/`NotIn`/`ArrayContainsAny`, `Null` + range operator).

`cargo mutants -p embyr-server --in-place --timeout 30 --in-diff <diff> -- --lib malformed_filter_shape_tests::`

## Pass 1 — 1 gap found

**Result**: 7 mutants — 5 caught, 1 unviable, **1 missed**.

**Missed mutant**: `delete match arm FilterOp::In in translate_filter` (the arm mapping `In` to the
`op_name` string `"in"` inside the rejection's own error-message builder). Deleting it falls
through to the `_ => "array-contains-any"` arm — the rejection still fires (still returns `Err`,
still `INVALID_ARGUMENT`), just with the wrong message text.

**Why it slipped through**: `in_given_a_non_array_value_is_rejected` asserted
`err.contains("in")`. The WRONG string `"array-contains-any"` itself contains the substring `"in"`
(from `"contains"`), so the mutant produced output the assertion accepted. Presence-only assertions
are a recurring blind spot this session when a plausible-but-wrong string can itself contain the
expected substring — the same shape of gap as the join-boundary miss found during
`firestore-query-filter-operator-support`.

**Fix**: tightened all 4 message-content assertions from `.contains(...)` to exact
`assert_eq!(err, "<full expected message>")` — `in_given_a_non_array_value_is_rejected`,
`not_in_given_a_non_array_value_is_rejected`, `array_contains_any_given_a_non_array_value_is_rejected`,
`less_than_given_a_null_value_is_rejected`.

## Pass 2 — confirms the gap closed

**Result**: 7 mutants — **6 caught, 1 unviable, 0 missed. 100% effective kill rate on every viable
mutant.** The previously-missed `FilterOp::In` arm deletion is now caught by the exact-match
assertion.

## Verdict

**PASS** (after 1 gap-closing iteration). All 4 new rejection branches
(`In`/`NotIn`/`ArrayContainsAny` + non-`Array`, range operator + `Null`) are exercised by both
their `matches!` guard and their exact error-message text; the regression-guard tests
(well-formed `In`, well-formed range comparison) confirm no existing behavior broke.
