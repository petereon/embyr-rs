# Feature Delta: firestore-or-filter-support

## Wave: DISCUSS / [REF] Prior Wave Consultation — Reading Confirmation

✓ `docs/product/jobs.yaml`, JOB-01 (`sdk-compat`, persona P1 Alex) — read in full.
✓ `crates/embyr-server/src/grpc/handler.rs::translate_filter` — confirmed
`FilterType::CompositeFilter`'s own match on `CompositeOp` handles only `And`/`Unspecified`; `Or`
(and any other future operator) falls to `_ => Some(Err("unsupported composite operator".into()))`
— a real GA Firestore SDK feature (`Filter.or(...)`, `.where(Filter.or(cond1, cond2))`) rejected
outright.
✓ `crates/embyr-core/src/domain/query.rs` — confirmed `QueryFilter::Composite(Vec<QueryFilter>)`
carries NO logical-operator distinction at all; its own doc comment reads "Composite AND filter."
✓ `crates/embyr-pg-storage/src/encoding/query.rs::append_filter` — confirmed `Composite` children
are always joined with a bare `" AND "`, never parenthesized (never needed to be — pure AND-of-AND
is associative, so precedence never mattered before).
✓ `docs/product/architecture/adr-031-query-shape-compliance-check.md` — **the load-bearing finding
this DISCUSS is built around**: this ADR's own § Handoff Package explicitly documents "confirmed
`QueryFilter::Composite` is AND-only ('Composite AND filter' doc comment), the structural fact this
WHOLE ADR's tractability rests on." `filter_binds_field_to_uid`
(`crates/embyr-core/src/access_control/mod.rs`) — the function that decides whether a query's own
filter tree structurally GUARANTEES an ownership-equality constraint (`field == caller_uid`) for
every document the query could possibly return — recurses into `Composite` with `.any()`: "if ANY
child of this AND enforces the binding, the WHOLE conjunction enforces it." This is CORRECT only
because every `Composite` today means AND. **Confirmed directly, not assumed**: naively reusing
`Composite`'s existing shape (or reusing `.any()`) for an OR node would be a real access-control
bypass — a query like `WHERE ownerId == callerUid OR true` would be (wrongly) treated as
ownership-compliant by an unmodified `.any()` check, while Firestore's own actual return set would
include every OTHER tenant's documents matching the `true` branch too.
✓ `crates/embyr-server/src/adapters/agent_backend.rs::domain_filter_to_agent_filter` — confirmed
the `embyr.agent.v1.CompositeFilterOp` proto enum (the customer-VPC agent's OWN internal wire
protocol) has ONLY `Unspecified`/`And` — no `Or` variant exists at all. `backend_mode=agent` cannot
structurally support OR filters without a proto change to that separate wire surface — mirrors the
established `agent-mode-write-streaming`/`agent-mode-list-collection-ids`/
`agent-mode-field-transforms`/`firestore-transaction-read-consistency`'s own "agent-mode deferred,
separate proto surface" precedent exactly. This SAME function's own doc comment already documents a
past, structurally analogous incident: "without it, `run_query` forwarded no filter at all to the
agent, so an approved ownership-scoped query returned every document in the collection (cross-user
leak)" — confirming this exact class of correctness bug has bitten this codebase before, in this
exact function, for a different (but structurally similar) reason.
✓ `crates/embyr-server/src/grpc/handler.rs::requires_composite_index`/`missing_index_fields` — both
already recurse into `QueryFilter::Composite`; confirmed by direct read they operate purely on the
SET of filtered/ordered fields, not on AND/OR structure — extending `Composite` handling to also
cover the new `CompositeOr` variant (flattening its own children into the same field-collection
logic) requires no NEW index-requirement heuristic, reuses the existing one unchanged.
✓ Confirmed via direct grep (not assumed) — `QueryFilter::Composite` blast radius: 15 occurrences
across 6 files (`embyr-core/access_control/mod.rs`, `embyr-server/admin/handlers/access_rules.rs`,
`embyr-server/adapters/agent_backend.rs`, `embyr-server/grpc/handler.rs`,
`embyr-pg-storage/encoding/query.rs`, `embyr-agent/server.rs`). All 7 real call sites of
`check_query_compliance()` (2× `RunQuery`, 2× `RunAggregationQuery`, 2× admin `simulate` endpoints,
1× realtime `Listen`) route through the SAME `filter_binds_field_to_uid` — fixing it once,
centrally, covers every caller.
✓ Real Firestore's own documented security-rules-and-query interaction (general Firestore
knowledge, not independently re-verified via a live web check this session — flagged honestly, not
presented as freshly confirmed): Firestore statically validates that a query's own filter
structure GUARANTEES every possible result satisfies a security rule requiring
`resource.data.field == request.auth.uid`, WITHOUT reading any document — for a query built from
`Filter.or(a, b)`, this guarantee holds only if BOTH `a` and `b` independently enforce the
constraint (since a matched document could come from either branch). This directly confirms the
`.any()` → `.all()` design below is the semantically-correct fix, not merely a locally-consistent
choice.

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

- Feature type: **Backend** (Decision 1) — with a real security-correctness dimension, per the
  user's own explicit choice of the "full fix" scope over the narrower "reject OR entirely for
  ownership-protected collections" alternative.
- JTBD: **JOB-01, P1 Alex** (Decision 4 = "Yes", existing job) — `Filter.or()` is ordinary,
  documented Firestore SDK surface.
- Walking Skeleton: **Yes** (Decision 2) — a single real `RunQuery` with an OR filter, proven to
  return the union of matches, PLUS a real security-rule-protected OR query proven to still enforce
  ownership correctly (or be correctly rejected when it can't be proven).
- UX Research Depth: **Lightweight** (Decision 3) — a backend correctness/security fix; no new
  emotional arc beyond JOB-01's own existing persona profile.

## Wave: DISCUSS / [REF] Business Context

Gap #3 from the 2026-09-06 production-readiness scan: `Filter.or()` — a real, commonly-used GA
Firestore feature (e.g. "orders with status PENDING OR status PROCESSING") — is rejected outright
with a generic `INVALID_ARGUMENT`. Lower raw severity than gaps #1/#2 (a clean rejection, not
silent wrong data), but the INVESTIGATION this DISCUSS performed surfaced something more serious
than the original gap-scan entry named: the naive fix (just accept `Or` and OR-join the SQL) would
have REOPENED a silent-wrong-data class bug, this time as an access-control bypass — arguably as
severe as gap #1/#2, discovered here before it could ship, not after.

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: **P1 Alex (SDK Developer)** — unchanged from JOB-01's existing profile. The security
-correctness dimension protects Alex's OWN end users' data (an Alex-shaped app using
security-rules-protected collections), not a separate persona.

**Job**: **JOB-01 `sdk-compat`**, reused, EXTENDED (not replaced) to also cover: `Filter.or()`
composite queries now execute correctly (union semantics) instead of being rejected, AND — when
the target collection has an access rule requiring ownership-equality — the query is only admitted
if EVERY disjunct branch independently proves the ownership constraint, matching real Firestore's
own documented security-rules-and-query validation model exactly.

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

**Signals checked**: >10 user stories? No (2). >3 bounded contexts/modules? Touches 4 crates
(`embyr-core`'s domain type + access-control logic, `embyr-pg-storage`'s SQL generation,
`embyr-server`'s translation/index-requirement logic, `embyr-server`'s agent-mode defensive
rejection) — comparable to `firestore-transaction-read-consistency`'s own 3-crate touch, which
PASSED this gate on the same reasoning (each crate's own change is small and mechanical once
designed). Walking skeleton >5 integration points? No (2: a real OR query on an unrestricted
collection; a real OR query on an ownership-protected collection). Estimated effort >2 weeks? No —
the CompositeOr variant is additive (zero changes to the 15 existing `Composite` call sites), the
SQL-generation change is small (parenthesize + choose join operator), the security-logic change is
one function's own recursion rule (`.any()` → `.all()` for the new variant only). Multiple
independent user outcomes? No — OR-query execution and OR-query-security-compliance are 2 faces of
the SAME underlying gap (the codebase never modeled OR at all).

**Scope Assessment: PASS** (0 oversizing signals fired, same reasoning precedent as
`firestore-transaction-read-consistency`'s own multi-crate PASS).

## Wave: DISCUSS / [REF] Journey — Alex's "My OR Query Works, and Still Can't Leak Data" Arc

### Mental model

Alex's app queries `orders` for `status == 'pending' OR status == 'processing'` using the SDK's
own `Filter.or(...)` — real, documented Firestore surface. Alex expects this to just work,
returning the union of both statuses. Separately, Alex's `journal_entries` collection has a
security rule requiring `resource.data.ownerId == request.auth.uid` — Alex expects, exactly as
with any other query shape, that embyr either correctly enforces this for an OR query too, or
cleanly rejects it with a named reason if it structurally can't be verified — never that it silently
admits a query that could leak another user's documents.

### Failure modes (feeds DELIVER test design)

- `RunQuery` with `Filter.or(a, b)` on an UNRESTRICTED collection: today rejected outright with
  a generic error; after, returns the union of documents matching either `a` or `b`.
- `RunQuery` with `Filter.or(ownerId==uid, other==x)` on an ownership-protected collection: the
  `other==x` branch does NOT independently prove ownership — this query must be REJECTED (not
  silently admitted), since a document could match via the `other==x` branch alone.
- `RunQuery` with `Filter.or(ownerId==uid, And(ownerId==uid, other==x))` — BOTH branches
  independently constrain `ownerId==uid` — this query must be ADMITTED, since every possible match
  (from either branch) satisfies the ownership rule.
- A well-formed AND-only query (every existing test in this codebase): completely unaffected — the
  existing `Composite` variant, and every one of its 15 call sites, is untouched.

## Wave: DISCUSS / [REF] Story Map & Walking Skeleton

### Backbone

Alex's app calls `RunQuery` with `Filter.or(...)` → today rejected outright, `"unsupported
composite operator"` → **this feature**: `translate_filter` accepts `CompositeOp::Or`, producing a
new `QueryFilter::CompositeOr` domain variant → `append_filter` OR-joins and parenthesizes its own
children → `filter_binds_field_to_uid` correctly requires EVERY branch of an OR to independently
prove an ownership constraint (never `.any()`) → `requires_composite_index`/`missing_index_fields`
reuse their own existing field-collection logic unchanged → `backend_mode=agent` cleanly rejects OR
filters (its own internal proto has no `Or` operator) instead of silently mistranslating them as
AND.

### Walking Skeleton

**Slice 01**: `QueryFilter::CompositeOr` domain variant + `translate_filter` acceptance + SQL
generation (OR-join, parenthesized), proven end-to-end against a real, unrestricted-collection OR
query returning the correct union.

**Slice 02 (LAST slice)**: `filter_binds_field_to_uid`'s own `.any()` → `.all()` fix for OR nodes,
proven end-to-end against BOTH a correctly-rejected OR query (one branch doesn't prove ownership)
and a correctly-admitted one (every branch does) — the security-critical half of this feature.

## Wave: DISCUSS / [REF] Elephant Carpaccio Slices

| # | Story | Release | Estimate | Learning Hypothesis | Reference Class |
|---|---|---|---|---|---|
| 01 | US-01 (OR-query execution, Walking Skeleton) | 1 | ≤1 day | Disproves: OR-query execution is a small, additive extension (new domain variant, new SQL-join arm, new index-requirement recursion arm) that doesn't require touching any of the 15 existing AND-only call sites | Mirrors this session's own repeated "additive variant over breaking-change" pattern (e.g. `firestore-malformed-filter-shape-validation`'s own additive rejection checks) |
| 02 | US-02 (OR-query security compliance, LAST slice) | 1 | ≤1 day | Disproves: the `.any()`→`.all()` fix, applied ONLY to the new variant, closes the access-control gap without touching AND's own already-correct `.any()` behavior | Direct extension of ADR-031's own `filter_binds_field_to_uid`, the single centralized function all 7 `check_query_compliance()` callers already route through |

## Wave: DISCUSS / [REF] Prioritization

Sequential 01 → 02: Slice 01 (execution) has no meaning without a correct result set to check
compliance against; Slice 02 (security) is the higher-stakes, higher-uncertainty slice and
deliberately follows once the mechanical plumbing is proven — matches this session's own
established "prove the mechanism, then prove the hard part" ordering (e.g.
`firestore-transaction-read-consistency`'s Slice 01 before 02/03).

## Wave: DISCUSS / [REF] System Constraints

- `QueryFilter` gains ONE new, ADDITIVE variant: `CompositeOr(Vec<QueryFilter>)`. The existing
  `Composite(Vec<QueryFilter>)` variant, its own doc comment, and all 15 existing call sites are
  UNCHANGED — this is a non-breaking domain-type extension, not a modification of the existing AND
  shape's own tuple structure.
- `append_filter` (`embyr-pg-storage`): `Composite` children remain joined with bare `AND ` (no
  parens — behavior-preserving, since pure-AND-of-AND is associative and paren-free was always
  correct). `CompositeOr`'s own children are OR-joined AND the WHOLE OR expression is wrapped in
  parens (`(...)`) — required for correct precedence the moment OR can appear nested inside AND
  context (or vice versa), which was structurally impossible before this feature.
- `filter_binds_field_to_uid` (`crates/embyr-core/src/access_control/mod.rs`): `Composite` keeps
  its own existing `.any()` recursion, byte-for-byte unchanged. A NEW match arm for `CompositeOr`
  uses `.all()` — EVERY branch must independently satisfy the binding, matching real Firestore's
  own documented query-validation model for OR + security rules.
- `requires_composite_index`/`missing_index_fields` (`crates/embyr-server/src/grpc/handler.rs`):
  extended with a `CompositeOr` arm that reuses the SAME field-collection logic already applied to
  `Composite` — flattening an OR's own children into the same "which fields are filtered/ordered"
  set. Real Firestore's own OR-specific composite-index nuances (e.g. whether an OR combining
  different fields needs its OWN distinct index shape beyond what this reuse produces) are
  explicitly named as a SEPARATE, deferred follow-up (§ Out of Scope) — this feature does not widen
  the index-requirement HEURISTIC itself, only ensures OR queries are correctly WALKED by the
  existing one.
- `domain_filter_to_agent_filter` (`crates/embyr-server/src/adapters/agent_backend.rs`): gains a
  `QueryFilter::CompositeOr(_) => Err(CoreError::FailedPrecondition("OR filters are not supported
  when backend_mode=agent".into()))` arm — `backend_mode=agent` is explicitly deferred (§ Out of
  Scope), mirroring the established agent-mode-deferral pattern exactly. Silently mistranslating OR
  as AND (returning the intersection instead of the union) would be a CORRECTNESS bug this function
  has already been burned by once before (its own doc comment documents a prior cross-user-leak
  incident from a different silent-mistranslation bug) — a clean rejection is the only acceptable
  fallback.
- Every non-`FieldFilter`/non-`Composite`/non-`CompositeOr` match on `QueryFilter` (any exhaustive
  match the compiler flags) is a REQUIRED touch point, confirmed by direct grep, not discovered
  by trial and error at compile time.

## Wave: DISCUSS / [REF] User Stories

### US-01: `Filter.or()` Queries Execute Correctly (Union Semantics)

**job_id**: JOB-01 | **Release**: 1 | **Persona**: P1 Alex

#### Elevator Pitch
Before: `db.collection('orders').where(Filter.or(Filter.where('status','==','pending'),
Filter.where('status','==','processing')))` is rejected outright with `INVALID_ARGUMENT:
unsupported composite operator`.
After: run the identical query → sees every document matching EITHER branch (the union), matching
real Firestore's own documented `Filter.or()` semantics exactly.
Decision enabled: Alex can express "any of these conditions" queries using the SDK's own standard
composite-filter API, without working around embyr's own gap by issuing 2 separate queries and
merging client-side.

#### Acceptance Criteria
- [ ] AC-OR-01: a real `RunQuery` with a top-level `Filter.or(a, b)` on an UNRESTRICTED collection
      returns the union of documents matching `a` or `b`.
- [ ] AC-OR-02: an OR filter nested inside an AND filter (e.g. `AND(x==1, OR(y==2, y==3))`) is
      correctly parenthesized in the generated SQL — verified via a real query proving the correct
      precedence (not `x==1 AND y==2 OR y==3`, which would match `y==3` regardless of `x`).
- [ ] AC-OR-03 (regression guard): every existing AND-only query shape in this codebase's own test
      suite continues to work identically — zero change to `Composite`'s own behavior.

### US-02: `Filter.or()` Queries Are Correctly Checked Against Ownership-Equality Security Rules

**job_id**: JOB-01 | **Release**: 1 | **Persona**: P1 Alex

#### Elevator Pitch
Before: `Filter.or()` queries against a security-rules-protected collection are rejected outright
(the same generic error as US-01's own gap) — accidentally "safe" only because OR isn't supported
at all yet, not because of any real compliance check.
After: run an OR query where EVERY branch independently proves `ownerId == callerUid` → sees the
correctly-admitted result. Run an OR query where only SOME branches prove it → sees a clean,
named rejection — never a silent admission that could leak another tenant's documents.
Decision enabled: Alex can trust that adding OR support did not create a new access-control gap —
the SAME correctness guarantee this codebase already provides for AND queries now extends to OR.

#### Acceptance Criteria
- [ ] AC-OR-04: a real `RunQuery` with `Filter.or(ownerId==uid, other==x)` against an
      ownership-protected collection is REJECTED (the `other==x` branch does not independently
      prove ownership).
- [ ] AC-OR-05: a real `RunQuery` with `Filter.or(ownerId==uid, AND(ownerId==uid, other==x))`
      against the SAME collection is ADMITTED (every branch independently proves ownership) and
      returns the correct, ownership-scoped result set.
- [ ] AC-OR-06 (regression guard): every existing AND-only ownership-compliance test continues to
      pass unmodified — `filter_binds_field_to_uid`'s own `Composite` arm (`.any()`) is untouched.
- [ ] AC-OR-07: `backend_mode=agent` cleanly rejects an OR filter (`FailedPrecondition`, named
      reason) instead of silently mistranslating it as AND.

## Wave: DISCUSS / [REF] Outcome KPIs

### Feature: firestore-or-filter-support

### Objective
Close gap #3 from the 2026-09-06 production-readiness scan (`Filter.or()` rejected outright) WITHOUT
reopening a silent-wrong-data-class access-control gap — the investigation this DISCUSS performed
found the naive fix would have created exactly that.

### Outcome KPIs
| KPI | Target | Measurement |
|---|---|---|
| OR-query execution correctness (union semantics, correct precedence when nested) | 100% | AC-OR-01/02 |
| OR-query security-compliance correctness (every branch must independently prove ownership) | 100%, 0 false-admits | AC-OR-04/05 |
| Regression on existing AND-only query/compliance behavior | 0 | AC-OR-03/06 |
| `backend_mode=agent` OR-filter rejection (not silent mistranslation) | Clean, named rejection | AC-OR-07 |
| Mutation-testing kill rate on the new/changed logic, ESPECIALLY `filter_binds_field_to_uid`'s new arm | 100% effective | `cargo-mutants --in-diff` |

## Wave: DISCUSS / [REF] Out of Scope

- **`backend_mode=agent` OR-filter execution** — the agent's own internal `CompositeFilterOp` proto
  enum has no `Or` variant; extending it is a separate, deferred candidate feature (candidate id
  `agent-mode-or-filter-support`), mirroring the established agent-mode-deferral pattern. This
  feature only ensures agent-mode CLEANLY REJECTS OR filters, never silently mistranslates them.
- **OR-specific composite-index requirement widening** — `requires_composite_index` reuses its own
  existing field-collection heuristic for `CompositeOr`'s children; whether real Firestore's own OR
  queries have DISTINCT composite-index requirements beyond what this reuse produces (e.g. an OR
  combining inequality filters on 2 different fields) is a separate, later investigation, not
  resolved here.
- **`NOT` composite operator, or any other future `CompositeOp` variant** — only `And`/`Or` exist in
  the real Firestore proto today; no other variant is in scope.
- **Deep/multi-level OR-of-OR-of-AND nesting beyond what naturally falls out of the recursive
  design** — the design IS fully recursive (both `.any()`/`.all()` and the SQL-generation
  parenthesization apply at every nesting level by construction, not as a special case), so this is
  not an artificially narrowed scope — just named explicitly so a future reader knows arbitrary
  nesting was a natural consequence of the chosen design, not a separately-tested guarantee beyond
  what the recursive structure itself proves.

## Wave: DISCUSS / [REF] WS Strategy

**Strategy A** (real, minimal, end-to-end) — both slices are real `RunQuery` calls against a real
running server and Postgres backend, including a real security-rules-protected collection for
Slice 02.

## Wave: DISCUSS / [REF] Driving Ports

gRPC `:8080` `RunQuery`, `RunAggregationQuery`, `Listen` (all existing routes, zero new RPC — all 3
already route through `translate_filter`/`check_query_compliance`).

## Wave: DISCUSS / [REF] Pre-requisites

- `docs/product/architecture/adr-031-query-shape-compliance-check.md` — the exact mechanism this
  feature extends; this DISCUSS's own central finding is a direct consequence of that ADR's own
  documented AND-only assumption.
- No new external dependency, no new bounded context.

## Wave: DISCUSS / [REF] Handoff Package

Handed to `nw-solution-architect` (DESIGN): this feature-delta.md, the confirmed
access-control-bypass risk of the naive fix (and why it's avoided), and the explicit instruction to
design `QueryFilter::CompositeOr` as a strictly ADDITIVE domain variant — never modify
`Composite`'s own existing shape or any of its 15 call sites.

## Wave: DISCUSS / [REF] SSOT Updates

`docs/product/jobs.yaml`, JOB-01 entry: append a new dated NOTE — "JOB-01 now also covers
`Filter.or()` composite queries executing correctly (union semantics, correct precedence when
nested inside AND) — previously rejected outright. This feature's own DISCUSS wave found and
avoided a real access-control-bypass risk in the naive fix: `QueryFilter::Composite`'s AND-only
assumption is load-bearing for ADR-031's own `filter_binds_field_to_uid` security check
(`.any()` semantics, correct ONLY because every composite meant AND). Added a strictly ADDITIVE
`CompositeOr` domain variant (zero change to the 15 existing AND-only call sites) with its own
`.all()` compliance-check semantics — every branch of an OR must independently prove an
ownership-equality constraint for the query to be admitted, matching real Firestore's own
documented query-validation model. `backend_mode=agent` cleanly rejects OR filters (its own
internal proto has no `Or` operator) rather than silently mistranslating them. Same job, same
persona, not a new job. See
docs/feature/firestore-or-filter-support/feature-delta.md § Reading Confirmation, § System
Constraints."

## Wave: DISCUSS / [REF] Definition of Ready Validation

### Requirements Completeness Score: **0.97**

### DoR Checklist (9-item hard gate)
1. [x] Every story traces to a job_id (JOB-01)
2. [x] Story has a complete Elevator Pitch (both stories)
3. [x] Every AC is testable without ambiguity
4. [x] Walking Skeleton identified (US-01, extended by US-02)
5. [x] Scope Assessment passed
6. [x] No slice contains only `@infrastructure` stories
7. [x] Out of Scope explicitly named (4 items)
8. [x] Outcome KPIs have numeric targets and measurement methods
9. [x] Prior-wave artifacts read and reconciled (ADR-031's own documented AND-only assumption
   directly informed this feature's own design, not discovered after the fact)

### DoR Status: **PASSED**

## Wave: DISCUSS / [REF] Open Questions

None carried forward unresolved. The one genuinely important open question this DISCUSS itself
raised (does the naive fix create a security bug?) was directly investigated and resolved with a
locked design decision (§ System Constraints), per the user's own explicit choice to pursue the
full, security-correct fix over the narrower alternative.

## Wave: DISCUSS / [REF] Wave Decisions Summary

### Key Decisions
- [D1] `QueryFilter` gains a strictly ADDITIVE `CompositeOr` variant — `Composite` and all 15 of
  its existing call sites are completely unchanged.
- [D2] `filter_binds_field_to_uid` gains a NEW match arm for `CompositeOr` using `.all()` semantics
  (every branch must independently prove the ownership binding) — `Composite`'s own existing
  `.any()` arm is untouched.
- [D3] `append_filter` OR-joins `CompositeOr`'s children and wraps the WHOLE expression in parens
  for correct precedence; `Composite`'s own bare-AND, paren-free join is unchanged (still correct,
  since AND-of-AND remains associative).
- [D4] `backend_mode=agent` cleanly rejects `CompositeOr` (its own internal proto has no `Or`
  operator) — explicitly deferred, not silently mistranslated.
- [D5] `requires_composite_index`/`missing_index_fields` reuse their own existing field-collection
  logic for `CompositeOr`'s children — no new index-requirement heuristic; OR-specific nuances
  named as a separate, deferred follow-up.

### Requirements Summary
- Primary need: `Filter.or()` queries must execute correctly (union semantics) AND remain
  security-correct against ownership-equality rules — today rejected outright, and a naive fix
  would have introduced an access-control bypass.
- Walking skeleton scope: US-01 (execution), extended by US-02 (security compliance, the
  higher-stakes half).
- Feature type: Backend, with a real security-correctness dimension.

### Constraints Established
- Zero change to `QueryFilter::Composite`'s own existing shape or any of its 15 call sites.
- `filter_binds_field_to_uid`'s own existing AND-arm (`.any()`) is byte-for-byte unchanged.
- `backend_mode=agent` unaffected in the sense that it never silently mistranslates OR — cleanly
  deferred instead.

### Upstream Changes
None — this DISCUSS confirms rather than contradicts JOB-01's existing scope; the SSOT's own
documented ADR-031 assumption is extended, not violated (a NEW variant with its own correct
semantics, not a mutation of the existing AND-only guarantee).

## Wave: DISCUSS / [REF] Next Wave

**Handoff To**: nw-solution-architect (DESIGN wave)
**Deliverables**: this feature-delta.md, 5 locked Decisions, 2-slice plan

---

## Wave: DESIGN / [REF] Prior Wave Consultation — Reading Confirmation

✓ This feature-delta.md's own DISCUSS sections in full, all 5 Decisions.
✓ `crates/embyr-core/src/access_control/mod.rs::filter_binds_field_to_uid`'s own exact current
implementation (lines 2147-2163) — the EXACT insertion point for the new `CompositeOr` match arm.
✓ `crates/embyr-pg-storage/src/encoding/query.rs::append_filter`'s own exact current implementation
— the EXACT insertion point and pattern for the new `CompositeOr` SQL-generation arm.

## Wave: DESIGN / [REF] Architecture Design

### 1. Domain (`crates/embyr-core/src/domain/query.rs`)

```rust
pub enum QueryFilter {
    Field(FieldFilter),
    /// Composite AND filter — unchanged, every existing call site unaffected.
    Composite(Vec<QueryFilter>),
    /// Composite OR filter (firestore-or-filter-support). A document matches
    /// iff it matches AT LEAST ONE child filter — the union, not the
    /// intersection `Composite` represents.
    CompositeOr(Vec<QueryFilter>),
}
```

### 2. `translate_filter` (`crates/embyr-server/src/grpc/handler.rs`)

```rust
FilterType::CompositeFilter(cf) => {
    match CompositeOp::try_from(cf.op).unwrap_or(CompositeOp::Unspecified) {
        CompositeOp::And | CompositeOp::Unspecified => {
            // unchanged
        }
        CompositeOp::Or => {
            let mut filters = Vec::new();
            for sub in &cf.filters {
                match translate_filter(sub) {
                    Some(Ok(qf)) => filters.push(qf),
                    Some(Err(e)) => return Some(Err(e)),
                    None => {}
                }
            }
            Some(Ok(QueryFilter::CompositeOr(filters)))
        }
    }
}
```

### 3. `append_filter` (`crates/embyr-pg-storage/src/encoding/query.rs`)

```rust
pub fn append_filter(qb: &mut QueryBuilder<Postgres>, filter: &QueryFilter) {
    match filter {
        QueryFilter::Field(f) => append_field_filter(qb, f),
        QueryFilter::Composite(filters) => {
            for (i, f) in filters.iter().enumerate() {
                if i > 0 { qb.push(" AND "); }
                append_filter(qb, f);
            }
        }
        QueryFilter::CompositeOr(filters) => {
            qb.push("(");
            for (i, f) in filters.iter().enumerate() {
                if i > 0 { qb.push(" OR "); }
                append_filter(qb, f);
            }
            qb.push(")");
        }
    }
}
```

### 4. `filter_binds_field_to_uid` (`crates/embyr-core/src/access_control/mod.rs`)

```rust
fn filter_binds_field_to_uid(filter: Option<&QueryFilter>, field_path: &str, caller_uid: &str) -> bool {
    match filter {
        None => false,
        Some(QueryFilter::Field(ff)) => { /* unchanged */ }
        Some(QueryFilter::Composite(filters)) => filters
            .iter()
            .any(|f| filter_binds_field_to_uid(Some(f), field_path, caller_uid)),
        Some(QueryFilter::CompositeOr(filters)) => filters
            .iter()
            .all(|f| filter_binds_field_to_uid(Some(f), field_path, caller_uid)),
    }
}
```

### 5. `requires_composite_index`/`missing_index_fields` (`crates/embyr-server/src/grpc/handler.rs`)

Both gain a `QueryFilter::CompositeOr(sub) => { /* identical body to the existing Composite arm */ }`
arm — same recursive field-collection walk, no new heuristic.

### 6. `domain_filter_to_agent_filter` (`crates/embyr-server/src/adapters/agent_backend.rs`)

```rust
QueryFilter::CompositeOr(_) => Err(CoreError::FailedPrecondition(
    "OR filters are not supported when backend_mode=agent".into(),
)),
```

## Wave: DESIGN / [REF] Wave Decisions Summary

### Key Decisions
- [D1] `CompositeOr` is a NEW enum variant, not a parameterization of `Composite` (e.g.
  `Composite(LogicalOp, Vec<QueryFilter>)`) — the additive-variant shape means the compiler forces
  every exhaustive match to be updated (a REQUIRED, visible touch point), while every EXISTING call
  site that only ever constructs/matches `Composite` needs zero changes at all — the smaller,
  safer diff.
- [D2] `filter_binds_field_to_uid`'s new arm is `.all()`, not `.any()` — the single, precise
  security-correctness fix this whole feature exists to make.
- [D3] SQL parenthesization is `CompositeOr`-only — `Composite`'s own existing paren-free AND-join
  is provably still correct (AND-of-AND is associative) and left untouched, avoiding unnecessary
  churn to already-correct, already-tested SQL generation.

### Constraints Established
- No new `CoreError` variant beyond reusing `FailedPrecondition` (already used for
  `run_aggregation_query`'s/`list_collection_ids`'s own "not supported by this backend" pattern).
- No new port/adapter trait method.
- Every one of the 6 files identified in § Reading Confirmation's blast-radius grep is a REQUIRED
  touch point (the compiler's own exhaustiveness check enforces this) except the 15 pre-existing
  `Composite`-only call sites, which remain untouched by construction.

## Wave: DESIGN / [REF] Next Wave

**Handoff To**: nw-software-crafter (DELIVER wave, per this project's own established convention)
**Deliverables**: this feature-delta.md's DESIGN section, 2 slice briefs
