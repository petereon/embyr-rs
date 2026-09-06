# Evolution: firestore-range-operator-value-type-support

**Date:** 2026-09-06
**Feature:** Range operators (`<`/`<=`/`>`/`>=`) now correctly support `Timestamp`/`Bytes`/
`Reference`-valued filter targets (previously panicking), and cleanly reject `Array`/`Map`-valued
targets with `INVALID_ARGUMENT` instead of crashing.
**Job:** JOB-01 (`sdk-compat`) — the third and final realization of this session's own
crash-elimination arc.
**ADRs:** none (folded into feature-delta.md's own DESIGN section).

## ⚠️ This FINALIZE closes a 3-feature crash-elimination arc

Starting from `composite-index-requirement-rules`'s own discovered gap (2026-09-05), this session
ran 3 features in sequence, each closing one more slice of the SAME underlying problem —
`append_field_filter`'s own SQL-generation match statements panicking instead of executing or
cleanly rejecting a query:

1. **`firestore-query-filter-operator-support`** — `In`/`NotIn`/`ArrayContains`/
   `ArrayContainsAny` operators, which previously had NO SQL translation at all for ANY value type.
2. **`firestore-equal-notequal-value-type-support`** — `Equal`/`NotEqual` operators, which panicked
   on `Timestamp`/`Bytes`/`Reference`/`Array`/`Map`-valued targets.
3. **`firestore-range-operator-value-type-support`** (this feature) — the 4 range operators, which
   panicked on the SAME 5 types.

**As of this FINALIZE, every `FilterOp`/`FieldValue` combination reachable via ordinary,
well-formed Firestore SDK usage now either executes correctly or is cleanly rejected — none of
them panic.** A narrower, structurally different, lower-priority class of gap remains (§ Follow-Up
Work) — malformed/adversarial raw gRPC requests that no real SDK would ever construct — named
explicitly as a separate concern, not conflated with this now-closed arc.

## Business Context

Direct inspection during this feature's own QUALITY_GATE confirmed the arc's own closure precisely:
of the 5 remaining `panic!` sites in `crates/embyr-pg-storage/src/encoding/query.rs`, every one is
now either unreachable (real Firestore SDKs never generate the malformed shape that would trigger
it) or dead code (an outer `_ => panic!` fallback whose match already exhaustively covers all 12
`FilterOp` variants).

## Key Decisions

| Decision | Verdict |
|---|---|
| Reuse JOB-01 — the third and final realization of this session's own "stop the crash" arc | feature-delta.md § Resolution 1 |
| `Array`/`Map` are out of scope for range-operator ORDERING — real Firestore itself confirmed not to support range queries on `Array` at all; `Map`'s own support is unconfirmed | § Resolution 2 |
| `Timestamp` uses Postgres `ROW(...)` comparison over its own `(seconds, nanos)` fields — zero arithmetic-overflow risk vs. combining into one scaled value | § Resolution 3 |
| `Bytes` decodes standard-base64 to raw `bytea` for comparison — base64 TEXT comparison would be WRONG, since its own alphabet doesn't preserve byte-value ordering | § Resolution 4 |
| `Reference` uses flat string comparison — real-Firestore-accurate for the evidenced same-depth case; a named, deferred divergence for cross-depth references | § Resolution 5 |

## Steps Completed

1. **Slice 01** — `push_scalar_comparison` gains 3 new match arms: `Timestamp` (`ROW(...)`
   comparison), `Bytes` (`decode(..., 'base64')` to raw `bytea`), `Reference` (flat string
   comparison, matching the existing `String` arm's own shape).
2. **Slice 02 (LAST slice)** — `translate_filter` (`embyr-server`) gains one check: a range
   operator against `Array`/`Map` is rejected with a clean `INVALID_ARGUMENT`, reusing the SAME
   `Result<_, String>` → `Status::invalid_argument` mechanism `CompositeOp::Unspecified` already
   uses — zero new validation infrastructure.

**QUALITY_GATE** — the first feature this session whose own diff spans 2 crates, run as 2 separate
`cargo-mutants` invocations: `embyr-pg-storage` (4/4 caught, 0 missed) and `embyr-server` (2/3
caught, 1 unviable, 0 missed). **100% effective kill rate across both, first pass, no gap-closing
follow-up needed.** During QUALITY_GATE, direct inspection of every remaining `panic!` site
confirmed the arc's own closure (§ above).

**Full regression**: `cargo test -p embyr-server`, 444 tests passed, 0 failures attributable to
this feature (1 pre-existing, unrelated `distributed_rate_limiting` flake, the same documented
failure mode from this session's prior evolution docs).

## Lessons Learned

1. **A single encoding choice (standard base64) silently breaks ordering semantics in a way that's
   easy to miss without checking the ACTUAL alphabet.** Base64's own character set
   (`A-Za-z0-9+/`) does not sort in the same order as the byte values it encodes — comparing
   base64-encoded TEXT directly is a subtly wrong shortcut that would have produced silently
   incorrect query results (not a crash) had it not been caught during DISCUSS's own live
   verification. This is a genuinely reusable lesson: any future feature comparing encoded binary
   data needs to decode first, never compare the encoding's own text representation.
2. **"Does real Firestore even support this at all" is sometimes the actual answer, not "how do we
   implement it."** `Array` range queries aren't merely an edge case this codebase handles
   differently — real Firestore itself doesn't support them. Building ordering logic for a
   construct real Firestore rejects would have been inventing non-Firestore behavior, the OPPOSITE
   of this initiative's own goal. Live verification caught this before any code was written.
3. **Closing a crash-elimination arc requires actively checking for arc closure, not just assuming
   the last feature was the last gap.** This FINALIZE's own QUALITY_GATE deliberately inspected
   every remaining `panic!` site and classified each by real-world reachability — the difference
   between "no more real-client crashes" and "no more panics of any kind" matters, and conflating
   them would have either overclaimed completeness or under-delivered by chasing unreachable
   defensive code.

## Key Files

- `crates/embyr-pg-storage/src/encoding/query.rs` — 3 new `push_scalar_comparison` match arms, 4
  new unit tests.
- `crates/embyr-server/src/grpc/handler.rs` — `translate_filter`'s new Array/Map rejection check,
  3 new unit tests.
- `tests/firestore_range_operator_value_type_support/acceptance/` — rng01 (4 real end-to-end
  tests including the byte-vs-base64-text ordering proof), rng02 (2 real `INVALID_ARGUMENT`
  rejection tests).
- `docs/feature/firestore-range-operator-value-type-support/feature-delta.md` — full DISCUSS/DESIGN
  narrative.
- `docs/feature/firestore-range-operator-value-type-support/deliver/mutation/mutation-report.md`
- `docs/product/jobs.yaml`, JOB-01 — new NOTE appended.

## Follow-Up Work

**`firestore-malformed-filter-shape-validation`** (candidate id, explicitly a DIFFERENT, LOWER
-priority category from the now-closed arc) — a malformed/adversarial raw gRPC request that no real
Firestore SDK would ever construct (e.g. a scalar value paired with `In`/`NotIn`/
`ArrayContainsAny`, which always expect an array; a `Null` value paired with a range operator) still
panics rather than cleanly rejecting. This is a hardening concern against malformed input, not a
"real client, real query, crashes" concern — the arc this FINALIZE closes was scoped entirely to
the latter.

Carried forward, unchanged, from prior features: list-size limit enforcement, empty-list behavior
confirmation, two-simultaneous-`IN`, the range-filter-orderBy-must-start-same-field query-validity
constraint, true segment-wise `Reference` comparison for cross-depth paths.
