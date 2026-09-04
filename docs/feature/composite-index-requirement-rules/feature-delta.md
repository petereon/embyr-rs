# Feature Delta: composite-index-requirement-rules

## Wave: DISCUSS / [REF] Prior Wave Consultation — Reading Confirmation

✓ `docs/product/jobs.yaml`, JOB-01 (`sdk-compat`, persona P1 Alex) — read in full. **Found and fixed
a back-propagation gap**: `firestore-composite-indexes-admin-api`'s own feature-delta.md (§ SSOT
Updates) promised a dated NOTE on this job but it was never actually appended (`grep -i composite
docs/product/jobs.yaml` returned nothing before this DISCUSS). Fixed as part of this feature's own
SSOT update (§ SSOT Updates below) — both the missed NOTE and this feature's own NOTE are appended
together, dated accurately to each feature's own completion date.
✓ `docs/product/journeys/sdk-developer.yaml` — Alex's existing journey; JOB-01 already listed, no
contradiction.
✓ `docs/feature/firestore-composite-indexes-admin-api/feature-delta.md`, `docs/product/architecture/
adr-068-composite-index-admin-crud.md` — the immediately-prior, FINALIZED feature. Confirmed its own
explicit Out-of-Scope entry: "Widening `requires_composite_index`'s own simplistic heuristic ... a
separate, evidenced follow-up." This feature is that follow-up.
✓ `crates/embyr-server/src/grpc/handler.rs`, `requires_composite_index`/`collect_filter_fields`
(lines 577-599) — direct re-read of the CURRENT heuristic: `if filter.is_none() || order_by.is_empty()
{ return false }`, then `true` iff any `orderBy` field is absent from the SET of all filtered field
paths (operator-agnostic — every `FilterOp` variant is treated identically). Zero existing unit
tests (confirmed via `grep -n requires_composite_index`) — only indirectly exercised by
`tests/acceptance/us_04_query_collection.rs`'s own 2 Docker-backed acceptance tests.
✓ `crates/embyr-core/src/domain/query.rs`, `FilterOp` (lines 90-103) — confirmed EVERY operator this
feature needs is ALREADY a modeled enum variant: `LessThan/LessThanOrEqual/GreaterThan/
GreaterThanOrEqual/Equal/NotEqual/ArrayContains/In/NotIn/ArrayContainsAny/IsNan/IsNotNan`. **Zero new
domain type needed** — this feature is purely a widening of `requires_composite_index`'s own
internal LOGIC, mirroring `security-rules-cel-functions`'s own "zero new type, purely a function
-body change" cleanliness from earlier this session.
✓ `crates/embyr-pg-storage/src/backend_adapter.rs::run_query` + `crates/embyr-pg-storage/src/
encoding/query.rs::append_filter`/`order_by_expr` — confirmed directly: the ACTUAL Postgres query
execution already handles multi-field `orderBy` correctly (a loop over every `OrderBy` clause) via
generic SQL translation — `requires_composite_index` is PURELY a Firestore-parity SIMULATION gate
for that shape, never a real query-plan necessity.
**Correction, discovered during this feature's own DELIVER (not caught at DISCUSS time — an
under-verified claim in this Reading Confirmation's own first draft)**: `append_field_filter`
(`crates/embyr-pg-storage/src/encoding/query.rs`, line ~44) does NOT actually support every
`FilterOp` — its match arm covers only `LessThan/LessThanOrEqual/GreaterThan/GreaterThanOrEqual/
Equal/NotEqual` (plus `IsNan`/`IsNotNan` special-cased above it); `ArrayContains`/`In`/`NotIn`/
`ArrayContainsAny` all hit its own `_ => panic!("unsupported filter op: {:?}", f.op)` arm. This is
a REAL, PRE-EXISTING, SEVERE bug (any real Alex app using `.where('x', 'in', [...])` today crashes
the specific gRPC request task, confirmed via a genuine test failure during Slice 02's own DELIVER,
not by re-reading) — completely independent of this feature (which never touches `run_query`/
`append_filter` at all) but discovered BECAUSE this feature's own Slice 02 acceptance tests needed
these operators to actually execute to prove their own "no false positive, query succeeds" claim.
Named explicitly in § Discovered Gap below, NOT fixed here (out of this feature's own scope —
fixing the query executor's own operator support is a materially different, larger, separately
-evidenced feature).
✓ `docs/SPEC.md` § Indexes Table (lines 144-146) — **the critical, load-bearing finding this
DISCUSS's own central Resolution is built on**: this codebase ALREADY has a locally-documented
target contract for composite-index enforcement, written by a prior wave: "complex queries
(multi-field `orderBy`, inequality filters combined with `orderBy` on a different field,
`array-contains-any`, `not-in`, `IN` with more than one equality constraint) require a matching
composite index ... Single-field queries are always satisfied without an explicit index." This is a
DOCUMENTED-BUT-NOT-FULLY-IMPLEMENTED gap — the exact "make it real" JOB-01 pattern this DISCUSS's
own Resolution 1 confirms (mirrors `aggregation-queries`/`batch-get-documents`/`firestore-write
-streaming`/`firestore-batch-write`/`firestore-list-rpcs`/`firestore-field-transforms`/`firestore
-composite-indexes-admin-api`'s own identical precedent).
✓ `tests/acceptance/us_04_query_collection.rs`, `query_without_ready_index_returns_failed_
precondition`/`query_succeeds_after_index_reaches_ready_status` (lines 538-654) — the ONLY existing
domain example: `WHERE category == "A" ORDER BY score DESC` (an EQUALITY filter, different field
from the `orderBy`) is used as the canonical "needs composite index" case, unchanged since the
ORIGINAL 2026-05-27 walking skeleton and reused verbatim in `firestore-composite-indexes-admin-api`'s
own `cix01` test. **Live web verification (below) confirms this existing test is Firestore
-ACCURATE — not a bug to fix, a behavior to preserve.**
✓ `crates/embyr-server/src/grpc/handler.rs` line 3082 — the current `FAILED_PRECONDITION` rejection
message is generic ("query requires a composite index; create the index before running this query")
— does NOT name which fields/order the missing index needs, contradicting SPEC.md's own promise
("Firestore standard error message format indicating which index is needed").

**Live web verification** (firebase.google.com, fetched directly — this DISCUSS's own Resolution 1
is entirely load-bearing on getting this right, so recollection alone was insufficient; mirrors
`security-rules-cel-cross-document-reads`'/`security-rules-cel-functions`'s own established
practice of live-verifying real Firestore semantics before locking a Resolution):
- `firebase.google.com/docs/firestore/query-data/index-overview` (fetched directly): confirmed, via
  the doc's own canonical worked example (`.where("country","==","USA").orderBy("population","asc")`
  listed under "Queries supported by manual [composite] indexes," not single-field) — **an EQUALITY
  filter combined with `orderBy` on a DIFFERENT field DOES require a composite index**, the SAME as
  an inequality filter. **This corrects my own initial hypothesis** (formed from SPEC.md's own
  narrower wording, which names only "inequality filters" for this specific trigger) — and confirms
  this codebase's EXISTING evidenced test example and its own CURRENT heuristic behavior on that
  specific shape are BOTH already Firestore-accurate. Also confirmed: 2+ `orderBy` fields always
  require composite (explicit `ORDER BY a ASC, b ASC` example), a lone `array-contains-any` or a lone
  `not-in` do NOT require composite (single-field-index-satisfied), and `in` combined with a
  SEPARATE equality filter on a different field does NOT require composite (compound-equality is
  single-field-satisfied) while `in` combined with a RANGE filter on a different field DOES.
- `firebase.google.com/docs/firestore/query-data/multiple-range-fields` (fetched directly):
  confirmed a SEPARATE, closely-related but structurally DIFFERENT real-Firestore constraint — "if
  you have a filter with a range comparison, your first `orderBy` must be on the SAME field" — this
  is a query-VALIDITY rule (the query is inexpressible at all, regardless of what indexes exist),
  not an index-REQUIREMENT rule. Named explicitly in § Out of Scope below — a genuinely adjacent,
  evidenced gap this feature's own DESIGN deliberately does not fold in, to keep the walking skeleton
  thin (Elephant Carpaccio discipline) and to avoid conflating two structurally different validation
  mechanisms in one slice.

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

- Feature type: **Backend** (Decision 1) — widens one PURE function's own internal logic in
  `crates/embyr-server/src/grpc/handler.rs`; zero new admin route, zero new RPC, zero SDK-visible
  surface change beyond a more accurate/more informative rejection.
- JTBD: **reuse JOB-01** (Decision 4 = "Yes", existing job) — the "make it real" pattern, confirmed
  by direct code + SPEC.md read, not merely asserted (§ Reading Confirmation).
- Walking Skeleton: **Yes** (Decision 2) — the SEVEREST currently-uncaught gap (multi-field
  `orderBy` silently allowed with zero index, when real Firestore always requires one), proven end
  -to-end against a real `RunQuery` call.
- UX Research Depth: **Lightweight** (Decision 3) — a pure-function widening plus one improved error
  message, one persona (Alex, fully profiled across 9+ prior features), no new emotional arc.

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: P1 Alex (SDK Developer), unchanged.

**Job**: JOB-01 `sdk-compat`, unchanged job_story. This feature's own realization: `requires_
composite_index()` today only catches ONE of the five documented-in-SPEC.md composite-index
triggers accurately (single-field-vs-different-orderBy-field, which happens to already be correct
for both equality and inequality operators) — it misses multi-field `orderBy` entirely, and misses
every filter-only (no `orderBy`) composite trigger entirely (its own top-level `order_by.is_empty()`
short-circuit). After this feature, all 5 SPEC.md-documented trigger shapes are detected correctly,
and the resulting rejection names the specific index Alex needs to create.

## Wave: DISCUSS / [REF] Job Discovery — Framing Resolution

### Resolution 1 — Reuse JOB-01, or a new job?

Identical shape to `firestore-composite-indexes-admin-api`'s own Resolution 1: a documented (in
SPEC.md)-but-not-fully-implemented gap in Alex's own "behave identically to real Firestore" job.

**Resolution**: **JOB-01 reuse, locked.**

### Resolution 2 (THE central, evidence-reversing finding) — Is the existing `category==`/`score`
-orderBy test Firestore-accurate, or a bug this feature must fix?

My own initial hypothesis (before live verification): SPEC.md's own wording — "inequality filters
combined with `orderBy` on a different field" — names ONLY inequality operators, implying a plain
EQUALITY filter combined with a different-field `orderBy` should NOT need a composite index. Under
that hypothesis, the codebase's OWN existing, load-bearing evidenced test (`category == "A"` +
`orderBy score`) would itself be Firestore-INACCURATE, and this feature would need to "supersede"
it (mirroring `security-rules-cel-cross-document-reads`'s own precedent for a genuinely-wrong prior
fixture).

**Live web verification directly refutes this hypothesis** (§ Reading Confirmation): real Firestore
requires a composite index for EQUALITY + different-field `orderBy`, identically to inequality +
different-field `orderBy` — confirmed via Firebase's own canonical documented example.

**Resolution**: **the existing test and the CURRENT heuristic's behavior on this specific shape are
BOTH already correct — locked, unchanged, explicitly preserved as a named regression guard in this
feature's own walking skeleton, not superseded.** SPEC.md's own wording is imprecise (implies an
equality exemption that doesn't exist) — corrected as a companion documentation fix in this
feature's own DELIVER (§ System Constraints), not treated as evidence of a code bug.

### Resolution 3 — Which of the remaining 4 SPEC.md-documented triggers are genuinely UNCAUGHT by
the current implementation, evidenced by direct trace, not assumed?

| SPEC.md trigger | Current implementation's own behavior | Genuinely uncaught? |
|---|---|---|
| Inequality filter + `orderBy` on a different field | Field-membership check is operator-agnostic — already catches this (same code path as the equality case, § Resolution 2) | No — already correct |
| Multi-field `orderBy` (2+ fields) | **Not checked at all.** With no filter: `order_by.is_empty()` is false but `filter.is_none()` short-circuits to `false` immediately — WRONG, real Firestore always requires composite for 2+ `orderBy` fields regardless of filters. With a filter where every `orderBy` field happens to be a filtered field: still returns `false` — ALSO WRONG per the verified "regardless of filters" rule. | **Yes — genuine gap.** |
| `array-contains-any` alone | Falls through to `false` (no orderBy triggers the check) — CORRECT (verified: lone `array-contains-any` is single-field-satisfied) | No — already correct, confirmed by evidence, not merely assumed |
| `not-in` alone | Same reasoning — CORRECT | No — already correct |
| `IN` with more than one equality-type constraint (i.e., `IN` + a RANGE filter on a different field, per verified fact — compound-equality-only is FINE) | **Not checked when `orderBy` is absent** — the top-level `order_by.is_empty()` short-circuit means a FILTER-ONLY query (e.g., `WHERE category IN [...] AND population > 690000`, zero `orderBy`) currently returns `false` unconditionally, when real Firestore requires composite for this exact shape | **Yes — genuine gap.** |

**Resolution**: **2 genuine gaps, locked as this feature's own 2 correctness-adding slices** (multi
-field `orderBy`; `IN` + range-on-a-different-field, independent of `orderBy` presence). The other
3 SPEC.md-documented shapes are ALREADY correctly handled by the existing, unmodified
field-membership check — confirmed by direct trace against each, not left as an unverified
assumption.

### Resolution 4 — Two `IN` filters together, or `IN` + `orderBy` on a different field: in scope?

Live web verification (§ Reading Confirmation) explicitly could not confirm real Firestore's own
behavior for two simultaneous `IN` filters (undocumented in the pages fetched). `IN` + `orderBy` on
a different field IS already correctly handled — it falls under the SAME general "any filtered
field not matching the sole `orderBy` field ⇒ composite" rule Resolution 3's table already confirms
correct, requiring no new logic.

**Resolution**: **two-simultaneous-`IN` is out of scope** — genuinely undetermined by live
verification, zero domain evidence any Trailmark-shaped query needs it, named and deferred rather
than guessed.

### Resolution 5 — Does this feature also fix the query-VALIDITY constraint (range filter's own
`orderBy` must start on the SAME field)?

Real, evidenced, closely-related (§ Reading Confirmation, `multiple-range-fields` doc) — but
structurally a DIFFERENT kind of check: a query-SHAPE-validity rule (the query is inexpressible
regardless of what indexes exist, likely `INVALID_ARGUMENT` in real Firestore, not `FAILED_
PRECONDITION`-for-missing-index), not an index-REQUIREMENT rule. Folding it into `requires_
composite_index` would conflate two different real-Firestore validation mechanisms into one
function and one PR, working against the Elephant Carpaccio thin-slice discipline for zero
compelling reason (zero existing domain example currently produces this shape either).

**Resolution**: **out of scope, named, deferred** — a genuinely evidenced follow-up in its own
right (candidate id proposed in § Out of Scope), not silently dropped.

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

**Signals checked**: >10 user stories? No (3). >3 bounded contexts/modules? No — ONE file
(`crates/embyr-server/src/grpc/handler.rs`), zero new crates, zero new domain type (§ Reading
Confirmation confirms `FilterOp` already models every operator needed). Walking skeleton >5
integration points? No (1: a real, previously-silently-accepted multi-`orderBy` query, run against a
real `RunQuery` call, now correctly rejected). Estimated effort >2 weeks? No — 3 slices, each
≤0.5-1 day (smaller than every CEL-parity epic this session, comparable to `firestore-composite
-indexes-admin-api`'s own footprint). Multiple independent user outcomes? The 3 slices are each
independently valuable and independently shippable (Elephant Carpaccio's own encouraged shape, not
an oversizing signal) — but all serve the SAME single "make `requires_composite_index` accurate"
outcome, never separately-pitchable features.

**Scope Assessment: PASS** (0 oversizing signals fired) — right-sized as one feature.

## Wave: DISCUSS / [REF] Journey — Alex's "The Gate Actually Matches Real Firestore" Arc

### Mental model

Alex writes a query his real Firebase app already uses in production — either a multi-field sort
(`orderBy('category').orderBy('score')`), or a filter-only query combining an `IN` clause with a
range comparison on a different field. Against real Firestore, BOTH shapes would have already
forced Alex to create a composite index during his own app's development (real Firestore rejects
them immediately). Against embyr today, BOTH shapes are silently ACCEPTED — no rejection, no
opportunity to create the index firestore-composite-indexes-admin-api's own feature just built —
because `requires_composite_index()` never checks either shape. This is a SILENT correctness gap,
not a loud one: Alex's query "succeeds" today in a way it would never succeed against real
Firestore, which is arguably a worse experience for JOB-01's own "behave identically to real
Firestore" promise than a correct rejection would be (a query that silently diverges from
production Firebase behavior is the exact failure mode JOB-01 exists to eliminate).

### Failure modes (feeds DISTILL scenario generation)

- A multi-`orderBy` query with NO filter at all: today silently succeeds; after this feature,
  correctly rejected `FAILED_PRECONDITION` until Alex creates the matching composite index.
- A multi-`orderBy` query where every `orderBy` field happens to already be a filtered field: today
  ALSO silently succeeds (the existing field-membership check is fooled); after this feature,
  correctly rejected regardless (real Firestore's own "regardless of filters" rule).
- An `IN` + range-on-a-different-field query with NO `orderBy`: today silently succeeds (the
  top-level `order_by.is_empty()` short-circuit bypasses the check entirely); after this feature,
  correctly rejected.
- The EXISTING `category==`/`score`-orderBy shape: unchanged, still correctly rejected without a
  ready index, still correctly succeeds once one exists — a named, explicit non-regression, not an
  accidental side effect (§ Resolution 2).
- The rejection message, after this feature, names the SPECIFIC fields/order the missing index
  needs (reusing `IndexFieldSpec`/`IndexFieldOrder` from `firestore-composite-indexes-admin-api`) —
  Alex can copy that shape directly into a `CreateIndex` call, closing the loop `firestore-composite
  -indexes-admin-api` opened.

## Wave: DISCUSS / [REF] Story Map & Walking Skeleton

### Backbone

Alex runs a query shape real Firestore would reject → embyr today silently accepts it (a
correctness gap) → this feature detects each SPEC.md-documented trigger shape correctly → the
rejection names the exact index needed → Alex creates it via `firestore-composite-indexes-admin
-api`'s own `CreateIndex` → the identical query succeeds.

### Walking Skeleton

**Slice 01**: multi-field `orderBy` detection — the single MOST SEVERE currently-uncaught gap (a
real, previously-silently-succeeding `RunQuery` now correctly rejected, proven end-to-end).

### Release 1 — `requires_composite_index` Matches Every SPEC.md-Documented Trigger (Slices 01–02, US-01–US-02)

Both genuine gaps (§ Resolution 3) closed.

### Release 2 — The Rejection Names the Missing Index (Slice 03, US-03)

Closes the loop with `firestore-composite-indexes-admin-api`'s own `CreateIndex` endpoint.

## Wave: DISCUSS / [REF] Elephant Carpaccio Slices

| # | Story | Release | Estimate | Learning Hypothesis | Reference Class |
|---|---|---|---|---|---|
| 01 | US-01 | 1 (Walking Skeleton) | 0.5 day | Disproves: a real, previously-silently-succeeding multi-`orderBy` `RunQuery` can be correctly rejected without breaking any existing single-`orderBy` regression (`category==`/`score`, § Resolution 2) | New logic, nearest reference class: this session's own established "add one new rule to a pure gating function, verify old rules unchanged" discipline (mirrors 4c's `compare_relational` timestamp-bug fix pattern — a targeted rule addition, not a rewrite) |
| 02 | US-02 | 1 | 0.5 day | Disproves: a filter-only (no `orderBy`) `IN`+range query can be detected without the top-level `order_by.is_empty()` short-circuit silently swallowing it, and without producing a false positive on the ALREADY-correct compound-equality-only shape (`IN` + a SEPARATE equality filter, verified NOT to need composite) | Same reference class as Slice 01 |
| 03 | US-03 | 2 | 0.5-1 day | Disproves: naming the specific missing index in the rejection message needs new production infrastructure beyond reusing `firestore-composite-indexes-admin-api`'s own already-shipped `IndexFieldSpec`/`IndexFieldOrder` types | Mirrors `security-rules-cel-functions`'s own "reuse an existing type, zero new type" discipline |

## Wave: DISCUSS / [REF] Prioritization

Ordered by severity AND genuine dependency: Slice 01 first — the single MOST severe gap (silently
accepting a query real Firestore always rejects, with NO existing partial correctness to lean on,
unlike Slice 02's narrower filter-only blind spot). Slice 02 second — narrower in domain
applicability (needs BOTH an `IN` filter AND a range filter on a different field to fire) but
structurally independent of Slice 01 (touches a different branch of the same function). Slice 03
last — depends on BOTH prior slices existing (it needs to know WHICH trigger fired to name the
correct missing fields), and is the lowest-urgency of the three (a message-quality improvement, not
a correctness gap — Alex already gets a `FAILED_PRECONDITION`, just a generic one, from Slices
01-02 alone).

## Wave: DISCUSS / [REF] System Constraints

- `crates/embyr-core/` is NOT touched by this feature — confirmed by construction (`requires_
  composite_index`/`collect_filter_fields` both live in `embyr-server`, operate on the ALREADY
  -complete `FilterOp` enum, § Reading Confirmation).
- `crates/embyr-pg-storage/`'s own `run_query` is NOT touched — confirmed by construction (§ Reading
  Confirmation: the real Postgres execution path already handles every filter/orderBy combination
  correctly; this feature only changes the SIMULATION GATE in front of it).
- `docs/SPEC.md`'s own composite-index sentence (line 146) will be corrected as a companion doc fix
  during DELIVER — its current wording ("inequality filters combined with `orderBy` on a different
  field") implies a nonexistent equality exemption; corrected to name BOTH equality and inequality
  explicitly, per the live-verified real-Firestore behavior (§ Resolution 2).
- Mutation-testing lesson, reapplied from this session's own accumulated QUALITY_GATE history
  (4c/4d/4e/`firestore-composite-indexes-admin-api`): `requires_composite_index`/`collect_filter_
  fields` are genuinely PURE functions with zero prior unit-test coverage — unit tests are written
  DURING DELIVER for every new branch this feature adds, and a dedicated `cargo-mutants` pass
  (Docker-free, `--lib`-scoped, matching `firestore-composite-indexes-admin-api`'s own precedent) is
  still budgeted at QUALITY_GATE regardless of during-slice discipline.

## Wave: DISCUSS / [REF] User Stories

### US-01: A Multi-Field Sort Correctly Requires a Composite Index (Walking Skeleton)

**job_id**: JOB-01 | **Release**: 1 | **Persona**: P1 Alex

#### Elevator Pitch
Before: a real `RunQuery` with 2+ `orderBy` fields (e.g. `orderBy('category').orderBy('score')`) is
silently accepted today, with zero composite index required — a query real Firestore has always
rejected without one.
After: run the identical `RunQuery` → sees `FAILED_PRECONDITION` until a matching composite index
exists (created via `POST /admin/v1/projects/:project_id/indexes`, `firestore-composite-indexes
-admin-api`), then the SAME query succeeds once it does.
Decision enabled: Alex trusts that a multi-field sort behaves IDENTICALLY to his own production
Firebase app — no silent divergence to debug later.

#### Acceptance Criteria
- [ ] AC-CIR-01: a real `RunQuery` with 2+ `orderBy` fields and NO filter is rejected `FAILED_
      PRECONDITION` when no matching composite index exists.
- [ ] AC-CIR-02: a real `RunQuery` with 2+ `orderBy` fields where EVERY `orderBy` field happens to
      already be a filtered field is STILL rejected `FAILED_PRECONDITION` (the "regardless of
      filters" rule, § Resolution 3) — proving the fix is not merely a field-membership tweak.
- [ ] AC-CIR-03 (regression guard): the EXISTING `category==`/`score`-orderBy single-`orderBy`-field
      shape (`tests/acceptance/us_04_query_collection.rs`'s own AC-04f/AC-04g) is UNCHANGED —
      correctly rejected without a ready index, correctly succeeds once one exists.

### US-02: A Filter-Only `IN` + Range Query Correctly Requires a Composite Index

**job_id**: JOB-01 | **Release**: 1 | **Persona**: P1 Alex

#### Elevator Pitch
Before: a real `RunQuery` combining an `IN` filter with a range filter on a DIFFERENT field, with NO
`orderBy` at all, is silently accepted today — the top-level `order_by.is_empty()` short-circuit
bypasses the check entirely.
After: run the identical `RunQuery` → sees `FAILED_PRECONDITION` until a matching composite index
exists.
Decision enabled: Alex trusts a filter-only compound query behaves identically to production
Firebase too, not just `orderBy`-bearing ones.

#### Acceptance Criteria
- [ ] AC-CIR-04: a real, filter-only (no `orderBy`) `RunQuery` combining `IN` on one field with a
      range comparison (`<`/`<=`/`>`/`>=`) on a DIFFERENT field is rejected `FAILED_PRECONDITION`
      when no matching composite index exists.
- [x] AC-CIR-05 (regression guard, false-positive check, UNIT-LEVEL — see § Discovered Gap): `IN`
      on one field combined with an EQUALITY filter on a different field (compound-equality-only,
      live-verified NOT to need composite) does NOT require a composite index — proving Slice 02
      doesn't over-widen the gate. Proven at the unit level
      (`in_filter_plus_a_separate_equality_filter_does_not_require_composite_index`); an end-to-end
      `RunQuery` proof is blocked by a pre-existing, unrelated query-executor gap (§ Discovered Gap).
- [x] AC-CIR-06 (regression guard, UNIT-LEVEL — see § Discovered Gap): a lone `array-contains-any`
      or a lone `not-in` (no other filter) does NOT require a composite index — unchanged from
      today. Proven at the unit level (`lone_array_contains_any_does_not_require_composite_index`,
      `lone_not_in_does_not_require_composite_index`); same end-to-end blocker as AC-CIR-05.

### US-03: The Rejection Names the Specific Missing Index

**job_id**: JOB-01 | **Release**: 2 | **Persona**: P1 Alex

#### Elevator Pitch
Before: `FAILED_PRECONDITION` says only "query requires a composite index; create the index before
running this query" — Alex must reverse-engineer which fields/order to pass to `CreateIndex`
himself.
After: run the identical rejected `RunQuery` → sees a message naming the exact `{collection_path,
fields}` shape `POST /admin/v1/projects/:project_id/indexes` needs.
Decision enabled: Alex copies the named shape directly into a `CreateIndex` call, closing the loop
`firestore-composite-indexes-admin-api` opened without needing to guess.

#### Acceptance Criteria
- [ ] AC-CIR-07: a `FAILED_PRECONDITION` rejection from ANY of the 3 detected trigger shapes
      (§ Resolution 3, Slices 01-02) includes the specific `collection_path` and `fields` (reusing
      `IndexFieldSpec`/`IndexFieldOrder` from `firestore-composite-indexes-admin-api`) the missing
      index needs.

## Wave: DISCUSS / [REF] Outcome KPIs

### Feature: composite-index-requirement-rules

### Objective
Close the 2 genuinely-uncaught SPEC.md-documented composite-index trigger shapes (multi-field
`orderBy`; `IN` + range on a different field, filter-only) while explicitly preserving the 3
already-correct shapes — and name the missing index in the rejection, closing the loop
`firestore-composite-indexes-admin-api` opened.

### Outcome KPIs
| KPI | Target | Measurement |
|---|---|---|
| SPEC.md-documented trigger shapes correctly detected | 5 of 5 (up from 3 of 5) | Direct: AC-CIR-01/02/04's own end-to-end proofs, cross-checked against AC-CIR-03/05/06's own regression guards |
| Regression on the existing evidenced domain example | 0 | AC-CIR-03, full `us_04_query_collection.rs` re-run clean |
| False positives introduced (a query that should NOT need composite, now wrongly gated) | 0 | AC-CIR-05/06's own explicit false-positive proofs (unit-level, § Discovered Gap) |
| Mutation-testing kill rate on the widened pure function | 100% effective (this session's own established bar) | `cargo-mutants --in-diff`, `--lib`-scoped |

## Wave: DELIVER / [REF] Discovered Gap — Query Executor Does Not Support Every `FilterOp`

Discovered mid-DELIVER (Slice 02), via a REAL test failure, not a re-read: `crates/embyr-pg-storage/
src/encoding/query.rs::append_field_filter` implements only `LessThan/LessThanOrEqual/GreaterThan/
GreaterThanOrEqual/Equal/NotEqual` (plus `IsNan`/`IsNotNan`) — `ArrayContains`/`In`/`NotIn`/
`ArrayContainsAny` all hit its own `panic!("unsupported filter op: {:?}", f.op)` fallback arm. This
directly contradicts this feature's own initial DISCUSS Reading Confirmation, which incorrectly
claimed (from insufficiently verifying `append_field_filter`'s full match arms, only its generic
dispatch shape) that every operator "already works end-to-end" — corrected in place above, not left
standing.

**Severity**: real and pre-existing, independent of this feature. Any live Alex application issuing
a `.where(field, 'in', [...])`/`.where(field, 'not-in', [...])`/`.where(field, 'array-contains-any',
[...])` query against embyr today crashes the specific gRPC request task with an unrecovered Rust
panic (confirmed directly: the client observes a raw `h2 protocol error`/`Cancelled` transport
reset, not a clean gRPC status code) — worse than a clean error response, though confirmed
NON-fatal to the server process or other connections (the panic unwinds only the one task).

**Why not fixed here**: implementing correct SQL translation for `IN`/`NOT IN`/`array-contains`
-family operators (`= ANY($1)`, JSONB `@>`/array-overlap, `NOT IN` semantics) is a materially
different, larger, separately-evidenced feature — fixing the QUERY EXECUTOR's own operator support,
not widening the composite-index REQUIREMENT-DETECTION heuristic this feature is scoped to. Folding
it in here would violate this feature's own Elephant Carpaccio thin-slice discipline for a
completely orthogonal reason.

**Impact on this feature's own ACs**: AC-CIR-05/AC-CIR-06 (the `IN`+equality and lone-`array
-contains-any`/`not-in` false-positive regression guards) are proven at the UNIT level only — the
pure `requires_composite_index` function's own boolean decision is correct and directly tested
(passing); an end-to-end `RunQuery` proof of "succeeds with no index required" is not currently
possible for these specific operators, for a reason entirely outside this feature's own code.

**Recommended follow-up**: a candidate feature (e.g. `firestore-query-filter-operator-support`) to
implement `In`/`NotIn`/`ArrayContains`/`ArrayContainsAny` SQL translation — HIGHER priority than
this feature's own remaining slices, arguably, since it is an active crash risk today, not merely an
accuracy gap. Flagged explicitly to the user, not silently deferred into an easy-to-miss Out-of
-Scope bullet.

## Wave: DISCUSS / [REF] Out of Scope

- **The query-validity constraint** (a range filter's own `orderBy` must start on the SAME field,
  § Resolution 5) — real-Firestore-accurate, live-verified, but a structurally DIFFERENT validation
  mechanism (query-shape validity, not index-requirement). Named, deferred, candidate id proposed:
  `firestore-range-orderby-field-validity`. Zero existing domain example currently produces this
  shape either.
- **Two simultaneous `IN` filters** — genuinely undetermined by live verification (§ Resolution 4),
  zero domain evidence. Named, deferred, no candidate feature id assigned.
- **Real Postgres index provisioning for query performance** — unchanged from `firestore-composite
  -indexes-admin-api`'s own identical Out-of-Scope entry; this feature widens the GATE only, never
  touches `run_query`'s own already-fully-functional execution (§ Reading Confirmation).
- **Real Firestore's own async `CREATING` index-build window** — unchanged from `firestore-composite
  -indexes-admin-api`'s own identical Out-of-Scope entry.
- **Per-field-set granularity in `IndexManager::is_index_ready`** (today: ANY ready index for a
  collection unblocks ANY query needing one, regardless of field match) — a PRE-EXISTING behavior
  this feature neither builds nor changes; named so it is never mistaken for scope this feature
  should have covered.

## Wave: DISCUSS / [REF] WS Strategy

**Strategy A** (real, minimal, end-to-end) — Slice 01 is a real, previously-silently-succeeding
`RunQuery` now correctly rejected, proven against a real seeded document, not a mock.

## Wave: DISCUSS / [REF] Driving Ports

gRPC `:8080` `RunQuery` (existing route, zero new RPC — the rejection behavior of an ALREADY
-existing call site changes for a wider set of query shapes). No new admin route (Slice 03 widens
an EXISTING rejection message's own content, no new endpoint).

## Wave: DISCUSS / [REF] Pre-requisites

- `firestore-composite-indexes-admin-api` (FINALIZED 2026-09-04) — provides `CreateIndex`/
  `IndexFieldSpec`/`IndexFieldOrder`, reused unchanged by Slice 03; provides the walking-skeleton
  -era `RunQuery`/`IndexManager` gate this feature widens.
- No new external dependency, no new bounded context, no new dependency edge — the SAME
  architectural footprint class as `firestore-composite-indexes-admin-api` (smallest of any feature
  built this session).

## Wave: DISCUSS / [REF] Handoff Package

Handed to `nw-solution-architect` (DESIGN): this feature-delta.md, all 5 Resolutions (especially
Resolution 2's own evidence-reversing finding — the existing test is CORRECT, not a bug — and
Resolution 3's own per-shape trace table), and the explicit instruction to design the exact
restructured `requires_composite_index` control flow (it can no longer short-circuit on `order_by.
is_empty()` before checking the `IN`+range rule) as part of DESIGN's own architecture design, plus
the companion `docs/SPEC.md` wording fix.

## Wave: DISCUSS / [REF] SSOT Updates

`docs/product/jobs.yaml`, JOB-01 entry: append TWO NOTEs (catching up a missed back-propagation
from the immediately-prior feature, plus this feature's own):
1. A NOTE dated 2026-09-04, `firestore-composite-indexes-admin-api` DISCUSS — the promised-but
   -never-applied catch-up: "JOB-01 now also covers a composite-index admin CRUD surface
   (Create/List/Delete), closing a documented-but-unbuilt gap ... This completes the 3-epic
   CEL-parity sequence ... " (verbatim from that feature's own feature-delta.md § SSOT Updates,
   applied late).
2. A NOTE dated 2026-09-04, this feature's own DISCUSS — "JOB-01 now also covers accurate
   composite-index REQUIREMENT DETECTION (widening `requires_composite_index`'s own heuristic to
   match all 5 of `docs/SPEC.md`'s own documented trigger shapes, up from 3 of 5), plus naming the
   specific missing index in the rejection — closing the loop `firestore-composite-indexes-admin
   -api` opened. Same job, same persona, not a new job. Central finding: the codebase's OWN existing
   evidenced test (equality filter + different-field orderBy) was ALREADY Firestore-accurate,
   confirmed by live web verification — this feature's own genuine gaps were multi-field `orderBy`
   (uncaught entirely) and `IN`+range-on-a-different-field when filter-only (bypassed by a top-level
   `order_by.is_empty()` short-circuit). See docs/feature/composite-index-requirement-rules/
   feature-delta.md § Resolution 2/3."

## Wave: DISCUSS / [REF] Definition of Ready Validation

### Requirements Completeness Score: **0.97**

### DoR Checklist (9-item hard gate)
1. [x] Every story traces to a job_id (JOB-01, all 3 stories)
2. [x] Every story has a complete Elevator Pitch (Before/After/Decision enabled)
3. [x] Every AC is testable without ambiguity
4. [x] Walking Skeleton identified (US-01)
5. [x] Scope Assessment passed
6. [x] No slice contains only `@infrastructure` stories (every slice has a direct Alex-facing value
   story)
7. [x] Out of Scope explicitly named (5 items)
8. [x] Outcome KPIs have numeric targets and measurement methods
9. [x] Prior-wave artifacts read and reconciled (a genuine back-propagation gap from the prior
   feature was found AND fixed, not merely noted)

### DoR Status: **PASSED**

## Wave: DISCUSS / [REF] Open Questions

None carried forward unresolved — the one open question that mattered most (whether the existing
evidenced test was itself Firestore-accurate) was independently resolved via live web verification,
reversing my own initial hypothesis rather than locking an unverified guess. Two-simultaneous-`IN`
is explicitly named as genuinely undetermined and deferred (§ Resolution 4), not silently guessed.

## Wave: DISCUSS / [REF] Wave Decisions Summary

### Key Decisions
- [D1] Reuse JOB-01 (Resolution 1) — the "make it real" pattern, confirmed by direct code + SPEC.md
  read.
- [D2] The existing `category==`/`score`-orderBy test and the current heuristic's behavior on that
  shape are BOTH correct — preserved as an explicit regression guard, not superseded (Resolution
  2) — a finding that REVERSED my own initial hypothesis under live verification.
- [D3] Exactly 2 genuine gaps closed: multi-field `orderBy` (regardless of filters); `IN` + range on
  a different field (independent of `orderBy` presence) — each individually traced against the
  current implementation, not assumed (Resolution 3).
- [D4] Two-simultaneous-`IN` is out of scope, genuinely undetermined by live verification
  (Resolution 4).
- [D5] The query-validity "range filter's `orderBy` must start on the same field" constraint is out
  of scope — a structurally different real-Firestore mechanism, named and deferred with a proposed
  candidate feature id (Resolution 5).

### Requirements Summary
- Primary need: `requires_composite_index()` only correctly detects 3 of SPEC.md's own 5 documented
  composite-index trigger shapes; 2 real gaps silently accept queries real Firestore would reject.
- Walking skeleton scope: multi-field `orderBy` detection, proven end-to-end against a real
  previously-silently-succeeding `RunQuery`.
- Feature type: Backend.

### Constraints Established
- Zero change to `embyr-core` or `embyr-pg-storage` — confirmed by construction, this feature is
  entirely `requires_composite_index`'s own internal logic plus one improved error message.
- `docs/SPEC.md`'s own composite-index wording is corrected as a companion fix during DELIVER.
- Unit tests for every new branch are written DURING DELIVER; a dedicated `cargo-mutants` pass is
  still budgeted at QUALITY_GATE regardless (this session's own established discipline).

### Upstream Changes
- Fixed a genuine back-propagation gap from `firestore-composite-indexes-admin-api`'s own FINALIZE
  (its promised `jobs.yaml` NOTE was never actually applied) — applied late, as part of this
  feature's own SSOT Updates, alongside this feature's own new NOTE.

## Wave: DISCUSS / [REF] Next Wave

**Handoff To**: nw-solution-architect (DESIGN wave)
**Deliverables**: this feature-delta.md, 5 locked Resolutions, 3-slice/2-release plan

---

## Wave: DESIGN / [REF] Prior Wave Consultation — Reading Confirmation

✓ This feature-delta.md's own DISCUSS sections in full, all 5 Resolutions.
✓ `crates/embyr-core/src/domain/query.rs`, `FilterOp` (line 89) — confirmed the enum derives
`Debug, Clone, PartialEq, Eq` but NOT `Copy` — the new per-operator classification logic this
feature adds needs to inspect `FilterOp` values by reference across multiple filter fields without
fighting borrow-checker lifetimes; adding `Copy` (safe for any field-less enum, zero behavior
change) is the simplest fix, confirmed by direct read rather than assumed.
✓ `crates/embyr-server/src/grpc/handler.rs`, `requires_composite_index`/`collect_filter_fields`
(lines 577-599) — re-confirmed the exact control-flow this feature restructures: the CURRENT
top-level `let Some(filter) = &query.filter else { return false }` then `if query.order_by.is_empty()
{ return false }` ordering is what causes Slice 02's own bug (a filter-only query never reaches any
filter-content inspection). This feature's own new rules must run BEFORE that early return, not
patch around it.

## Wave: DESIGN / [REF] Architecture Design

Three additive changes, no new file, no new type beyond one reused (`IndexFieldSpec`/
`IndexFieldOrder` from `firestore-composite-indexes-admin-api`):

1. **`FilterOp` gains `Copy`** (`crates/embyr-core/src/domain/query.rs`) — zero behavior change,
   unblocks per-operator classification without lifetime friction.
2. **`collect_filter_fields` returns `Vec<(&str, FilterOp)>`** (was `Vec<&str>`) — the ONE signature
   change this feature needs; every existing call site (there is exactly one, inside `requires_
   composite_index` itself) is updated in the same commit.
3. **`requires_composite_index` restructured** to check, IN ORDER:
   a. `query.order_by.len() >= 2` → `true` (Slice 01, unconditional, checked first).
   b. `IN` on field A + a range operator (`<`/`<=`/`>`/`>=`) on a DIFFERENT field → `true` (Slice
      02, checked BEFORE any `order_by`-emptiness short-circuit — this is what fixes the filter
      -only blind spot).
   c. The ORIGINAL rule, unchanged: if `order_by` is non-empty and any `orderBy` field is absent
      from the filtered-field set → `true` (covers equality AND inequality single-field-vs
      -different-orderBy shapes, § Resolution 2/3 — no operator distinction needed here, confirmed
      correct as-is).
   d. Otherwise `false`.
4. **A new pure function** `missing_index_fields(query: &StructuredQuery) -> Vec<IndexFieldSpec>`
   (Slice 03) — derives the `Vec<IndexFieldSpec>` a caller would need to pass to `CreateIndex` from
   the SAME `query.filter`/`query.order_by` data `requires_composite_index` already inspected (no
   new query analysis): filtered-field paths (in filter order) as `Asc`, followed by any `orderBy`
   fields not already included (in their own declared order/direction) — matches real Firestore's
   own convention of listing equality fields before the sort field(s) in a composite index
   definition.
5. **The `FAILED_PRECONDITION` message** (line ~3082) is built from `missing_index_fields`'s own
   output — reuses `IndexFieldSpec`'s existing `Serialize` impl (ADR-068) to render the shape
   directly, e.g. `format!("query requires a composite index on {}/{:?}; create it via POST /admin/
   v1/projects/{{project_id}}/indexes", collection_path, missing_index_fields(&domain_query))`.

## Wave: DESIGN / [REF] Companion Fix

`docs/SPEC.md` line 146: `"inequality filters combined with orderBy on a different field"` →
`"any filter (equality or inequality) combined with orderBy on a different field"` — corrects the
imprecise wording Resolution 2's own live verification found, applied during DELIVER alongside
Slice 01 (documentation-only, zero behavior implication).

## Wave: DESIGN / [REF] Wave Decisions Summary

### Key Decisions
- [D1] `FilterOp` gains `Copy` — the smallest possible unblock for per-operator classification,
  zero behavior change (confirmed safe: a field-less enum).
- [D2] `collect_filter_fields`'s signature widens to carry `FilterOp` — the ONE existing call site
  is updated in the same commit, no parallel/duplicate helper.
- [D3] The 2 new rules (multi-`orderBy`; `IN`+range) are checked BEFORE the original rule's own
  `order_by`-emptiness path, not layered on top of it — directly fixes Slice 02's own root cause
  (the ORIGINAL short-circuit ordering) rather than adding a second, parallel check path.
- [D4] `missing_index_fields` is a NEW pure function, not a side effect bolted onto `requires_
  composite_index` itself — keeps the boolean gate and the message-formatting concern separably
  testable (mirrors this session's own "one function, one job" discipline throughout the CEL-parity
  epics).

### Constraints Established
- No new dependency, no new bounded context, no new port/adapter trait method.
- Zero change to `embyr-core`'s own `Operand`/`Condition`/`evaluate()` or `embyr-pg-storage`'s own
  `run_query` — confirmed by construction.
- Unit tests for `requires_composite_index` (all 5 trigger shapes + the false-positive regression
  guards) and `missing_index_fields` are written DURING DELIVER; a `cargo-mutants --in-diff`
  `--lib`-scoped pass is still budgeted at QUALITY_GATE regardless (this session's own established
  discipline).

## Wave: DESIGN / [REF] Next Wave

**Handoff To**: nw-software-crafter (DELIVER wave, per this project's own established convention)
**Deliverables**: this feature-delta.md's DESIGN section
