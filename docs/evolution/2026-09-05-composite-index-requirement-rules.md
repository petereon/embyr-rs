# Evolution: composite-index-requirement-rules

**Date:** 2026-09-05
**Feature:** Widens `requires_composite_index()`'s own heuristic to match all 5 of `docs/SPEC.md`'s
own documented composite-index trigger shapes (up from 3 of 5), and names the specific missing
index in the `FAILED_PRECONDITION` rejection.
**Job:** JOB-01 (`sdk-compat`) — a further realization, closing a documented-but-not-fully
-implemented gap.
**ADRs:** none (folded into this feature-delta.md's own DESIGN section — small enough scope that a
separate ADR file added nothing).

## Business Context

`docs/SPEC.md` already documented a precise target contract for composite-index enforcement,
written by a prior wave: multi-field `orderBy`, any filter combined with `orderBy` on a different
field, `array-contains-any`, `not-in`, and `IN` with more than one equality constraint all require a
composite index; single-field queries are always exempt. The actual `requires_composite_index()`
implementation only correctly detected 1 of these 5 shapes (the single-field-vs-different-orderBy
-field case) — 2 real gaps silently accepted queries real Firestore would reject: multi-field
`orderBy` was never checked at all, and any filter-only (no `orderBy`) composite trigger was
bypassed entirely by a top-level `order_by.is_empty()` short-circuit.

**The central finding of this feature's own DISCUSS wave reversed its own initial hypothesis.**
SPEC.md's own wording — "inequality filters combined with `orderBy` on a different field" — implied
a plain EQUALITY filter should be exempt from that trigger, which would have meant this codebase's
own existing, load-bearing evidenced test (`category == "A"` + `orderBy score`, used since the
ORIGINAL 2026-05-27 walking skeleton) was itself Firestore-inaccurate. Live web verification against
firebase.google.com directly refuted this: real Firestore requires a composite index for EQUALITY +
different-field `orderBy` identically to inequality — the existing test and the current heuristic's
own behavior on that specific shape were BOTH already correct. SPEC.md's own wording was the thing
that was imprecise, corrected as a companion fix in this feature's own DELIVER.

## Key Decisions

| Decision | Verdict |
|---|---|
| Reuse JOB-01 — the "make it real" pattern, same shape as `aggregation-queries`/`batch-get-documents`/`firestore-composite-indexes-admin-api` | feature-delta.md § Resolution 1 |
| The existing `category==`/`score`-orderBy test and the current heuristic's behavior on that shape are BOTH correct — preserved as an explicit regression guard, NOT superseded | § Resolution 2, reversed the DISCUSS's own initial hypothesis under live verification |
| Exactly 2 genuine gaps closed: multi-field `orderBy` (regardless of filters); `IN` + range on a different field (independent of `orderBy` presence) | § Resolution 3, each individually traced against the current implementation |
| Two-simultaneous-`IN` is out of scope — genuinely undetermined by live verification | § Resolution 4 |
| A separate, adjacent real-Firestore constraint (a range filter's own `orderBy` must start on the same field) is out of scope — a structurally different query-VALIDITY mechanism, not an index-requirement rule | § Resolution 5 |
| `FilterOp` gains `Copy`; `collect_filter_fields` carries the operator alongside each field path — the smallest unblock for per-operator classification | DESIGN § D1/D2 |
| The 2 new rules run BEFORE the original rule's own `order_by`-emptiness path — fixes Slice 02's own root cause directly rather than layering a parallel check | DESIGN § D3 |

## Steps Completed

1. **Slice 01** (US-01, Walking Skeleton) — 2+ `orderBy` fields always require composite,
   regardless of filters, checked first, unconditionally. A one-line, purely additive change; zero
   modification to `collect_filter_fields`.
2. **Slice 02** (US-02) — `IN` on one field combined with a range filter (`<`/`<=`/`>`/`>=`) on a
   DIFFERENT field requires composite, independent of `orderBy` presence — closes the filter-only
   blind spot the original top-level short-circuit created.
3. **Slice 03** (US-03, LAST slice) — the `FAILED_PRECONDITION` rejection now names the specific
   `collection_path`/`fields` the missing index needs (`missing_index_fields`, a new pure function
   reusing `IndexFieldSpec`/`IndexFieldOrder` from `firestore-composite-indexes-admin-api`
   unchanged), formatted as the exact JSON shape `CreateIndex` expects.

**QUALITY_GATE** — `cargo-mutants --in-diff` across both touched files (`handler.rs`, `query.rs`),
`--lib`-scoped, Docker-free: **2 caught, 24 unviable, 0 missed, 0 timeouts — 100% effective kill
rate on the FIRST pass**, the first feature this session where no gap-closing follow-up was needed
(unit tests for every new branch's own distinguishing boundary were written during each slice's own
DELIVER from the start, applying every prior feature's own accumulated lesson proactively rather
than reactively).

**Full regression**: `cargo test -p embyr-server`, 441 tests passed, 56 target binaries, 0 failures
attributable to this feature (1 pre-existing, unrelated `distributed_rate_limiting` flake, the same
documented failure mode from this session's own prior evolution docs).

## ⚠️ Discovered Gap — `append_field_filter` Panics on `In`/`NotIn`/`ArrayContains`/`ArrayContainsAny`

**This is a real, pre-existing, HIGHER-SEVERITY bug, unrelated to this feature's own scope,
discovered mid-DELIVER via a genuine crashing test — not fixed here, flagged prominently.**

`crates/embyr-pg-storage/src/encoding/query.rs::append_field_filter` implements SQL translation for
only `LessThan/LessThanOrEqual/GreaterThan/GreaterThanOrEqual/Equal/NotEqual` (plus `IsNan`/
`IsNotNan`, special-cased). Every other `FilterOp` variant — `In`, `NotIn`, `ArrayContains`,
`ArrayContainsAny` — hits its own `panic!("unsupported filter op: {:?}", f.op)` fallback arm.

**Impact**: any live customer application issuing a real `.where(field, 'in', [...])`,
`.where(field, 'not-in', [...])`, or `.where(field, 'array-contains-any', [...])` query against
embyr TODAY crashes the specific gRPC request task. The client observes a raw `h2 protocol error`/
`Cancelled` transport reset, not a clean gRPC status code — a worse failure mode than an ordinary
error response, though confirmed non-fatal to the server process or other connections (the panic
unwinds only the one task).

**Why not fixed as part of this feature**: this feature is scoped to WHEN a composite index is
required (the gating decision), not whether the query engine can execute a given filter operator at
all — a materially different, larger, separately-evidenced feature (correct SQL translation for
`IN`/`NOT IN`/array-containment semantics: `= ANY($1)`, JSONB `@>`/overlap, etc.).

**How it was found**: this feature's OWN DISCUSS Reading Confirmation initially, incorrectly,
claimed every filter operator already worked end-to-end (under-verified — only `append_filter`'s
generic dispatch shape was checked, not `append_field_filter`'s own full match arms). Slice 02's own
acceptance tests needed `In`/`ArrayContainsAny`/`NotIn` to actually execute successfully to prove
their own "no false positive" regression guards — they crashed instead, surfacing the real gap. The
DISCUSS doc was corrected in place rather than left standing; AC-CIR-05/AC-CIR-06 were downgraded to
unit-level-only proof (the pure gating function's own decision is correct and tested; the query
executor's crash is the separate, unrelated defect).

**Recommended follow-up**: a candidate feature (e.g. `firestore-query-filter-operator-support`) to
implement correct SQL translation for these 4 operators — recommended as HIGHER priority than any
remaining composite-index work, since it is an active production crash risk today, not merely an
accuracy gap.

## Lessons Learned

1. **Live web verification can reverse, not just confirm, a hypothesis formed from local
   documentation** — this feature's own SPEC.md-derived initial hypothesis (equality filters exempt
   from the different-field-`orderBy` trigger) was directly wrong; treating SPEC.md's own wording as
   ground truth without independent verification would have led to "fixing" a test that was already
   correct, and potentially breaking real, evidenced, working behavior.
2. **Writing unit tests for the SPECIFIC BOUNDARY each new rule introduces (not just its own happy
   path) during DELIVER, from the start, can eliminate the need for a QUALITY_GATE gap-closing pass
   entirely** — this is the first feature this session where that happened, achieved by directly
   applying the exact lesson every PRIOR feature's own QUALITY_GATE had to learn reactively (e.g.
   `firestore-composite-indexes-admin-api`'s own "test the EXACT boundary value, not just above and
   below" finding).
3. **A DISCUSS-time Reading Confirmation claim about a DIFFERENT layer of the system (the query
   executor, not the function this feature actually changes) still needs direct verification, not
   inference from a nearby, correctly-verified fact** — confirming that `run_query`'s generic filter
   dispatch worked was not the same claim as confirming EVERY filter operator's own translation was
   implemented; conflating the two produced a real, though ultimately harmless-to-this-feature,
   documentation error that only a real acceptance test caught.
4. **A severe bug discovered as a side effect of an unrelated feature's own acceptance testing
   should be flagged prominently and separately, not folded into routine scope-boundary notes** —
   this bug is arguably more urgent than the feature that found it; treating it as just another
   "Out of Scope" bullet would have buried a real production risk.

## Key Files

- `crates/embyr-server/src/grpc/handler.rs` — `requires_composite_index` (restructured control
  flow, 2 new rules), `collect_filter_fields` (new `Vec<(&str, FilterOp)>` signature),
  `missing_index_fields` (new), the `FAILED_PRECONDITION` message, 14 new unit tests
  (`composite_index_requirement_tests`).
- `crates/embyr-core/src/domain/query.rs` — `FilterOp` gains `Copy` (zero behavior change).
- `crates/embyr-pg-storage/` — **untouched by this feature** (the discovered gap lives here, but is
  explicitly out of scope).
- `tests/composite_index_requirement_rules/acceptance/` — cir01 through cir03, 3 acceptance
  targets, shared `common/mod.rs` (re-exports `firestore_composite_indexes_admin_api`'s own
  harness).
- `docs/feature/composite-index-requirement-rules/feature-delta.md` — full DISCUSS/DESIGN
  narrative, including the § Discovered Gap section.
- `docs/feature/composite-index-requirement-rules/slices/` — 3 elephant-carpaccio slice briefs.
- `docs/feature/composite-index-requirement-rules/deliver/mutation/mutation-report.md`
- `docs/SPEC.md` line 146 — corrected wording (equality named explicitly alongside inequality).
- `docs/product/jobs.yaml`, JOB-01 — 2 NOTEs appended (this feature's own, plus a caught-up
  back-propagation gap from `firestore-composite-indexes-admin-api`'s own FINALIZE, which had
  promised but never applied its own NOTE).

## Follow-Up Work

- **`firestore-query-filter-operator-support`** (candidate id, HIGHER priority than the items
  below) — implement correct SQL translation for `In`/`NotIn`/`ArrayContains`/`ArrayContainsAny` in
  `append_field_filter`, closing a real, active production crash risk (§ Discovered Gap above).
- **The query-validity constraint** (a range filter's own `orderBy` must start on the same field) —
  real-Firestore-accurate, live-verified, structurally different from index-requirement detection.
  Candidate id proposed: `firestore-range-orderby-field-validity`.
- **Two simultaneous `IN` filters** — genuinely undetermined by live verification, zero domain
  evidence. Named, deferred, no candidate feature id assigned.
- Carried forward, unchanged, from `firestore-composite-indexes-admin-api`: real Postgres index
  provisioning for query performance; real Firestore's own async `CREATING` index-build window;
  per-field-set granularity in `IndexManager::is_index_ready`.
