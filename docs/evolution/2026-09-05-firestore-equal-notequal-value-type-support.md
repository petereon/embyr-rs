# Evolution: firestore-equal-notequal-value-type-support

**Date:** 2026-09-05
**Feature:** Fixes `Equal`/`NotEqual` panicking on `Timestamp`/`Bytes`/`Reference`/`Array`/`Map`
-valued filter targets — a real, active production crash risk flagged by
`firestore-query-filter-operator-support`'s own FINALIZE.
**Job:** JOB-01 (`sdk-compat`) — direct continuation of that feature's own "make it real" pattern.
**ADRs:** none (folded into feature-delta.md's own DESIGN section — the smallest feature built this
session).

## Business Context

`push_scalar_comparison` (`crates/embyr-pg-storage/src/encoding/query.rs`) implemented `Equal`/
`NotEqual` (and the 4 range operators) via a match on the filter's own value type covering only
`Integer`/`String`/`Double`/`Boolean` — `Timestamp`/`Bytes`/`Reference`/`Array`/`Map` all hit its
own `_ => panic!(...)` fallback. Any live query filtering by an exact timestamp, a document
reference, an array, or a map crashed the request.

**Result: fixed, with zero new production logic.** `push_value_equality` — built one feature earlier
to solve the identical problem for `In`/`NotIn` — already compares the WHOLE discriminated-union
JSON value rather than an unwrapped, type-cast scalar, which works uniformly for every `FieldValue`
variant. `Equal`/`NotEqual` simply needed to call it instead of `push_scalar_comparison`.

## Key Decisions

| Decision | Verdict |
|---|---|
| Reuse JOB-01 — direct continuation of the immediately-prior feature's own flagged follow-up | feature-delta.md § Resolution 1 |
| Route `Equal`/`NotEqual` to the already-existing `push_value_equality`, not new cast logic | § Resolution 2 |
| Scope locked to `Equal`/`NotEqual` only, per explicit kickoff framing — range-operator support for the same 5 types needs a genuinely different (ordering, not equality) mechanism, named and deferred | § Resolution 3 |

## Steps Completed

**Slice 01 (the entire feature)** — `append_field_filter`'s own `Equal`/`NotEqual` match arms now
call `push_value_equality(qb, &f.field_path, &f.value, negate)` instead of `push_scalar_comparison`.
Doc comments on both helpers updated for accuracy. 5 new unit tests plus 2 pre-existing tests
updated (their own assertions depended on the old `->>'v'`-extraction SQL shape, now stale — fixed
in place, superseded-scenario style, not silently left wrong). 1 acceptance test file, 4 real
end-to-end proofs: `Equal` on `Timestamp`, `NotEqual` on `Reference`, `Equal` on `Array` (whole
-order equality), and a `String` regression guard.

**QUALITY_GATE** — `cargo-mutants --in-diff`, `--lib`-scoped: **4/4 caught, 0 missed, 0 unviable, 0
timeouts, 100% effective kill rate on the first pass** — no gap-closing follow-up needed, since this
feature inherits `push_value_equality`'s own already-hardened correctness (proven at 100% mutation
coverage one feature earlier) rather than introducing new surface to test.

**Full regression**: `cargo test -p embyr-server`, 441 tests passed, 0 failures attributable to this
feature (1 pre-existing, unrelated `distributed_rate_limiting` flake, the same documented failure
mode from this session's prior evolution docs; one earlier run also hit a transient "never executed"
build-race binary-link failure, unrelated to this feature, cleared on retry).

## Lessons Learned

1. **A previously-built, already-hardened helper can close an entirely separate flagged bug with
   zero new logic — recognizing "this is the same shape of problem" is the actual work.** The
   DISCUSS wave's own central finding wasn't a new mechanism, it was recognizing that `push_value_
   equality` (built for a different pair of operators, one feature earlier) already solved this
   exact problem generically. The laziest fix was also the most correct one.
2. **A deliberate behavior tightening (removing an accidental permissive coercion) is worth naming
   and testing explicitly, even when it's a side effect rather than the main goal.** Switching from
   `->>'v'`-extraction-then-cast to whole-object comparison silently removed a pre-existing
   permissive cross-numeric-type coercion (`Equal(Integer(5))` could previously match a stored
   `Double(5.0)` via Postgres's own text-to-bigint coercion) — named as AC-ENV-07 and tested
   directly, not left as an unremarked side effect a future reader might mistake for a regression.
3. **Reusing a helper doesn't mean the OLD tests referencing the previous dispatch path are still
   correct — they need updating, not just leaving to pass by coincidence.** The pre-existing
   `equal_still_generates_the_same_shape_as_before_the_refactor` test's own name and assertion were
   BOTH about the old `push_scalar_comparison`-based shape; updated in place (superseded-scenario
   style, matching this session's own established convention) rather than deleted or ignored.

## Key Files

- `crates/embyr-pg-storage/src/encoding/query.rs` — `Equal`/`NotEqual` dispatch (2 lines), doc
  comment updates, 5 new + 2 updated unit tests.
- `crates/embyr-core/`/`crates/embyr-server/` — untouched, confirmed by construction.
- `tests/firestore_equal_notequal_value_type_support/acceptance/` — 1 acceptance target, 4 tests.
- `docs/feature/firestore-equal-notequal-value-type-support/feature-delta.md` — full DISCUSS/DESIGN
  narrative.
- `docs/feature/firestore-equal-notequal-value-type-support/deliver/mutation/mutation-report.md`
- `docs/product/jobs.yaml`, JOB-01 — new NOTE appended.

## Follow-Up Work

- **`firestore-range-operator-value-type-support`** (candidate id) — range operators (`<`/`<=`/
  `>`/`>=`) still panic on `Timestamp`/`Bytes`/`Reference`/`Array`/`Map`-valued targets via
  `push_scalar_comparison`'s own unchanged fallback. Real Firestore commonly supports `Timestamp`
  range queries (`.where('createdAt', '>', someDate)`) — a real, likely high-priority gap, needing
  a genuinely different (ordering-based) mechanism than this feature's own reuse-only design.
- Carried forward, unchanged, from `firestore-query-filter-operator-support`: list-size limit
  enforcement, empty-list behavior confirmation, two-simultaneous-`IN`, the range-filter-orderBy
  -must-start-same-field query-validity constraint.
