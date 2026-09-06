# Mutation Testing Report — firestore-range-operator-value-type-support

**Tool**: cargo-mutants 27.0.0
**Date**: 2026-09-06
**Scope**: this feature's own diff spans TWO crates for the first time this session
(`crates/embyr-pg-storage/src/encoding/query.rs`'s 3 new `push_scalar_comparison` match arms, and
`crates/embyr-server/src/grpc/handler.rs`'s new `translate_filter` check) — run as 2 separate
`cargo-mutants` invocations, one per crate, both `--in-diff`-scoped against `8b59c20~1..HEAD` and
`--lib`-scoped to each crate's own new unit tests.

## Pass 1a — `embyr-pg-storage`

`cargo mutants -p embyr-pg-storage --in-place --timeout 30 --in-diff <diff> -- --lib encoding::query::tests::`

**Result**: 4 mutants — **4 caught, 0 missed, 0 unviable, 0 timeouts. 100% effective kill rate,
first pass.**

## Pass 1b — `embyr-server`

`cargo mutants -p embyr-server --in-place --timeout 30 --in-diff <diff> -- --lib range_operator_value_type_tests::`

**Result**: 3 mutants — **2 caught, 1 unviable, 0 missed, 0 timeouts. 100% effective kill rate on
every viable mutant, first pass.**

## Combined verdict

**PASS.** 100% effective kill rate across both crates on the first pass — no gap-closing follow
-up needed. The 4 new unit tests in `embyr-pg-storage` directly assert the exact generated SQL
shape for each new type (`ROW(...)` for `Timestamp`, `decode(...)` for `Bytes`, plain string
comparison for `Reference`); the 3 new unit tests in `embyr-server` directly assert
`translate_filter`'s own rejection behavior for `Array`/`Map` plus a regression guard proving
`Equal` on `Array` is unaffected.

## A note on residual, narrower panics — NOT closed by this feature, named explicitly

Direct inspection of every remaining `panic!` in `crates/embyr-pg-storage/src/encoding/query.rs`
after this feature's own fixes:

| Line | Panic | Reachable via ordinary, well-formed Firestore SDK usage? |
|---|---|---|
| 75, 96, 138 | `ArrayContainsAny`/`In`/`NotIn` given a NON-array value | **No** — every real Firestore SDK always sends an `ArrayValue` for these 3 operators; only a malformed/adversarial raw gRPC request bypassing normal SDK construction could trigger this |
| 153 | `append_field_filter`'s own outer `_ => panic!("unsupported filter op")` | **No** — dead code; all 12 `FilterOp` variants are exhaustively handled by the 2 preceding `match` blocks (`IsNan`/`IsNotNan` handled first, the remaining 10 handled second) |
| 251 | `push_scalar_comparison`'s own outer fallback, now narrowed to `Null`/`Array`/`Map` for range operators specifically | **No** — `Array`/`Map` are rejected upstream (this feature's own fix); `Null` against a range operator is not a construct any real Firestore SDK generates (real Firestore itself restricts `null` to equality-only comparisons) |

**This session's own 3-feature crash-elimination arc** (`In`/`NotIn`/`ArrayContains`/
`ArrayContainsAny` operator support → `Equal`/`NotEqual` value-type support → range-operator
value-type support) **has closed every FilterOp/value-type combination reachable via ordinary,
well-formed Firestore SDK usage** — the exact class of bug this whole arc targeted, starting from
`composite-index-requirement-rules`'s own original discovery.

A narrower, lower-severity, DIFFERENT class of gap remains: a MALFORMED or adversarial raw gRPC
request (one no real Firestore SDK would ever construct — an operator/value-SHAPE mismatch, like a
scalar value paired with `In`) still panics rather than cleanly rejecting. This is a hardening
concern (defense against malformed input), not a "real client, real query, crashes" concern — named
as its own, explicitly lower-priority follow-up (feature-delta.md-equivalent candidate id:
`firestore-malformed-filter-shape-validation`), not conflated with the now-closed arc.

Proceeding to FINALIZE.
