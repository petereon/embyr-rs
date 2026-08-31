# agent-mode-list-collection-ids — Feature Delta

**Wave**: DISCUSS | **Agent**: Luna (nw-product-owner) | **Date**: 2026-08-31
**Status**: Ready for DESIGN handoff — one escalated open question (cross-version graceful-degradation UX, shared framing with sibling features). See § Handoff Package.
**Upstream**: No DISCOVER/DIVERGE wave ran — commissioned directly by the orchestrator as one of 4 confirmed `backend_mode=agent` parity gaps. `firestore-list-rpcs`'s own DISCUSS (2026-08-31) scoped `ListCollectionIds` to non-agent modes only and confirmed the agent proto has no such RPC (§ Reading Confirmation below) — named as a candidate follow-up feature, not silently dropped.

---

## Wave: DISCUSS / [REF] Prior Wave Consultation — Reading Confirmation

✓ `proto/embyr/agent/v1/storage_agent.proto` (full, 372 lines) — confirmed no `ListCollectionIds` RPC or message exists. The 15 RPCs present are document CRUD, `RunQuery`/`RunAggregationQuery`, transaction lifecycle, `Ping`, `ListDocuments`, and `Subscribe` — no distinct-child-collection enumeration primitive of any kind.
✓ `crates/embyr-agent/src/server.rs` (full, 860 lines) — confirmed `list_documents()`'s own `build_collection_path` fix (this session, `firestore-list-rpcs` DESIGN wave, lines 522-548) is the closest structural precedent: parent-path parsing + pagination (`encode_page_token`/`decode_page_token`) already correctly generalized. `ListCollectionIds` needs a genuinely NEW query primitive — "distinct child collection IDs under a parent" — which is NOT a variant of `run_query`'s existing per-document filtering; it requires either a dedicated SQL query shape or a new `PostgresBackendAdapter` method.
✓ `docs/feature/firestore-list-rpcs/feature-delta.md` (targeted grep, § Reading Confirmation, § Elephant Carpaccio Slices, § Escalation 1) — confirmed that feature shipped `ListCollectionIds` for `direct_pg`/`aws_secret`/`gcp_secret` only, and its own Escalation 1 was about `ListDocuments`'s agent-mode ROUTING (a different, already-solved question — reuse `run_query`) — NOT about `ListCollectionIds`, which was scoped out of that feature entirely for agent mode from the start, with no partial work landed.
✓ `crates/embyr-server/src/adapters/agent_backend.rs` (full, 627 lines) — confirmed `AgentBackendAdapter` has no method resembling collection-ID enumeration; `run_query` is the only query-shaped call it makes today.
✓ `docs/SPEC.md` § Agent gRPC Protocol (lines 493-503) — confirmed the documented (not implemented) protocol summary explicitly lists **"Collection ID listing"** as one of the mirrored `StorageAdapter` capabilities the agent is intended to expose — this gap was always part of the documented design intent, just never implemented, unlike `agent-mode-write-streaming` (absent even from the aspirational description).
✓ `docs/product/jobs.yaml` lines 14-26 (JOB-01) — confirmed same motivating job as sibling features: "all SDK calls succeed unchanged," now including `listCollections()` for agent-mode customers.

No contradictions found between this feature's scope and prior evidence.

---

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

| # | Decision | Value |
|---|---|---|
| 1 | Feature Type | Backend — closes an SDK-facing RPC-level parity gap (`ListCollectionIds`) for `backend_mode=agent`, mirroring `firestore-list-rpcs`'s own US-02 for the other three backend modes |
| 2 | Walking Skeleton | YES — one story IS the walking skeleton (single-project, single-parent enumeration, happy path first scenario) |
| 3 | UX Research Depth | Lightweight, SDK-facing — no TUI; the experience is the Firebase SDK's `docRef.listCollections()` call working transparently against an agent-mode project |
| 4 | JTBD Analysis | Confirmed: extends JOB-01 (sdk-compat), same persona P1 Alex, same "make it real" pattern |

---

## Wave: DISCUSS / [REF] Pre-requisites

- `firestore-list-rpcs` (shipped, non-agent modes) — the `page_token` pagination shape (hex-encoded offset) and the distinct-child-collection query semantics this feature mirrors on the agent side.
- `crates/embyr-agent/src/server.rs::build_collection_path`/`encode_page_token`/`decode_page_token` (shipped, this binary) — directly reusable pagination helpers, no new pagination scheme needed.
- No dependency on the other 3 sibling `backend_mode=agent` features (investigated and confirmed independent, § agent-mode-write-streaming's own Bundle-vs-Split Note, recorded once there).

---

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: **P1 — Alex (SDK Developer)**, Meridian Health, `backend_mode=agent` — same persona and company as `agent-mode-write-streaming`, for continuity within this session's own 4-feature split.

**job_id decision**: `JOB-01` (`sdk-compat`), extended not new.

---

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

| Signal | Threshold | This feature | Fired? |
|---|---|---|---|
| User stories | >10 | 1 (US-01) | **NO** |
| Bounded contexts / modules | >3 | 1 — BC-2 Document Storage, query surface | **NO** |
| Walking Skeleton integration points | >5 | 3 — (1) new unary RPC + messages on `storage_agent.proto`, (2) new `embyr-agent` handler reusing `build_collection_path`/pagination helpers, (3) new `AgentBackendAdapter` method | **NO** |
| Estimated effort | >2 weeks | ~1.5 days, single slice | **NO** |
| Independent shippable outcomes | multiple unrelated | **NO** — one coherent outcome | **NO** |

**0 of 5 signals fired. Verdict: PASS — single feature, single story, right-sized.**

---

## Wave: DISCUSS / [REF] Journey (Lightweight, SDK-facing, per Decision 3)

**Alex's mental model**: Alex's app calls `docRef.listCollections()` against every other Meridian Health project (`direct_pg` staging) to discover subcollections dynamically (e.g., enumerating a patient document's `visits`, `labResults`, `medications` subcollections without hardcoding the list). Against `meridian-health-prod` (agent-mode), the identical call fails outright.

**Emotional arc** (Confidence Building): **Start** — mild friction; a working staging feature breaks specifically in production. **Middle** — Alex calls `listCollections()`, gets back the correct set of child collection IDs, paginated correctly for a document with many subcollections. **End** — confident; schema-discovery code works uniformly across environments.

**Shared artifact**: the `page_token` hex-offset scheme (source: `crates/embyr-agent/src/server.rs::encode_page_token`/`decode_page_token`, already shipped) — reused unchanged, not reinvented.

**Failure modes**: a parent document has zero subcollections (empty result, not an error) | a parent path is malformed | pagination spans more collections than fit in one page | a sibling subcollection under a DIFFERENT parent is incorrectly included (child-scoping bug).

---

## Wave: DISCUSS / [REF] Story Map & Walking Skeleton

**Persona**: P1 Alex | **Goal**: `docRef.listCollections()` works against `backend_mode=agent` projects exactly as it already does against the other three backend modes.

### Backbone

| A. Enumerate Child Collections | B. Paginate Results |
|---|---|
| Return distinct child collection IDs under a parent document **[WS]** | Correctly page through results for a parent with many subcollections **[WS]** |
| Never include a grandchild or sibling-parent collection **[WS]** | — |

### Walking Skeleton

Call the new RPC against `meridian-health-prod` for `patients/p_204` → returns `["visits", "labResults", "medications"]` (the document's real subcollections, no grandchildren, no siblings). This is Slice 01, US-01 — the entire feature.

---

## Wave: DISCUSS / [REF] WS Strategy

**Strategy: B (Real, Narrow Slice)** — real `embyr-agent` binary, real Postgres, one representative parent document with multiple subcollections. Mirrors `firestore-list-rpcs`'s own WS Strategy for the non-agent version of this RPC.

---

## Wave: DISCUSS / [REF] Elephant Carpaccio Slices

| Slice | Story | Release | Estimate | Learning Hypothesis (disproves) | Production-data taste test |
|---|---|---|---|---|---|
| 01 (WS) | US-01 | 1 | 1.5 days | The "distinct child collection IDs" query cannot be expressed against this codebase's existing document-storage schema without a new query primitive incompatible with the current `PostgresBackendAdapter` shape | Real Postgres, a real parent document with 3+ real subcollections and at least one sibling-parent document with its own distinct subcollection (to prove no cross-parent leakage), asserting the correct set and no extras |

**Total estimate: 1.5 days, single slice.**

**Taste tests applied**: 4+ components — 3 components (proto RPC, agent handler, adapter method), reusing 2 already-shipped helpers (pagination, `build_collection_path`) — PASS. New-abstraction-first — the one new abstraction (distinct-child-collection query) ships in this single slice, nothing deferred — PASS. Falsifiable hypothesis — PASS. Production data only — PASS. No identical-except-scale slices — N/A, single slice.

---

## Wave: DISCUSS / [REF] Prioritization

| Priority | Slice | Target Outcome | Rationale |
|---|---|---|---|
| 1 | Slice 01 (WS) | `listCollections()` works against agent-mode projects | Single-slice feature; no sequencing decision needed |

---

## Wave: DISCUSS / [REF] System Constraints

- **The distinct-child-collection query is a genuinely new primitive**, not a `run_query` variant — DESIGN must decide whether it lives as a new `PostgresBackendAdapter` method (shared with the non-agent `ListCollectionIds` implementation, if that implementation's own query shape is reusable) or as agent-binary-local logic.
- **Reuse `build_collection_path`/`encode_page_token`/`decode_page_token` unchanged** — no new pagination scheme.
- **Cross-version graceful degradation** — same open question as `agent-mode-write-streaming`'s own Escalation 2; referenced here, not re-litigated (§ Handoff Package).

---

## Wave: DISCUSS / [REF] Driving Ports

| Port | Trigger | Notes |
|---|---|---|
| Firebase SDK `docRef.listCollections()` | Client SDK call | Already shipped for non-agent modes (`firestore-list-rpcs`); this feature closes the identical entry point for `backend_mode=agent` |

---

## Wave: DISCUSS / [REF] User Stories

<!-- markdownlint-disable MD024 -->

### US-01: Alex's App Enumerates a Document's Subcollections Against an Agent-Mode Project

**job_id**: JOB-01 (sdk-compat) — extends, "make it real" pattern

#### Problem

Alex's app dynamically discovers a patient document's subcollections (`visits`, `labResults`, `medications`) via `docRef.listCollections()` against Meridian Health's staging project (`direct_pg`). The identical call against `meridian-health-prod` (`backend_mode=agent`) fails — the RPC does not exist on the agent's own gRPC surface.

#### Who

- Alex, SDK Developer at Meridian Health | Same persona and deployment context as `agent-mode-write-streaming` | Needs schema-discovery calls to work uniformly across environments without hardcoding subcollection names per backend mode

#### Solution

A new unary RPC on `storage_agent.proto`, reusing the agent binary's own already-shipped `build_collection_path` and pagination helpers, returns the distinct set of child collection IDs directly under a given parent document.

#### Elevator Pitch

Before: Alex's app calling `db.doc('patients/p_204').listCollections()` against `meridian-health-prod` receives an `Unimplemented` gRPC error.
After: Alex's app calls `db.doc('patients/p_204').listCollections()` → sees `['visits', 'labResults', 'medications']` returned, identical to the same call against a `direct_pg` project.
Decision enabled: Alex decides schema-discovery code can run identically across every Meridian Health environment, including production.

#### Domain Examples

##### 1: Happy Path — Alex enumerates patients/p_204's subcollections

`patients/p_204` has 3 subcollections: `visits`, `labResults`, `medications`. Alex's call returns exactly those 3 IDs.

##### 2: Edge Case — A document with zero subcollections

`patients/p_209` is a newly-created patient record with no subcollections yet. Alex's call returns an empty list, not an error.

##### 3: Boundary — Pagination across many subcollections

`patients/p_204` (in a load-test seed) has 150 subcollections. Alex's first call returns a page of results plus a `next_page_token`; a follow-up call with that token returns the remainder with no overlap or gap.

##### 4: Error/Boundary — Sibling-parent isolation

`patients/p_204` has subcollection `visits`; a DIFFERENT document `patients/p_205` also has its own `visits` subcollection. Alex's call for `p_204` returns `visits` exactly once, never conflated with `p_205`'s own `visits`.

#### UAT Scenarios (BDD)

##### Scenario: Subcollections of a document are listed correctly
```gherkin
Given patients/p_204 in meridian-health-prod has subcollections visits, labResults, and medications
When Alex's app calls listCollections() on patients/p_204
Then the response contains exactly visits, labResults, and medications
```

##### Scenario: A document with no subcollections returns an empty list
```gherkin
Given patients/p_209 in meridian-health-prod has no subcollections
When Alex's app calls listCollections() on patients/p_209
Then the response contains zero collection IDs
And no error is returned
```

##### Scenario: Pagination returns the full set across multiple pages with no overlap
```gherkin
Given patients/p_204 has 150 distinct subcollections
When Alex's app calls listCollections() with a page size of 100
Then the first page returns 100 collection IDs and a next_page_token
And a follow-up call with that token returns the remaining 50 with no duplicates
```

##### Scenario: Subcollections are scoped to the correct parent document
```gherkin
Given patients/p_204 has subcollection visits
And patients/p_205 has its own separate subcollection visits
When Alex's app calls listCollections() on patients/p_204
Then the response includes visits exactly once
And no collection belonging to patients/p_205 appears in the response
```

#### Acceptance Criteria

- [ ] `listCollections()` against a document with subcollections returns exactly the correct, distinct set of child collection IDs
- [ ] `listCollections()` against a document with no subcollections returns an empty list, not an error
- [ ] Pagination via `page_token`/`next_page_token` returns the complete set with no duplicates or gaps across pages
- [ ] Subcollections are correctly scoped to their own parent document — never conflated with a sibling document's identically-named subcollection
- [ ] Exercised against a real `embyr-agent` binary and real Postgres

#### Outcome KPIs

- **Who**: Alex (SDK Developer, agent-mode customers)
- **Does what**: Completes a `listCollections()` call against an agent-mode project
- **By how much**: 0% → 100% of well-formed requests succeed with correctly-scoped, correctly-paginated results
- **Measured by**: Count of successful `listCollections()` calls against the reference test suite
- **Baseline**: 0% — the RPC does not exist on the agent proto today

#### Technical Notes (Optional)

- Reuses `build_collection_path`/`encode_page_token`/`decode_page_token` unchanged.
- The distinct-child-collection query is new; DESIGN decides its exact shape (§ System Constraints).
- Depends on DESIGN resolving whether the underlying query logic is shared with the non-agent `ListCollectionIds` implementation or agent-binary-local.

---

## Wave: DISCUSS / [REF] Outcome KPIs (Feature-Level Summary)

### Feature: agent-mode-list-collection-ids

### Objective

Alex's schema-discovery code (`docRef.listCollections()`) works identically across every embyr backend mode Meridian Health runs, including `backend_mode=agent`.

### Outcome KPIs

| # | Who | Does What | By How Much | Baseline | Measured By | Type |
|---|-----|-----------|-------------|----------|-------------|------|
| 1 | Alex | Completes a `listCollections()` call against an agent-mode project | 0% → 100% of well-formed requests succeed | 0% (RPC does not exist) | Reference test suite | North Star |

### Metric Hierarchy

- **North Star**: KPI #1
- **Leading Indicators**: N/A — single-slice feature
- **Guardrail Metrics**: sibling-parent isolation never regresses (a correctness invariant, not a KPI)

### Measurement Plan

| KPI | Data Source | Collection Method | Frequency | Owner |
|-----|------------|-------------------|-----------|-------|
| Success rate | Reference test suite | Automated assertion | Per CI run | embyr-agent + embyr-server (DELIVER) |

### Hypothesis

We believe adding a new unary `ListCollectionIds`-equivalent RPC to `storage_agent.proto`, reusing already-shipped pagination helpers, will make `listCollections()` work identically against `backend_mode=agent` projects.
We will know this is true when Alex's schema-discovery code succeeds against a real agent-mode deployment with correct scoping and pagination.

---

## Wave: DISCUSS / [REF] Definition of Ready Validation

### Story: US-01

| DoR Item | Status | Evidence/Issue |
|----------|--------|----------------|
| Problem statement clear | PASS | `listCollections()` fails outright against agent-mode, works elsewhere |
| User/persona identified | PASS | Alex, Meridian Health, agent-mode |
| 3+ domain examples | PASS | Happy path, empty-subcollections edge case, pagination boundary, sibling-isolation boundary (4 total) |
| UAT scenarios (3-7) | PASS | 4 scenarios |
| AC derived from UAT | PASS | 5 AC items |
| Right-sized | PASS | 1.5 days, 4 scenarios |
| Technical notes | PASS | Reuse points and open DESIGN question named |
| Dependencies tracked | PASS | No blocking dependency; informational precedent from `firestore-list-rpcs` |
| Outcome KPIs | PASS | Numeric target and measurement method defined |

### DoR Status: PASSED

---

## Wave: DISCUSS / [REF] Handoff Package

### Escalation 1 — Cross-version graceful degradation (shared framing)

Same open question as `agent-mode-write-streaming`'s own Escalation 2 (full detail there, not repeated in full here): `docs/SPEC.md` documents a "major version must match" policy with no runtime enforcement. If `embyr-server` calls this new RPC against an agent binary that predates it, the failure is a clean `Unimplemented` today. DESIGN should resolve this once, ideally reusable across this feature and the other 2 wire-touching sibling features (`agent-mode-write-streaming`, `agent-mode-field-transforms`).

### Handoff Confirmation

Next step (NOT performed by this agent): orchestrator dispatches `nw-solution-architect` for the DESIGN wave — full rigor with ADRs (at minimum: proto message/RPC design; the distinct-child-collection query shape and whether it's shared with the non-agent implementation) and Reuse Analysis.
