# Feature Delta: firestore-query-filter-operator-support

## Wave: DISCUSS / [REF] Prior Wave Consultation — Reading Confirmation

✓ `docs/product/jobs.yaml`, JOB-01 (`sdk-compat`, persona P1 Alex) — read in full.
✓ `docs/feature/composite-index-requirement-rules/feature-delta.md` § Discovered Gap — the
originating finding: this feature's own existence was flagged there, discovered mid-DELIVER via a
REAL crashing test, not speculation.
✓ `crates/embyr-pg-storage/src/encoding/query.rs::append_field_filter` (full, 66 lines) — confirmed
the EXACT crash: its own match on `f.op` covers only `LessThan/LessThanOrEqual/GreaterThan/
GreaterThanOrEqual/Equal/NotEqual` (plus `IsNan`/`IsNotNan`, special-cased above); `In`, `NotIn`,
`ArrayContains`, `ArrayContainsAny` all hit `_ => panic!("unsupported filter op: {:?}", f.op)`. The
subsequent value-type match (`Integer/String/Double/Boolean` cast-and-bind, `_ => panic!(...)`)
already exists and is REUSED unchanged by this feature for `In`/`NotIn` (§ Decision below) — not a
new gap this feature introduces or is responsible for widening (a `Timestamp`/`Bytes`/`Reference`/
`Array`/`Map`-valued filter target already panics for `Equal` TODAY, unrelated to this feature).
✓ `crates/embyr-pg-storage/src/encoding/field_value.rs::field_value_to_json`/`json_to_field_value`
— confirmed the EXACT JSONB storage shape: every field is a discriminated-union object
`{"t": "<code>", "v": <value>}` (e.g. `{"t": "S", "v": "beach"}`); an array field is `{"t": "A",
"v": [<element>, <element>, ...]}`, each element itself a full discriminated-union object — never a
bare scalar array. This is the load-bearing fact this feature's own SQL design depends on.
✓ `crates/embyr-server/src/encoding/firestore_proto.rs::proto_value_to_field_value` (lines 46-70) —
confirmed: a proto `ArrayValue` (the wire shape real Firestore clients send for `in`/`not-in`/
`array-contains-any`'s own value) translates to `FieldValue::Array(Vec<FieldValue>)` — so
`FieldFilter.value` for these 3 operators is ALWAYS a `FieldValue::Array` at the point `append_
field_filter` receives it; `ArrayContains`'s own value is always a single scalar (the one element
being searched for), never wrapped in an `Array`.
✓ `crates/embyr-pg-storage/src/backend_adapter.rs` (lines 295-362) — confirmed `sqlx` ALREADY binds
`serde_json::Value` directly for `::jsonb`-cast columns/expressions elsewhere in this exact crate
(`fields_to_json(...)` bound via `.bind(&fields_json)`, no new sqlx feature or dependency needed) —
this feature's own JSONB containment (`@>`) predicates reuse this identical, already-proven binding
mechanism.
✓ `crates/embyr-core/src/domain/field_value.rs::FieldValue` — confirmed the full variant list
(`Null/Boolean/Integer/Double/Timestamp/String/Bytes/Reference/Array/Map`) — no new variant needed.

**Live web verification** (firebase.google.com/docs/rules/... and docs.cloud.google.com/firestore/
docs/query-data/queries, fetched directly — this feature's own `not-in` semantic is genuinely subtle
enough that recollection alone risked a SILENTLY WRONG result, not merely a crash, so verification
was warranted here exactly as it was for `composite-index-requirement-rules`'s own equality
-vs-inequality finding):
- **`not-in` excludes documents where the filtered field does not exist at all** — a field "exists"
  once set to ANY value including `null`/`""`/`NaN`; only a field ENTIRELY ABSENT from the document
  is excluded from `not-in` results, regardless of the excluded value list.
- **`!=` (not-equal) has the IDENTICAL field-must-exist requirement.**
- **Documented list-size maximums**: `in`/`array-contains-any` — 30 values; `not-in` — 10 values.
- **Empty-list rejection**: NOT confirmed by the sources fetched — real Firestore's own behavior
  for an empty `in`/`not-in`/`array-contains-any` list is unverified. Named explicitly, not guessed
  (§ Resolution 3).

**A load-bearing design finding, confirmed by direct semantic reasoning against the verified facts
above, not merely assumed**: `!=`'s own EXISTING implementation (unmodified, already shipped) is
ALREADY field-presence-correct TODAY, by accident of SQL's own three-valued logic — `fields->'{fp}'
->>'v'` on a MISSING field evaluates to SQL `NULL`, and `NULL != $1` (or any cast of `NULL`) is SQL
`NULL`, which a `WHERE` clause treats as `false` — the row is excluded, exactly matching the
verified real-Firestore behavior, with ZERO code changes needed to prove it. This SAME property
extends for free to this feature's own `NotIn` design (§ Decision — AND-chain of per-element `!=`
clauses): if the field is missing, EVERY per-element `!=` clause is `NULL`, so the `AND`-chain is
`NULL`, so the row is excluded — the field-must-exist rule falls out of Postgres's own `NULL`
propagation, needing NO explicit existence check in the common (non-empty-list) case.

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

- Feature type: **Backend** (Decision 1) — a pure SQL-translation fix inside one file
  (`crates/embyr-pg-storage/src/encoding/query.rs`); zero new admin route, zero new RPC, zero SDK
  -visible surface change beyond "queries that used to crash now return correct results."
- JTBD: **reuse JOB-01** (Decision 4 = "Yes", existing job) — closes a documented crash bug in
  Alex's own "SDK behaves identically to real Firestore" job; the "make it real" pattern, but for a
  CRASH, not merely an accuracy gap — the highest-severity realization of this pattern this session.
- Walking Skeleton: **Yes** (Decision 2) — `ArrayContains` (the simplest, single-scalar JSONB
  containment check), proven end-to-end against a real, previously-panicking `RunQuery`.
- UX Research Depth: **Lightweight** (Decision 3) — a SQL-translation bug fix, one persona (Alex),
  no new emotional arc; severity is the driving factor, not UX novelty.

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: P1 Alex (SDK Developer), unchanged.

**Job**: JOB-01 `sdk-compat`, unchanged job_story. This feature's own realization: any of Alex's
real Firebase app's queries using `.where(field, 'in'|'not-in'|'array-contains'|'array-contains
-any', ...)` — all 4 real, commonly-used Firestore query operators — crash the request against
embyr today. After this feature, all 4 execute correctly, matching real Firestore's own documented
semantics (including the subtle `not-in`/field-must-exist rule, live-verified above).

## Wave: DISCUSS / [REF] Job Discovery — Framing Resolution

### Resolution 1 — Reuse JOB-01, or a new job?

Identical shape to every prior "make it real" realization this session — a documented (here: by a
crashing test, not by SPEC.md) gap in Alex's own "behave identically to real Firestore" job.

**Resolution**: **JOB-01 reuse, locked.** This is the highest-severity instance of the pattern —
every prior realization closed an ACCURACY gap (a query silently behaved differently); this one
closes a CRASH.

### Resolution 2 — SQL design for each operator, confirmed against the exact JSONB storage shape

| Operator | `f.value` shape | SQL design |
|---|---|---|
| `ArrayContains` | single scalar | `fields->'{field}'->'v' @> $1::jsonb`, where `$1` is bound as a ONE-ELEMENT JSON array `[<discriminated-union-encoded target>]` — Postgres JSONB `@>` containment checks the LHS array contains every element of the RHS array; a 1-element RHS means "contains this element." |
| `ArrayContainsAny` | `FieldValue::Array(targets)` | `(fields->'{field}'->'v' @> $1::jsonb OR fields->'{field}'->'v' @> $2::jsonb OR ...)`, one `@>` disjunct per target, each a 1-element JSON array — Postgres has no native "array contains any of these elements" operator, this is the simplest correct composition. |
| `In` | `FieldValue::Array(targets)` | `(<scalar equality against target_1> OR <scalar equality against target_2> OR ...)`, REUSING the EXISTING per-`FieldValue`-type cast-and-bind dispatch (`Integer`→`bigint`, `String`→text, `Double`→`float8`, `Boolean`→`boolean`) unchanged, extracted into a shared helper (§ DESIGN). |
| `NotIn` | `FieldValue::Array(targets)` | `(<scalar inequality against target_1> AND <scalar inequality against target_2> AND ...)`, same shared helper, `!=` instead of `=` — the field-must-exist rule (§ Reading Confirmation) falls out for free via `NULL` propagation, IDENTICAL to how `!=` already works today, for any NON-EMPTY target list. |

**Resolution**: **locked, per the table above.** No new domain type, no new `FilterOp` variant — a
pure SQL-generation widening reusing 2 already-proven mechanisms (JSONB `@>` containment via an
already-bound `serde_json::Value`; the existing scalar cast-and-bind dispatch).

### Resolution 3 — Empty target list (`In`/`NotIn`/`ArrayContainsAny` with zero elements): what happens?

Live verification could not confirm whether real Firestore even ALLOWS an empty list at the API
level (§ Reading Confirmation). This feature's own OR/AND-composition design has a genuine
degenerate case here: an OR of ZERO disjuncts has no natural SQL fallback (never simply "match
nothing" without an explicit `false`), and — more importantly — `NotIn`'s own free field
-must-exist property (§ Reading Confirmation) is DERIVED FROM having at least one per-element
clause; with zero elements there is no clause to derive it from.

| Option | Description | Fit |
|---|---|---|
| **(A) Let it fall through to whatever the empty OR/AND naturally produces** | An empty `OR` chain built via `.join(" OR ")` on an empty `Vec` produces an EMPTY SQL string — a syntax error, a NEW crash this feature would introduce | **Rejected** |
| **(B) Explicit fallback**: empty `In`/`ArrayContainsAny` → SQL literal `FALSE` (matches nothing, the natural reading of "value must equal/contain one of zero possibilities"); empty `NotIn` → `fields->'{field}' IS NOT NULL` (the field-must-exist rule alone, since "not equal to any of zero excluded values" is vacuously true for any EXISTING field) | Never crashes, matches the analogous non-empty-list behavior's own field-existence property exactly, defensible even without a confirmed real-Firestore empty-list behavior to match | **Strongest fit** |

**Resolution**: **(B) is locked.** Named explicitly as a best-effort, non-verified edge case (not
silently guessed) — if real Firestore is later confirmed to reject empty lists as a client-side
validation error instead, that would be a separate, narrow follow-up (a request-validation check,
not a SQL-translation change).

### Resolution 4 — List-size limits (30 for `in`/`array-contains-any`, 10 for `not-in`): enforce here?

Real Firestore enforces these as a query-validation error before the query ever reaches its own
execution engine. This codebase has ZERO precedent of validating ANY filter's own input shape at
`RunQuery` time for other operators either (nothing currently checks that `Equal`'s own value isn't
an unsupported type before the SQL-generation panic, e.g.) — building a NEW validation layer here
would be scope creep unrelated to this feature's own severity-driven goal (stop crashing; return
correct results for the evidenced range).

**Resolution**: **not enforced, named, deferred.** An oversized list produces a slower query (many
`OR`/`AND` disjuncts), never a wrong result or a crash — a performance concern, not a correctness
one, out of this feature's own scope.

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

**Signals checked**: >10 user stories? No (4, one per operator). >3 bounded contexts/modules? No —
ONE file (`crates/embyr-pg-storage/src/encoding/query.rs`), zero new crates, zero new domain type.
Walking skeleton >5 integration points? No (1: a real, previously-panicking `ArrayContains` query,
proven correct end-to-end). Estimated effort >2 weeks? No — 4 slices, each ≤1 day, all touching the
SAME function via well-understood, already-proven mechanisms (JSONB `@>`, existing scalar dispatch).
Multiple independent user outcomes? Each operator is independently valuable and independently
shippable (fixing `ArrayContains` alone already stops SOME crashes) — Elephant Carpaccio's own
encouraged shape, not an oversizing signal.

**Scope Assessment: PASS** (0 oversizing signals fired) — right-sized as one feature.

## Wave: DISCUSS / [REF] Journey — Alex's "My Real Query Stops Crashing" Arc

### Mental model

Alex's real Firebase app already uses `.where('tags', 'array-contains', 'urgent')` or `.where(
'status', 'in', ['open', 'pending'])` in production — ordinary, common Firestore idioms, never
exotic. Against embyr today, EVERY one of these calls crashes the specific request (a raw transport
reset, not a clean error Alex's own SDK error-handling could even catch cleanly). After this
feature, all 4 operators execute correctly, with results matching real Firestore's own documented
semantics.

### Failure modes (feeds DISTILL scenario generation)

- A field entirely absent from a document, queried with `not-in`: correctly EXCLUDED from results
  (field-must-exist rule, live-verified, falls out of `NULL` propagation for free).
- A field explicitly set to `null`, queried with `not-in` against a list not containing `null`:
  correctly INCLUDED (the field "exists" per Firestore's own definition, even when its value is
  `null`) — this is the one path where the AND-chain's own `NULL`-propagation property needs a
  direct test proving the FIELD-PRESENT-BUT-VALUE-NULL case behaves differently from the
  FIELD-ABSENT case (§ Slice 04's own acceptance test).
- An empty `In`/`ArrayContainsAny` list: matches nothing, never crashes (§ Resolution 3).
- An empty `NotIn` list: matches every document that HAS the field, regardless of value (§
  Resolution 3).
- A heterogeneous `In` list (e.g. `[1, "two"]`): each element dispatches independently through its
  own `FieldValue` type — no special handling needed, confirmed by the design's own per-element
  composition.

## Wave: DISCUSS / [REF] Story Map & Walking Skeleton

### Backbone

Alex's real Firebase app issues a query using one of the 4 currently-panicking operators → embyr
today crashes the request → this feature makes each operator translate to correct SQL → the query
succeeds with correct results, matching real Firestore's own documented semantics.

### Walking Skeleton

**Slice 01**: `ArrayContains` — the single simplest operator (one JSONB `@>` check, no
composition), proven end-to-end against a real, previously-panicking query.

### Release 1 — Every Currently-Panicking Operator Stops Crashing (Slices 01–04, US-01 through US-04)

All 4 operators, each independently shippable.

## Wave: DISCUSS / [REF] Elephant Carpaccio Slices

| # | Story | Release | Estimate | Learning Hypothesis | Reference Class |
|---|---|---|---|---|---|
| 01 | US-01 | 1 (Walking Skeleton) | 0.5 day | Disproves: a real, previously-panicking `ArrayContains` query can be made to execute correctly using the SAME `serde_json::Value` binding mechanism already proven elsewhere in this crate, without a new sqlx feature or dependency | Mirrors this crate's own existing `fields_to_json(...)`/`.bind(&fields_json)` precedent, applied to a new (smaller) JSON shape |
| 02 | US-02 | 1 | 0.5 day | Disproves: `ArrayContainsAny`'s own OR-of-containment-checks composition needs anything beyond N copies of Slice 01's own single-check shape | Same reference class as Slice 01 |
| 03 | US-03 | 1 | 1 day | Disproves: `In`'s own OR-of-scalar-equality composition can be built by extracting the EXISTING per-`FieldValue`-type dispatch into a shared helper without changing `Equal`'s own observable behavior (regression guard) | Mirrors `composite-index-requirement-rules`'s own "targeted rule addition, not a rewrite" discipline |
| 04 | US-04 | 1 | 1 day | Disproves: `NotIn`'s own field-must-exist rule (live-verified, § Reading Confirmation) can be achieved via `NULL`-propagation alone (zero explicit existence-check code) for the non-empty-list case, with an explicit fallback only for the empty-list edge case (§ Resolution 3) | Same reference class as Slice 03 |

## Wave: DISCUSS / [REF] Prioritization

Ordered by learning leverage AND dependency: Slice 01 first — proves the CORE new mechanism (JSONB
`@>` containment via an already-bound JSON value) with the simplest possible shape. Slice 02 second
— a direct, low-risk generalization of Slice 01 (N copies of the same check, OR'd). Slice 03 third
— introduces the SECOND mechanism (reusing the existing scalar-dispatch helper), independent of
Slices 01-02, but ordered after them since it requires a small refactor (extracting the shared
helper) worth doing once the JSONB-containment half of the feature is already proven and committed.
Slice 04 last — depends on Slice 03's own shared helper, and carries the highest verification
burden (the live-verified field-must-exist rule needs its own direct acceptance-test proof,
including the FIELD-NULL-vs-FIELD-ABSENT distinction), so it benefits from landing after everything
else is stable.

## Wave: DISCUSS / [REF] System Constraints

- `crates/embyr-core/` is NOT touched by this feature — confirmed by construction (no new
  `FilterOp` variant, no new `FieldValue` variant; `append_field_filter`'s own signature is
  unchanged).
- `crates/embyr-server/`'s own proto-translation layer (`translate_field_op`, `proto_value_to_
  field_value`) is NOT touched — confirmed by construction (§ Reading Confirmation: it ALREADY
  correctly produces `FilterOp::In`/`NotIn`/`ArrayContains`/`ArrayContainsAny` and the right
  `FieldValue` shapes; the gap is entirely downstream, in SQL generation).
- No new `sqlx` feature, no new dependency — the `serde_json::Value`-to-`::jsonb` binding mechanism
  this feature reuses is ALREADY proven working in this exact crate (§ Reading Confirmation).
- Mutation-testing lesson, reapplied from this session's own accumulated QUALITY_GATE history:
  unit tests for EVERY new branch (each operator, the empty-list edge cases, the field-null-vs
  -absent distinction) are written DURING DELIVER, and a `cargo-mutants --in-diff` pass is still
  budgeted at QUALITY_GATE regardless.

## Wave: DISCUSS / [REF] User Stories

### US-01: `array-contains` Queries Stop Crashing (Walking Skeleton)

**job_id**: JOB-01 | **Release**: 1 | **Persona**: P1 Alex

#### Elevator Pitch
Before: a real `RunQuery` using `.where('tags', 'array-contains', 'urgent')` panics the specific
gRPC request task — the client observes a raw transport reset.
After: run the identical `RunQuery` → sees the correct set of documents whose `tags` array contains
`'urgent'`, no crash.
Decision enabled: Alex's real Firebase app's `array-contains` queries work identically to
production Firestore.

#### Acceptance Criteria
- [ ] AC-QFO-01: a real `RunQuery` with an `array-contains` filter on a field whose array DOES
      contain the target value returns that document.
- [ ] AC-QFO-02: a real `RunQuery` with an `array-contains` filter on a field whose array does NOT
      contain the target value does NOT return that document (no false positive).
- [ ] AC-QFO-03: no panic, no transport reset — the request completes with a normal gRPC response
      either way.

### US-02: `array-contains-any` Queries Stop Crashing

**job_id**: JOB-01 | **Release**: 1 | **Persona**: P1 Alex

#### Elevator Pitch
Before: a real `RunQuery` using `.where('tags', 'array-contains-any', ['urgent', 'important'])`
panics.
After: run the identical `RunQuery` → sees every document whose `tags` array contains AT LEAST ONE
of the listed values, no crash.
Decision enabled: Alex's app's multi-value tag-matching queries work identically to production.

#### Acceptance Criteria
- [ ] AC-QFO-04: a real `RunQuery` with an `array-contains-any` filter returns documents matching
      ANY listed value (proves the OR composition, not just a single-value degenerate case).
- [ ] AC-QFO-05: a document matching NONE of the listed values is correctly excluded.

### US-03: `in` Queries Stop Crashing

**job_id**: JOB-01 | **Release**: 1 | **Persona**: P1 Alex

#### Elevator Pitch
Before: a real `RunQuery` using `.where('status', 'in', ['open', 'pending'])` panics.
After: run the identical `RunQuery` → sees every document whose `status` equals ANY of the listed
values, no crash.
Decision enabled: Alex's app's status-filtering queries work identically to production.

#### Acceptance Criteria
- [ ] AC-QFO-06: a real `RunQuery` with an `in` filter returns documents matching ANY listed value.
- [ ] AC-QFO-07 (regression guard): the pre-existing `Equal` operator's own behavior is UNCHANGED
      after the shared scalar-dispatch helper extraction (proves the refactor is behavior
      -preserving).

### US-04: `not-in` Queries Stop Crashing, Matching Real Firestore's Field-Must-Exist Rule

**job_id**: JOB-01 | **Release**: 1 | **Persona**: P1 Alex

#### Elevator Pitch
Before: a real `RunQuery` using `.where('status', 'not-in', ['closed'])` panics.
After: run the identical `RunQuery` → sees every document whose `status` field EXISTS and is not
any of the listed values — a document missing the `status` field entirely is correctly EXCLUDED
(live-verified real-Firestore behavior), no crash.
Decision enabled: Alex's app's exclusion-filtering queries work identically to production,
including the subtle field-must-exist edge case.

#### Acceptance Criteria
- [ ] AC-QFO-08: a real `RunQuery` with a `not-in` filter returns documents whose field EXISTS and
      is not among the listed values.
- [ ] AC-QFO-09: a document where the filtered field is ENTIRELY ABSENT is correctly EXCLUDED
      (live-verified field-must-exist rule).
- [ ] AC-QFO-10: a document where the filtered field is explicitly set to `null` (not absent) is
      correctly INCLUDED when `null` is not among the excluded values — proves AC-QFO-09 isn't
      merely "any falsy value excluded," but specifically "absent vs. present" (§ Journey Failure
      Modes).

## Wave: DISCUSS / [REF] Outcome KPIs

### Feature: firestore-query-filter-operator-support

### Objective
Eliminate a real, active production crash risk: 4 commonly-used Firestore query operators
(`in`/`not-in`/`array-contains`/`array-contains-any`) currently panic the request instead of
executing.

### Outcome KPIs
| KPI | Target | Measurement |
|---|---|---|
| Currently-panicking operators fixed | 4 of 4 | Direct: AC-QFO-01/04/06/08's own end-to-end proofs |
| Panics remaining in `append_field_filter` for any of these 4 operators | 0 | Direct code inspection + acceptance-test proof of no crash |
| Regression on `Equal`'s own pre-existing behavior after the shared-helper refactor | 0 | AC-QFO-07 |
| Field-must-exist rule correctness for `not-in` | Matches live-verified real Firestore exactly | AC-QFO-09/AC-QFO-10 |
| Mutation-testing kill rate on the widened function | 100% effective (this session's own established bar) | `cargo-mutants --in-diff`, `--lib`-scoped |

## Wave: DISCUSS / [REF] Out of Scope

- **List-size limit enforcement** (30 for `in`/`array-contains-any`, 10 for `not-in`) — real
  -Firestore-accurate, but a performance/validation concern, not correctness (§ Resolution 4).
  Named, deferred.
- **Empty-list behavior, beyond the best-effort fallback locked in Resolution 3** — real Firestore's
  own exact behavior for this case was not confirmed by live verification. Named, deferred pending
  stronger evidence.
- **`Equal`/`NotEqual`'s own existing value-type gap** (they already panic on `Timestamp`/`Bytes`/
  `Reference`/`Array`/`Map`-valued targets) — pre-existing, orthogonal to THIS feature's own scope
  (making `In`/`NotIn`/`ArrayContains`/`ArrayContainsAny` stop panicking on their own OPERATOR
  dispatch, not widening every operator's own VALUE-type coverage). Named explicitly so it is never
  mistaken for something this feature was supposed to fix.
- **The composite-index-requirement heuristic's own treatment of these operators** —
  `composite-index-requirement-rules` (FINALIZED 2026-09-05) already correctly detects when `In`
  combined with a range filter needs a composite index; this feature does not touch that logic.

## Wave: DISCUSS / [REF] WS Strategy

**Strategy A** (real, minimal, end-to-end) — Slice 01 is a real, previously-panicking `RunQuery`
now proven to return correct results, not a mock.

## Wave: DISCUSS / [REF] Driving Ports

gRPC `:8080` `RunQuery` (existing route, zero new RPC — the EXECUTION behavior of an already
-existing call site changes for 4 filter shapes that previously crashed it).

## Wave: DISCUSS / [REF] Pre-requisites

- None beyond the already-shipped `RunQuery`/`StructuredQuery`/`FilterOp` machinery (all
  pre-existing, unmodified by this feature).
- No new external dependency, no new bounded context — the smallest architectural footprint of any
  feature built this session (a bug fix confined to one function's own SQL-generation logic).

## Wave: DISCUSS / [REF] Handoff Package

Handed to `nw-solution-architect` (DESIGN): this feature-delta.md, all 4 Resolutions (especially
Resolution 2's own exact SQL design table and the `NULL`-propagation finding that eliminates the
need for explicit existence-checking in the non-empty-list case), and the explicit instruction to
design the exact shared scalar-dispatch helper's own signature (extracted from the existing `Equal`
-path match) as part of DESIGN's own architecture design.

## Wave: DISCUSS / [REF] SSOT Updates

`docs/product/jobs.yaml`, JOB-01 entry: append a new dated NOTE — "JOB-01 now also covers 4
previously-panicking Firestore query filter operators (`in`/`not-in`/`array-contains`/
`array-contains-any`) — same job, same persona, not a new job. The highest-severity realization of
this job's own 'make it real' pattern this session: closes an active production CRASH, not merely
an accuracy gap. Discovered as a flagged follow-up from `composite-index-requirement-rules`'s own
FINALIZE (2026-09-05). Central design finding: `not-in`'s own live-verified field-must-exist rule
requires zero explicit existence-check code for the non-empty-list case — it falls out of Postgres's
own `NULL` three-valued-logic propagation, identically to how the pre-existing `!=` operator already
gets this right by accident. See docs/feature/firestore-query-filter-operator-support/
feature-delta.md § Resolution 2."

## Wave: DISCUSS / [REF] Definition of Ready Validation

### Requirements Completeness Score: **0.97**

### DoR Checklist (9-item hard gate)
1. [x] Every story traces to a job_id (JOB-01, all 4 stories)
2. [x] Every story has a complete Elevator Pitch (Before/After/Decision enabled)
3. [x] Every AC is testable without ambiguity
4. [x] Walking Skeleton identified (US-01)
5. [x] Scope Assessment passed
6. [x] No slice contains only `@infrastructure` stories (every slice has a direct Alex-facing value
   story)
7. [x] Out of Scope explicitly named (4 items)
8. [x] Outcome KPIs have numeric targets and measurement methods
9. [x] Prior-wave artifacts read and reconciled (the originating `composite-index-requirement-rules`
   § Discovered Gap finding is confirmed, not re-litigated)

### DoR Status: **PASSED**

## Wave: DISCUSS / [REF] Open Questions

None carried forward unresolved — the one genuinely uncertain question (empty-list behavior) was
explicitly named and given a defensible, non-crashing best-effort resolution (§ Resolution 3) rather
than left open or silently guessed.

## Wave: DISCUSS / [REF] Wave Decisions Summary

### Key Decisions
- [D1] Reuse JOB-01 (Resolution 1) — the highest-severity "make it real" realization this session
  (a crash, not an accuracy gap).
- [D2] SQL design locked per-operator (Resolution 2): JSONB `@>` containment for `ArrayContains`/
  `ArrayContainsAny`; reused scalar-dispatch OR/AND composition for `In`/`NotIn`.
- [D3] Empty-list edge case: explicit `FALSE` fallback for `In`/`ArrayContainsAny`, explicit
  `fields->'{field}' IS NOT NULL` fallback for `NotIn` (Resolution 3).
- [D4] List-size limits not enforced — a performance concern, not correctness, out of scope
  (Resolution 4).
- [D5] `NotIn`'s own field-must-exist rule needs ZERO explicit existence-check code for non-empty
  lists — it falls out of Postgres's own `NULL` propagation, mirroring how `!=` already works today.

### Requirements Summary
- Primary need: 4 real, commonly-used Firestore query operators crash the request today instead of
  executing.
- Walking skeleton scope: `ArrayContains`, proven end-to-end against a real, previously-panicking
  query.
- Feature type: Backend.

### Constraints Established
- Zero change to `embyr-core` or `embyr-server`'s own proto-translation layer — confirmed by
  construction, this feature is entirely `append_field_filter`'s own SQL-generation logic.
- No new dependency — the JSON-binding mechanism this feature reuses is already proven in this
  crate.
- Unit tests for every new branch (each operator, the empty-list fallback, the field-null-vs-absent
  distinction) are written DURING DELIVER; a `cargo-mutants` pass is still budgeted at QUALITY_GATE
  regardless.

### Upstream Changes
- None — this feature is a direct, flagged follow-up from `composite-index-requirement-rules`'s own
  FINALIZE, not a contradiction of any prior DISCOVER/DIVERGE assumption.

## Wave: DISCUSS / [REF] Next Wave

**Handoff To**: nw-solution-architect (DESIGN wave)
**Deliverables**: this feature-delta.md, 4 locked Resolutions, 4-slice/1-release plan

---

## Wave: DESIGN / [REF] Prior Wave Consultation — Reading Confirmation

✓ This feature-delta.md's own DISCUSS sections in full, all 4 Resolutions.
✓ `crates/embyr-pg-storage/src/encoding/query.rs::append_field_filter`, the exact scalar
cast-and-bind match block (lines 54-72) — confirmed the EXACT shape to extract into a shared helper:
4 `FieldValue` variants (`Integer`/`String`/`Double`/`Boolean`), each producing a `(cast-expr, op,
bind)` triple, `_ => panic!(...)` for everything else — UNCHANGED behavior for `Equal`/`NotEqual`/
range operators after extraction, confirmed by construction (the extraction is a pure refactor, the
call sites for those 6 pre-existing operators pass the SAME `op` string they already did).

## Wave: DESIGN / [REF] Architecture Design

Two additive/refactor changes, zero new file, zero new type:

1. **Extract `push_scalar_comparison(qb, field_path, op, value)`** — the existing match-on
   -`FieldValue`-type cast-and-bind block (lines 54-72), parameterized by the comparison operator
   string (`"="`,`"!="`,`"<"`, etc. — already a `&str`, no new type needed). The 6 pre-existing
   operators (`LessThan` through `NotEqual`) call it ONCE each, exactly as today, just via the
   extracted function — a pure refactor, zero behavior change (AC-QFO-07's own regression guard
   proves this directly).
2. **`ArrayContains`/`ArrayContainsAny`**: a new helper `push_array_contains(qb, field_path,
   target: &FieldValue)` pushes `fields->'{field_path}'->'v' @> `, then `qb.push_bind(serde_json::
   json!([field_value_to_json(target)]))`, then `::jsonb`. `ArrayContains` calls it once;
   `ArrayContainsAny` calls it once per element of the target `FieldValue::Array`, joined with
   `" OR "` inside parens (or `push("FALSE")` for an empty array, Resolution 3).
3. **`In`/`NotIn`**: iterate the target `FieldValue::Array`'s own elements, calling `push_scalar_
   comparison` once per element with `"="` (`In`) or `"!="` (`NotIn`), joined with `" OR "` (`In`)
   or `" AND "` (`NotIn`) inside parens. Empty-array fallback: `push("FALSE")` for `In`; `push(
   format!("fields->'{field_path}' IS NOT NULL"))` for `NotIn` (Resolution 3).
4. **`field_value_to_json`** (`crates/embyr-pg-storage/src/encoding/field_value.rs`) is REUSED
   unchanged, made accessible from `query.rs` via its own existing `pub fn` visibility (already
   `pub`, confirmed by direct read) — no new export needed.

## Wave: DESIGN / [REF] Wave Decisions Summary

### Key Decisions
- [D1] `push_scalar_comparison` is a pure refactor extraction, not a new abstraction invented from
  scratch — the exact match arms move verbatim, parameterized only by the operator string.
- [D2] `push_array_contains` reuses `field_value_to_json` unchanged (already `pub`) — no new
  serialization logic, no duplicated discriminated-union encoding.
- [D3] `In`/`NotIn`/`ArrayContainsAny`'s own composition (`OR`/`AND` joining, parenthesized) is
  built with a plain `Vec<String>`-then-`.join(...)` — no new query-builder abstraction, matching
  this codebase's own existing "push raw SQL fragments, bind values separately" style throughout
  this file.

### Constraints Established
- No new dependency, no new bounded context, no new port/adapter trait method.
- Zero change to `embyr-core`/`embyr-server`'s own proto-translation layer — confirmed by
  construction.
- Every empty-list edge case (Resolution 3) is handled by an EXPLICIT fallback branch, never left to
  fall through to a malformed/empty SQL fragment.

## Wave: DESIGN / [REF] Next Wave

**Handoff To**: nw-software-crafter (DELIVER wave, per this project's own established convention)
**Deliverables**: this feature-delta.md's DESIGN section
