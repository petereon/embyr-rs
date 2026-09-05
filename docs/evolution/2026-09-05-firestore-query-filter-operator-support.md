# Evolution: firestore-query-filter-operator-support

**Date:** 2026-09-05
**Feature:** Fixes a real, active production crash — `append_field_filter`
(`crates/embyr-pg-storage/src/encoding/query.rs`) previously panicked on 4 common Firestore query
filter operators (`in`, `not-in`, `array-contains`, `array-contains-any`); any live query using one
of them crashed the specific gRPC request task.
**Job:** JOB-01 (`sdk-compat`) — the highest-severity realization of this job's own "make it real"
pattern this session: a crash, not merely an accuracy gap.
**ADRs:** none (folded into feature-delta.md's own DESIGN section — small enough scope that a
separate ADR file added nothing).

## Business Context

Discovered as a flagged follow-up from `composite-index-requirement-rules`'s own FINALIZE
(2026-09-05): while proving that feature's own regression guards, a real acceptance test crashed
with `panic!("unsupported filter op: {:?}", f.op)` from `append_field_filter`'s own fallback match
arm. The function only implemented SQL translation for `<`/`<=`/`>`/`>=`/`==`/`!=` (plus `IsNan`/
`IsNotNan`, special-cased) — every other `FilterOp` variant crashed. This closes that gap for all 4
of the remaining variants: `In`, `NotIn`, `ArrayContains`, `ArrayContainsAny`.

**Result: the production crash is fixed.** All 4 operators now execute correctly, verified end-to
-end against real Postgres, matching real Firestore's own documented semantics.

## Key Decisions

| Decision | Verdict |
|---|---|
| Reuse JOB-01 — the highest-severity "make it real" realization this session (a crash, not an accuracy gap) | feature-delta.md § Resolution 1 |
| `ArrayContains`/`ArrayContainsAny`: JSONB `@>` containment, reusing `field_value_to_json`'s own already-proven encoding — new `push_array_contains` helper | § Resolution 2 |
| `In`/`NotIn`: whole-discriminated-union-object equality/inequality (NOT the pre-existing `->>'v'`-extraction dispatch) — new `push_value_equality` helper | § Resolution 2, revised mid-DELIVER (see § Lessons Learned) |
| `push_scalar_comparison` extracted from the original inline match — a pure refactor serving only the 6 pre-existing operators (`LessThan` through `NotEqual`) | DESIGN § D1 |
| Empty target list: explicit `FALSE` fallback for `In`/`ArrayContainsAny`; explicit `fields->'{field}' IS NOT NULL` fallback for `NotIn` | § Resolution 3 |
| List-size limits (30 for `in`/`array-contains-any`, 10 for `not-in`) not enforced — a performance concern, not correctness, out of scope | § Resolution 4 |

## Steps Completed

1. **Slice 01** (US-01, Walking Skeleton) — `ArrayContains`: `push_array_contains`, a single JSONB
   `@>` containment check against a one-element array.
2. **Slice 02** (US-02) — `ArrayContainsAny`: OR of `push_array_contains` checks, one per target.
3. **Slice 03** (US-03) — `In`: extracted `push_scalar_comparison` (pure refactor, zero behavior
   change for pre-existing operators, proven by an explicit regression-guard test); OR of scalar
   -equality checks for `In` itself.
4. **Slice 04** (US-04, LAST slice) — `NotIn`: AND of scalar-inequality checks, matching real
   Firestore's own live-verified field-must-exist rule (a document missing the filtered field
   entirely is excluded from `not-in` results; a document with the field explicitly `null` is
   included).

All 4 slices landed in one DELIVER commit (a deliberate deviation from this session's own strict
per-slice-commit convention — the 4 operators are tightly interleaved in one `match` statement and
one function, making a genuinely cohesive single change; per-slice commits would have artificially
fragmented one atomic diff).

**QUALITY_GATE** — `cargo-mutants --in-diff`, `--lib`-scoped, Docker-free. First pass found 15
misses across 2 root causes: (1) join-boundary mutants on the `ArrayContainsAny`/`In`/`NotIn`
separator-insertion loops, where 2-target tests only checked separator PRESENCE (which survives a
mutant that shifts the separator's own position); (2) 10 misses in `push_scalar_comparison`'s own
pre-existing operators/types, genuinely untested by ANY prior unit test, exposed to `--in-diff`
scope only because this feature's own refactor moved those lines. Both closed with targeted tests.
**Second pass: 27/27 caught, 0 missed, 0 unviable, 0 timeouts — the cleanest QUALITY_GATE result of
any feature this session.**

**Full regression**: `cargo test -p embyr-server`, 441 tests passed, 56 target binaries, 0 failures
attributable to this feature (1 pre-existing, unrelated `distributed_rate_limiting` flake, the same
documented failure mode from this session's prior evolution docs).

## Lessons Learned

1. **The single most important finding of this feature: a real bug was discovered and fixed in this
   feature's OWN first-draft design, not a pre-existing one — caught by a genuinely failing
   acceptance test, not by inspection.** The initial `NotIn` implementation reused
   `push_scalar_comparison`'s `->>'v'`-extraction dispatch (the same mechanism as the pre-existing
   `!=` operator). This CANNOT distinguish a field explicitly set to `Null` (`{"t":"N"}`, a real
   JSONB value with no `"v"` key) from a field that is entirely ABSENT (`fields->'{field}'` itself
   SQL `NULL`) — both produce SQL `NULL` after `->>'v'` extraction, so both were wrongly excluded
   identically, breaking the live-verified field-must-exist rule for the present-but-null case
   (AC-QFO-10). **Fix**: compare the WHOLE discriminated-union JSON object instead of the unwrapped
   value (`push_value_equality`) — only genuine field absence produces SQL `NULL` this way. This
   also eliminated the `In`/`NotIn` "unsupported filter value type" panic as a side benefit
   (`field_value_to_json` covers every `FieldValue` variant uniformly).
2. **This is a genuinely reusable pattern worth remembering**: any future filter/comparison logic in
   this codebase that needs to distinguish "field present but null/falsy" from "field absent" must
   compare the WHOLE stored JSON value, never the unwrapped `"v"` — extracting `"v"` first
   structurally erases that distinction for `Null`-valued fields specifically (though NOT for other
   falsy-but-non-null values like `0`/`false`/`""`, which retain their own `"v"` key and compare
   correctly either way).
3. **A pure refactor's own "for free" side benefit can retroactively expose old test debt, and
   fixing it cheaply beats deferring it.** Extracting `push_scalar_comparison` made 5 pre-existing
   operators' own untested type-dispatch arms directly unit-testable for the first time — closing
   that gap cost one small parameterized test, cheaper than the original inline-match version would
   have been to test at all.
4. **A "does this look right" review of composition logic (OR/AND joining) is not the same as
   testing it precisely enough to catch a boundary shift.** Both `contains(" OR ")` checks that
   initially passed were TRUE both before and after 2 of the 3 join-boundary mutants — presence
   -only assertions are a weaker signal than they look for anything involving positional/count
   correctness; an exact-count assertion against a 3+ element input is what actually distinguishes
   correct composition from a subtly-shifted one.

## Key Files

- `crates/embyr-pg-storage/src/encoding/query.rs` — `push_array_contains`, `push_value_equality`
  (new); `push_scalar_comparison` (extracted, pure refactor); `In`/`NotIn`/`ArrayContains`/
  `ArrayContainsAny` arms in `append_field_filter`; 13 unit tests.
- `crates/embyr-core/`/`crates/embyr-server/` — **untouched**, confirmed by construction.
- `tests/firestore_query_filter_operator_support/acceptance/` — qfo01 through qfo04, 4 acceptance
  targets, shared `common/mod.rs` (re-exports `security_rules`'s own full composition-root
  harness).
- `docs/feature/firestore-query-filter-operator-support/feature-delta.md` — full DISCUSS/DESIGN
  narrative.
- `docs/feature/firestore-query-filter-operator-support/slices/` — 4 elephant-carpaccio slice
  briefs.
- `docs/feature/firestore-query-filter-operator-support/deliver/mutation/mutation-report.md`
- `docs/product/jobs.yaml`, JOB-01 — new NOTE appended.

## Follow-Up Work

- **List-size limit enforcement** (30 for `in`/`array-contains-any`, 10 for `not-in`) — real
  -Firestore-accurate, a performance/validation concern, not correctness. Named, deferred.
- **Empty-list behavior beyond the best-effort fallback locked in Resolution 3** — real Firestore's
  own exact behavior for this case was not confirmed by live verification. Named, deferred pending
  stronger evidence.
- **`Equal`/`NotEqual`'s own existing value-type gap** (they still panic on `Timestamp`/`Bytes`/
  `Reference`/`Array`/`Map`-valued targets, via `push_scalar_comparison`'s own `_ => panic!(...)`
  fallback) — pre-existing, orthogonal to this feature's own scope. Named explicitly, not fixed
  here.
- Carried forward, unchanged, from `composite-index-requirement-rules`: two-simultaneous-`IN`
  (undetermined by live verification), the range-filter-orderBy-must-start-same-field query
  -validity constraint.
