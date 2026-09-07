# Feature Delta: firestore-end-cursor-support

## Wave: DISCUSS / [REF] Prior Wave Consultation — Reading Confirmation

✓ `docs/product/jobs.yaml`, JOB-01 (`sdk-compat`, persona P1 Alex) — read in full.
✓ `docs/SPEC.md` §Cursors — the wire contract for all 4 cursor kinds (`startAt`/`startAfter`/
`endAt`/`endBefore`) is fully documented here, including the DESC-order operator flip:

| Cursor | `before` | `isEnd` | Effective boundary (ASC) | Effective boundary (DESC) |
|---|---|---|---|---|
| `startAt(v)` | true | false | `field >= v` | `field <= v` |
| `startAfter(v)` | false | false | `field > v` | `field < v` |
| `endAt(v)` | false | true | `field <= v` | `field >= v` |
| `endBefore(v)` | true | true | `field < v` | `field > v` |

✓ `crates/embyr-server/src/grpc/handler.rs::handle_run_query` — confirmed `sq_proto.start_at` is
translated to a domain `Cursor` (lines 3070-3077) and threaded into `domain_query.start_at`, but
`end_at` is hardcoded `None` at line 3095 — `sq_proto.end_at` is parsed nowhere. The 2 other
`StructuredQuery` construction sites in this file (`handle_list_documents` line ~2331,
`handle_run_aggregation_query` line ~3382) both hardcode `start_at`/`end_at` to `None` too, but
neither of those RPCs' own proto request shapes carry a client-supplied cursor at all — confirmed
out of scope, not a gap.
✓ `crates/embyr-pg-storage/src/backend_adapter.rs::run_query` — confirmed the `start_at` cursor's
own SQL-generation block (lines 598-628): matches `Integer`/`String`/`Double` field-value types
only (silently no-ops for other types — an existing, separate, narrower gap named in § Out of
Scope), and — **critically** — hardcodes `let op = if cursor.before { ">=" } else { ">" };`
regardless of the query's own `order_by` direction. Per SPEC.md's own table above, this is WRONG
for a `DESC`-ordered query: `startAt`/`startAfter` must flip to `<=`/`<` when the field is sorted
descending. **This is a genuine, separate, pre-existing bug in the ALREADY-SHIPPED `start_at`
cursor, discovered while reading the code this feature's own fix must mirror** — not something this
feature's own scope originally named, but too closely coupled to leave uncorrected: naively mirroring
`start_at`'s existing (wrong-under-DESC) operator logic into a NEW `end_at` implementation would
ship a second copy of the same bug on day one.
✓ `google.firestore.v1.StructuredQuery`'s own generated proto (`target/.../google.firestore.v1.rs`)
— confirmed `end_at: Option<Cursor>` uses the IDENTICAL `Cursor` message shape as `start_at` (same
`values`/`before` fields) — no new proto-parsing logic needed beyond what `start_at`'s own
translation already established.
✓ `crates/embyr-core/src/domain/query.rs` — confirmed `StructuredQuery::end_at: Option<Cursor>`
already exists as a domain field (added when the struct itself was first designed) — this feature
POPULATES it, it does not need to be added.

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

- Feature type: **Backend** (Decision 1).
- JTBD: **JOB-01, P1 Alex** (Decision 4 = "Yes", existing job) — a real Firestore SDK's
  `.endAt(v)`/`.endBefore(v)` query methods are core, ordinary SDK surface; this is squarely
  "behave identically to Google Firestore," JOB-01's own functional dimension.
- Walking Skeleton: **Yes** (Decision 2) — a single real `RunQuery` call with an `endAt`/
  `endBefore` cursor, proven to return the correct, bounded result set instead of silently
  over-returning.
- UX Research Depth: **Lightweight** (Decision 3) — a backend correctness fix mirroring an
  already-proven mechanism; no new emotional arc beyond JOB-01's own existing persona profile.

## Wave: DISCUSS / [REF] Business Context

Identified as gap #2 in the 2026-09-06 production-readiness scan, in the SAME severity class as
the just-closed #1 (`firestore-transaction-read-consistency`): a well-behaved client silently gets
WRONG results, not an error. A client paginating backward (`endBefore` on the last-seen document to
walk toward the start of a result set) or windowing a range (`endAt` to bound a report to "everything
up to this timestamp") gets a query that returns MORE rows than it should, with no signal anything
is wrong — the exact "silent, not loud" failure class this session has repeatedly ranked above
crash-shaped gaps, because a crash is at least visible.

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: **P1 Alex (SDK Developer)** — unchanged from JOB-01's existing profile.

**Job**: **JOB-01 `sdk-compat`**, reused, EXTENDED (not replaced) to also cover: `RunQuery`'s
`endAt`/`endBefore` cursors now bound the result set correctly, matching real Firestore, instead of
being silently ignored; and (as a closely-coupled correction surfaced while building this)
`startAt`/`startAfter` now correctly flip their own comparison operator for `DESC`-ordered queries,
matching SPEC.md's own already-documented contract, instead of always using the `ASC` operator
regardless of sort direction.

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

**Signals checked**: >10 user stories? No (2). >3 bounded contexts/modules? No — 2 files
(`crates/embyr-server/src/grpc/handler.rs`, `crates/embyr-pg-storage/src/backend_adapter.rs`), zero
new crates, zero new domain type (`Cursor`/`StructuredQuery::end_at` already exist). Walking
skeleton >5 integration points? No (1: a real `endAt` `RunQuery`, proven to bound results
correctly). Estimated effort >2 weeks? No — mirrors an already-proven mechanism (`start_at`'s own
proto-translation and SQL-generation shape) almost verbatim, plus one small shared
operator-resolution helper. Multiple independent user outcomes? No — `end_at` support and the
`DESC`-flip fix are 2 faces of the SAME underlying correctness gap (an operator table that was
never fully implemented).

**Scope Assessment: PASS** (0 oversizing signals fired).

## Wave: DISCUSS / [REF] Journey — Alex's "My Backward-Paginated Report Stops Where It Should" Arc

### Mental model

Alex's app queries `orders` ordered by `createdAt`, and needs the LAST page of a backward-paginated
UI: `.orderBy('createdAt').endBefore(lastSeenDoc).limit(20)`. Alex expects (because this is real
Firestore's own documented cursor behavior) the query to return only documents sorted strictly
before `lastSeenDoc`. Today, embyr silently ignores the `endBefore` cursor entirely — the query
returns up to 20 documents from the START of the whole ordered set, an entirely different, wrong
page, with no error to signal anything went wrong.

### Failure modes (feeds DELIVER test design)

- `RunQuery` with `endAt(v)` on an `ASC`-ordered field: today ignores it (returns extra rows);
  after, bounds the result to `field <= v`.
- `RunQuery` with `endBefore(v)` on an `ASC`-ordered field: today ignores it; after, bounds to
  `field < v`.
- `RunQuery` with `startAt(v)` on a `DESC`-ordered field (the pre-existing, closely-coupled bug):
  today wrongly applies `field >= v` (the ASC operator) instead of the correct `field <= v`; after,
  correctly flips per direction.
- A well-formed query with NEITHER cursor set: unaffected — this feature adds no new restriction to
  a query that doesn't request one.

## Wave: DISCUSS / [REF] Story Map & Walking Skeleton

### Backbone

Alex's app calls `RunQuery` with `endAt`/`endBefore` set → today silently ignored, wrong (too many)
rows returned → **this feature**: `end_at` is parsed off the wire and bounds the SQL query exactly
like `start_at` already does, using a SHARED, direction-aware operator table so both bounds (and
the pre-existing `DESC`-order gap in `start_at`) are correct together.

### Walking Skeleton

**Slice 01 (the entire feature)**: `end_at` proto-to-domain translation + SQL-generation, plus the
shared direction-aware operator helper (fixing `start_at`'s own existing DESC bug as part of the
same small change), proven end-to-end against real `endAt`/`endBefore`/DESC-ordered `startAt`
queries.

## Wave: DISCUSS / [REF] Elephant Carpaccio Slices

| # | Story | Release | Estimate | Learning Hypothesis | Reference Class |
|---|---|---|---|---|---|
| 01 | US-01 (entire feature, Walking Skeleton) | 1 | ≤0.5 day | Disproves: `end_at` support is a trivial mirror of `start_at`'s own already-proven proto-translation/SQL-generation shape, needing only a small shared direction-aware operator helper | Mirrors `crates/embyr-pg-storage/src/backend_adapter.rs`'s own existing `start_at` block (lines 598-628) almost verbatim |

## Wave: DISCUSS / [REF] Prioritization

A single slice — no ordering decision needed.

## Wave: DISCUSS / [REF] System Constraints

- `crates/embyr-core/src/domain/query.rs` is UNCHANGED — `Cursor`/`StructuredQuery::end_at` already
  exist as domain types.
- `handle_run_query`'s own proto-to-domain translation gains one more field-population line,
  mirroring `start_at`'s existing shape exactly (same `Cursor` message, same
  `proto_value_to_field_value` per-value translation).
- `push_scalar_comparison`-style multi-type support: the EXISTING `start_at` block only handles
  `Integer`/`String`/`Double` field-value types (silently no-ops for `Timestamp`/`Boolean`/others).
  This feature's own `end_at` implementation matches that SAME scope exactly — extending cursor
  support to every `FieldValue` type (mirroring the range-operator arc's own earlier value-type
  widening) is named as an explicit, SEPARATE follow-up, not silently bundled in here (§ Out of
  Scope).
- The shared direction-aware operator helper is a small, PURE function (`(is_end: bool, before:
  bool, direction: &OrderDirection) -> &'static str`) — no new SQL, no new query-shape, just a
  correct lookup replacing 2 separate hardcoded (and, for `start_at`, direction-blind) operator
  choices.
- REST/gRPC-Web transport: `RunQuery` is served by the SAME `handle_run_query` function regardless
  of transport (gRPC-Web/REST wrap the same tonic service) — no separate REST-specific
  cursor-parsing code path exists, so this fix covers both transports by construction.

## Wave: DISCUSS / [REF] User Stories

### US-01: `endAt`/`endBefore` Cursors Bound RunQuery Results Correctly

**job_id**: JOB-01 | **Release**: 1 | **Persona**: P1 Alex

#### Elevator Pitch
Before: `db.collection('orders').orderBy('createdAt').endBefore(lastDoc).get()` silently ignores
the `endBefore` cursor and returns documents from the start of the whole ordered set — an entirely
wrong page, no error.
After: run the identical query → sees only documents sorted strictly before `lastDoc`, matching
real Firestore's own documented cursor semantics exactly.
Decision enabled: Alex can implement backward pagination and range-windowed reports (e.g. "all
orders up to close-of-business yesterday") using the SDK's own standard cursor API, trusting the
result set is correctly bounded.

#### Acceptance Criteria
- [ ] AC-EC-01: a real `RunQuery` with `end_at` set (`before: false`, the `endAt` case) on an
      `ASC`-ordered field returns only documents with `field <= v`.
- [ ] AC-EC-02: a real `RunQuery` with `end_at` set (`before: true`, the `endBefore` case) on an
      `ASC`-ordered field returns only documents with `field < v`.
- [ ] AC-EC-03: a real `RunQuery` with `start_at` set on a `DESC`-ordered field correctly flips its
      own operator (`field <= v` for `startAt`, `field < v` for `startAfter`) — the pre-existing,
      closely-coupled bug this feature also fixes.
- [ ] AC-EC-04 (regression guard): a real `RunQuery` with `start_at` set on an `ASC`-ordered field
      (the already-shipped, already-tested case) continues to work identically to today — zero
      regression on the happy path the existing mechanism already proved.
- [ ] AC-EC-05 (regression guard): a real `RunQuery` with NEITHER cursor set is completely
      unaffected.

## Wave: DISCUSS / [REF] Outcome KPIs

### Feature: firestore-end-cursor-support

### Objective
Close gap #2 from the 2026-09-06 production-readiness scan: `endAt`/`endBefore` cursors now
correctly bound `RunQuery` results instead of being silently ignored, and (as a closely-coupled
correction) `startAt`/`startAfter` now correctly account for sort direction.

### Outcome KPIs
| KPI | Target | Measurement |
|---|---|---|
| `end_at` cursor kinds correctly bounding results | 2 of 2 (`endAt`, `endBefore`) | AC-EC-01/02 |
| `start_at` DESC-direction correctness | Fixed | AC-EC-03 |
| Regression on existing `start_at` ASC behavior or cursor-less queries | 0 | AC-EC-04/05 |
| Mutation-testing kill rate on the new/changed operator-resolution logic | 100% effective | `cargo-mutants --in-diff` |

## Wave: DISCUSS / [REF] Out of Scope

- **Extending cursor support (start OR end) to every `FieldValue` type** — the existing `start_at`
  mechanism only handles `Integer`/`String`/`Double`; `end_at` matches that same scope exactly.
  Widening both to `Timestamp`/`Boolean`/`Bytes`/`Reference` (mirroring this session's own earlier
  range-operator value-type arc) is a separate, later follow-up.
- **Multi-field cursors** — SPEC.md names "lexicographic OR-of-AND expansion" for multi-field
  cursors; the existing `start_at` mechanism is explicitly single-field-orderBy-only ("step
  04-01" comment), and this feature preserves that same scope for `end_at`.
- **Page-token-based pagination** — SPEC.md's own note ("When a query has cursors, page tokens are
  ignored") describes an interaction this feature doesn't change; page-token pagination itself is
  unaffected.

## Wave: DISCUSS / [REF] WS Strategy

**Strategy A** (real, minimal, end-to-end) — the single slice is a real `RunQuery` with a real
`endAt`/`endBefore` cursor, proven against a real running server and Postgres backend.

## Wave: DISCUSS / [REF] Driving Ports

gRPC `:8080` `RunQuery` (existing route, zero new RPC; covers REST/gRPC-Web transport by
construction, same handler).

## Wave: DISCUSS / [REF] Pre-requisites

- `crates/embyr-pg-storage/src/backend_adapter.rs`'s own already-shipped `start_at` cursor block —
  the direct template this feature mirrors.
- No new external dependency, no new bounded context.

## Wave: DISCUSS / [REF] Handoff Package

Handed to `nw-solution-architect` (DESIGN): this feature-delta.md, and the explicit instruction to
design the shared direction-aware operator helper as a single small pure function used by BOTH
`start_at` and `end_at`, rather than duplicating a (now direction-correct) block twice.

## Wave: DISCUSS / [REF] SSOT Updates

`docs/product/jobs.yaml`, JOB-01 entry: append a new dated NOTE — "JOB-01 now also covers
`RunQuery`'s `endAt`/`endBefore` cursors correctly bounding results (previously silently ignored),
plus a closely-coupled fix to `startAt`/`startAfter`'s own operator selection for `DESC`-ordered
queries (previously always used the `ASC` operator regardless of sort direction, a pre-existing bug
discovered while implementing this feature). Same job, same persona, not a new job. See
docs/feature/firestore-end-cursor-support/feature-delta.md."

## Wave: DISCUSS / [REF] Definition of Ready Validation

### Requirements Completeness Score: **0.97**

### DoR Checklist (9-item hard gate)
1. [x] Every story traces to a job_id (JOB-01)
2. [x] Story has a complete Elevator Pitch
3. [x] Every AC is testable without ambiguity
4. [x] Walking Skeleton identified (US-01)
5. [x] Scope Assessment passed
6. [x] No slice contains only `@infrastructure` stories
7. [x] Out of Scope explicitly named (3 items)
8. [x] Outcome KPIs have numeric targets and measurement methods
9. [x] Prior-wave artifacts read and reconciled (SPEC.md's own documented cursor operator table
   confirmed by direct code inspection, surfacing a genuine pre-existing DESC-order bug not
   originally named in this feature's own candidate framing)

### DoR Status: **PASSED**

## Wave: DISCUSS / [REF] Open Questions

None carried forward unresolved.

## Wave: DISCUSS / [REF] Wave Decisions Summary

### Key Decisions
- [D1] JOB-01/P1 Alex confirmed correct — `endAt`/`endBefore` are ordinary SDK cursor methods, a
  direct "behave identically to Google Firestore" case.
- [D2] Mechanism: mirror `start_at`'s own existing proto-translation/SQL-generation shape for
  `end_at`; introduce ONE shared, direction-aware operator-resolution helper used by both bounds,
  fixing `start_at`'s own pre-existing DESC-order bug as a byproduct of not duplicating it.
- [D3] Scope matches `start_at`'s own existing limits exactly: `Integer`/`String`/`Double` only,
  single-field `orderBy` only — widening either is an explicit, separate follow-up.

### Requirements Summary
- Primary need: `endAt`/`endBefore` cursors must bound `RunQuery` results correctly — today
  silently ignored, causing wrong (too many) rows to be returned with no error.
- Walking skeleton scope: US-01, the entire feature.
- Feature type: Backend.

### Constraints Established
- No new domain type, no new `CoreError` variant, no new RPC.
- `end_at`'s own scope (value types, single-field-orderBy-only) matches `start_at`'s existing scope
  exactly — no silent scope creep beyond what's already shipped for the sibling mechanism.

### Upstream Changes
None — this DISCUSS confirms JOB-01's existing scope; SPEC.md's own documented cursor contract is
realized more completely, not amended.

## Wave: DISCUSS / [REF] Next Wave

**Handoff To**: nw-solution-architect (DESIGN wave)
**Deliverables**: this feature-delta.md, 3 locked Decisions, 1-slice plan

---

## Wave: DESIGN / [REF] Prior Wave Consultation — Reading Confirmation

✓ This feature-delta.md's own DISCUSS sections in full, all 3 Decisions.
✓ `crates/embyr-pg-storage/src/backend_adapter.rs`'s own `start_at` block (lines 598-628) — the
exact insertion point and pattern `end_at`'s own block mirrors, and the exact 2 lines
(`let op = if cursor.before { ">=" } else { ">" };` inside each of the 3 `match` arms) the new
shared helper replaces.

## Wave: DESIGN / [REF] Architecture Design

### 1. Shared operator-resolution helper (new, in `crates/embyr-pg-storage/src/encoding/query.rs`
   or `backend_adapter.rs` directly — colocated with the cursor SQL-generation code, not a new
   module, since it has exactly 2 call sites)

```rust
/// Resolve the SQL comparison operator for a cursor bound (firestore-end-cursor-support).
///
/// `is_end`: `false` for `start_at`, `true` for `end_at`.
/// `before`: the cursor's own `before` flag (SPEC.md §Cursors: `startAt`/`endBefore` = true,
/// `startAfter`/`endAt` = false).
/// `direction`: the first `orderBy` field's own sort direction — cursors are single-field-only
/// (§ Out of Scope), so only `order_by[0]`'s direction matters.
fn cursor_operator(is_end: bool, before: bool, direction: &OrderDirection) -> &'static str {
    // ASC table (SPEC.md §Cursors): startAt=">=", startAfter=">", endAt="<=", endBefore="<".
    let asc_op = match (is_end, before) {
        (false, true) => ">=",  // startAt
        (false, false) => ">", // startAfter
        (true, false) => "<=", // endAt
        (true, true) => "<",   // endBefore
    };
    // DESC flips every operator (SPEC.md §Cursors' own DESC column).
    match (direction, asc_op) {
        (OrderDirection::Ascending, op) => op,
        (OrderDirection::Descending, ">=") => "<=",
        (OrderDirection::Descending, ">") => "<",
        (OrderDirection::Descending, "<=") => ">=",
        (OrderDirection::Descending, "<") => ">",
        (OrderDirection::Descending, _) => unreachable!("asc_op is always one of the 4 above"),
    }
}
```

### 2. `start_at` block: replace its own hardcoded `let op = if cursor.before { ">=" } else { ">" };`
   with `let op = cursor_operator(false, cursor.before, &ob.direction);` — same 3-arm
   `Integer`/`String`/`Double` match, same SQL-emission shape, unchanged otherwise.

### 3. `end_at` block: a NEW block, structurally identical to `start_at`'s own (same `Integer`/
   `String`/`Double` match, same `qb.push`/`qb.push_bind` shape), using
   `cursor_operator(true, cursor.before, &ob.direction)` for its own operator.

### 4. `handle_run_query`: gains one more field-population line, mirroring `start_at`'s own
   existing shape exactly:

```rust
let end_at = sq_proto.end_at.as_ref().and_then(|c| {
    let values: Option<Vec<FieldValue>> = c
        .values
        .iter()
        .map(crate::encoding::firestore_proto::proto_value_to_field_value)
        .collect();
    values.map(|vals| Cursor { values: vals, before: c.before })
});
```
and `domain_query`'s own `end_at: None` becomes `end_at: end_at,` (`end_at,` after shorthand).

## Wave: DESIGN / [REF] Wave Decisions Summary

### Key Decisions
- [D1] `cursor_operator` is ONE small pure function, colocated with the cursor SQL-generation code
  (not a new module) — used by both `start_at` and `end_at`'s own blocks, eliminating the
  duplicated-and-now-provably-inconsistent operator logic the 2 blocks would otherwise each carry.
- [D2] `end_at`'s own block is a structural mirror of `start_at`'s (same value-type match, same
  SQL shape) — deliberately NOT a shared block-level abstraction beyond the operator helper itself,
  since the 2 blocks differ in which `Cursor` field they read (`query.start_at` vs `query.end_at`)
  and ponytail's own "two rungs work, take the higher one" guidance doesn't favor forcing a shared
  block for 2 call sites this small.

### Constraints Established
- No new `CoreError` variant, no new domain type, no new port/adapter trait method.
- `start_at`'s own existing tests (proving the ASC case, already shipped) must continue to pass
  unmodified — `cursor_operator`'s own ASC branch is a pure refactor of already-correct logic.

## Wave: DESIGN / [REF] Next Wave

**Handoff To**: nw-software-crafter (DELIVER wave, per this project's own established convention)
**Deliverables**: this feature-delta.md's DESIGN section
