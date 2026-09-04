# Feature Delta: firestore-composite-indexes-admin-api

## Wave: DISCUSS / [REF] Prior Wave Consultation — Reading Confirmation

✓ `crates/embyr-server/src/adapters/index_manager.rs` (full, 32 lines) — `IndexManager::
is_index_ready(project_id, collection_id)` runs `SELECT count(*) FROM composite_indexes WHERE
project_id = $1 AND collection_path = $2 AND status = 'ready'` — a per-COLLECTION existence check
(not per exact field-set), already correctly wired.
✓ `crates/embyr-server/src/grpc/handler.rs::requires_composite_index`/`collect_filter_fields`
(lines 577-599) — `RunQuery`'s own gate: a query needing a composite index is any filter+`orderBy`
combination where an `orderBy` field is NOT among the filtered fields. Wired into `handle_run_query`
(lines 3073-3085): `requires_index && !is_index_ready(...)` → `Status::failed_precondition`.
Confirms directly: **this gate has existed since the original walking-skeleton slice (04-02,
`docs/evolution/2026-05-27-embyr-rs.md`), is correctly wired, and is completely un-satisfiable
today** — `composite_indexes` starts empty for every project and NO code path anywhere in this
repository ever inserts a row into it outside of test fixtures.
✓ `migrations/0003_composite_indexes.sql` (full) — `composite_indexes(id UUID PK, project_id,
collection_path, fields JSONB, status VARCHAR DEFAULT 'ready', created_at, UNIQUE(project_id,
collection_path, fields))`. The `status` column's own DEFAULT is already `'ready'` — this table was
built ANTICIPATING a create-is-immediately-ready v1, never a pending/build-latency model.
✓ `tests/acceptance/us_04_query_collection.rs::query_succeeds_after_index_reaches_ready_status`
(lines 599-654) — the ONLY place in this entire codebase a `composite_indexes` row is ever
inserted, and it is a raw `sqlx::query` INSERT directly against the system DB from a TEST, not a
production code path. Confirms the EXACT `fields` JSONB shape already assumed elsewhere in this
codebase: `[{"field": "category", "order": "ASC"}, {"field": "score", "order": "DESC"}]` — this
feature's own request/response body reuses this shape verbatim, not a newly-invented one.
✓ `crates/embyr-server/src/admin/router.rs` (lines 240-354) — direct read of every existing admin
CRUD route's own registration shape. Two directly-reusable precedents: `.route("/admin/v1/
service_accounts", get(list_service_accounts).post(create_service_account))` (GET+POST chained on
one path) and `.route("/admin/v1/service_accounts/:sa_id", delete(delete_service_account))`
(DELETE-by-id on a distinct sibling path) — this feature's own 2 routes (`/admin/v1/projects/
:project_id/indexes` for POST+GET, `/admin/v1/projects/:project_id/indexes/:index_id` for DELETE)
mirror this exactly.
✓ `crates/embyr-server/src/admin/handlers/access_rules.rs::define_access_rule` (Owner/Admin gated
in-handler, `SessionContext` + `verify_project_ownership`) — the exact auth/ownership-check shape
every admin CRUD handler in this codebase already uses; this feature's own 3 handlers reuse it
unchanged.
✓ `docs/product/jobs.yaml`, JOB-01 (`sdk-compat`, persona P1 Alex) — read in full, including the
`aggregation-queries` NOTE (lines 27-50) and `batch-get-documents` NOTE: both closed a
**documented-but-unbuilt gap** in Alex's own "make embyr behave identically to real Firestore"
job, reusing JOB-01 rather than creating a new job — the identical shape as this feature's own
gap (a gating mechanism that already exists in code, with no way to ever satisfy it).
✓ `docs/product/journeys/sdk-developer.yaml` — Alex's existing journey; no step for index
management exists yet, consistent with this being a genuinely new (if narrow) capability within
his already-established job, not a contradiction of any prior DISCOVER finding.

**No live web verification performed this DISCUSS** — this feature's own request/response shape is
grounded entirely in THIS codebase's own already-established `fields` JSONB convention (confirmed
above by direct test-fixture read), not real Firestore's own `google.firestore.admin.v1.Index`
protobuf shape, which this project's admin HTTP surface has never mirrored 1:1 for any prior admin
resource (`access_rules`, `write_access_rules`, etc. all use this project's own JSON conventions,
not a transliterated Firestore Admin proto).

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

- Feature type: **Backend** (Decision 1) — a new admin HTTP CRUD surface over an existing,
  already-wired gating table; zero SDK/data-plane surface change.
- JTBD: **reuse JOB-01** (Decision 4 = "Yes", existing job) — mirrors the `aggregation-queries`/
  `batch-get-documents` "make it real" pattern exactly (§ Reading Confirmation): a
  documented-but-unbuilt gap in Alex's own "behave identically to real Firestore" job, not a new
  job. A NOTE is appended to `jobs.yaml`'s JOB-01 entry (§ SSOT Updates).
- Walking Skeleton: **Yes** (Decision 2) — the smallest slice that unblocks the existing, otherwise
  permanently-stuck `FAILED_PRECONDITION` gate: `CreateIndex` alone, proven against a real
  previously-stuck `RunQuery`.
- UX Research Depth: **Lightweight** (Decision 3) — a narrow, 3-verb CRUD admin surface, one
  persona (Alex, fully profiled across 8+ prior features), no new emotional arc.

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: P1 Alex (SDK Developer), unchanged.

**Job**: JOB-01 `sdk-compat`, unchanged job_story. This feature's own realization: Alex's query
needs a composite index (a filter field + a different `orderBy` field) — `RunQuery` already,
correctly, rejects it with `FAILED_PRECONDITION` (real-Firestore-accurate behavior, built in the
original walking skeleton). But today there is no way for Alex to EVER satisfy that rejection — the
gate is permanently closed. After this feature, Alex calls a real admin endpoint to create the
index and the identical query succeeds immediately.

## Wave: DISCUSS / [REF] Job Discovery — Framing Resolution

### Resolution 1 — Reuse JOB-01, or a new job?

Mirrors the EXACT precedent `aggregation-queries` and `batch-get-documents` already established
(§ Reading Confirmation): a capability that closes a documented-but-unbuilt gap in Alex's own
"behave identically to real Firestore" job is a realization of JOB-01, not a new job — the gap here
(a real, correctly-wired `FAILED_PRECONDITION` gate with no way to satisfy it) is structurally the
same shape as those two features' own gaps (a proto/SDK surface that existed in spec but was never
wired to real logic).

**Resolution**: **JOB-01 reuse, locked.**

### Resolution 2 — Synchronous `ready`, or simulate real Firestore's own async `CREATING` window?

Real Firestore's own `CreateIndex` is asynchronous — an index starts `CREATING` and transitions to
`READY` after real build time (minutes, proportional to existing document count). This backend has
no analogous secondary-index-build process to simulate (§ Resolution 3 below: this feature does
not provision an actual Postgres index) — there is no REAL latency to model, only a real Firestore
UX behavior to optionally imitate.

| Option | Description | Fit against evidence |
|---|---|---|
| **(A) Simulate the async window** — `CreateIndex` returns `status: "creating"`, a background process (or a fixed delay) later flips it to `ready` | Real Firestore parity | **Rejected** — zero domain evidence any customer's own test suite or workflow depends on transiently observing a `CREATING` state; building a fake background transition for a state with no real underlying work is complexity with no evidenced payoff |
| **(B) Synchronous `ready`** — `CreateIndex` inserts the row with `status = 'ready'` (the column's own existing DEFAULT) and returns immediately | Matches the `composite_indexes` table's own already-built DEFAULT (§ Reading Confirmation — this table was built anticipating exactly this v1 shape); Alex's query succeeds on the very next call, the fastest possible path to unblocking him | **Strongest fit** |

**Resolution**: **(B) is locked.** Named, not silently simplified: real Firestore's own async
`CREATING` window is NOT reproduced in v1 — a future feature, if ever evidenced (e.g. a customer
explicitly testing FOR the pending state), would need its own DESIGN.

### Resolution 3 — Does `CreateIndex` provision a real Postgres index (for query performance), or is it metadata-only?

`IndexManager::is_index_ready` and `requires_composite_index` are PURE GATING logic — real query
EXECUTION already goes through `adapter.run_query(...)`, which runs correctly on Postgres via its
own query planner regardless of whether a matching secondary B-tree index physically exists (SQL
correctness never depended on a Firestore-style index in the first place; only PERFORMANCE would).

**Resolution**: **metadata-only, locked.** `CreateIndex` inserts a `composite_indexes` row; it does
NOT issue a `CREATE INDEX` DDL statement against the underlying Postgres `documents` table. Real
index provisioning for query performance is a separate, unevidenced future concern (no domain
example or metric in this codebase currently shows composite-query latency as a problem) — named,
deferred, out of this feature's own scope.

### Resolution 4 — Does `DeleteIndex` check whether a live rule/query still depends on it?

Real Firestore allows deleting an index even if a live query would subsequently fail — the
customer's own responsibility to manage. No domain evidence suggests this backend should behave
differently, and building a dependency-safety check here would be new complexity this codebase's
existing admin surfaces (e.g. `write_access_rules` redefinition, which can equally "break" a
previously-passing write) do not build either.

**Resolution**: **no dependency check, locked** — matches this codebase's own already-established
"admin mutations don't second-guess the customer" precedent elsewhere.

### Resolution 5 — Does `CreateIndex` on a duplicate `(collection_path, fields)` spec need its own explicit handling?

The `composite_indexes` table already carries `UNIQUE (project_id, collection_path, fields)`. An
unhandled duplicate insert would surface as a raw Postgres constraint-violation error leaking
through to the HTTP response — inconsistent with every other admin handler in this codebase, which
translate DB-level conflicts into a clean, named HTTP response.

**Resolution**: **`CreateIndex` on an exact-duplicate spec is idempotent — returns the EXISTING
row's own data with 200, never a 409 or a raw DB error.** Locked: mirrors real Firestore's own
actual behavior (creating an index identical to one that already exists is a no-op success, not an
error) more closely than inventing a new conflict-response shape this codebase has no other
precedent for.

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

**Signals checked**: >10 user stories? No (3). >3 bounded contexts/modules? No — one file
(`admin/handlers/index_manager.rs`, new), one router file edit, zero new crates, zero new
dependency edge (the `composite_indexes` table and `IndexManager` already exist and are already
wired). Walking skeleton >5 integration points? No (1: real `CreateIndex` unblocking a real,
previously-stuck `RunQuery`). Estimated effort >2 weeks? No — 3 slices, each ≤0.5 day (this is the
narrowest CEL/admin feature scoped this session, smaller even than 4c/4d/4e's own smallest slices).
Multiple independent user outcomes? No — create/list/delete are three faces of one coherent
"manage the indexes my queries need" capability, never independently shippable (list/delete are
meaningless without create having shipped first).

**Scope Assessment: PASS** (0 signals fired) — right-sized as one feature.

## Wave: DISCUSS / [REF] Journey — Alex's "My Query Finally Works" Arc

### Mental model

Alex writes a query that filters on one field and orders by another. `RunQuery` correctly rejects
it — exactly like real Firestore would — but Alex has no way to fix it (no equivalent of real
Firestore's own "click here to create the index" console link). After this feature: Alex calls
`POST /admin/v1/projects/:project_id/indexes` with the same field/order shape the rejection would
have needed, then re-runs the identical query — it succeeds immediately.

### Failure modes (feeds DISTILL scenario generation)

- Alex creates an index for the WRONG field/order combination: the original query still fails with
  the SAME `FAILED_PRECONDITION` (unchanged behavior) — `is_index_ready`'s own per-collection
  check does not distinguish field sets today (§ Reading Confirmation), so ANY ready index for that
  collection unblocks ANY query needing one for that collection — a real, PRE-EXISTING behavior
  this feature does not change or worsen (named explicitly, not silently inherited).
- Alex creates the SAME index spec twice: the second call is a clean idempotent success (Resolution
  5), never a raw DB error.
- Alex deletes an index a live query still needs: the NEXT query attempt fails again with
  `FAILED_PRECONDITION` (Resolution 4) — no special warning, matching this codebase's existing
  admin-mutation precedent.

## Wave: DISCUSS / [REF] Story Map & Walking Skeleton

### Backbone

Alex's query fails with `FAILED_PRECONDITION` → Alex calls `CreateIndex` with the field/order shape
his query needs → the SAME query, re-run, succeeds → Alex lists his project's own indexes to see
what's provisioned → Alex deletes one he no longer needs.

### Walking Skeleton

**Slice 01**: `CreateIndex` alone — a real, previously-permanently-stuck `RunQuery` succeeds after
one real `CreateIndex` call. The single riskiest, highest-value assumption: does inserting a
`composite_indexes` row through a NEW handler actually unblock the ALREADY-EXISTING, unmodified
`is_index_ready` gate, end-to-end, with zero changes to `handler.rs`'s own query path.

## Wave: DISCUSS / [REF] Elephant Carpaccio Slices

| # | Story | Release | Estimate | Learning Hypothesis | Reference Class |
|---|---|---|---|---|---|
| 01 | US-01 | 1 (Walking Skeleton) | 0.5 day | Disproves: a new admin `CreateIndex` handler cannot correctly satisfy the EXISTING, unmodified `is_index_ready` gate without also touching `handler.rs`'s own `RunQuery` path | Mirrors `create_service_account`'s own "insert a row, existing gate reads it" precedent (`admin/router.rs`) |
| 02 | US-02 | 1 | 0.5 day | Disproves: listing a project's own indexes needs anything beyond a straightforward filtered `SELECT` — confirmatory | Mirrors `list_service_accounts`'s own identical shape |
| 03 | US-03 | 1 | 0.5 day | Disproves: deleting an index needs a dependency-safety check beyond a straightforward `DELETE ... WHERE id = $1 AND project_id = $2` — confirmatory (Resolution 4 already locked "no check") | Mirrors `delete_service_account`'s own identical shape |

## Wave: DISCUSS / [REF] Prioritization

Ordered by learning leverage AND genuine dependency: Slice 01 first (proves the CORE assumption —
the only genuinely uncertain question this feature asks, and the one that actually unblocks Alex);
Slices 02–03 are low-uncertainty, straightforward CRUD confirmations mirroring an already
-proven-many-times-over admin-handler shape in this codebase, sequenced last because listing/
deleting are meaningless before creation ships.

## Wave: DISCUSS / [REF] System Constraints

- Zero change to `crates/embyr-server/src/grpc/handler.rs`'s own `RunQuery`/`requires_composite_
  index`/`is_index_ready` call sites — this feature is purely additive (a new admin surface writing
  to a table the query path already reads unchanged).
- `IndexManager` gains no new method for this feature's own Create/List/Delete handlers — they
  operate on the system DB pool directly (mirrors `access_rules.rs`'s own handlers, which never
  route through a dedicated manager type for their own CRUD either; `IndexManager` exists
  specifically as the query-path's own READ-side helper, a distinct concern).
- Mutation-testing lesson, reapplied from this session's own accumulated CEL-parity QUALITY_GATE
  history (4c/4d/4e): any new PURE function this feature adds (if any — this feature is mostly I/O
  -shaped CRUD, likely with little pure logic to mutate) still gets unit tests written DURING
  DELIVER, and a `cargo-mutants` pass is still budgeted at QUALITY_GATE regardless.

## Wave: DISCUSS / [REF] User Stories

### US-01: Alex Creates a Composite Index and His Query Finally Succeeds (Walking Skeleton)

**job_id**: JOB-01 | **Release**: 1 | **Persona**: P1 Alex

#### Elevator Pitch
Before: `RunQuery` correctly rejects a filter+orderBy query needing a composite index with
`FAILED_PRECONDITION`, but no admin endpoint exists to ever satisfy that gate.
After: run `POST /admin/v1/projects/:project_id/indexes` with `{"collection_path": "products",
"fields": [{"field": "category", "order": "ASC"}, {"field": "score", "order": "DESC"}]}` → sees a
200 with the created index's own `id`/`status: "ready"` — then re-running the identical, previously
-rejected `RunQuery` succeeds.
Decision enabled: Alex unblocks a real query his SDK code needs, without any operator intervention.

#### Acceptance Criteria
- [ ] AC-CIX-01: `POST /admin/v1/projects/:project_id/indexes` with a valid `{collection_path,
      fields}` body inserts a `composite_indexes` row with `status = 'ready'` and returns 200 with
      the created index (`id`, `collection_path`, `fields`, `status`, `created_at`).
- [ ] AC-CIX-02: a real `RunQuery` that previously failed `FAILED_PRECONDITION` for the SAME
      collection succeeds immediately after `CreateIndex`, with zero change to any query-path code.
- [ ] AC-CIX-03: `CreateIndex` called twice with the IDENTICAL `(collection_path, fields)` spec is
      idempotent — the second call returns 200 with the SAME existing row, never a 409 or raw DB
      error (Resolution 5).
- [ ] AC-CIX-04: only Owner/Admin roles may call `CreateIndex` — mirrors `define_access_rule`'s own
      in-handler role gate exactly.

### US-02: Alex Lists His Project's Own Composite Indexes

**job_id**: JOB-01 | **Release**: 1 | **Persona**: P1 Alex

#### Elevator Pitch
Before: no way to see which composite indexes exist for a project.
After: run `GET /admin/v1/projects/:project_id/indexes` → sees every index currently defined for
that project (id, collection_path, fields, status, created_at).
Decision enabled: Alex audits what's provisioned before deciding whether he needs to create or
delete anything.

#### Acceptance Criteria
- [ ] AC-CIX-05: `GET /admin/v1/projects/:project_id/indexes` returns every `composite_indexes` row
      for that project (any role, read-only — mirrors `list_service_accounts`'s own any-role gate).
- [ ] AC-CIX-06: a project with zero indexes returns an empty list, never an error.

### US-03: Alex Deletes a Composite Index He No Longer Needs

**job_id**: JOB-01 | **Release**: 1 | **Persona**: P1 Alex

#### Elevator Pitch
Before: no way to remove a composite index once created.
After: run `DELETE /admin/v1/projects/:project_id/indexes/:index_id` → sees a 200/204 — the index
is gone from subsequent `ListIndexes` calls, and a query newly requiring it (with no other ready
index for that collection) fails `FAILED_PRECONDITION` again.
Decision enabled: Alex cleans up an index he created by mistake or no longer needs.

#### Acceptance Criteria
- [ ] AC-CIX-07: `DELETE /admin/v1/projects/:project_id/indexes/:index_id` removes the row; a
      subsequent `ListIndexes` no longer includes it.
- [ ] AC-CIX-08: deleting an index does NOT check whether a live query still depends on it
      (Resolution 4) — the next such query simply fails `FAILED_PRECONDITION` again, same as if the
      index had never been created.
- [ ] AC-CIX-09: only Owner/Admin roles may call `DeleteIndex`.

## Wave: DISCUSS / [REF] Outcome KPIs

### Feature: firestore-composite-indexes-admin-api

### Objective
Close the last mile of the composite-index gating mechanism the original walking skeleton built —
give Alex a real way to satisfy the `FAILED_PRECONDITION` gate `RunQuery` already, correctly,
enforces.

### Outcome KPIs
| KPI | Target | Measurement |
|---|---|---|
| Previously-permanently-stuck queries newly unblockable | 100% (any query needing a composite index can now get one) | Direct: AC-CIX-02's own end-to-end proof |
| Regression | 0 | Full regression suite re-run clean after every slice |
| Production code touched outside this feature's own new handler file | 0 lines (query-path `handler.rs` untouched) | Confirmed directly, mirrors 4e's own "confirmatory slice" discipline |

## Wave: DISCUSS / [REF] Out of Scope

- **Real Firestore's own async `CREATING` window / build-latency simulation** — zero domain
  evidence (Resolution 2). Named, deferred.
- **Actual Postgres index provisioning for query performance** — this feature is metadata-only
  (Resolution 3); no domain example currently shows composite-query latency as a problem. Named,
  deferred, no candidate feature id assigned.
- **Widening `requires_composite_index`'s own simplistic heuristic** (single orderBy-vs-filter
  -field check, vs real Firestore's actual multi-field/array-contains/exemption rules) — explicitly
  out of THIS feature's own scope per the user's own framing at kickoff; a separate, evidenced
  follow-up.
- **`UpdateIndex`** — real Firestore itself has no such verb (index specs are immutable once
  created; you delete and recreate) — not a v1 gap, matches real Firestore's own permanent design.
- **Single-field index exemptions API** (real Firestore's own separate mechanism for opting a
  single field OUT of its default automatic indexing) — zero domain evidence; this backend's own
  query execution never depended on single-field indexing being explicitly managed in the first
  place (Postgres executes ad-hoc regardless).
- **A dependency-safety check on `DeleteIndex`** — zero domain evidence (Resolution 4).
- **Per-field-set granularity in `is_index_ready`** (today: ANY ready index for a collection
  unblocks ANY query needing one for that collection) — a PRE-EXISTING behavior this feature
  neither builds nor changes (§ Journey Failure Modes); named explicitly so it is not mistaken for
  a regression this feature introduced.

## Wave: DISCUSS / [REF] WS Strategy

**Strategy A** (real, minimal, end-to-end) — Slice 01 is a real `CreateIndex` call unblocking a
real, previously-`FAILED_PRECONDITION` `RunQuery`, not a mock.

## Wave: DISCUSS / [REF] Driving Ports

Admin HTTP `:9090` — 2 NEW routes: `POST/GET /admin/v1/projects/:project_id/indexes`,
`DELETE /admin/v1/projects/:project_id/indexes/:index_id`. No new gRPC route, no change to any
existing route.

## Wave: DISCUSS / [REF] Pre-requisites

- The original walking-skeleton query feature (`docs/evolution/2026-05-27-embyr-rs.md`, slice
  04-02) — provides `composite_indexes`, `IndexManager`, `requires_composite_index`, all already
  built and already wired, unmodified by this feature.
- No new external dependency, no new bounded context, no new dependency edge — the smallest
  architectural footprint of any admin feature built this session.

## Wave: DISCUSS / [REF] Handoff Package

Handed to `nw-solution-architect` (DESIGN): this feature-delta.md, all 5 Resolutions (especially
Resolution 2's own "synchronous ready, no async CREATING simulation" lock and Resolution 5's own
idempotent-duplicate-create decision), and the explicit instruction to design the exact
`CreateIndexBody`/`IndexResponse` JSON shapes reusing the `fields` JSONB convention already
confirmed in `tests/acceptance/us_04_query_collection.rs`, not inventing a new one.

## Wave: DISCUSS / [REF] SSOT Updates

`docs/product/jobs.yaml`, JOB-01 entry: append a new dated NOTE (mirroring `aggregation-queries`'/
`batch-get-documents`' own identical NOTE-append convention) — "JOB-01 now also covers a
composite-index admin CRUD surface (Create/List/Delete), closing a documented-but-unbuilt gap:
the `composite_indexes` table + `IndexManager`/`requires_composite_index` gating have existed,
correctly wired, since the original walking skeleton (04-02) — with no way for Alex to ever satisfy
the resulting `FAILED_PRECONDITION` until this feature. Same job, same persona, not a new job —
mirrors the aggregation-queries/batch-get-documents 'make it real' pattern exactly."

## Wave: DISCUSS / [REF] Definition of Ready Validation

### Requirements Completeness Score: **0.97**

### DoR Checklist (9-item hard gate)
1. [x] Every story traces to a job_id (JOB-01, all 3 stories)
2. [x] Every story has a complete Elevator Pitch (Before/After/Decision enabled)
3. [x] Every AC is testable without ambiguity
4. [x] Walking Skeleton identified (US-01)
5. [x] Scope Assessment passed
6. [x] No slice contains only `@infrastructure` stories (every slice has a direct Alex-facing
   value story)
7. [x] Out of Scope explicitly named (7 items)
8. [x] Outcome KPIs have numeric targets and measurement methods
9. [x] Prior-wave artifacts read and reconciled (JOB-01's own existing dimensions/journey are
   confirmed, not contradicted, by this feature's own scope)

### DoR Status: **PASSED**

## Wave: DISCUSS / [REF] Open Questions

None carried forward unresolved — every open question this DISCUSS raised (job reuse, sync-vs
-async status, metadata-only-vs-real-provisioning, delete-dependency-checking, duplicate-create
handling) was independently resolved with a locked Resolution above.

## Wave: DISCUSS / [REF] Wave Decisions Summary

### Key Decisions
- [D1] Reuse JOB-01 (Resolution 1) — mirrors the `aggregation-queries`/`batch-get-documents`
  "make it real" pattern exactly.
- [D2] Synchronous `status: 'ready'` on create, no async `CREATING` simulation (Resolution 2) —
  matches the `composite_indexes` table's own already-built DEFAULT.
- [D3] Metadata-only — `CreateIndex` never issues a real Postgres `CREATE INDEX` DDL (Resolution 3).
- [D4] No dependency-safety check on `DeleteIndex` (Resolution 4) — matches this codebase's own
  existing admin-mutation precedent.
- [D5] `CreateIndex` on an exact duplicate spec is idempotent, 200 with the existing row, never a
  409 (Resolution 5).

### Requirements Summary
- Primary need: the composite-index gating mechanism built in the original walking skeleton has
  no way to ever be satisfied — Alex's own filter+orderBy queries are permanently stuck.
- Walking skeleton scope: `CreateIndex` alone, unblocking a real, previously-stuck `RunQuery`.
- Feature type: Backend.

### Constraints Established
- Zero change to `handler.rs`'s own `RunQuery`/gating call sites.
- `fields` JSON shape reuses the EXACT convention already established in
  `tests/acceptance/us_04_query_collection.rs`, not a newly-invented one.
- Metadata-only — no real Postgres index provisioning, no async status transition.

### Upstream Changes
- None — this feature closes a documented-but-unbuilt gap in JOB-01's own already-established
  scope; no DISCOVER/DIVERGE assumption from any prior feature is contradicted.

## Wave: DISCUSS / [REF] Next Wave

**Handoff To**: nw-solution-architect (DESIGN wave)
**Deliverables**: this feature-delta.md, 5 locked Resolutions, 3-slice/1-release plan

---

## Wave: DESIGN / [REF] Prior Wave Consultation — Reading Confirmation

✓ This feature-delta.md's own DISCUSS sections in full, all 5 Resolutions.
✓ `crates/embyr-server/src/admin/handlers/access_rules.rs::define_access_rule` (lines 615-660) —
confirmed the EXACT auth/ownership shape this feature's own `create_composite_index`/
`delete_composite_index` reuse: `Path(project_id)`, `State(state): State<UserAdminState>`,
`session: SessionContext`, `if session.role < Role::Admin { return Err(StatusCode::FORBIDDEN) }`,
`verify_project_ownership(pool, &project_id, session.account_id).await?`.
✓ `crates/embyr-server/src/admin/handlers/service_accounts.rs::list_service_accounts`/
`create_service_account`/`delete_service_account` (lines 64-190+) — confirmed the exact SQL CRUD
shape: `list` is a plain filtered `SELECT ... ORDER BY created_at ASC`; `create` is
`INSERT ... RETURNING`; `delete` first `SELECT`s to confirm ownership/existence
(`.ok_or(StatusCode::NOT_FOUND)?`), then deletes. This feature's own handlers reuse this shape,
adapted from account-scoping (`account_id = $1`) to project-scoping (`project_id = $1`) to match
`define_access_rule`'s own scoping instead.
✓ `crates/embyr-server/src/admin/router.rs` (lines 240-354) — confirmed exact route-chaining
syntax: `.route(path, get(x).post(y))` for one path carrying 2 verbs, `.route(other_path,
delete(z))` for a distinct sibling path carrying the id-scoped verb.

## Wave: DESIGN / [REF] Reuse Analysis

| Existing mechanism | Reused unchanged for this feature? |
|---|---|
| `SessionContext` + `Role::Admin` gate + `verify_project_ownership` | Yes — identical to `define_access_rule`'s own shape |
| `UserAdminState` (carries `system_db.pool()`) | Yes — no new state field, no new adapter type |
| `composite_indexes` table (migration 0003) | Yes, unchanged schema — this feature is the FIRST production code to ever write to it |
| `fields` JSONB `[{"field", "order"}]` shape | Yes — reused verbatim from `tests/acceptance/us_04_query_collection.rs`'s own already-established convention, never re-invented |
| `IndexManager::is_index_ready` / `requires_composite_index` (query path) | Untouched — this feature is purely upstream of them (it populates the table they already read) |
| Router `get(x).post(y)` / `delete(z)` chaining convention | Yes — identical to `service_accounts`' own registration shape |

**Nothing in this feature requires a new bounded context, a new port/adapter trait method, a new
`IndexManager` method, or any change to `crates/embyr-server/src/grpc/handler.rs`** — confirmed by
direct code read before locking ADR-068, not assumed from the epic's own framing.

## Wave: DESIGN / [REF] Architecture Design

See ADR-068 (`docs/product/architecture/adr-068-composite-index-admin-crud.md`) for the full
request/response shape design. Summary:

1. **New file**: `crates/embyr-server/src/admin/handlers/composite_indexes.rs` — 3 handlers
   (`create_composite_index`, `list_composite_indexes`, `delete_composite_index`), plus
   `CreateCompositeIndexBody`/`IndexFieldSpec`/`CompositeIndexResponse` request/response types.
2. **Routes** (`admin/router.rs`): `.route("/admin/v1/projects/:project_id/indexes",
   get(list_composite_indexes).post(create_composite_index))`,
   `.route("/admin/v1/projects/:project_id/indexes/:index_id",
   delete(delete_composite_index))`.
3. **`fields` shape**: `Vec<IndexFieldSpec { field: String, order: IndexFieldOrder }>` where
   `IndexFieldOrder` is a 2-variant enum (`Asc`/`Desc`) serializing to `"ASC"`/`"DESC"` — matching
   `us_04_query_collection.rs`'s own fixture text exactly (§ Reading Confirmation), stored as JSONB
   via `serde_json::to_value`.
4. **Idempotent create** (Resolution 5): `INSERT ... ON CONFLICT (project_id, collection_path,
   fields) DO UPDATE SET collection_path = EXCLUDED.collection_path RETURNING ...` — a no-op
   self-update on conflict (mirrors `upsert_access_rule`'s own `ON CONFLICT ... DO UPDATE` shape,
   ADR-028's own precedent) rather than a `SELECT`-then-branch, avoiding a race between the
   existence check and the insert.
5. **Delete** (Resolution 4, no dependency check): `DELETE FROM composite_indexes WHERE id = $1
   AND project_id = $2` directly — no existence pre-check needed beyond confirming the row
   actually existed (0 rows affected → 404), mirroring `delete_service_account`'s own ownership
   -check shape adapted to project-scoping.

## Wave: DESIGN / [REF] Wave Decisions Summary

### Key Decisions
- [D1] `fields` is a typed `Vec<IndexFieldSpec>` (not a raw `serde_json::Value` passthrough) —
  validates `order` is exactly `"ASC"`/`"DESC"` at the request-deserialization boundary (a
  malformed body is a clean 422, never a value silently stored that the query path would later
  fail to compare against real Firestore's own `Direction` enum shape).
- [D2] Idempotent create via `ON CONFLICT ... DO UPDATE`, not `SELECT`-then-branch — avoids a
  TOCTOU race between two concurrent identical `CreateIndex` calls, and directly mirrors this
  codebase's own `upsert_access_rule` precedent rather than inventing a new conflict-handling shape.
- [D3] No new `IndexManager` method — this feature's own 3 handlers query `composite_indexes`
  directly via `state.system_db.pool()`, exactly like `access_rules.rs`'s own handlers never route
  their own CRUD through a dedicated manager type either; `IndexManager` remains scoped to its own
  existing, narrow read-side concern (the query path's own gate).

### Constraints Established
- No new dependency, no new bounded context, no new port/adapter trait method.
- Zero lines touched in `crates/embyr-server/src/grpc/handler.rs` — confirmed by construction, this
  feature is purely upstream of the query path's own existing, unmodified gate.
- `collection_path` + `fields` together form the natural request key; `id` (a `Uuid`, matching
  `service_accounts.id`'s own `::text` cast convention) is the delete-target key, never
  `collection_path` + `fields` again for delete (avoids re-serializing a JSONB value into a URL).

## Wave: DESIGN / [REF] Next Wave

**Handoff To**: nw-software-crafter (DELIVER wave, per this project's own established convention —
DISTILL folds into per-slice TDD, not a separate artifact)
**Deliverables**: this feature-delta.md's DESIGN section, ADR-068
