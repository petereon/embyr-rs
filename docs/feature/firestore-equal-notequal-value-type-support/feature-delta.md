# Feature Delta: firestore-equal-notequal-value-type-support

## Wave: DISCUSS / [REF] Prior Wave Consultation — Reading Confirmation

✓ `docs/product/jobs.yaml`, JOB-01 (`sdk-compat`, persona P1 Alex) — read in full.
✓ `docs/evolution/2026-09-05-firestore-query-filter-operator-support.md` § Follow-Up Work — the
originating flag: "`Equal`/`NotEqual`'s own existing value-type gap ... pre-existing, orthogonal to
this feature's own scope. Named explicitly, not fixed here." This feature closes it.
✓ `crates/embyr-pg-storage/src/encoding/query.rs::push_scalar_comparison` (lines 157-177) —
confirmed the EXACT crash: its own match on `value` covers only `Integer/String/Double/Boolean`;
`_ => panic!("unsupported filter value type: {:?}", value)` fires for `Timestamp`/`Bytes`/
`Reference`/`Array`/`Map`. Called by ALL 6 comparison operators (`LessThan` through `NotEqual`,
line 47-53) — this feature scopes to exactly 2 of those 6 (`Equal`/`NotEqual`), per the user's own
explicit framing at kickoff; the other 4 (range operators) are a separately-evidenced, deferred
concern (§ Out of Scope).
✓ `crates/embyr-pg-storage/src/encoding/query.rs::push_value_equality` (lines 213-217) — confirmed
this ALREADY EXISTS, built during `firestore-query-filter-operator-support` (Slices 03-04) to solve
this EXACT problem for `In`/`NotIn`: it compares the WHOLE discriminated-union JSON object
(`fields->'{field}' {=|!=} $1::jsonb`, via `field_value_to_json`, never `->>'v'` extraction) — which
works UNIFORMLY for every `FieldValue` variant, since `field_value_to_json` already covers all of
them (`Null/Boolean/Integer/Double/Timestamp/String/Bytes/Reference/Array/Map`). **This feature
needs ZERO new production logic** — `Equal`/`NotEqual` simply need to CALL this already-existing,
already-tested function instead of `push_scalar_comparison`.
✓ `crates/embyr-pg-storage/src/encoding/field_value.rs::field_value_to_json` — re-confirmed it
handles every `FieldValue` variant including `Timestamp`/`Bytes`/`Reference`/`Array`/`Map` (the 5
variants that currently crash) — the exact fact `push_value_equality`'s own correctness depends on.
✓ `crates/embyr-pg-storage/src/encoding/query.rs`'s own unit test module (`mod tests`) — confirmed
`push_value_equality`'s own existing tests (`not_in_compares_the_whole_field_value_never_the
_unwrapped_v`, `in_with_a_null_target_does_not_panic`) already prove the mechanism works generically
— this feature's own tests extend coverage to the 4 previously-crashing types specifically, not
prove the mechanism itself again.

**No live web verification needed this DISCUSS** — unlike `firestore-query-filter-operator-support`
(which needed to verify a SEMANTIC rule, `not-in`'s field-must-exist behavior), this feature is a
pure mechanical fix: real Firestore has always supported `==`/`!=` against every field type
including timestamps, references, arrays, and maps (whole-value equality, not a range comparison) —
this is uncontroversial, ordinary Firestore behavior, not a subtle edge case requiring verification.

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

- Feature type: **Backend** (Decision 1) — a 2-line dispatch change plus test coverage, confined to
  `crates/embyr-pg-storage/src/encoding/query.rs`.
- JTBD: **reuse JOB-01** (Decision 4 = "Yes", existing job) — the same "close a documented crash"
  realization as `firestore-query-filter-operator-support`, its own direct predecessor.
- Walking Skeleton: **Yes** (Decision 2) — `Equal` on a `Timestamp` value (the single most common
  real-world case: `.where('createdAt', '==', someDate)`), proven end-to-end against a real,
  previously-panicking `RunQuery`.
- UX Research Depth: **Lightweight** (Decision 3) — a crash fix reusing an already-built mechanism,
  one persona, no new emotional arc.

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: P1 Alex (SDK Developer), unchanged.

**Job**: JOB-01 `sdk-compat`, unchanged job_story. This feature's own realization: any of Alex's
real Firebase app's queries using `.where(field, '=='|'!=', <non-scalar value>)` — e.g. filtering by
a `Timestamp`, a document `Reference`, an `Array`, or a `Map` — crash the request against embyr
today. After this feature, both operators execute correctly for every `FieldValue` type.

## Wave: DISCUSS / [REF] Job Discovery — Framing Resolution

### Resolution 1 — Reuse JOB-01, or a new job?

Identical shape to every "make it real" realization this session, and a direct continuation of the
immediately-prior feature's own flagged follow-up.

**Resolution**: **JOB-01 reuse, locked.**

### Resolution 2 — Reuse `push_value_equality` directly, or build new cast logic for the 5
previously-uncovered types?

| Option | Description | Fit |
|---|---|---|
| **(A) Add 5 new match arms to `push_scalar_comparison`** (`Timestamp`/`Bytes`/`Reference`/`Array`/`Map`, each needing its own cast/encoding logic) | Mirrors the EXISTING 4-arm shape | **Rejected** — duplicates logic `push_value_equality` already solved correctly for `In`/`NotIn`; would need to reinvent JSON-object comparison for composite types (`Array`/`Map` have no meaningful `->>'v'`-extractable scalar at all) |
| **(B) Route `Equal`/`NotEqual` to the ALREADY-EXISTING `push_value_equality`** — `Equal` → `push_value_equality(.., false)`, `NotEqual` → `push_value_equality(.., true)` | Zero new production logic; works uniformly for ALL 10 `FieldValue` variants (not just the 5 gap-closing ones), since `field_value_to_json` already covers everything | **Strongest fit** |

**Resolution**: **(B) is locked.** This is the laziest, most correct fix available: it is not
"support 5 more types," it is "stop using the wrong helper for 2 operators that never needed
type-specific casting in the first place" — equality/inequality against a value never needed a
numeric/string/boolean CAST, only a match against the value AS STORED, which whole-object JSON
comparison already provides.

**A direct semantic consequence, confirmed by reasoning, not assumed**: this ALSO fixes a subtler,
previously-latent correctness issue for the 4 ALREADY-supported types (`Integer`/`String`/`Double`/
`Boolean`) — the OLD `push_scalar_comparison`-based `Equal` compared only the unwrapped, type-CAST
`->>'v'` value, meaning a stored `Double(5.0)` field queried with `Equal(Integer(5))` would attempt
`(fields->'f'->>'v')::bigint = 5` — casting the STORED text `"5"` to `bigint`, which happens to
SUCCEED and match today (Postgres's own numeric-text coercion is permissive). Under `push_value_
equality`'s whole-object comparison, this same query would correctly NOT match (`{"t":"D","v":5.0}
!= {"t":"I","v":5}` as JSON) — matching real Firestore's own type-strict equality semantics (a
Firestore `==` filter never coerces across `Integer`/`Double`) more precisely than the OLD behavior
did. Named explicitly as a beneficial side effect, not silently absorbed (§ System Constraints).

### Resolution 3 — Scope: `Equal`/`NotEqual` only, or all 6 comparison operators?

The user's own explicit framing at kickoff scoped this feature to `Equal`/`NotEqual` specifically —
the 2 operators the ORIGINATING flagged follow-up named. The 4 range operators (`LessThan` through
`GreaterThanOrEqual`) share `push_scalar_comparison`'s own identical panic on the same 5 types — but
`push_value_equality`'s own whole-object-equality mechanism is STRUCTURALLY INAPPLICABLE to range
comparisons (`<`/`<=`/`>`/`>=` need ORDERING, not equality — JSON object comparison has no
meaningful "less than" for a `{"t":"TS","s":...,"n":...}` object). Real Firestore DOES support range
queries on `Timestamp` fields (a genuinely common idiom: `.where('createdAt', '>', someDate)`) — so
this is a REAL, separately-evidenced gap, not a hypothetical one, but it needs a DIFFERENT mechanism
(numeric/lexicographic ordering over the type's own natural comparison, e.g. `(seconds, nanos)`
tuple ordering for `Timestamp`) that this feature's own reuse-only design does not build.

**Resolution**: **locked to `Equal`/`NotEqual` only, per the user's own explicit scope.** Range
-operator support for `Timestamp` (and the other 4 types) is named, deferred, with a candidate
feature id proposed (§ Out of Scope) — a real, likely HIGH-priority follow-up, not merely a
theoretical gap.

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

**Signals checked**: >10 user stories? No (1). >3 bounded contexts/modules? No — ONE file, zero new
crates, zero new type, zero new function (reuses an existing one). Walking skeleton >5 integration
points? No (1: a real, previously-panicking `Equal`-on-`Timestamp` query, proven correct). Estimated
effort >2 weeks? No — 1 slice, well under a day (a 2-line dispatch change plus tests). Multiple
independent user outcomes? No — `Equal` and `NotEqual` are two faces of the identical fix (both
route to the same already-existing function), never independently meaningful to split further.

**Scope Assessment: PASS** (0 oversizing signals fired) — the smallest feature built this session
by every measured signal; a single Elephant Carpaccio slice is the entire feature.

## Wave: DISCUSS / [REF] Journey — Alex's "My Timestamp/Reference/Array/Map Filter Stops Crashing" Arc

### Mental model

Alex's real Firebase app filters documents by an exact timestamp, a document reference, an array
value, or a map value — all ordinary, supported Firestore `==`/`!=` idioms. Against embyr today,
every one of these crashes the request. After this feature, all 5 previously-uncovered types (plus
the 4 already-working ones, now MORE correctly type-strict) work identically.

### Failure modes (feeds DISTILL scenario generation)

- `.where('createdAt', '==', someTimestamp)`: today panics; after, correctly matches documents with
  that EXACT timestamp.
- `.where('ownerRef', '!=', someDocRef)`: today panics; after, correctly excludes the matching
  reference.
- `.where('tags', '==', ['a','b'])` (whole-array equality, order-sensitive per real Firestore's own
  semantics — `field_value_to_json`'s own `Array` encoding preserves element order): today panics;
  after, correctly matches only an identically-ordered array.
- A cross-type comparison (`Equal(Integer(5))` against a stored `Double(5.0)` field): previously
  matched (an accidental, permissive Postgres text-cast coercion); after this feature, correctly
  does NOT match — a deliberate, named behavior tightening (§ Resolution 2), not a regression.

## Wave: DISCUSS / [REF] Story Map & Walking Skeleton

### Backbone

Alex's real Firebase app issues an `Equal`/`NotEqual` query against a `Timestamp`/`Bytes`/
`Reference`/`Array`/`Map` field → embyr today crashes the request → this feature routes both
operators to the already-existing `push_value_equality` → the query succeeds with correct results.

### Walking Skeleton

**Slice 01 (the entire feature)**: route `Equal`/`NotEqual` to `push_value_equality`, proven
end-to-end against a real, previously-panicking `Equal`-on-`Timestamp` query, plus unit coverage
for all 5 previously-crashing types and the Resolution 2 type-strictness tightening.

## Wave: DISCUSS / [REF] Elephant Carpaccio Slices

| # | Story | Release | Estimate | Learning Hypothesis | Reference Class |
|---|---|---|---|---|---|
| 01 | US-01 | 1 (Walking Skeleton, the entire feature) | ≤0.25 day | Disproves: `Equal`/`NotEqual` can be made to work for every `FieldValue` type by reusing an already-existing, already-tested function, with zero new production logic | Mirrors `firestore-query-filter-operator-support`'s own `In`/`NotIn` design exactly — this feature is that design's own direct generalization to 2 more operators |

## Wave: DISCUSS / [REF] Prioritization

A single slice — no ordering decision needed.

## Wave: DISCUSS / [REF] System Constraints

- `crates/embyr-core/`/`crates/embyr-server/` are NOT touched — confirmed by construction (no new
  domain type, no new proto-translation logic; a pure dispatch-target change in one function).
- `push_scalar_comparison` remains UNCHANGED and continues to serve the 4 range operators
  exclusively — its own `_ => panic!(...)` fallback still fires for those operators against the 5
  uncovered types, a NAMED, deferred, separate gap (§ Out of Scope), not silently left ambiguous.
- The Resolution 2 type-strictness tightening (cross-numeric-type comparisons no longer coerce) is
  a DELIBERATE, documented behavior change — any existing test relying on the old permissive
  coercion (none found via direct search, confirmed by grep) would need updating; none exist.
- Mutation-testing lesson, reapplied from this session's own accumulated QUALITY_GATE history: unit
  tests for the specific type/operator combinations this feature closes are written DURING DELIVER;
  a `cargo-mutants --in-diff` pass is still budgeted at QUALITY_GATE regardless.

## Wave: DISCUSS / [REF] User Stories

### US-01: `Equal`/`NotEqual` Queries Stop Crashing on Every Field Type

**job_id**: JOB-01 | **Release**: 1 | **Persona**: P1 Alex

#### Elevator Pitch
Before: a real `RunQuery` using `.where('createdAt', '==', someTimestamp)` (or `!=`, or a
`Reference`/`Array`/`Map`-valued target) panics the specific gRPC request task.
After: run the identical `RunQuery` → sees the correct set of matching documents, no crash.
Decision enabled: Alex's real Firebase app's equality/inequality filters work identically to
production Firestore, for every field type, not just the 4 scalar types embyr already supported.

#### Acceptance Criteria
- [ ] AC-ENV-01: a real `RunQuery` with an `Equal` filter on a `Timestamp`-valued field returns
      documents whose field matches EXACTLY that timestamp.
- [ ] AC-ENV-02: a real `RunQuery` with a `NotEqual` filter on a `Reference`-valued field correctly
      excludes the matching reference and includes non-matching ones.
- [ ] AC-ENV-03: a real `RunQuery` with an `Equal` filter on an `Array`-valued field matches only an
      identically-ordered, identically-valued array (whole-array equality, not containment).
- [ ] AC-ENV-04: a real `RunQuery` with an `Equal` filter on a `Map`-valued field matches only an
      identical map.
- [ ] AC-ENV-05: no panic, no transport reset, for any of the above — the request completes with a
      normal gRPC response either way.
- [ ] AC-ENV-06 (regression guard, Resolution 2): `Equal`'s own pre-existing behavior for
      `Integer`/`String`/`Double`/`Boolean` targets is UNCHANGED for TYPE-MATCHED comparisons (a
      `String` field queried with an equal `String` target still matches correctly).
- [ ] AC-ENV-07 (documented behavior tightening, Resolution 2): a CROSS-TYPE numeric comparison
      (`Equal(Integer(5))` against a stored `Double(5.0)` field) now correctly does NOT match,
      proving the old permissive text-cast coercion is gone.

## Wave: DISCUSS / [REF] Outcome KPIs

### Feature: firestore-equal-notequal-value-type-support

### Objective
Eliminate the last named, flagged crash risk from the `firestore-query-filter-operator-support`
follow-up list: `Equal`/`NotEqual` panicking on `Timestamp`/`Bytes`/`Reference`/`Array`/`Map`-valued
filter targets.

### Outcome KPIs
| KPI | Target | Measurement |
|---|---|---|
| Previously-panicking `Equal`/`NotEqual` type combinations fixed | 5 of 5 (`Timestamp`/`Bytes`/`Reference`/`Array`/`Map`) | Direct: AC-ENV-01 through AC-ENV-04's own end-to-end proofs |
| New production logic added | 0 lines (pure reuse of `push_value_equality`) | Confirmed directly, mirrors this session's own "confirmatory slice" discipline where applicable |
| Regression on the 4 pre-existing type-matched comparisons | 0 | AC-ENV-06 |
| Mutation-testing kill rate on the changed dispatch | 100% effective (this session's own established bar) | `cargo-mutants --in-diff`, `--lib`-scoped |

## Wave: DISCUSS / [REF] Out of Scope

- **Range-operator (`<`/`<=`/`>`/`>=`) support for `Timestamp`/`Bytes`/`Reference`/`Array`/`Map`** —
  a REAL, likely HIGH-priority gap (real Firestore commonly supports `Timestamp` range queries), but
  needs a genuinely different mechanism (ordering, not equality) that this feature's own
  reuse-only design does not build. Named, deferred, candidate id proposed:
  `firestore-range-operator-value-type-support`. Explicitly locked out per the user's own scope at
  kickoff (§ Resolution 3), not silently expanded into.
- **`IsNan`/`IsNotNan` on non-`Double` types** — unrelated; these 2 operators already have their own
  dedicated, unaffected code path (string-sentinel equality, special-cased before the main match).

## Wave: DISCUSS / [REF] WS Strategy

**Strategy A** (real, minimal, end-to-end) — the single slice is a real, previously-panicking
`Equal`-on-`Timestamp` query proven to return correct results, not a mock.

## Wave: DISCUSS / [REF] Driving Ports

gRPC `:8080` `RunQuery` (existing route, zero new RPC).

## Wave: DISCUSS / [REF] Pre-requisites

- `firestore-query-filter-operator-support` (FINALIZED 2026-09-05) — provides `push_value_equality`,
  reused unchanged.
- No new external dependency, no new bounded context — the smallest architectural footprint of any
  feature built this session (smaller even than its own direct predecessor).

## Wave: DISCUSS / [REF] Handoff Package

Handed to `nw-solution-architect` (DESIGN): this feature-delta.md, both Resolutions, and the
explicit instruction to design the exact 2-line dispatch change plus the doc-comment updates
`push_scalar_comparison`/`push_value_equality` each need to reflect their own now-narrower/wider
callers.

## Wave: DISCUSS / [REF] SSOT Updates

`docs/product/jobs.yaml`, JOB-01 entry: append a new dated NOTE — "JOB-01 now also covers `Equal`/
`NotEqual` support for every `FieldValue` type (`Timestamp`/`Bytes`/`Reference`/`Array`/`Map`,
previously panicking) — same job, same persona, not a new job. Direct continuation of
`firestore-query-filter-operator-support`'s own flagged follow-up. Zero new production logic: routes
both operators to that feature's own already-existing `push_value_equality` helper. Side effect: a
previously-permissive cross-numeric-type coercion (`Equal(Integer)` matching a stored `Double`) is
now correctly type-strict, matching real Firestore's own semantics more precisely. Range-operator
support for these same 5 types is a separate, real, deferred gap (candidate id
`firestore-range-operator-value-type-support`). See
docs/feature/firestore-equal-notequal-value-type-support/feature-delta.md § Resolution 2/3."

## Wave: DISCUSS / [REF] Definition of Ready Validation

### Requirements Completeness Score: **0.97**

### DoR Checklist (9-item hard gate)
1. [x] Every story traces to a job_id (JOB-01)
2. [x] Story has a complete Elevator Pitch
3. [x] Every AC is testable without ambiguity
4. [x] Walking Skeleton identified (US-01, the entire feature)
5. [x] Scope Assessment passed
6. [x] No slice contains only `@infrastructure` stories
7. [x] Out of Scope explicitly named (2 items)
8. [x] Outcome KPIs have numeric targets and measurement methods
9. [x] Prior-wave artifacts read and reconciled (the originating flagged follow-up is confirmed,
   not re-litigated)

### DoR Status: **PASSED**

## Wave: DISCUSS / [REF] Open Questions

None — this is a mechanical reuse fix with a clear, already-verified mechanism; the one adjacent
question (range-operator support) is explicitly scoped out, not left ambiguous.

## Wave: DISCUSS / [REF] Wave Decisions Summary

### Key Decisions
- [D1] Reuse JOB-01 (Resolution 1).
- [D2] Route `Equal`/`NotEqual` to the ALREADY-EXISTING `push_value_equality` — zero new production
  logic (Resolution 2). Beneficial side effect: cross-numeric-type comparisons become correctly
  type-strict.
- [D3] Scope locked to `Equal`/`NotEqual` only, per the user's own explicit framing — range-operator
  support for the same 5 types is a real, separate, deferred gap (Resolution 3).

### Requirements Summary
- Primary need: `Equal`/`NotEqual` crash on `Timestamp`/`Bytes`/`Reference`/`Array`/`Map`-valued
  filter targets.
- Walking skeleton scope: the entire feature — a 2-line dispatch change reusing an existing helper.
- Feature type: Backend.

### Constraints Established
- Zero new production logic — pure reuse of `push_value_equality`.
- `push_scalar_comparison` unchanged, continues serving only the 4 range operators.
- A deliberate, named behavior tightening (cross-numeric-type coercion removed) — not a silent side
  effect.

### Upstream Changes
- None — a direct, flagged follow-up from `firestore-query-filter-operator-support`'s own FINALIZE.

## Wave: DISCUSS / [REF] Next Wave

**Handoff To**: nw-solution-architect (DESIGN wave)
**Deliverables**: this feature-delta.md, 3 locked Resolutions, 1-slice plan

---

## Wave: DESIGN / [REF] Prior Wave Consultation — Reading Confirmation

✓ This feature-delta.md's own DISCUSS sections in full.
✓ `crates/embyr-pg-storage/src/encoding/query.rs`, lines 47-53 (the `Equal`/`NotEqual` dispatch
sites) and lines 213-217 (`push_value_equality`'s own exact signature) — confirmed the precise
2-line edit: `FilterOp::Equal => push_value_equality(qb, &f.field_path, &f.value, false)`,
`FilterOp::NotEqual => push_value_equality(qb, &f.field_path, &f.value, true)`.

## Wave: DESIGN / [REF] Architecture Design

One change, zero new function, zero new type:

1. **`append_field_filter`'s own match** (lines 47-53): `FilterOp::Equal`/`FilterOp::NotEqual` call
   `push_value_equality` instead of `push_scalar_comparison`.
2. **Doc comments updated in place** (no behavior change, correctness of documentation only):
   `push_scalar_comparison`'s own doc comment (already mentions it serves "the 6 pre-existing
   operators... which it continues to serve exclusively" from the prior feature) is corrected to
   say "4" instead of "6", naming `Equal`/`NotEqual`'s own new home explicitly.
   `push_value_equality`'s own doc comment (currently says "used by `In`/`NotIn`") is widened to
   also name `Equal`/`NotEqual`.

## Wave: DESIGN / [REF] Wave Decisions Summary

### Key Decisions
- [D1] A 2-line dispatch change, zero new function — the simplest possible fix, confirmed correct
  by direct reasoning about `push_value_equality`'s own existing, already-tested semantics.
- [D2] Doc comments updated for accuracy, not behavior — `push_scalar_comparison`'s own "6 operators
  it serves" claim becomes stale the moment `Equal`/`NotEqual` move away from it.

### Constraints Established
- No new dependency, no new bounded context, no new port/adapter trait method.
- `push_scalar_comparison`'s own remaining 4 callers (`LessThan` through `GreaterThanOrEqual`) are
  completely unaffected — confirmed by construction (the edit touches only 2 of 6 match arms).

## Wave: DESIGN / [REF] Next Wave

**Handoff To**: nw-software-crafter (DELIVER wave, per this project's own established convention)
**Deliverables**: this feature-delta.md's DESIGN section
