# Feature Delta: firestore-range-operator-value-type-support

## Wave: DISCUSS / [REF] Prior Wave Consultation — Reading Confirmation

✓ `docs/product/jobs.yaml`, JOB-01 (`sdk-compat`, persona P1 Alex) — read in full.
✓ `docs/evolution/2026-09-05-firestore-equal-notequal-value-type-support.md` § Follow-Up Work — the
originating flag: range operators still panic on `Timestamp`/`Bytes`/`Reference`/`Array`/`Map`
-valued targets, "a real, likely high-priority gap ... needing a genuinely different (ordering
-based) mechanism."
✓ `crates/embyr-pg-storage/src/encoding/query.rs::push_scalar_comparison` (lines 173-193) —
confirmed the EXACT crash: its own match on `value` covers only `Integer/String/Double/Boolean`;
`_ => panic!("unsupported filter value type: {:?}", value)` fires for the 5 remaining types, for
ALL 4 range operators (`LessThan` through `GreaterThanOrEqual`) — `Equal`/`NotEqual` no longer route
here (moved to `push_value_equality` by the immediately-prior feature).
✓ `crates/embyr-pg-storage/src/encoding/field_value.rs::field_value_to_json` — confirmed the EXACT
storage shapes: `Timestamp(s, n)` → `{"t":"TS","s":s,"n":n}` (NO `"v"` key — `push_scalar_
comparison`'s own `->>'v'` extraction pattern cannot apply unchanged); `Bytes(b)` → `{"t":"BY","v":
"<standard-base64>"}`; `Reference(r)` → `{"t":"R","v":"<full-resource-path>"}`.
✓ `crates/embyr-server/src/grpc/handler.rs::translate_filter` (lines 3925-3938) — confirmed the
EXACT, ALREADY-WORKING validation mechanism this feature reuses for `Array`/`Map`: `translate_filter`
returns `Option<Result<QueryFilter, String>>`; BOTH of its own 2 call sites (lines 2989-2994,
3312-3317) already do `.and_then(translate_filter).transpose().map_err(Status::invalid_argument)?`
— an `Err(String)` returned from `translate_filter` ALREADY surfaces as a clean gRPC
`INVALID_ARGUMENT`, not a panic. This is the exact mechanism `CompositeOp::Unspecified` already uses
("unsupported composite operator") — this feature's own `Array`/`Map`-plus-range-operator rejection
reuses it unchanged, adding zero new infrastructure.

**Live web verification** (firebase.google.com/docs/firestore/manage-data/data-types, fetched
directly — this feature's own central Resolution is entirely load-bearing on getting per-type
ordering semantics right, so recollection alone was insufficient):
- **Overall documented value-type ordering**: `Null < Boolean < Integer/Double (numeric) <
  Timestamp < String < Bytes < Reference < GeoPoint < Array < Vector < Map`.
- **`Timestamp`**: full range-query support, standard chronological ordering — the common,
  evidenced idiom (`.where('createdAt', '>', someDate)`).
- **`Reference`**: ordered "by path elements (collection, document ID, collection, document
  ID...)" — a STRUCTURAL, segment-by-segment comparison, NOT a flat lexicographic string compare of
  the full resource path (§ Resolution 3 examines whether this distinction matters for embyr's own
  domain).
- **`Bytes`**: ordered by raw byte value ("byte order") — confirms standard base64 text comparison
  is WRONG (base64's own alphabet ordering `A-Za-z0-9+/` does not match byte-value ordering); must
  decode to raw bytes for correct comparison.
- **`Array`**: HAS a documented ordering (element-by-element, shorter-array-first-on-equal-prefix)
  — but this ordering exists for `orderBy`/cross-type SORTING purposes, NOT as a queryable range
  predicate. Corroborated externally: "you cannot perform range comparisons directly on array
  fields using operators like less than or greater than" — Firestore's only supported array-field
  QUERY operators are `array-contains`/`array-contains-any`/equality, never `<`/`>`/`<=`/`>=`.
- **`Map`**: HAS a documented ordering (sorted by key, then by value) — but its own range-QUERY
  usability was NOT confirmed by any source found. Treated as genuinely unverified, not guessed.

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

- Feature type: **Backend** (Decision 1) — new match arms in one pure SQL-generation function, plus
  a validation check in one existing proto-translation function.
- JTBD: **reuse JOB-01** (Decision 4 = "Yes", existing job) — direct continuation of the 2 prior
  "stop the crash" realizations this session.
- Walking Skeleton: **Yes** (Decision 2) — `GreaterThan` on a `Timestamp` (the single most common
  real-world case, matching the ordinary `.where('createdAt', '>', someDate)` idiom), proven
  end-to-end against a real, previously-panicking `RunQuery`.
- UX Research Depth: **Lightweight** (Decision 3) — a crash fix, one persona, no new emotional arc.

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: P1 Alex (SDK Developer), unchanged.

**Job**: JOB-01 `sdk-compat`, unchanged job_story. This feature's own realization: range queries on
`Timestamp` (an ordinary, extremely common Firestore idiom), `Bytes`, and `Reference` fields crash
the request today. `Array`/`Map` range queries — which real Firestore itself does not support
(`Array`, confirmed) or does not evidence supporting (`Map`, unconfirmed) — get a clean,
Firestore-consistent rejection instead of a crash.

## Wave: DISCUSS / [REF] Job Discovery — Framing Resolution

### Resolution 1 — Reuse JOB-01, or a new job?

Identical shape to the 2 immediately-prior "stop the crash" realizations.

**Resolution**: **JOB-01 reuse, locked.**

### Resolution 2 — Are `Array`/`Map` in scope for THIS feature at all?

Real Firestore itself does NOT support range queries on `Array` fields (confirmed by live
verification) — there is no real semantic to implement; building ordering logic for a construct
real Firestore itself rejects would be inventing non-Firestore behavior, the opposite of this
initiative's own "behave identically to real Firestore" goal. `Map`'s own range-query support is
genuinely UNCONFIRMED (not confirmed absent, not confirmed present) — building support for an
unverified capability risks being wrong in either direction.

| Option | Description | Fit |
|---|---|---|
| **(A) Implement array/map ordering anyway** (per the documented cross-type sort order) | Matches SOME real Firestore behavior (the `orderBy` ordering) | **Rejected** — that ordering is documented for `orderBy`/cross-type comparison, not confirmed as a valid range-QUERY predicate; `Array` is explicitly confirmed NOT to support range queries at all, so implementing one would be inventing behavior real Firestore itself rejects |
| **(B) Reject `Array`/`Map` + range-operator combinations with a clean, named error** (reusing `translate_filter`'s own existing `Result<_, String>` → `Status::invalid_argument` mechanism) | Matches real Firestore's own actual behavior for `Array` (a client-side/server-side rejection, not silent execution); for `Map`, a conservative, safe default pending future evidence — a clean rejection is trivially fixable by loosening later if `Map` range-query support is later confirmed, whereas silently executing a WRONG ordering would be a correctness bug hiding behind a "works" result | **Strongest fit** |

**Resolution**: **(B) is locked — `Array`/`Map` are explicitly OUT of scope for ordering support in
this feature, replaced with a clean rejection instead of a crash.** This still fully satisfies this
feature's own crash-elimination goal (AC-RNG-05 below) without inventing unverified semantics.

### Resolution 3 — `Timestamp` ordering mechanism

`Timestamp` is stored as `{"t":"TS","s":<seconds>,"n":<nanos>}` — no single `"v"` key to extract
and cast the way the existing 4 scalar types do.

| Option | Description | Fit |
|---|---|---|
| **(A) Combine `seconds`/`nanos` into one sortable numeric value** (e.g. `seconds * 1_000_000_000 + nanos`) at query-generation time, cast-compare as `bigint` | Single-column comparison, simple SQL | **Rejected** — risks `i64` overflow for edge-case timestamps (real Firestore's own documented timestamp range extends further than a nanosecond-scaled `i64` can safely represent at its extremes), and duplicates arithmetic logic between bind-time (Rust) and storage (already-separate `s`/`n` fields) for no benefit over the alternative |
| **(B) Postgres `ROW(...)` comparison**: `ROW((fields->'{f}'->'s')::bigint, (fields->'{f}'->'n')::int) {op} ROW($1, $2)` — Postgres compares row values lexicographically (first element decides unless equal, then the second) | Directly matches "seconds, then nanos" chronological ordering with zero arithmetic/overflow risk; a standard, well-understood SQL idiom | **Strongest fit** |

**Resolution**: **(B) is locked.**

### Resolution 4 — `Bytes` ordering mechanism

Stored as `{"t":"BY","v":"<standard-base64>"}`. Live verification confirms real Firestore orders
`Bytes` by raw byte value — standard base64's own alphabet (`A-Za-z0-9+/`) does NOT preserve
byte-value ordering when compared as text (confirmed by direct reasoning: e.g. `+` (0x2B) sorts
between digits and letters in ASCII, but base64's own alphabet places it after the entire alphanumeric
range).

**Resolution**: **decode to raw `bytea` in SQL for comparison, locked**:
`decode(fields->'{f}'->>'v', 'base64') {op} decode($1, 'base64')`, binding the base64 STRING (not
raw bytes) as the parameter and decoding both sides identically in SQL — Postgres's own `bytea`
comparison is byte-wise unsigned, matching real Firestore's own documented ordering directly.

### Resolution 5 — `Reference` ordering mechanism

Live verification confirms real Firestore's own ordering is STRUCTURAL (segment-by-segment: collection,
document ID, collection, document ID...), not a flat string comparison of the full resource path.

| Option | Description | Fit |
|---|---|---|
| **(A) True segment-wise comparison** (split both paths on `/`, compare element arrays the way real Firestore does) | Exactly matches real Firestore's own documented semantic in every case, including cross-depth references | **Rejected for v1** — meaningfully more complex SQL (array splitting + array comparison), and zero domain evidence any Trailmark-shaped query ever compares references of DIFFERENT path depths within the SAME field (a `Reference`-valued field virtually always points into one fixed target collection in practice) |
| **(B) Flat lexicographic string comparison** of the full resource path (`fields->'{f}'->>'v' {op} $1`, identical shape to the existing `String` arm) | For the evidenced, common case — SAME-DEPTH references (identical collection structure, differing only in the final document ID or an earlier segment) — flat string comparison and true segment-wise comparison produce IDENTICAL results, since a shared prefix compares character-for-character equal up to the first differing segment, at which point both mechanisms compare that segment's own text identically | **Strongest fit for v1** |

**Resolution**: **(B) is locked.** The ONE case where (B) could diverge from real Firestore's own
true segment-wise semantic — comparing references of DIFFERENT path depths within the same
comparison — is named explicitly as a known, zero-evidence, deferred divergence (§ Out of Scope),
not silently accepted as fully correct in every case.

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

**Signals checked**: >10 user stories? No (2). >3 bounded contexts/modules? No — 2 files
(`embyr-pg-storage`'s `query.rs`, `embyr-server`'s `handler.rs`), zero new crates, zero new domain
type. Walking skeleton >5 integration points? No (1: a real, previously-panicking `GreaterThan`
-on-`Timestamp` query, proven correct). Estimated effort >2 weeks? No — 2 slices, each ≤1 day.
Multiple independent user outcomes? The 3 newly-supported types (`Timestamp`/`Bytes`/`Reference`)
and the 2 newly-rejected-cleanly types (`Array`/`Map`) are naturally 2 slices (one adds real
capability, one closes a crash with a clean rejection) — a genuine, evidenced split, not
over-slicing.

**Scope Assessment: PASS** (0 oversizing signals fired) — right-sized as one feature.

## Wave: DISCUSS / [REF] Journey — Alex's "My Date-Range Filter Stops Crashing" Arc

### Mental model

Alex's real Firebase app filters documents by a date range (`.where('createdAt', '>', lastWeek)`),
a byte-range comparison, or a reference-ordering query — all ordinary, real Firestore-supported
idioms for 3 of the 5 previously-uncovered types. For the other 2 (`Array`/`Map`), Alex's app was
NEVER able to do this against real Firestore either — so a clean rejection (matching real
Firestore's own actual behavior) is the CORRECT outcome, not merely an acceptable one.

### Failure modes (feeds DISTILL scenario generation)

- `.where('createdAt', '>', someTimestamp)`: today panics; after, correctly returns documents with
  a strictly later timestamp (verified via `ROW` comparison, § Resolution 3).
- `.where('payload', '<', someBytes)`: today panics; after, correctly compares by raw byte value
  (§ Resolution 4) — a false-positive-free proof requires 2 byte sequences whose BASE64 text
  ordering would DISAGREE with their true byte-value ordering (a hand-picked evidenced case,
  proving the `decode(...)` fix, not merely that SOME comparison happens).
- `.where('ownerRef', '<', someReference)`: today panics; after, correctly compares by resource
  path (§ Resolution 5).
- `.where('tags', '>', ['a'])` (a range comparison on an `Array` field): today panics; after,
  returns a CLEAN `INVALID_ARGUMENT`, matching real Firestore's own actual rejection of this
  construct — not a crash, not a silently-wrong result.
- `.where('metadata', '>', {'a': 1})` (a range comparison on a `Map` field): same clean rejection,
  a conservative default pending future evidence Map range queries are real-Firestore-supported.

## Wave: DISCUSS / [REF] Story Map & Walking Skeleton

### Backbone

Alex's real Firebase app issues a range-operator query against a `Timestamp`/`Bytes`/`Reference`
-valued field → embyr today crashes the request → this feature adds correct ordering for all 3 →
the query succeeds with correct results. Separately: an `Array`/`Map` range query → today crashes →
after, a clean `INVALID_ARGUMENT`, matching real Firestore's own actual behavior.

### Walking Skeleton

**Slice 01**: `Timestamp` ordering via `ROW(...)` comparison — the single most common, most
severe-to-leave-broken real-world case.

## Wave: DISCUSS / [REF] Elephant Carpaccio Slices

| # | Story | Release | Estimate | Learning Hypothesis | Reference Class |
|---|---|---|---|---|---|
| 01 | US-01 | 1 (Walking Skeleton) | 0.5 day | Disproves: `Timestamp`/`Bytes`/`Reference` range comparisons can be made to work correctly via 3 new, type-specific match arms in `push_scalar_comparison`, without a new domain type or a new function | New logic (per-type ordering mechanisms), nearest reference class: `push_scalar_comparison`'s own existing 4-arm shape, widened by 3 more arms following the same "one match arm per `FieldValue` variant" pattern |
| 02 | US-02 | 1 | 0.5 day | Disproves: `Array`/`Map` + range-operator combinations can be rejected cleanly by reusing `translate_filter`'s own EXISTING `Result<_, String>` → `Status::invalid_argument` mechanism, without any new validation infrastructure | Mirrors `CompositeOp::Unspecified`'s own identical "unsupported construct → clean `Err(String)`" precedent in the SAME function |

## Wave: DISCUSS / [REF] Prioritization

Slice 01 first — the highest-value, most-evidenced case (3 real, commonly-used types gaining real
capability). Slice 02 second — a narrower, purely defensive fix (2 types gaining a clean rejection
instead of a crash, no new capability, lower urgency since the CRASH itself is the only problem
being solved, not a missing feature).

## Wave: DISCUSS / [REF] System Constraints

- `crates/embyr-core/` is NOT touched — confirmed by construction (no new domain type; `FieldValue`
  already has `Timestamp`/`Bytes`/`Reference`/`Array`/`Map` variants).
- `push_value_equality` is NOT touched — confirmed by construction (whole-object JSON comparison is
  structurally inapplicable to range/ordering comparisons; this feature's own mechanism is entirely
  separate, living in `push_scalar_comparison`'s own widened match).
- `translate_filter`'s own EXISTING `Result<QueryFilter, String>` signature is unchanged — the
  Array/Map rejection is a NEW `if` check inside an EXISTING match arm, not a signature change.
- Mutation-testing lesson, reapplied from this session's own accumulated QUALITY_GATE history: unit
  tests for each new type/operator combination, PLUS the Array/Map rejection path, are written
  DURING DELIVER; a `cargo-mutants --in-diff` pass is still budgeted at QUALITY_GATE regardless.

## Wave: DISCUSS / [REF] User Stories

### US-01: Range Queries Stop Crashing on `Timestamp`/`Bytes`/`Reference` Fields (Walking Skeleton)

**job_id**: JOB-01 | **Release**: 1 | **Persona**: P1 Alex

#### Elevator Pitch
Before: a real `RunQuery` using `.where('createdAt', '>', someTimestamp)` (or a range comparison on
a `Bytes`/`Reference`-valued field) panics the specific gRPC request task.
After: run the identical `RunQuery` → sees the correct set of matching documents, no crash.
Decision enabled: Alex's real Firebase app's date-range, byte-range, and reference-ordering filters
work identically to production Firestore.

#### Acceptance Criteria
- [ ] AC-RNG-01: a real `RunQuery` with a `GreaterThan` filter on a `Timestamp`-valued field
      returns documents with a strictly later timestamp (correct chronological ordering).
- [ ] AC-RNG-02: a real `RunQuery` with a `LessThan` filter on a `Bytes`-valued field correctly
      compares by raw BYTE value, not by base64 TEXT value — proven with a hand-picked byte pair
      whose base64 text ordering disagrees with their true byte-value ordering.
- [ ] AC-RNG-03: a real `RunQuery` with a `GreaterThanOrEqual` filter on a `Reference`-valued field
      correctly compares by resource path.
- [ ] AC-RNG-04 (regression guard): the 4 pre-existing types (`Integer`/`String`/`Double`/
      `Boolean`) are unchanged.

### US-02: `Array`/`Map` Range Queries Get a Clean Rejection Instead of a Crash

**job_id**: JOB-01 | **Release**: 1 | **Persona**: P1 Alex

#### Elevator Pitch
Before: a real `RunQuery` using a range operator (`<`/`<=`/`>`/`>=`) against an `Array`- or `Map`
-valued field panics the specific gRPC request task.
After: run the identical `RunQuery` → sees a clean `INVALID_ARGUMENT` gRPC error, matching real
Firestore's own actual rejection of this construct.
Decision enabled: Alex sees a real, actionable error message instead of a raw transport reset,
matching what real Firestore itself would tell him.

#### Acceptance Criteria
- [ ] AC-RNG-05: a real `RunQuery` with a range-operator filter on an `Array`-valued field returns
      a clean `INVALID_ARGUMENT` — no panic, no transport reset.
- [ ] AC-RNG-06: a real `RunQuery` with a range-operator filter on a `Map`-valued field returns the
      same clean `INVALID_ARGUMENT`.

## Wave: DISCUSS / [REF] Outcome KPIs

### Feature: firestore-range-operator-value-type-support

### Objective
Eliminate the last named crash risk from the `firestore-equal-notequal-value-type-support` follow
-up list: range operators panicking on `Timestamp`/`Bytes`/`Reference`/`Array`/`Map`-valued filter
targets.

### Outcome KPIs
| KPI | Target | Measurement |
|---|---|---|
| Previously-panicking range-operator type combinations fixed with real ordering | 3 of 5 (`Timestamp`/`Bytes`/`Reference`) | Direct: AC-RNG-01 through AC-RNG-03's own end-to-end proofs |
| Previously-panicking combinations closed with a clean rejection | 2 of 5 (`Array`/`Map`) | Direct: AC-RNG-05/06 |
| Panics remaining in `push_scalar_comparison` for any FilterOp/value combination this feature touches | 0 | Direct code inspection + acceptance-test proof |
| Regression on the 4 pre-existing type-matched comparisons | 0 | AC-RNG-04 |
| Mutation-testing kill rate on the widened function | 100% effective (this session's own established bar) | `cargo-mutants --in-diff`, `--lib`-scoped |

## Wave: DISCUSS / [REF] Out of Scope

- **True segment-wise `Reference` comparison for cross-depth paths** — real-Firestore-accurate in
  the general case, but zero domain evidence any Trailmark-shaped query compares references of
  different path depths within one field (§ Resolution 5). Named, deferred.
- **`Array`/`Map` range-query ORDERING support** — real Firestore itself does not support this for
  `Array` (confirmed); `Map`'s own support is genuinely unconfirmed. Building either would risk
  inventing non-Firestore behavior. Named, deferred pending stronger evidence (§ Resolution 2).
- **A general filter-operator-vs-value-type VALIDATION layer** — this feature adds ONE targeted
  check (range operator + `Array`/`Map`) inside the existing `translate_filter` function; it does
  NOT build a general-purpose "is this operator legal for this type" validation framework. Other
  potentially-invalid combinations (if any exist) remain unaddressed, named explicitly so this
  feature's own narrow fix is not mistaken for a comprehensive validation pass.

## Wave: DISCUSS / [REF] WS Strategy

**Strategy A** (real, minimal, end-to-end) — Slice 01 is a real, previously-panicking
`GreaterThan`-on-`Timestamp` query proven to return correct results, not a mock.

## Wave: DISCUSS / [REF] Driving Ports

gRPC `:8080` `RunQuery` (existing route, zero new RPC).

## Wave: DISCUSS / [REF] Pre-requisites

- `firestore-equal-notequal-value-type-support` (FINALIZED 2026-09-05) — the feature that flagged
  this gap.
- No new external dependency, no new bounded context.

## Wave: DISCUSS / [REF] Handoff Package

Handed to `nw-solution-architect` (DESIGN): this feature-delta.md, all 5 Resolutions (especially
Resolution 3's own exact `ROW(...)` SQL shape and Resolution 4's own `decode(..., 'base64')`
mechanism), and the explicit instruction to design the exact `translate_filter` check (which
`FilterOp` variants count as "range operators," checked against which `FieldValue` variants) as
part of DESIGN's own architecture design.

## Wave: DISCUSS / [REF] SSOT Updates

`docs/product/jobs.yaml`, JOB-01 entry: append a new dated NOTE — "JOB-01 now also covers range
-operator (`<`/`<=`/`>`/`>=`) support for `Timestamp`/`Bytes`/`Reference`-valued filter targets
(previously panicking), plus a clean `INVALID_ARGUMENT` rejection (instead of a crash) for
`Array`/`Map`-valued targets, matching real Firestore's own actual behavior (`Array` range queries
are confirmed unsupported by real Firestore itself; `Map`'s own support is unconfirmed, treated
conservatively) — same job, same persona, not a new job. Direct continuation of
`firestore-equal-notequal-value-type-support`'s own flagged follow-up. `Timestamp` uses Postgres
`ROW(...)` comparison over its own stored `(seconds, nanos)` fields; `Bytes` decodes standard-base64
to raw `bytea` for byte-wise comparison (base64 TEXT comparison would be WRONG — its own alphabet
does not preserve byte-value ordering); `Reference` uses flat string comparison (real-Firestore
-accurate for same-depth references, a named divergence for cross-depth ones). See
docs/feature/firestore-range-operator-value-type-support/feature-delta.md § Resolution 2 through 5."

## Wave: DISCUSS / [REF] Definition of Ready Validation

### Requirements Completeness Score: **0.97**

### DoR Checklist (9-item hard gate)
1. [x] Every story traces to a job_id (JOB-01, both stories)
2. [x] Every story has a complete Elevator Pitch
3. [x] Every AC is testable without ambiguity
4. [x] Walking Skeleton identified (US-01)
5. [x] Scope Assessment passed
6. [x] No slice contains only `@infrastructure` stories
7. [x] Out of Scope explicitly named (3 items)
8. [x] Outcome KPIs have numeric targets and measurement methods
9. [x] Prior-wave artifacts read and reconciled (the originating flagged follow-up confirmed, live
   -verified against real Firestore's own documented semantics)

### DoR Status: **PASSED**

## Wave: DISCUSS / [REF] Open Questions

None carried forward unresolved — every genuinely uncertain question (Array/Map range-query
legality, Reference ordering mechanism, Bytes ordering correctness) was independently resolved via
live web verification, not guessed.

## Wave: DISCUSS / [REF] Wave Decisions Summary

### Key Decisions
- [D1] Reuse JOB-01 (Resolution 1).
- [D2] `Array`/`Map` are OUT of scope for ordering — a clean `INVALID_ARGUMENT` rejection instead,
  reusing `translate_filter`'s own existing error mechanism (Resolution 2).
- [D3] `Timestamp` uses Postgres `ROW(...)` comparison over its own `(seconds, nanos)` fields
  (Resolution 3).
- [D4] `Bytes` decodes base64 to raw `bytea` for correct byte-wise comparison — base64 TEXT
  comparison would be WRONG (Resolution 4).
- [D5] `Reference` uses flat string comparison — real-Firestore-accurate for the evidenced,
  same-depth case; a named, deferred divergence for cross-depth references (Resolution 5).

### Requirements Summary
- Primary need: range operators crash on `Timestamp`/`Bytes`/`Reference`/`Array`/`Map`-valued
  filter targets.
- Walking skeleton scope: `Timestamp` ordering via `ROW(...)` comparison.
- Feature type: Backend.

### Constraints Established
- Zero change to `embyr-core`, zero change to `push_value_equality`.
- `translate_filter`'s own signature unchanged — the Array/Map check is a new `if` inside an
  existing match arm.
- Unit tests for every new type/operator combination plus the rejection path are written DURING
  DELIVER; `cargo-mutants --in-diff` still budgeted at QUALITY_GATE.

### Upstream Changes
- None — a direct, flagged follow-up from `firestore-equal-notequal-value-type-support`'s own
  FINALIZE.

## Wave: DISCUSS / [REF] Next Wave

**Handoff To**: nw-solution-architect (DESIGN wave)
**Deliverables**: this feature-delta.md, 5 locked Resolutions, 2-slice plan

---

## Wave: DESIGN / [REF] Prior Wave Consultation — Reading Confirmation

✓ This feature-delta.md's own DISCUSS sections in full, all 5 Resolutions.
✓ `crates/embyr-pg-storage/src/encoding/query.rs::push_scalar_comparison` (lines 173-193) — exact
insertion point for the 3 new match arms, confirmed the function's own existing signature
(`field_path: &str, op: &str, value: &FieldValue`) needs NO change — `op` is already a plain `&str`
(`"<"`,`"<="`,`">"`,`">="` for the callers this feature touches), directly usable inside a `ROW(...)
{op} ROW(...)` or `decode(...) {op} decode(...)` SQL fragment unchanged.
✓ `crates/embyr-server/src/grpc/handler.rs::translate_filter` (lines 3925-3938) — exact insertion
point for the Array/Map check: immediately after computing `op`/`value`, before constructing
`QueryFilter::Field`.
✓ `crates/embyr-core/src/domain/query.rs::FilterOp` — confirmed the 4 range-operator variant names
(`LessThan`, `LessThanOrEqual`, `GreaterThan`, `GreaterThanOrEqual`) for the `matches!` check
DESIGN locks below.

## Wave: DESIGN / [REF] Architecture Design

Two additive changes, zero new file, zero new type:

1. **`push_scalar_comparison` gains 3 new match arms** (`crates/embyr-pg-storage/src/encoding/
   query.rs`):
   ```rust
   FieldValue::Timestamp(s, n) => {
       qb.push(format!(
           "ROW((fields->'{field_path}'->'s')::bigint, (fields->'{field_path}'->'n')::int) {op} ROW("
       ));
       qb.push_bind(*s);
       qb.push(", ");
       qb.push_bind(*n);
       qb.push(")");
   }
   FieldValue::Bytes(b) => {
       qb.push(format!(
           "decode(fields->'{field_path}'->>'v', 'base64') {op} decode("
       ));
       qb.push_bind(STANDARD.encode(b));
       qb.push(", 'base64')");
   }
   FieldValue::Reference(r) => {
       qb.push(format!("fields->'{field_path}'->>'v' {op} "));
       qb.push_bind(r.clone());
   }
   ```
   (`STANDARD` = `base64::engine::general_purpose::STANDARD`, already imported in
   `field_value.rs`; `query.rs` gains its own `use` for it, reusing the SAME encoding
   `field_value_to_json` already applies — never a second, divergent base64 implementation.)
   The `_ => panic!(...)` fallback narrows to cover only `Null`/`Array`/`Map` — `Array`/`Map`
   should NEVER actually reach this function post-Slice-02 (rejected upstream in
   `translate_filter`), so this panic becomes genuinely-unreachable defensive code for those 2
   variants; `Null` was already unreachable before this feature too (no `FilterOp` construction
   path produces a range-comparison against a `Null` target in the current codebase) and remains
   so.
2. **`translate_filter`'s own `FieldFilter` arm gains one check** (`crates/embyr-server/src/grpc/
   handler.rs`), inserted between computing `value` and constructing `QueryFilter::Field`:
   ```rust
   if matches!(op, FilterOp::LessThan | FilterOp::LessThanOrEqual | FilterOp::GreaterThan | FilterOp::GreaterThanOrEqual)
       && matches!(value, FieldValue::Array(_) | FieldValue::Map(_))
   {
       return Some(Err(format!(
           "range comparison operators are not supported on {} values",
           if matches!(value, FieldValue::Array(_)) { "array" } else { "map" }
       )));
   }
   ```

## Wave: DESIGN / [REF] Wave Decisions Summary

### Key Decisions
- [D1] `push_scalar_comparison`'s own existing signature (`field_path: &str, op: &str, value:
  &FieldValue`) needs NO change — all 3 new arms fit its existing shape.
- [D2] The Array/Map rejection lives in `translate_filter` (embyr-server), NOT in
  `push_scalar_comparison` (embyr-pg-storage) — matches this codebase's own existing layering
  (proto-level validation vs. SQL-generation), and reuses the ALREADY-WIRED `Result<_, String>` →
  `Status::invalid_argument` mechanism at both of `translate_filter`'s own call sites, zero new
  plumbing.
- [D3] `Bytes`'s own new match arm reuses `STANDARD` (the SAME base64 engine
  `field_value_to_json` already uses) rather than introducing a second encoding — a single new
  `use` import, not a new dependency.

### Constraints Established
- No new dependency, no new bounded context, no new port/adapter trait method.
- `push_value_equality` remains completely untouched — confirmed by construction.
- The 3 new `push_scalar_comparison` arms and the 1 new `translate_filter` check are the ENTIRE
  production-code diff for this feature.

## Wave: DESIGN / [REF] Next Wave

**Handoff To**: nw-software-crafter (DELIVER wave, per this project's own established convention)
**Deliverables**: this feature-delta.md's DESIGN section
