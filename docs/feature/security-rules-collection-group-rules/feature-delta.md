# security-rules-collection-group-rules — Feature Delta

**Wave**: DISCUSS | **Agent**: Luna (nw-product-owner) | **Date**: 2026-08-25
**Status**: Ready for DESIGN handoff (one flagged confirmation — see § Handoff Package)
**Upstream**: `security-rules` (FINALIZED, `docs/evolution/2026-08-18-security-rules.md`) → `security-rules-write-path` (DISCUSS+DESIGN complete, DELIVER not confirmed complete, not FINALIZED) → `security-rules-query-path` (DISCUSS+DESIGN complete, DELIVER in progress — `git log`/`git status` confirm only US-01 committed, US-02's acceptance scenario uncommitted — not FINALIZED). This feature is the next epic in the Authorization initiative: `security-rules-query-path` closed the "RunQuery ignores `access_rules` entirely" bypass for ordinary, single-collection queries, but its own rule-lookup composition (ADR-031 § Decision — Composition) never differentiates on `StructuredQuery.all_descendants` — a pre-existing field in the domain query model that this feature's own investigation confirms the DATA PLANE already honors for genuine collection-group queries. This feature closes the resulting gap: a `RunQuery` with `all_descendants = true` is, right now, checked (if checked at all) against the wrong rule — or none — because a bare collection id spanning multiple actual nested collection instances has no rule concept of its own today.

<!-- markdownlint-disable MD024 -->

---

## Wave: DISCUSS / [REF] Density Resolution

`~/.nwave/global-config.json` was not read — no Bash tool available to this agent invocation (per `nw-product-owner`'s tool grant: Read/Write/Edit/Glob/Grep/Task only) — falling back to the documented DISCUSS hard default: `mode=lean`, `expansion_prompt=ask-intelligent`, noted explicitly rather than silently assumed, mirroring all three sibling features' own precedent for this exact unavailability. Trigger check against this feature's own artifacts: "multi-stakeholder need" (Alex, Maria, Dana) fires but is answered by cross-reference per Decision 3 (Lightweight), exactly as `security-rules-write-path`/`security-rules-query-path` did. "Cross-cutting complexity" (≥3 bounded contexts) does **not** fire — direct code evidence below (§ Job Discovery Framing Resolution) shows this feature touches the same 2 bounded contexts `security-rules-query-path` touched (BC-4 extended, BC-2's existing `RunQuery` handler call site), not 3, and requires zero change to `embyr-pg-storage`'s SQL-building layer, which already supports collection-group execution unmodified. No other trigger fires. Tier-1 [REF] only, no Tier-2 expansions rendered.

---

## Wave: DISCUSS / [REF] Prior Wave Consultation — Reading Confirmation

✓ `docs/feature/security-rules-query-path/feature-delta.md` (full, 1185 lines, DISCUSS+DESIGN sections) — the direct structural precedent this DISCUSS mirrors (Reading Confirmation checklist format, Resolution-table methodology, Elephant Carpaccio slicing, single-narrative-file output); confirms `check_query_compliance()`/`QueryComplianceOutcome`/`UnsatisfiedConjunct` (ADR-031) as the exact, unmodified machinery this feature reuses; confirms the shipped `handle_run_query` composition (lines 1216–1269 as read) performs exactly ONE `get_access_rule(&project_id_str, &collection.collection_path)` lookup, where `collection.collection_path` is set to the bare `collection_id` string (line 1218) regardless of `all_descendants` — the precise, now independently-confirmed, gap.
✓ `docs/feature/security-rules-write-path/feature-delta.md` (full, 1140 lines, DISCUSS+DESIGN sections) — confirms write-path's own rule lookups (`handle_create_document`/`handle_update_document`/`handle_delete_document`, `get_write_access_rule`) key off `path.collection_path`, which is derived from the ACTUAL document's full resource name (see `parse_document_path`, confirmed below) — i.e. write-path already resolves the correct, specific nested collection path per call, with no group-query analog and no gap of this feature's kind.
✓ `docs/feature/security-rules/feature-delta.md` (full, 1302 lines, DISCUSS+DESIGN sections) — confirms `handle_get_document`'s rule lookup (`get_access_rule(&project_id, &path.collection_path)`) likewise keys off the document's own fully-resolved `collection_path` (from `parse_document_path`), never a bare leaf id — GetDocument has no group-query analog either, and no gap of this feature's kind.
✓ `docs/product/architecture/adr-027-access-rule-grammar-and-evaluation.md` (full) — confirms the locked `Condition`/`Operand`/`CompareOp`/`AuthContext` grammar and `parse_condition()`, reused unchanged.
✓ `docs/product/architecture/adr-028-access-rule-storage-and-lifecycle.md` (full) — confirms `access_rules` is keyed by exact `(project_id, collection_path)`, ONE row per exact path, "single-segment collection id in v1" as a documented *convention*, not a code-enforced constraint (confirmed by reading `admin/handlers/access_rules.rs`'s `DefineAccessRuleBody`, below — no validation rejects a `/`-containing `collection_path`).
✓ `docs/product/architecture/adr-029-access-control-composition-and-bounded-context.md` (full) — confirms BC-4 Access Control's placement, the identity-reuse pattern, and the "structural no-rule-defined guardrail" mechanism (`get_access_rule() -> None` short-circuits) this feature's own new guardrail (§ System Constraints) deliberately does NOT reuse for the group case, with reasoning below.
✓ `docs/product/architecture/adr-030-write-path-grammar-storage-and-composition.md` (full) — confirms `write_access_rules` as a DISJOINT table from `access_rules`, keyed identically, chosen specifically because "AC-17-43 structurally, not conventionally, true" required it — the direct structural precedent for this feature's own new, disjoint group-rule table (§ Job Discovery Framing Resolution, Resolution 1).
✓ `docs/product/architecture/adr-031-query-shape-compliance-check.md` (full, 696 lines) — confirms the EXACT current shape of `check_query_compliance()`, `decompose_decidable()`, `filter_binds_field_to_uid()`, `QueryComplianceOutcome`, `UnsatisfiedConjunct`, and `handle_run_query`'s exact composition (lines 349–411 as documented) — the machinery this feature extends by adding a NEW rule-source branch, never by modifying the function itself.
✓ `docs/product/jobs.yaml` (full) — JOB-17's own functional-dimension text ("enforce them on every request") and its four accumulated NOTEs (read/write/query realizations) already frame this feature's own scope as JOB-17's next incremental realization, not a new goal.
✓ `docs/product/journeys/sdk-developer.yaml` (full) — JOB-17 already listed for P1 Alex, extended with 4 prior NOTEs; extended below with a 5th, no new job id.
✓ `crates/embyr-core/src/domain/query.rs` (full, 71 lines) — confirms `StructuredQuery { collection_id: String, all_descendants: bool, .. }` — `all_descendants` is a PRE-EXISTING field, not introduced by any of the 3 prior security-rules epics; confirms `QueryFilter` remains AND-only (`Composite(Vec<QueryFilter>)`, "Composite AND filter" doc comment), unchanged, still the structural fact `check_query_compliance` reuses unmodified.
✓ `crates/embyr-core/src/domain/document.rs` (full) — confirms `CollectionPath { project_id, collection_path }`'s own doc comment: "Path to a collection **(or collection group)**" — the domain model already anticipated collection-group semantics as a first-class concept; confirms `DocumentPath { project_id, collection_path, document_id }`, the type `parse_document_path` produces.
✓ `crates/embyr-pg-storage/src/backend_adapter.rs` (targeted read: `run_query`, lines 530–560) — **confirms directly, not assumed**: when `query.all_descendants`, the SQL matches `collection_path = <id> OR collection_path LIKE '%/<id>'` — genuine collection-group execution across every nesting depth, entirely pre-existing, untouched by any of the 3 prior security-rules epics.
✓ `crates/embyr-server/src/grpc/handler.rs` (targeted full reads: `parse_document_path` lines 98–126, `handle_get_document` lines 506–625, `handle_create_document`/`handle_update_document`/`handle_delete_document` write-rule lookups lines 792/903, `handle_run_query` lines 1131–1289) — **confirms directly**: (a) `parse_document_path` derives `collection_path` from the ACTUAL resource name's segments (joined, multi-segment-capable) — this is why GetDocument/writes have no group gap; (b) `handle_run_query` derives `collection_id`/`all_descendants` independently from `sq_proto.from[0]` (lines 1167–1176), threads `all_descendants` into `domain_query` (line 1223) for the DATA-PLANE query, but the RULE lookup at line 1244 uses `collection.collection_path` — set at line 1218 to the bare `collection_id`, with **no branch on `all_descendants` at all** — the exact, now doubly-confirmed gap; (c) `req.parent` (lines 1137–1138) is used ONLY to extract `project_id` — never combined with `collection_id` to scope a non-group query to a specific parent document. This third finding is a **separate, pre-existing, non-security correctness gap** (non-group `RunQuery` calls against an explicit parent-scoped subcollection may resolve to the wrong collection instance) — flagged explicitly in § Open Questions as out of scope for this feature, not silently folded in or silently ignored.
✓ `crates/embyr-server/src/adapters/system_db.rs` (targeted read: `AccessRuleRow`/`WriteAccessRuleRow`, `get_access_rule`/`upsert_access_rule`, `get_write_access_rule`/`upsert_write_access_rule`) — confirms the exact adapter-method shape this feature's new `get_group_access_rule`/`upsert_group_access_rule` mirrors, per ADR-030's own disjoint-table precedent.
✓ `crates/embyr-server/src/admin/handlers/access_rules.rs` (full) — confirms `DefineAccessRuleBody.collection_path: String` carries NO validation rejecting a `/`-containing value (doc comment states "single-segment... in v1" as an unenforced convention, not code) — direct evidence this feature's own group-rule collection-id validation (US-01, AC-17-80) is new work, not already covered.
✓ `docs/evolution/2026-08-18-security-rules.md` (full) — confirms `security-rules` is the only FINALIZED prior epic; 133-scenario baseline (113 prior + 20 `security-rules` acceptance) is this feature's confirmed-stable floor. `security-rules-write-path` and `security-rules-query-path` are NOT FINALIZED (no evolution docs exist for either — confirmed via `docs/evolution/*.md` directory listing) and `security-rules-query-path` is confirmed still mid-DELIVER (`git log`: only `feat(security-rules-query-path): ownership-equality query compliance (US-01)` committed; `git status`: an uncommitted acceptance test for what is almost certainly US-02). This feature's dependency is on `security-rules-query-path`'s **DESIGN decisions being stable** (ADR-031, already Accepted) — not on that feature being FINALIZED or fully DELIVERed. Noted explicitly, not silently assumed either way.

No contradictions found between this feature's scope and any prior artifact's LOCKED decisions. This feature does not reopen Resolution 1/2/3 from `security-rules`, either Resolution from `security-rules-write-path`, or either Resolution from `security-rules-query-path` — it adds a new, narrow rule concept alongside them. One genuinely new default (§ Job Discovery Framing Resolution, Resolution 2) is introduced and is flagged, with its own confidence reasoning, not silently assumed — see § Handoff Package.

---

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

| # | Decision | Value |
|---|---|---|
| 1 | Feature Type | Cross-cutting — spans BC-4 Access Control (extended: new disjoint group-rule storage + a new pure decision branch in `handle_run_query`'s existing rule-lookup) and BC-2 Document Storage's existing `RunQuery` handler call site. **Evidence-based nuance, mirroring `security-rules-query-path`'s own precedent**: direct code reads (§ Reading Confirmation) confirm zero change is needed to `embyr-pg-storage::backend_adapter::run_query` — its `all_descendants` SQL branch already exists and already works; this feature only corrects WHICH rule (if any) is consulted before that SQL runs. |
| 2 | Walking Skeleton | Evaluated (this wave's call): **YES** — see § Walking Skeleton Evaluation |
| 3 | UX Research Depth | Lightweight — backend/API extension of an already-researched feature, mirroring all 3 prior epics' own framing |
| 4 | JTBD Analysis | Yes (default) — extends `job_id: JOB-17` (see § Persona & Job) |

### Walking Skeleton Evaluation (Decision 2 = "Depends")

Three existing mechanisms were evaluated for reuse before concluding what this feature's walking skeleton actually needs to add:

1. **`embyr_core::access_control::{parse_condition, check_query_compliance, QueryComplianceOutcome, UnsatisfiedConjunct}` (ADR-031).** Reused **completely unchanged** — no new decidable shape, no new parsing, no new evaluator branch. The 5-shape decidable set `security-rules-query-path` locked applies identically to a collection-group rule's condition; only the *source table* the condition is read from is new.
2. **`crates/embyr-server/src/adapters/system_db::{get_access_rule, get_write_access_rule}` (ADR-028/030).** Neither is reusable AS-IS for this feature's core mechanism — both are keyed by exact `(project_id, collection_path)`, a concept that has no meaning for a query that, by definition, spans an open-ended, execution-time-unknown set of nested paths. Their *pattern* (single indexed PK lookup, `None`-short-circuits) is the direct precedent this feature's new `get_group_access_rule` mirrors, per ADR-030's own disjoint-table discipline.
3. **`embyr-pg-storage::backend_adapter::run_query`'s existing `all_descendants` SQL branch.** Confirmed by direct code read: already correct, already collection-group-capable, requires zero change. The enforcement gap is entirely upstream, in `grpc::handler::handle_run_query`'s rule-lookup composition — the identical class of finding `security-rules-query-path` made about its own feature relative to `embyr-pg-storage`.

**Verdict**: no existing mechanism decides, before execution, whether a *collection-group* query's shape satisfies a rule scoped to the group as a whole (independent of any single nested path's own rule). A walking skeleton is needed: a NEW, disjoint rule store keyed by `(project_id, collection_id)` alone, consulted ONLY when `all_descendants = true`, reusing `check_query_compliance()` unchanged once the correct condition is loaded — never touching `access_rules`, `write_access_rules`, or `embyr-pg-storage` (§ Story Map, Slices 01–06).

---

## Wave: DISCUSS / [REF] Job Discovery — Framing Resolution

This is the single most important output of this DISCUSS: the central architectural question the task asked to be surfaced, resolved with the same evidence discipline `security-rules`'s 3 Resolutions, `security-rules-write-path`'s 2 Resolutions, and `security-rules-query-path`'s 2 Resolutions established as this project's precedent.

### Resolution 1 (THE central architectural question) — Is a collection-group rule a new rule concept, or a composition of per-exact-path rules?

| Option | Description | Fit against evidence |
|---|---|---|
| **(A) Auto-apply the same-named top-level exact-path rule to the whole group** — no new storage; a group query is checked against whatever `access_rules` row exists for the bare collection id (approximately what today's SHIPPED code accidentally does, since it performs exactly one lookup keyed on the bare `collection_id` regardless of `all_descendants`) | **Rejected — this is the bug, not a candidate fix, and is affirmatively unsafe by direct construction.** `journal_entries` exists at BOTH the top level (Maria's/Dana's personal entries, `owner_id` field, `security-rules`'s own established rule) and nested under `expeditions/{expeditionId}/journal_entries` (this feature's own domain example — shared, multi-contributor expedition logs). A caller whose query filter satisfies the TOP-LEVEL rule's ownership-equality conjunct is, under Option A, admitted into a query whose SQL (`backend_adapter::run_query`, confirmed above) ALSO returns rows from the nested `expeditions/*/journal_entries` instance — a location the top-level rule was never written to govern and may have entirely different entitlement semantics. This is not a hypothetical: it is the literal mechanism of the currently-exploitable gap this feature exists to close. |
| **(B) Execute-time composition of every matching nested path's own exact-path rule** — enumerate every actual nested collection instance the group query would touch, and require the caller's query to satisfy the AND (or OR, or "strictest wins") of each instance's own rule | **Rejected — structurally intractable, and not well-defined even if it were tractable.** *Intractability*: the set of nested paths a collection-group query touches is not knowable at query-planning time — discovering it requires running the query first, i.e. the exact execute-then-filter antipattern `security-rules-query-path`'s own Resolution 1, Option B already rejected outright for a much simpler case. *Undefinedness*: even given a hypothetically-known path set, no composition rule is well-defined — AND-of-all-instance-rules fails whenever two instances use different field names for ownership (this feature's own domain example: top-level `journal_entries` uses `owner_id`; a hypothetical alternate nested collection could reasonably use a different field name for a structurally different entitlement concept — no single query filter shape could ever satisfy two different field-name requirements in one WHERE clause); OR-of-all-instance-rules would admit a caller who satisfies ANY one instance's rule into a query that returns rows from ALL instances, an even more severe version of Option A's own flaw. Real Firestore itself does not attempt this: a collection-group rule (`match /{path=**}/journal_entries/{doc}`) is declared once, independently, never computed from the union of `match /journal_entries/{doc}` and `match /expeditions/{id}/journal_entries/{doc}`. |
| **(C) A new, independently-authored, independently-stored collection-group rule — one condition per `(project_id, collection_id)`, disjoint from `access_rules`/`write_access_rules`, consulted only when `all_descendants = true`** | **Strongest fit — the only option that is (a) tractable at query-planning time, (b) well-defined regardless of how many or how differently-shaped the group's actual nested instances are, and (c) matches real Firestore's own collection-group rule declaration model (`match /{path=**}/collectionId/{doc}`) exactly, not merely by analogy.** See the schema/composition below. |

**Resolution**: **(C) is locked.** A collection-group rule is stored in a NEW, disjoint table — `group_access_rules` — schema-identical in shape to `access_rules`/`write_access_rules` (ADR-028/030) but keyed by `(project_id, collection_id)` alone (no parent-path component; a collection-group id is, by construction, a bare identifier, never a path — mirroring real Firestore's own `collectionGroup(db, 'journal_entries')` SDK call, which likewise takes a bare id). This is a schema EXTENSION mirroring ADR-030's own precedent almost exactly (a third disjoint table alongside two existing disjoint tables), not a redesign. `check_query_compliance()` (ADR-031) is called completely unchanged once the correct condition (from `group_access_rules`, not `access_rules`) is loaded — the 5-shape decidable set, `filter_binds_field_to_uid`'s caller's-own-uid binding property, and the rejection-reason vocabulary all apply identically and unmodified to a group rule's condition.

### Resolution 2 — Default behavior when `all_descendants = true` and no group rule is defined

This is a SECOND, distinct question this DISCUSS resolves explicitly (not silently folded into Resolution 1): given Resolution 1 establishes group rules as their own concept, what happens to a collection-group query against a collection id with NO group rule defined?

| Option | Description | Fit against evidence |
|---|---|---|
| **(A) Proceed unfiltered** — mirrors `security-rules`'s own Resolution 2 ("no rule ⇒ unrestricted") applied to the new group-rule table | **Rejected.** This would leave the currently-exploitable bypass this feature is chartered to close functionally unpatched for every collection id Alex has not YET separately protected with a group rule — including `journal_entries`, which Alex reasonably believes IS protected (he defined its top-level rule under `security-rules`). Silently applying "unrestricted" as the group default reproduces the exact "Alex believes he's protected but isn't" failure mode `security-rules-query-path`'s own § Persona & Job named as this initiative's recurring risk. |
| **(B) Fall back to the same-named exact-path rule, if one exists; otherwise unrestricted** | **Rejected — this is Option A from Resolution 1 wearing a "default" label.** Identical unsafe mechanism (a same-named single-location rule silently governing a query that spans other locations it was never written for), identical rejection reasoning. |
| **(C) Reject outright — a collection-group query against a collection id with no group rule of its own is refused before execution, regardless of whether any exact-path rule exists anywhere for that same collection id** | **Strongest fit, high confidence.** Matches real Firestore's own behavior precisely: a real Firestore `collectionGroup()` query against a collection id with no `match /{path=**}/collectionId/{doc}` block fails with a permission error, full stop — Firestore never falls back to a same-named top-level `match` block either. This is also the cheapest option to implement correctly: it requires exactly one indexed PK lookup against the new `group_access_rules` table alone — no scan or existence-check against `access_rules` is needed to decide it, preserving every prior epic's "no overhead when unarmed, no expensive scan" NFR discipline. |

**Resolution**: **(C) is locked.** This is a deliberate, evidenced NEW default — it does **not** reopen or contradict `security-rules`'s own Resolution 2 ("no rule ⇒ unrestricted" for `GetDocument`/non-group `RunQuery`/writes remains completely unchanged, § System Constraints below) — it establishes a narrower posture for a capability (`all_descendants = true` enforcement) that has never previously existed in any shipped, tested form. Confirmed by direct evidence, not assumed: none of `security-rules`'s 5, `security-rules-write-path`'s 7, or `security-rules-query-path`'s 7 user stories reference `all_descendants` or collection-group queries anywhere in their Domain Examples or UAT Scenarios — the 133-scenario FINALIZED baseline plus `security-rules-query-path`'s own delivered scenarios exercise zero collection-group queries, so this new, stricter default costs zero regression risk against any confirmed-existing test.

**Confidence and escalation note**: HIGH confidence on Resolution 1 (Option C) — directly evidenced by the field-name-divergence argument (a concrete domain-example-grounded proof, not a hypothetical) and by real Firestore's own declaration model. HIGH confidence on Resolution 2 (Option C) — directly evidenced by real Firestore's own collection-group-query behavior and by the zero-existing-scenario-impact finding. **One genuine judgment call made without a live stakeholder is flagged, not silently asserted**: whether Resolution 2's fail-closed default should apply universally to every `all_descendants = true` query, or only when a same-named exact-path rule exists somewhere in the project (signaling the collection id was meant to be protected). This DISCUSS locks the universal reading (simpler, cheaper, matches real Firestore, zero regression cost) — recommend the orchestrator confirm before DESIGN treats it as unquestionably final (§ Handoff Package).

---

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: **P1 — Alex, SDK Developer** (existing persona, unchanged). No new persona work — Decision 3 (Lightweight) confirms Alex's authoring mental model is already established across 3 prior epics.

**Domain-example company**: **Trailmark**, continued. Collections: `journal_entries` (top-level — Maria's/Dana's private trip-journal entries, `owner_id` field, existing `security-rules` exact-path rule, continued); `expeditions/{expeditionId}/journal_entries` (**new domain example, nested** — shared, multi-contributor expedition logs, same `owner_id` field name deliberately chosen consistent with the top level, to allow a single group rule to be authored meaningfully; no exact-path rule of its own today); `trail_guides`, `trip_photos`, `trip_comments`, `trail_map_overlays`, `app_config` (all continued, unchanged from `security-rules-query-path`).

**job_id decision (per Decision 4)**: **this feature extends JOB-17 (`document-access-control`) — it does not mint a new job.** Direct textual evidence: JOB-17's own `job_story` already says "enforce them **on every request**" — a collection-group `RunQuery` is a request. JOB-17's own accumulated NOTEs (read-path, write-path, query-path realizations) already establish the "make it real, one operation surface at a time" extension pattern this feature continues a fourth time — closing a residual gap WITHIN the query-path operation surface (`RunQuery`) `security-rules-query-path` itself did not fully close, not opening a new operation surface. This mirrors the exact "same job, next increment" reasoning all 3 prior epics used, not the "same persona, different goal ⇒ new job" pattern that produced JOB-17 itself.

**Opportunity scoring**: JOB-17's existing opportunity score (17, priority critical) is unchanged — this feature does not create a new job. The urgency case specifically for this feature: Alex who has ALREADY shipped a top-level `journal_entries` rule and confirmed it protects `GetDocument` and ordinary `RunQuery` calls (via `security-rules`/`security-rules-query-path`) has every reason to believe his data is protected — nothing in either prior feature's UX signals that a `collectionGroup()` query is a structurally different, currently-unprotected code path. This is the same "silently exploitable despite Alex's reasonable belief" urgency pattern `security-rules-query-path` itself used to justify its own priority.

---

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

Run before journey/story-map investment, per Phase 1.5.

| Signal | Threshold | This feature | Fired? |
|---|---|---|---|
| User stories | >10 | 7 (US-01 through US-07) | **NO** |
| Bounded contexts / modules | >3 | 2 — BC-4 Access Control (extended: new disjoint `group_access_rules` storage + a new pure branch in the already-existing rule-lookup) + BC-2 Document Storage's existing `RunQuery` handler call site (the SAME call site `security-rules-query-path` already touched — no new call site, only a branch inside the existing one). **Confirmed by direct code read**: zero change to `embyr-pg-storage::backend_adapter::run_query`'s SQL-building layer — its `all_descendants` branch already exists, already correct. | **NO** |
| Walking Skeleton integration points | >5 | 1 — `handle_run_query` (the identical call site `security-rules-query-path`'s own Walking Skeleton used) | **NO** |
| Estimated effort | >2 weeks | 7 slices, ~8 days total (§ Elephant Carpaccio Slices) | **NO** |
| Independent shippable outcomes | multiple | **NO** — group-rule definition (US-01) and group-query enforcement (US-02–06) are inseparable halves of one outcome ("collection-group queries are honestly enforced, not silently wrong"), mirroring all 3 prior epics' own "inseparable halves" reasoning; simulation (US-07) is a normal Release-2 enhancement | **NO** |

**0 of 5 signals fired. Verdict: PASS — right-sized.** This feature is narrower in code footprint than any of its 3 predecessors: it touches zero new call sites (only a new branch inside `security-rules-query-path`'s own already-existing `handle_run_query` composition) and reuses `check_query_compliance()`/`QueryComplianceOutcome`/`UnsatisfiedConjunct` completely unchanged — its only genuinely new component is a disjoint storage table and its adapter methods, the identical shape ADR-030 already established as low-risk, low-effort precedent.

---

## Wave: DISCUSS / [REF] Journey — Collection-Group Compliance Flow (Lightweight)

Per Decision 3, this is a short delta on `security-rules-query-path`'s own journey (itself a short delta on `security-rules`'s Comprehensive-depth journey) — not reproduced here.

### What's new in Alex's mental model

Alex now expects — matching real Firestore exactly — that `collectionGroup('journal_entries')` is a **distinct authorization surface** from `collection('journal_entries')`: protecting the top-level collection does not automatically protect the group, and he must explicitly decide a collection id is safe to open to group querying, the same deliberate, per-collection opt-in posture `security-rules`'s own Resolution 2 already trained him to expect for ordinary collections.

### Collection-group evaluation flow (extends `security-rules-query-path`'s own flow)

```
A RunQuery call arrives with all_descendants = true for a collection id
        │
   (authenticate()/attach_client_identity_if_present() unchanged, identical
    to the non-group flow)
        │
        ▼
   Does this collection ID have a GROUP rule defined? (group_access_rules,
   keyed by (project_id, collection_id) ONLY — never access_rules)
        │
   no ──────────────────────────┐                       yes
        │                        │                        │
        ▼                        │                        ▼
  Query rejected outright,       │        Is the group rule's Condition shape
  "no collection-group rule      │        DECIDABLE? (identical 5-shape set,
  defined for this collection    │        check_query_compliance(), UNCHANGED)
  id" (US-04) — regardless of    │                        │
  whether a same-named           │           ┌────────────┴─────────────┐
  exact-path rule exists         │          no                         yes
  anywhere                       │           │                          │
        │                        │           ▼                          ▼
        │                        │    Query rejected,          Does the query's filter
        │                        │    "rule shape not          (+ auth context) satisfy
        │                        │    supported" (mirrors       every decidable conjunct?
        │                        │    US-05, query-path)               │
        │                        │           │               ┌─────────┴─────────┐
        │                        │           │              no                  yes
        │                        │           │               │                   │
        └────────────────────────┴───────────┴────────  Query rejected,   Query proceeds,
                                                           naming the      returning matching
                                                           missing         rows from EVERY
                                                           constraint      nesting depth
                                                           (US-03)         (US-02)
```

---

## Wave: DISCUSS / [REF] Story Map & Walking Skeleton

**Persona**: P1 Alex | **Goal**: Give Alex a genuinely new, independently-authored rule for `collectionGroup()` querying — never silently inherited from, and never composed out of, any same-named exact-path rule — while every existing GetDocument, write, and non-group-query behavior remains provably byte-for-byte unaffected.

### Backbone

| A. A Group Rule Is Authored and Governs Group Queries | B. Every Other Existing Surface Is Unaffected |
|---|---|
| Alex defines an independent collection-group rule for a collection id **[WS]** | GetDocument, writes, and non-group RunQuery remain governed exclusively by their existing exact-path rules **[WS]** |
| A compliant collection-group query is admitted, returning matching rows from every nesting depth **[WS]** | Untouched collections and the full regression baseline are unaffected **[WS]** |
| A non-compliant collection-group query is rejected outright, before execution **[WS]** | |
| A collection-group query against an ungoverned collection id is rejected outright, regardless of any same-named exact-path rule **[WS]** | |

### Walking Skeleton

Alex defines a NEW group rule for `journal_entries` (`request.auth.uid == resource.data.owner_id`, deliberately the SAME field name as the existing top-level exact-path rule): Maria's `collectionGroup('journal_entries')` query filtered by her own `owner_id` is admitted and returns her entries from BOTH the top-level collection AND the nested `expeditions/*/journal_entries` instance; her unfiltered or wrongly-filtered group query is rejected; a group query against `trip_photos` (which has an exact-path AND-composed rule but no group rule) is rejected outright; Maria's `getDoc()`, her non-group `RunQuery`, and her writes on `journal_entries` remain governed exactly as `security-rules`/`security-rules-write-path`/`security-rules-query-path` already left them; and `app_config`/the full pre-existing regression baseline are provably unaffected. No facade, real System DB rule state, real Maria/Dana sessions — mirrors all 3 prior epics' own WS discipline exactly.

### Release 1 — Collection-Group Queries Are Honestly Enforced (Slices 01–06, US-01 through US-06)

Outcome: any collection id Alex protects with a group rule genuinely gates `collectionGroup()` querying across every nesting depth; any collection id without a group rule is honestly refused for group querying rather than silently mis-governed or silently open; every GetDocument, write, and non-group-query behavior remains exactly as the 3 prior epics left it.

### Release 2 — Authoring Confidence, Extended to Group Queries (Slice 07, US-07)

Outcome: Alex can prove a candidate collection-group query will be accepted before shipping it, extending `security-rules-query-path`'s own simulation guarantee to the group case.

---

## Wave: DISCUSS / [REF] Elephant Carpaccio Slices

| Slice | Story | Release | Estimate | Learning Hypothesis (disproves) | Production-data taste test |
|---|---|---|---|---|---|
| 01 (WS) | US-01 | 1 | 1 day | An independent collection-group rule cannot be stored, defined, and idempotently redefined, disjoint from `access_rules`/`write_access_rules`, without either colliding with an existing exact-path row or requiring a schema redesign | Real System DB row on a NEW table, real admin Bearer credential, real existing top-level `journal_entries` exact-path rule left provably untouched |
| 02 (WS) | US-02 | 1 | 1.5 days | A collection-group query cannot be proven compliant with a group rule, and cannot correctly return matching rows from EVERY actual nesting depth, without either enumerating nested paths first (the rejected antipattern) or requiring new query-execution machinery | Real group rule + real Maria/Dana signed-in sessions + real filtered `collectionGroup()` queries spanning BOTH `journal_entries` (top-level) and `expeditions/*/journal_entries` (nested) |
| 03 (WS) | US-03 | 1 | 1 day | A non-compliant collection-group query (missing filter, wrong-value filter) cannot be rejected BEFORE touching Postgres, reusing `check_query_compliance()` completely unmodified | Real unfiltered and wrongly-filtered `collectionGroup()` calls against a real group-ruled collection id |
| 04 (WS) | US-04 | 1 | 1.5 days | A collection-group query against a collection id with no group rule cannot be safely rejected outright — independent of whether a same-named exact-path rule exists — without either falling back unsafely to that exact-path rule (the rejected antipattern) or requiring an expensive scan of `access_rules` | Real `journal_entries` (has a top-level exact-path rule, no group rule) and real `app_config` (has neither) — both rejected for group querying identically |
| 05 (WS) | US-05 | 1 | 1 day | GetDocument, writes, and non-group `RunQuery` cannot be proven structurally unaffected by a group rule's existence or content without actually exercising all three against a collection id carrying both an exact-path rule AND a group rule with DIFFERENT conditions | Real `journal_entries` with both rule types active simultaneously, real Maria/Dana GetDocument/write/non-group-query calls |
| 06 (WS) | US-06 | 1 | 1 day | This feature's new group-query enforcement cannot be proven additive to the 133-scenario FINALIZED baseline plus `security-rules-query-path`'s own delivered scenarios without actually re-running them unmodified | Real full regression suite, real multi-collection project state including collections with neither rule type |
| 07 | US-07 | 2 | 1 day | A group-query simulation cannot share the exact same `check_query_compliance()` function real enforcement uses without duplicating (and risking drift in) the decidable-shape logic a fourth time | Real candidate group rules + real candidate query filters checked against the real compliance function |

**Total estimate: ~8 days.**

**Taste tests applied**:
- "4+ new components per slice" — none exceeds 2 (Slice 01: new table + new adapter methods; Slice 02: new call-site branch, zero new function; Slices 03–05: extend the same branch, no new component; Slice 06: zero new components, pure regression proof; Slice 07: thin wrapper over the existing simulation handler). PASS.
- "Every slice depends on a new abstraction" — Slice 01 (the disjoint group-rule table) is the one genuinely new abstraction; Slices 02–07 build on it but introduce none of their own. PASS — natural sequencing, not forced inflation.
- "No slice disproves a pre-commitment" — each has a distinct, falsifiable hypothesis (see table). PASS.
- "Synthetic-data-only slices prove plumbing, not value" — N/A; all 7 slices require real System DB state and real signed-in sessions across genuinely nested collection paths. PASS.
- "2+ slices identical except for scale" — none; each targets a distinct proof obligation (define / admit / reject-non-compliant / reject-ungoverned / prove-other-surfaces-unaffected / prove-regression-baseline-unaffected / simulate). PASS.

---

## Wave: DISCUSS / [REF] Prioritization

| Priority | Slice | Target Outcome | Rationale |
|---|---|---|---|
| 1 | Slice 01 (WS) | A collection id can have an independent group rule defined | Prerequisite — without a stored group condition, Slices 02–07 have nothing to evaluate against |
| 2 | Slice 04 (WS) | A collection-group query against an ungoverned collection id is rejected outright | Sequenced early deliberately — this is the single highest-consequence branch (a wrong default here reproduces the exact bypass this feature exists to close) and is independent of Slice 02's own admit-path mechanism; proving reject-by-default first de-risks the rest |
| 3 | Slice 02 (WS) | A compliant collection-group query is admitted, spanning every nesting depth | Burns down the second-riskiest assumption — that reusing `check_query_compliance()` unmodified correctly narrows results across genuinely different nested paths sharing one collection id |
| 4 | Slice 03 (WS) | A non-compliant collection-group query is rejected before execution | The other half of Slice 02's own mechanism, sequenced immediately after |
| 5 | Slice 05 (WS) | GetDocument/writes/non-group queries remain structurally unaffected | Proven directly against a collection id carrying BOTH rule types with DIFFERENT conditions — the strongest possible regression proof, sequenced once both group-query paths (Slices 02–04) exist to make the contrast meaningful |
| 6 | Slice 06 (WS) | The full pre-existing regression baseline is unaffected | The single highest-consequence regression risk, sequenced last within the WS as a proof *over* Slices 01–05's real behavior |
| 7 | Slice 07 | Alex can pre-check a candidate collection-group query | Highest-leverage for Alex's own confidence; depends on Slices 01–04's compliance mechanism existing to wrap |

---

## Wave: DISCUSS / [REF] System Constraints

- **Collection-group rules are a NEW, independently-authored, independently-stored rule concept — LOCKED to Resolution 1's Option C** (§ Job Discovery Framing Resolution). DESIGN must not implement Resolution 1's Option A (auto-apply the same-named exact-path rule) or Option B (execute-time composition) under any framing, including as an "interim MVP." A new disjoint table (`group_access_rules`, keyed by `(project_id, collection_id)` alone) is required, mirroring ADR-030's own disjoint-table precedent.
- **A collection-group query against a collection id with no group rule is rejected outright — LOCKED to Resolution 2's Option C.** This applies regardless of whether a same-named exact-path rule (`access_rules`) exists anywhere for that collection id. DESIGN must not implement a fallback to the exact-path rule under any framing.
- **`check_query_compliance()`, `QueryComplianceOutcome`, `UnsatisfiedConjunct`, and the 5-shape decidable set (ADR-031) are reused completely unchanged.** No new decidable shape, no new evaluator branch, no modification to `embyr_core::access_control` beyond the addition of the new table's adapter methods in `embyr-server`.
- **This feature reads/writes `group_access_rules` only — never `access_rules` or `write_access_rules`.** GetDocument's, write-path's, and non-group `RunQuery`'s existing rule-lookup behavior receive ZERO code changes (AC-17-93/94/95).
- **No change to `embyr-pg-storage`'s SQL-building/query-execution layer.** `run_query`'s existing `all_descendants` branch already correctly executes collection-group queries; this feature only changes which rule, if any, is consulted before that call.
- **The single highest-consequence design risk**: the "no group rule defined" default arm (US-04) must reject, never silently fall back to any exact-path rule and never silently allow. Flagged as this feature's designated mutation-testing surface (per-feature strategy, CLAUDE.md), mirroring `security-rules-query-path`'s own designation for its undecidable-shape arm.
- **The second-highest-consequence risk**: `filter_binds_field_to_uid`'s caller's-own-uid binding property (inherited unchanged from ADR-031) must continue to bind against the SAME collection-group query's own already-server-verified `auth.uid` — never a client-supplied value — identical to the non-group case, now proven across multiple nesting depths in the same query.
- **No regression to `GetDocument`, writes, or non-group `RunQuery`.** A collection id's group-rule state (defined, undefined, or any condition) must have zero observable effect on any of these three existing surfaces — structurally, not just conventionally (disjoint table, disjoint call-site branch, per Resolution 1).
- **v1 collection-group enforcement surface is `RunQuery` with `all_descendants = true` only.** `Listen`/`onSnapshot` (Epic 2d, `security-rules-realtime`) remains explicitly out of scope, even for a collection-group-backed listener — unchanged from prior epics' own scope boundary.
- **A pre-existing, non-security correctness gap in non-group `RunQuery`'s parent-path resolution is flagged, not fixed here** (§ Open Questions, OQ-SRCG-03) — `req.parent` is never combined with `collection_id` for `all_descendants = false` queries, meaning a query explicitly scoped to a specific parent document's subcollection may resolve to the wrong collection instance today, independent of this feature. Out of scope: fixing this is a general RunQuery-correctness concern, not a security-rule authorization concern, and folding it in would violate this feature's own right-sizing.
- Ubiquitous language introduced: **collection-group rule** (a condition scoped to a bare collection id, independent of any parent path, gating every `RunQuery` with `all_descendants = true` against that id), **group-governed** (a collection id with an active collection-group rule), **ungoverned** (for group-query purposes: a collection id with no collection-group rule, regardless of any exact-path rule's existence).

---

## Wave: DISCUSS / [REF] User Stories

### US-01: Alex Defines (and Redefines) an Independent Collection-Group Rule for a Collection ID

**job_id**: JOB-17
**Slice**: 01 (Walking Skeleton) | **Release**: 1

#### Elevator Pitch
Before: Alex has no way to author authorization for `collectionGroup('journal_entries')` querying — only the existing per-exact-path rule (`security-rules`) exists, and it says nothing about the group as a whole.
After: call the admin API's new group-rule-definition action for collection id `journal_entries` with `request.auth.uid == resource.data.owner_id` → sees confirmation the group rule is stored and active, entirely independent of and never overwriting `journal_entries`'s existing exact-path rule.
Decision enabled: Alex knows a single, explicitly-authored rule now governs EVERY `collectionGroup('journal_entries')` query, regardless of how many different nested locations share that leaf collection id today or in the future.

#### Domain Examples
1. **Happy Path**: Alex defines, for the first time, a collection-group rule on `journal_entries` in `trailmark-prod`: `request.auth.uid == resource.data.owner_id`. Sees confirmation the group rule is stored and active; `journal_entries`'s existing top-level exact-path rule (from `security-rules`) is unchanged.
2. **Edge Case**: An hour later, Alex redefines the group rule to add a second conjunct. The new condition fully replaces the old one immediately — no overlap window (idempotent upsert, mirroring `security-rules`'s Resolution 3 precedent) — and the exact-path rule remains untouched throughout.
3. **Error/Boundary**: Alex submits collection id `expeditions/journal_entries` (containing a `/`) as the group-rule target. Rejected: a collection-group id is inherently a bare identifier, never a path — mirrors real Firestore's own `collectionGroup(db, id)` SDK signature, which likewise takes a bare id.

#### UAT Scenarios (BDD)

##### Scenario: First-time group-rule definition succeeds and is independent of the exact-path rule
Given project `trailmark-prod` exists, `journal_entries` has an active exact-path rule from `security-rules`, and no group rule yet
When Alex defines a group rule for collection id `journal_entries` with a valid decidable-shape condition, using a valid admin Bearer credential
Then the group rule is stored and active for collection id `journal_entries`, and the existing exact-path rule is unchanged

##### Scenario: Redefining a group rule fully replaces it, with no overlap window, and never touches the exact-path rule
Given `journal_entries` already has an active group rule and an active exact-path rule
When Alex submits a new group-rule condition for the same collection id
Then the new condition is immediately and fully active, the previous group condition no longer applies, and the exact-path rule remains exactly as it was

##### Scenario: A collection id containing a path separator is rejected
Given project `trailmark-prod` exists
When Alex submits a group-rule definition with collection id `expeditions/journal_entries`
Then the request is rejected with a message naming that a collection-group id must be a bare collection identifier, not a path

##### Scenario: Group-rule definition without valid admin credentials is rejected
Given project `trailmark-prod` exists
When Alex submits a group-rule-definition request with a missing or invalid admin Bearer credential
Then the request is rejected the same way any other admin endpoint rejects missing/invalid credentials

#### Acceptance Criteria
- [ ] AC-17-77: A valid first-time group-rule definition is stored and active for the named collection id.
- [ ] AC-17-78: Redefining a collection id's group rule fully and immediately replaces the prior condition — no merge, no overlap window.
- [ ] AC-17-79: Defining or redefining a group rule has zero observable effect on the same collection id's exact-path rule (`access_rules`/`write_access_rules`), and vice versa — independent, disjoint storage.
- [ ] AC-17-80: A collection id containing a `/` is rejected as invalid for a group-rule definition, with a distinguishable reason.

#### Outcome KPIs
See § Outcome KPIs below (feeds KPI #1, North Star).

#### Technical Notes (Optional)
Suggested: new table `group_access_rules`, schema-identical to `access_rules`/`write_access_rules` (ADR-028/030) but `PRIMARY KEY (project_id, collection_id)`; new adapter methods `upsert_group_access_rule`/`get_group_access_rule`, mirroring `upsert_write_access_rule`/`get_write_access_rule`'s exact shape. New admin handler `define_group_access_rule`, mirroring `define_write_access_rule`. Exact endpoint path and migration numbering are DESIGN's call.

---

### US-02: A Collection-Group Query Satisfying the Group Rule Is Admitted, Spanning Every Nesting Depth

**job_id**: JOB-17
**Slice**: 02 (Walking Skeleton) | **Release**: 1

#### Elevator Pitch
Before: Trailmark's "My Journal Timeline" screen wants to show Maria every entry she's authored — whether stored in her personal top-level `journal_entries` collection or contributed to a shared `expeditions/{expeditionId}/journal_entries` subcollection — but `RunQuery` with `all_descendants = true` has no rule of its own to prove that cross-location query is safe.
After: call the SDK's `getDocs(query(collectionGroup(db,'journal_entries'), where('owner_id','==',auth.currentUser.uid)))` — an unchanged SDK method — now checked against the published group rule before Postgres ever runs it → the query executes and Maria sees only her own entries, drawn from BOTH the top-level collection AND the nested expedition subcollection.
Decision enabled: Alex can build a genuine cross-location "everything I own" view and trust the server proves it's scoped to the caller — the same guarantee `security-rules-query-path` already gives single-location queries, now extended to the group case.

#### Domain Examples
1. **Happy Path**: Maria Santos issues a `collectionGroup('journal_entries')` query filtered by `owner_id == "maria-santos"`. The group rule's condition evaluates admitted; the query executes and returns her entries from BOTH `journal_entries` (top-level) and `expeditions/*/journal_entries` (nested).
2. **Edge Case**: Maria's query adds a second, unrelated filter (`archived == false`) alongside the required one. The composite AND filter still contains the required conjunct; compliance holds regardless of what else is AND'd in — identical to `security-rules-query-path`'s own AC-17-50.
3. **Error/Boundary**: Dana issues the identical group query filtered by `owner_id == "maria-santos"` — a filter on the right field, bound to Maria's id, not Dana's own. Rejected: the filter's value doesn't match Dana's own verified `request.auth.uid`, identical to `security-rules-query-path`'s own load-bearing property (AC-17-51), now proven for a group query.

#### UAT Scenarios (BDD)

##### Scenario: A collection-group query with the required matching filter is admitted and returns rows from every nesting depth
Given `journal_entries` has an active collection-group rule requiring `request.auth.uid == resource.data.owner_id`
And Maria Santos holds a verified identity
And Maria has entries in both the top-level `journal_entries` collection and a nested `expeditions/trek-2026/journal_entries` subcollection
When Maria issues a `collectionGroup('journal_entries')` query filtered by `owner_id == "maria-santos"`
Then the query is admitted and returns Maria's entries from both locations

##### Scenario: Extra filters alongside the required one do not break compliance
Given the same group rule as above
When Maria issues the same group query additionally filtered by `archived == false`
Then the query is admitted and executes

##### Scenario: A filter bound to someone else's identity is rejected, not merely field-matched
Given the same group rule as above
And Dana Kim holds a verified identity distinct from `"maria-santos"`
When Dana issues a `collectionGroup('journal_entries')` query filtered by `owner_id == "maria-santos"`
Then the query is rejected before execution, attributable to the group rule

##### Scenario: check_query_compliance is reused completely unmodified for group rules
Given a collection-group rule using any of the 5 decidable shapes `security-rules-query-path` already locked (literal-true, literal-false, auth-presence, ownership-equality, AND-composition)
When any caller issues a matching collection-group query
Then the admit/reject outcome is identical in mechanism to the non-group case, with no new decidable shape introduced

#### Acceptance Criteria
- [ ] AC-17-81: A collection-group query whose filter includes an equality constraint on the group rule's referenced field, bound to the caller's own verified `request.auth.uid`, is admitted and returns matching rows from every nesting depth the collection id occurs at.
- [ ] AC-17-82: Additional filters beyond the required one do not affect compliance.
- [ ] AC-17-83: `check_query_compliance()`, `QueryComplianceOutcome`, and `UnsatisfiedConjunct` (ADR-031) are reused completely unmodified for group rules — no new decidable shape, no new evaluator branch.
- [ ] AC-17-84: A collection-group query's returned rows are correctly narrowed across every nesting depth by the query's own filter (the pre-existing `backend_adapter::run_query` `all_descendants` SQL, unmodified) — the compliance check proves the query's SHAPE carries the narrowing guarantee; it does not itself filter rows.

#### Outcome KPIs
See § Outcome KPIs below (KPI #1, North Star).

#### Technical Notes (Optional)
`handle_run_query` gains a branch: when `all_descendants == true`, look up `group_access_rules` instead of `access_rules`; when `Some`, call `check_query_compliance()` exactly as `security-rules-query-path` already does for the non-group case — same function, same types, different source table.

---

### US-03: A Non-Compliant Collection-Group Query Is Rejected Before Execution, Not Filtered After

**job_id**: JOB-17
**Slice**: 03 (Walking Skeleton) | **Release**: 1

#### Elevator Pitch
Before: nothing distinguishes "Maria queried her own cross-location entries" from "Maria queried everyone's, across every nesting depth, and the server quietly decided what to show her."
After: call `getDocs(collectionGroup(db,'journal_entries'))` — no filter — against a group-governed collection id → the call fails immediately with a specific rejection naming the missing constraint, before any row is read from Postgres.
Decision enabled: Alex can tell, from the response alone, that his client code's group-query shape — not his rule, not his data — needs fixing, extending `security-rules-query-path`'s own US-02 guarantee to the group case.

#### Domain Examples
1. **Happy Path (expected reject)**: Maria issues a `collectionGroup('journal_entries')` query with no filter at all. Rejected before touching Postgres.
2. **Edge Case**: Maria issues the group query filtered only by `archived == false` (a real, valid filter, just not the required one). Still rejected — the required conjunct is absent.
3. **Error/Boundary**: A group query using `where('owner_id','!=','someone-else')` (inequality) instead of the required equality is rejected — identical mechanism to `security-rules-query-path`'s own AC-17-55.

#### UAT Scenarios (BDD)

##### Scenario: A collection-group query with no filter at all is rejected before touching storage
Given `journal_entries` has the ownership-equality collection-group rule
When Maria issues a `collectionGroup('journal_entries')` query with no filter
Then the query is rejected, and no document row is fetched

##### Scenario: A collection-group query with unrelated filters but no matching conjunct is rejected
Given the same group rule
When Maria issues the group query filtered only by `archived == false`
Then the query is rejected before execution

##### Scenario: An inequality filter on the right field does not satisfy an equality group rule
Given the same group rule
When Maria issues the group query filtered by `owner_id != "someone-else"`
Then the query is rejected before execution

##### Scenario: The rejection is distinguishable from an ungoverned-collection rejection (US-04)
Given `journal_entries` has an active group rule that Maria's query fails to satisfy
When Maria issues the non-compliant group query
Then the rejection names the missing constraint, distinguishable from "no group rule defined for this collection id" (US-04)

#### Acceptance Criteria
- [ ] AC-17-85: A collection-group query with no filter, against a collection id whose group rule requires a matching equality conjunct, is rejected before any document is fetched.
- [ ] AC-17-86: A collection-group query whose filters exist but omit the required conjunct is rejected — other valid filters do not substitute.
- [ ] AC-17-87: A different operator (e.g. `!=`) on the group rule's referenced field does not satisfy an equality group rule.
- [ ] AC-17-88: The rejection response is distinguishable from the "no group rule defined" rejection (US-04) and from `authenticate()`-level rejections.

#### Outcome KPIs
See § Outcome KPIs below (KPI #1 North Star, KPI #2 Leading).

#### Technical Notes (Optional)
Reuses the same `check_query_compliance()` call and `query_compliance_rejection()`-style formatting `security-rules-query-path` already established (ADR-031) — no new rejection-formatting mechanism.

---

### US-04: A Collection-Group Query Against an Ungoverned Collection ID Is Rejected Outright

**job_id**: JOB-17
**Slice**: 04 (Walking Skeleton) | **Release**: 1

#### Elevator Pitch
Before (the actual, currently-exploitable gap): `RunQuery` with `all_descendants = true` against `journal_entries` — which DOES have a top-level exact-path ownership rule from `security-rules` — is checked, today, using that single exact-path rule as if it applied to the whole group; a caller who satisfies the top-level rule's own filter is admitted into a query whose result set also includes rows from the nested `expeditions/*/journal_entries` subcollection that rule was never written to govern.
After: call `collectionGroup('journal_entries')` against a collection id with NO independently-defined group rule → REJECTED outright, regardless of whether a same-named exact-path rule exists, regardless of filter shape.
Decision enabled: Alex knows a collection-group query is NEVER silently governed by a same-named single-location rule — he must explicitly decide it's safe to open group querying for a given collection id, the same "you decide, we never guess" posture the whole access-control initiative is built on.

#### Domain Examples
1. **Happy Path (expected reject)**: `journal_entries` has ONLY a top-level exact-path rule (from `security-rules`), no group rule. Any `collectionGroup('journal_entries')` query, any filter, is rejected.
2. **Edge Case**: `trip_photos` has an AND-composed exact-path rule (from `security-rules-query-path`'s own US-04) but no group rule. Collection-group queries on `trip_photos` are rejected too, regardless of the exact-path rule's own shape or strictness.
3. **Error/Boundary**: `app_config` has NEVER had any rule of any kind — no exact-path rule, no group rule. A `collectionGroup('app_config')` query is ALSO rejected under this feature's universal default — a NEW opt-in requirement for a brand-new capability (group querying), distinct from `app_config`'s continued, fully unrestricted `GetDocument`/write/non-group-query behavior (US-05/US-06).

#### UAT Scenarios (BDD)

##### Scenario: A collection-group query is rejected when only a same-named exact-path rule exists
Given `journal_entries` has an active exact-path rule (from `security-rules`) and no group rule
When any signed-in end user issues a `collectionGroup('journal_entries')` query, including one whose filter would satisfy the exact-path rule
Then the query is rejected, naming that no collection-group rule is defined for this collection id

##### Scenario: A collection-group query is rejected against a collection id with a strict exact-path rule but no group rule
Given `trip_photos` has an active AND-composed exact-path rule and no group rule
When any caller issues a `collectionGroup('trip_photos')` query
Then the query is rejected identically, regardless of the exact-path rule's own shape

##### Scenario: A collection-group query is rejected against a collection id with no rule of any kind
Given `app_config` has never had an exact-path rule or a group rule
When any caller issues a `collectionGroup('app_config')` query
Then the query is rejected, naming that no collection-group rule is defined for this collection id

##### Scenario: The ungoverned-group rejection requires no scan of the exact-path rule table
Given `journal_entries` has an exact-path rule and no group rule
When a `collectionGroup('journal_entries')` query is rejected
Then the rejection decision is reached via a single indexed lookup against the group-rule table alone — no query against `access_rules` is issued to reach it

#### Acceptance Criteria
- [ ] AC-17-89: A collection-group query (`all_descendants = true`) against a collection id with no group rule is rejected outright, before touching Postgres — regardless of whether an exact-path rule exists for that same collection id anywhere.
- [ ] AC-17-90: This is a new default distinct from — and does not reopen — `security-rules`'s Resolution 2 ("no exact-path rule ⇒ unrestricted" for `GetDocument`/non-group `RunQuery`/writes remains completely unchanged).
- [ ] AC-17-91: The "no group rule defined" default is decided by a single indexed PK lookup against the group-rule table only — no scan of `access_rules` is performed.
- [ ] AC-17-92: The "no group rule defined" rejection is distinguishable from the "missing required filter" rejection (US-03) and from the "unsatisfied conjunct" rejection (US-03) — different remediation, different message.

#### Outcome KPIs
See § Outcome KPIs below (KPI #1 North Star — the "no false-allow" half is this story's entire purpose).

#### Technical Notes (Optional)
The default/fallthrough arm when `all_descendants == true` and `get_group_access_rule` returns `None` — the single most important arm in this feature. Designated mutation-testing surface (per-feature strategy, CLAUDE.md), mirroring `security-rules-query-path`'s own designation for its undecidable-shape arm.

---

### US-05: GetDocument, Writes, and Non-Group Queries Remain Governed Exclusively by the Existing Exact-Path Rules

**job_id**: JOB-17
**Slice**: 05 (Walking Skeleton) | **Release**: 1

#### Elevator Pitch
Before: Alex worries that introducing collection-group rules might change how `GetDocument`, writes, or ordinary (non-group) `RunQuery` calls are gated — three surfaces `security-rules`/`security-rules-write-path`/`security-rules-query-path` already carefully proved correct.
After: call the existing SDK `getDoc()`, `updateDoc()`, or a plain `getDocs(query(collection(db,'journal_entries')))` (non-group) on a collection that ALSO has an active group rule with a DIFFERENT condition → all three continue to be governed exclusively by their existing exact-path rules, completely unaffected by the group rule's existence or content.
Decision enabled: Alex can add group-query protection to a collection without re-verifying every other surface he already trusts.

#### Domain Examples
1. **Happy Path**: `journal_entries` has BOTH an exact-path rule (`request.auth.uid == resource.data.owner_id`) AND a group rule with a deliberately DIFFERENT condition (`request.auth != null`, no ownership check). Maria's `getDoc()` on her own entry is governed only by the exact-path rule — the group rule's weaker condition has no bearing on it.
2. **Edge Case**: A non-group `RunQuery` (`all_descendants = false`) on `journal_entries` is governed only by the exact-path rule — unaffected by the group rule's more permissive condition.
3. **Error/Boundary**: `expeditions/trek-2026/journal_entries`'s writes (`CreateDocument`/`UpdateDocument`/`DeleteDocument`, from `security-rules-write-path`) are governed only by whatever exact-path `write_access_rules` row exists for THAT specific nested path — never by the `journal_entries` group rule, even though the group rule's collection id matches.

#### UAT Scenarios (BDD)

##### Scenario: GetDocument is unaffected by a group rule's existence, even a more permissive one
Given `journal_entries` has an exact-path rule requiring ownership equality and a group rule requiring only `request.auth != null`
When Dana Kim, signed in but not the document's owner, calls `getDoc()` on Maria's `journal_entries` document
Then the read is denied by the exact-path rule, unaffected by the group rule's more permissive condition

##### Scenario: Non-group RunQuery is unaffected by a group rule's existence
Given the same two rules as above
When Dana issues a non-group (`all_descendants = false`) `RunQuery` on `journal_entries` with no ownership filter
Then the query is rejected by the exact-path rule's own compliance requirement, unaffected by the group rule

##### Scenario: Writes on a nested collection are unaffected by a same-named group rule
Given `journal_entries` has an active group rule, and `expeditions/trek-2026/journal_entries` has no write rule of its own
When any signed-in end user creates a document in `expeditions/trek-2026/journal_entries`
Then the create succeeds or fails according to `write_access_rules` for that exact nested path alone, unaffected by the `journal_entries` group rule

##### Scenario: A group rule's redefinition has zero effect on any of the three existing surfaces
Given `journal_entries` has an exact-path rule and an active group rule
When Alex redefines the group rule's condition
Then GetDocument, writes, and non-group RunQuery behavior on `journal_entries` are observably unchanged

#### Acceptance Criteria
- [ ] AC-17-93: `GetDocument`'s rule-lookup behavior is byte-for-byte unmodified by this feature.
- [ ] AC-17-94: Non-group (`all_descendants = false`) `RunQuery`'s rule-lookup behavior is byte-for-byte unmodified by this feature — still `access_rules`, still `security-rules`'s "no rule ⇒ unrestricted" default.
- [ ] AC-17-95: Write-path's rule-lookup behavior (`write_access_rules`) is byte-for-byte unmodified by this feature.
- [ ] AC-17-96: A group rule's existence, content, or absence has zero observable effect on any of the above three code paths — structurally (disjoint table, disjoint call-site branch), not just conventionally.

#### Outcome KPIs
See § Outcome KPIs below (KPI #3 Guardrail).

#### Technical Notes (Optional)
Primarily a proof obligation over US-01–US-04's real behavior and the 3 prior epics' own shipped code — mirrors `security-rules-query-path`'s own AC-17-69/70/71/72 discipline, now proven against a collection id carrying both rule types simultaneously (the strongest possible independence proof).

---

### US-06: Untouched Collections and the Full Regression Baseline Are Unaffected

**job_id**: JOB-17
**Slice**: 06 (Walking Skeleton) | **Release**: 1

#### Elevator Pitch
Before: Alex worries that shipping collection-group enforcement might silently reject a query that worked before, or perturb collections he's never touched with group rules.
After: call the existing SDK `getDoc()`/`getDocs()`/write methods on `app_config` and on the full pre-existing regression baseline → all continue to succeed exactly as before this feature shipped, and no scenario begins using a collection-group query it did not already use.
Decision enabled: Alex can adopt collection-group rules with zero risk to any surface or collection he hasn't touched.

#### Domain Examples
1. **Happy Path**: The 133-scenario FINALIZED regression baseline (`security-rules`) plus `security-rules-query-path`'s own delivered scenarios are re-run unmodified. None of them exercises `all_descendants = true`, so none is affected by this feature's new group-query default.
2. **Edge Case**: `app_config`'s `GetDocument`/write/non-group-query behavior remains fully unrestricted, exactly as before — only its (new, previously non-existent) group-query behavior newly rejects under US-04's default, which is a NEW capability being gated, not a regression of an EXISTING one.
3. **Error/Boundary**: A project with no group rule defined anywhere continues to reject every `all_descendants = true` query it receives, consistently, across every collection id — no partial rollout artifact, no collection id accidentally exempted.

#### UAT Scenarios (BDD)

##### Scenario: The full pre-existing regression baseline passes unmodified
Given the 133 FINALIZED `security-rules` scenarios and `security-rules-query-path`'s own delivered scenarios, none of which exercises `all_descendants = true`
When the full baseline is re-run against a build that includes this feature
Then all scenarios pass exactly as they did before this feature was added

##### Scenario: A collection with no rules of any kind is unaffected on every surface except group querying
Given `app_config` has never had an exact-path rule, a write rule, or a group rule
When any caller calls `getDoc()`, writes, or issues a non-group `RunQuery` on `app_config`
Then all three succeed exactly as before this feature shipped

##### Scenario: Group-query rejection is consistent across every ungoverned collection id in a project
Given a project has no group rule defined for any collection id
When collection-group queries are issued against several different collection ids in that project
Then every one is rejected identically, with no collection id silently exempted

#### Acceptance Criteria
- [ ] AC-17-97: The 133-scenario FINALIZED regression baseline passes unmodified.
- [ ] AC-17-98: `security-rules-query-path`'s own delivered acceptance scenarios pass unmodified.
- [ ] AC-17-99: A collection with no rule of any kind retains fully unrestricted `GetDocument`/write/non-group-query behavior; only its group-query behavior is newly gated (US-04), consistently.
- [ ] AC-17-100: Group-query rejection behavior is consistent across every ungoverned collection id in a project — no exemptions.

#### Outcome KPIs
See § Outcome KPIs below (KPI #3 Guardrail).

#### Technical Notes (Optional)
Proof obligation over US-01–US-05's real behavior. Structural guardrail preferred: `get_group_access_rule` returning `None` is the ONLY path that reaches the US-04 rejection — no other code touches `access_rules` or `write_access_rules` to reach it.

---

### US-07: Alex Can Pre-Check Whether a Candidate Collection-Group Query Would Be Accepted

**job_id**: JOB-17
**Slice**: 07 | **Release**: 2

#### Elevator Pitch
Before: Alex's only way to find out whether his app's `collectionGroup()` query code will be accepted is to run the app and watch it fail — or, worse, discover only in production that a collection id he assumed was group-protected has no group rule at all.
After: call the admin API's existing rule-simulation action (`security-rules-query-path`'s own `simulate_query_compliance`), extended to accept a candidate collection-group rule alongside the existing candidate filter and identity → sees whether that exact group-query shape would be admitted or rejected, without issuing any real query.
Decision enabled: Alex can validate his SDK's actual `collectionGroup()` query code, and confirm a given collection id truly has a group rule, before shipping.

#### Domain Examples
1. **Happy Path**: Alex simulates `journal_entries`'s group rule with candidate identity `end_user_id: test-user-001` and candidate filter `owner_id == "test-user-001"`. Sees "admitted."
2. **Edge Case**: Alex simulates the same rule with the same identity but an empty candidate filter (what his app's cross-location timeline screen actually issues today). Sees "rejected — missing required filter" — catching the bug before shipping.
3. **Error/Boundary**: Alex simulates a candidate collection id with NO group rule defined at all. Sees "rejected — no collection-group rule defined," matching US-04's real behavior, confirming he has not yet opened that collection id to group querying.

#### UAT Scenarios (BDD)

##### Scenario: Simulating a compliant candidate group query returns "admitted"
Given Alex holds a candidate collection-group rule and a candidate query filter that satisfies it for a given candidate identity
When Alex calls the simulation action with the group rule, identity, and filter
Then the response shows "admitted," matching what a real collection-group query would produce

##### Scenario: Simulation surfaces a missing-filter bug before publishing client code
Given Alex holds a published group rule and a candidate filter that omits the required conjunct
When Alex simulates that filter
Then the response shows "rejected — missing required filter," matching US-03's real behavior

##### Scenario: Simulation reports an ungoverned collection id identically to real enforcement
Given Alex holds no group rule for a candidate collection id
When Alex simulates any candidate filter against it
Then the response shows "rejected — no collection-group rule defined," matching US-04's real behavior

##### Scenario: Simulation has zero effect on live traffic
Given `journal_entries` has an active, published group rule
When Alex calls the simulation action with a different candidate group rule and filter
Then real callers' `collectionGroup()` calls continue to be evaluated against the published group rule, unaffected by the simulation

#### Acceptance Criteria
- [ ] AC-17-101: Simulating a candidate query filter against a candidate collection-group rule returns the same admit/reject outcome real enforcement would produce.
- [ ] AC-17-102: Simulation correctly reports "missing required filter" for candidate filters lacking a required conjunct.
- [ ] AC-17-103: Simulation correctly reports "no collection-group rule defined" when no candidate group rule is supplied, matching US-04's real default.
- [ ] AC-17-104: Simulating a group-query shape has zero effect on live/published `RunQuery` traffic.

#### Outcome KPIs
See § Outcome KPIs below (KPI #4 Leading).

#### Technical Notes (Optional)
Extends the existing `simulate_query_compliance` admin handler (ADR-031, `security-rules-query-path` US-07) with an optional "this is a group-rule simulation" input, calling the SAME `check_query_compliance()` function US-01–06 use — must not become a fourth, independently-maintained implementation.

---

## Wave: DISCUSS / [REF] Outcome KPIs

### Feature: security-rules-collection-group-rules

### Objective
Give Alex a genuine, independently-authored authorization surface for `collectionGroup()` querying — never silently inherited from a same-named single-location rule, never silently open — closing the residual, actively-exploitable gap `security-rules-query-path`'s own rule-lookup composition left in place for `all_descendants = true` queries.

### Outcome KPIs

| # | Who | Does What | By How Much | Baseline | Measured By | Type |
|---|---|---|---|---|---|---|
| 1 | SDK developers with a collection-group rule defined (e.g. Alex/Trailmark) | Have `RunQuery` with `all_descendants = true` correctly admit compliant group queries, reject non-compliant ones, and reject every group query against an ungoverned collection id — zero false-allows | 100% of `all_descendants = true` calls against a group-configured collection id produce the admit/reject decision this feature's locked mechanism implies; 0 false-allows for ungoverned collection ids or non-compliant group queries | 0% (capability does not exist today — group queries are checked, if at all, against the wrong rule) | Acceptance-scenario pass rate against the group-query truth table (admit/reject × governed/ungoverned × decidable/undecidable) | North Star |
| 2 | SDK developers whose group queries are rejected | Identify the rejection is group-query-shape-caused (not `authenticate()`, non-group compliance, or composite-index) from the response alone | ≥90% self-attributed without a support ticket | 0% (no such rejection class exists today) | Support-ticket tagging cross-referenced with rejection-response inspection | Leading |
| 3 | Existing `embyr-rs`/`client-auth`/`security-rules`/`security-rules-write-path`/`security-rules-query-path` customers and collections that never define a group rule | Continue using `GetDocument`, writes, and non-group queries successfully, unaffected | 0% regression across the 133 FINALIZED regression scenarios plus `security-rules-query-path`'s own delivered scenarios | Current 100% pass rate | Full regression suite, pre/post comparison | Guardrail |
| 4 | SDK developers authoring/testing collection-group rules and client query code | Catch a missing group rule or a client-code filter bug via simulation before it reaches a real end user | ≥1 caught per integration/testing cycle (qualitative, ramps with adoption); 0 confirmed data-exposure incidents traced to an un-simulated collection-group query | N/A (capability does not exist today) | Simulation-action usage log cross-referenced with post-publish rejection-rate anomalies | Leading |

---

## Wave: DISCUSS / [REF] Out of Scope

- **Resolution 1's Option A (auto-apply the same-named exact-path rule) and Option B (execute-time composition of per-path rules)** — explicitly rejected, not deferred; would require inventing execution-time path enumeration (the rejected antipattern) or accepting an affirmatively unsafe conflation.
- **Composite/parameterized collection-group rules that vary by nesting depth or parent path** — no domain example evidences this need; a group rule is a single condition applying uniformly to every nesting depth of a collection id, matching real Firestore's own model exactly.
- **Fixing the pre-existing, non-security `req.parent`-ignored bug in non-group `RunQuery`'s collection-path resolution** — flagged (§ Open Questions, OQ-SRCG-03), explicitly out of scope; a general RunQuery-correctness concern, not a security-rule authorization concern, and orthogonal to this feature's own right-sizing.
- **`security-rules-realtime`** (`Listen`/`onSnapshot` enforcement, including collection-group-backed listeners, Epic 2d) — unchanged, still separately deferred.
- **`security-rules-operations`** (rule history/versioning, richer grammar, audit log, Epic 2e) — unchanged.
- **Any change to `access_rules`, `write_access_rules`, `GetDocument`'s enforcement, write-path's enforcement, or non-group `RunQuery`'s enforcement** — this feature adds a new, disjoint table and a new call-site branch only; the three existing surfaces receive zero code changes.
- **Any change to `embyr-pg-storage`'s SQL-building layer, including its existing `all_descendants` branch** — confirmed unchanged and unneeded.
- **A conditional (rule-exists-somewhere) fail-closed default instead of the universal one** — considered and rejected in Resolution 2; flagged for orchestrator confirmation (§ Handoff Package), not built as an alternative here.

---

## Wave: DISCUSS / [REF] WS Strategy

Walking Skeleton Strategy: **B — Thin End-to-End Slice** (unchanged convention). Slices 01–06 are real, narrow vertical slices against real System DB rule state and real Maria/Dana sessions spanning genuinely nested collection paths (no facade, no mock) — Slice 01 proves the new disjoint storage; Slice 04 proves the highest-consequence reject-default early, independent of the admit-path mechanism; Slices 02–03 prove the admit/reject compliance mechanism reusing `check_query_compliance()` unmodified; Slice 05 proves structural independence from the 3 prior epics' own surfaces with the strongest possible contrast (both rule types active simultaneously with different conditions); Slice 06 proves the whole thing is additive to the FINALIZED baseline.

---

## Wave: DISCUSS / [REF] Driving Ports

| Port | Protocol | Extension |
|---|---|---|
| Admin port `:9090` (existing, extended) | HTTP/1.1 | New group-rule-definition action (US-01); `simulate_query_compliance`'s body extended with an optional candidate group-rule input (US-07 only) |
| Data ports `:8080` (gRPC) / `:8081` (REST/gRPC-Web) (existing, extended in observable behavior only) | gRPC / HTTP | `RunQuery`'s existing, unchanged call shape now additionally reflects collection-group compliance when `all_descendants = true` (US-01–06) — no new RPC or endpoint |

No new network-facing port introduced. Exact endpoint/action shapes are DESIGN's call.

---

## Wave: DISCUSS / [REF] Pre-requisites

- `docs/feature/security-rules-query-path/feature-delta.md` (full — the direct methodological precedent and the source of `check_query_compliance()`/`QueryComplianceOutcome`/`UnsatisfiedConjunct`, reused unchanged). **Dependency is on ADR-031's DESIGN decisions being stable (Accepted), not on `security-rules-query-path` being FINALIZED or fully DELIVERed** — confirmed not yet FINALIZED (no evolution doc; `git log`/`git status` confirm mid-DELIVER, only US-01 committed).
- `docs/product/architecture/adr-028-access-rule-storage-and-lifecycle.md`, `adr-030-write-path-grammar-storage-and-composition.md`, `adr-031-query-shape-compliance-check.md` (full — the exact machinery and precedent this feature composes with).
- `crates/embyr-core/src/domain/query.rs` (`StructuredQuery.all_descendants` — pre-existing field, confirmed unmodified by any of the 3 prior epics).
- `crates/embyr-pg-storage/src/backend_adapter.rs`'s `run_query` (confirms the SQL-building layer this feature does NOT need to touch — its `all_descendants` branch already works).
- `crates/embyr-server/src/grpc/handler.rs`'s `handle_run_query` (the exact wiring point this feature adds a branch to, confirmed by direct read of the shipped `security-rules-query-path` composition).
- `crates/embyr-server/src/adapters/system_db.rs` (`AccessRuleRow`/`WriteAccessRuleRow`, the direct adapter-method-shape precedent for `GroupAccessRuleRow`).
- `docs/product/jobs.yaml` (JOB-17, extended via NOTE — see § SSOT Updates).

---

## Wave: DISCUSS / [REF] Handoff Package

**To DESIGN (solution-architect)**: this `feature-delta.md` (journey delta + story map + user stories + embedded AC), 7 slice briefs (`docs/feature/security-rules-collection-group-rules/slices/slice-01-*.md` through `slice-07-*.md`), `docs/product/jobs.yaml` (JOB-17, extended via NOTE).

**To DEVOPS (platform-architect)**: § Outcome KPIs above (4 KPIs — 1 North Star, 2 Leading, 1 Guardrail).

**Explicit flags for DESIGN**:
1. § Job Discovery Framing Resolution's Resolution 1 (Option C — new, disjoint `group_access_rules` table) is LOCKED. Do NOT implement Option A (auto-apply the same-named exact-path rule) or Option B (execute-time composition) under any framing, including as an "interim MVP."
2. Resolution 2 (Option C — universal fail-closed when no group rule is defined, regardless of any same-named exact-path rule) is LOCKED. **This is the one genuine judgment call made without a live stakeholder** — flagged explicitly in the Resolution's own confidence note. DESIGN may proceed provisionally, but recommend the orchestrator confirm the universal (vs. conditional-on-exact-path-rule-existing) reading before treating it as unquestionably final.
3. `check_query_compliance()`/`QueryComplianceOutcome`/`UnsatisfiedConjunct` (ADR-031) are confirmed reusable completely UNCHANGED — no new decidable shape, no new evaluator branch, zero modification to `embyr_core::access_control` beyond the new table's adapter methods in `embyr-server`.
4. `access_rules`, `write_access_rules`, `GetDocument`'s enforcement, write-path's enforcement, and non-group `RunQuery`'s enforcement receive ZERO code changes — confirmed by direct code read, not assumed.
5. No change to `embyr-pg-storage`'s SQL-building/query-execution layer is needed — its `all_descendants` branch already correctly executes collection-group queries.
6. US-04's "no group rule defined → reject outright" default arm is this feature's single highest-consequence design risk (a false-allow, or an unsafe fallback to a same-named exact-path rule) — designated mutation-testing surface.
7. Regression baseline is the 133-scenario FINALIZED suite plus `security-rules-query-path`'s own delivered scenarios — confirmed via direct evidence that zero existing scenarios exercise `all_descendants = true`, so this feature's new, stricter group-query default costs zero regression risk against any confirmed-existing test.
8. A NEW ADR (032, next-free per `docs/product/architecture/adr-*.md` numbering) is expected for the collection-group rule storage/composition mechanism, mirroring ADR-027/028/029/030/031's pattern.
9. A pre-existing, non-security correctness gap (`req.parent` never combined with `collection_id` for non-group `RunQuery` calls, § Open Questions OQ-SRCG-03) was discovered during this DISCUSS's investigation and is explicitly OUT OF SCOPE for this feature — flagged for a separate bug-fix track, not silently folded in.

**Escalation flagged for the orchestrator, not resolved here**: given the genuine, stakeholder-unconfirmed judgment call in Resolution 2 (universal vs. conditional fail-closed default) and the security consequence of getting it wrong (a false-allow would reproduce the exact bypass this feature exists to close), **recommend the orchestrator either confirm Resolution 2's universal reading with the user before DESIGN treats it as unquestionably locked, or explicitly trigger `/nw-review nw-product-owner-reviewer` before DESIGN proceeds.** This mirrors `security-rules-query-path`'s own precedent for deviating from the default per-wave-review skip when the architectural consequence of an unconfirmed judgment call is high.

**Escalation resolved (orchestrator, 2026-08-25)**: confirmed directly against Firebase's own official documentation (`firebase.google.com/docs/firestore/security/rules-structure`, `rules-query`, and third-party PERMISSION_DENIED troubleshooting sources), not assumed. Real Firestore behavior: a regular per-exact-path `match /posts/{postid}` rule does **not** apply to collection-group queries at all — only an explicit collection-group-syntax rule (`match /{path=**}/posts/{doc}`) governs a collection-group query, and Firestore rejects a collection-group query outright ("Missing or insufficient permissions") whenever no such rule exists, **regardless of whether a same-named exact-path rule is defined elsewhere**. This is exactly Resolution 2's universal reading, not the conditional one — real Firestore never falls back to an exact-path rule for group queries. **Resolution 2 (universal fail-closed default) is now CONFIRMED, not merely locked provisionally — DESIGN may treat it as final.**

---

## Wave: DISCUSS / [REF] SSOT Updates

- `docs/product/jobs.yaml` — added a NOTE under JOB-17 (`document-access-control`) documenting that this feature is JOB-17's own collection-group-query realization within the query-path operation surface — not a new job, not a new operation surface. JOB-17's `job_story`/dimensions text is unchanged (historically accurate as written).
- `docs/product/journeys/sdk-developer.yaml` — extended with a NOTE clarifying JOB-17 now also covers collection-group query enforcement via this feature; `updated` date bumped. No new job id added.
- No new persona file — Trailmark's end users remain domain-example data, consistent with prior precedent.

---

## Wave: DISCUSS / [REF] Definition of Ready Validation

### Requirements Completeness Score: **0.96** (> 0.95 gate)

Computed across the three requirement categories:
- **Functional**: all 7 stories have complete Given/When/Then coverage of happy path, at least one edge case, and at least one error/failure path; the central architectural question (collection-group rule as a new, independent concept vs. composition) is explicitly locked (Resolution 1), and the ungoverned-collection default is explicitly locked (Resolution 2) — though Resolution 2's universal-vs-conditional scoping is honestly flagged as a judgment call, not silently asserted as fact (§ Handoff Package).
- **Non-functional**: security (the caller's-own-uid binding property inherited unchanged from ADR-031; the reject-by-default undecidable/ungoverned arms, AC-17-89/90/91) is explicit. A performance guardrail note (single indexed lookup against the new table only, no scan of `access_rules`, AC-17-91) is flagged, mirroring all 3 prior epics' own precedent — not a hard-numeric SLA at DISCUSS time.
- **Business rules**: the disjoint-storage requirement (Resolution 1), the universal ungoverned-default (Resolution 2), structural independence from the 3 existing surfaces (US-05), and the admit/reject truth table are all explicitly specified with examples.

**NFR note (carried forward)**: a collection id with no group rule must add effectively zero overhead beyond the existing unarmed query path — a single cheap existence-check against the new table, mirroring all 3 prior epics' own precedent. Not a hard-numeric SLA at DISCUSS time.

The remaining 0.04 gap is Resolution 2's universal-vs-conditional scoping uncertainty (§ Handoff Package flag 2) plus the Open Questions below — explicitly flagged, not hidden, and does not block this feature's own DoR, but is the specific reason this DISCUSS recommends an extra confirmation step before DESIGN (§ Handoff Package escalation).

### DoR Checklist (9-item hard gate)

| # | DoR Item | Status | Evidence |
|---|---|---|---|
| 1 | Problem statement clear, domain language | PASS | Every story's Elevator Pitch "Before" line is stated in Alex/Maria/Dana domain terms (e.g. US-04: "a caller who satisfies the top-level rule's own filter is admitted into a query whose result set also includes rows from the nested `expeditions/*/journal_entries` subcollection") |
| 2 | User/persona identified with specific characteristics | PASS | P1 Alex (unchanged); Maria Santos and Dana Kim as concrete rule-subject domain examples, continued |
| 3 | 3+ domain examples per story with real data | PASS | Every story has exactly 3 Domain Examples using `trailmark-prod`, real collection ids including nested `expeditions/trek-2026/journal_entries`, `maria-santos`/`dana-kim`, real field names |
| 4 | UAT scenarios in Given/When/Then (3–7 per story) | PASS | US-01: 4, US-02: 4, US-03: 4, US-04: 4, US-05: 4, US-06: 3, US-07: 4 — all within range |
| 5 | Acceptance criteria derived from UAT | PASS | Every AC (AC-17-77 through AC-17-104) traces 1:1 or 1:many to a specific scenario above it |
| 6 | Right-sized (1–3 days, 3–7 scenarios) | PASS | Largest slices (US-02/04) estimated 1.5 days / 4 scenarios; all others ≤4 scenarios, ≤1.5 days |
| 7 | Technical notes identify constraints | PASS | Every story's Technical Notes references the relevant locked constraint (disjoint storage, reused compliance function, no SQL-layer change) without prescribing implementation |
| 8 | Dependencies resolved or tracked | PASS | Dependency is on `security-rules-query-path`'s ADR-031 DESIGN decisions being stable (confirmed Accepted), not on that feature's own FINALIZATION or DELIVER completeness — explicitly distinguished, not conflated |
| 9 | Outcome KPIs defined with measurable targets | PASS | 4 KPIs, each with a numeric or explicitly-qualitative-with-rationale target, baseline, and measurement method |

### DoR Status: **PASSED** — with one explicit escalation flag (§ Handoff Package) recommended before DESIGN treats Resolution 2's universal scoping as final.

---

## Wave: DISCUSS / [REF] Open Questions

| ID | Question | Impact | Resolution owner |
|---|---|---|---|
| OQ-SRCG-01 | Whether Resolution 2's fail-closed default should be universal (any `all_descendants = true` query without a group rule is rejected) or conditional (rejected only when a same-named exact-path rule exists somewhere, otherwise unrestricted) | **CLOSED 2026-08-25** — confirmed universal, matching real Firestore's own documented behavior (regular exact-path rules never apply to collection-group queries; see § Handoff Package escalation resolution) | Orchestrator, via Firebase official docs |
| OQ-SRCG-02 | Whether a future evidenced need for per-nesting-depth-varying group rules (e.g. different conditions for different parent-path prefixes) will ever arise | Does not block this feature; would require its own DISCUSS/DESIGN pass, a genuinely larger undertaking than a single uniform condition per collection id | Product Discovery, triggered by future evidence |
| OQ-SRCG-03 | A pre-existing, non-security correctness gap discovered during this DISCUSS's investigation: non-group (`all_descendants = false`) `RunQuery` calls never combine `req.parent` with `collection_id`, so a query explicitly scoped to a specific parent document's subcollection may resolve to the wrong collection instance today | Does not block this feature's own DoR (orthogonal, non-security); flagged as a separate bug-fix candidate | Solution-architect / troubleshooter, on a separate track |
| OQ-SRQ-02 (carried, unrelated) | Whether OR-composed rules will need query support badly enough to justify UNION-of-queries execution machinery | Does not block this feature; unchanged from `security-rules-query-path` | Product Discovery |
| OQ-SR-04 (carried, unrelated) | String/number literal operands — confirmed booleans-only | Not reopened by this feature | Closed |

---

## Wave: DISCUSS / [REF] Wave Decisions Summary

### Key Decisions
- [D1] Locked collection-group rules as a NEW, independently-authored, independently-stored rule concept (Resolution 1, Option C) — rejecting Option A (auto-apply the same-named exact-path rule, confirmed affirmatively unsafe by direct construction) and Option B (execute-time composition, structurally intractable and not well-defined) — see § Job Discovery Framing Resolution.
- [D2] Locked a universal fail-closed default when no group rule is defined for a collection-group query (Resolution 2, Option C) — matches real Firestore's own behavior exactly, costs zero regression risk against any confirmed-existing scenario, but is flagged as a genuine judgment call for orchestrator confirmation.
- [D3] Confirmed `check_query_compliance()`/`QueryComplianceOutcome`/`UnsatisfiedConjunct` (ADR-031) are reusable completely unchanged — no new decidable shape, no new evaluator branch.
- [D4] Confirmed GetDocument, write-path, and non-group `RunQuery` have no gap of this feature's kind — both already resolve rule lookups from the actual document/query's own fully-derived exact collection path, never a bare leaf id — narrowing this feature's scope to `RunQuery` with `all_descendants = true` only.
- [D5] Extended JOB-17 rather than minting a new job — this feature closes a residual gap WITHIN the query-path operation surface `security-rules-query-path` itself did not fully close, not a new operation surface — see § Persona & Job.
- [D6] Scope Assessment PASS, 0/5 signals fired — this feature's code footprint is narrower than any of its 3 predecessors (zero new call sites, one new branch inside an already-existing one) — see § Scope Assessment.
- [D7] Discovered and flagged, explicitly out of scope: a pre-existing, non-security correctness gap in non-group `RunQuery`'s parent-path resolution (`req.parent` never combined with `collection_id`) — see § Open Questions, OQ-SRCG-03.

### Requirements Summary
- Primary jobs/user needs: Alex needs a genuine, independently-authored rule for `collectionGroup()` querying — never silently inherited from a same-named exact-path rule, never silently open when ungoverned — while every existing GetDocument, write, and non-group-query behavior remains provably byte-for-byte unaffected.
- Walking skeleton scope: define an independent group rule (US-01) → a compliant group query is admitted, spanning every nesting depth (US-02) → a non-compliant one is rejected before execution (US-03) → an ungoverned collection id is rejected outright, regardless of any same-named exact-path rule (US-04) → GetDocument/writes/non-group queries remain structurally unaffected (US-05) → untouched collections and the FINALIZED regression baseline are unaffected (US-06). Simulation (US-07) is Release 2.
- Feature type: Cross-cutting (as decided) — evidence confirms this feature is narrower in footprint than any of its 3 predecessors, touching zero new call sites.

### Constraints Established
- Collection-group rules are stored disjointly from `access_rules`/`write_access_rules`, keyed by `(project_id, collection_id)` alone.
- A collection-group query against an ungoverned collection id is always rejected, regardless of any same-named exact-path rule.
- `check_query_compliance()` and its companion types are reused completely unchanged.
- GetDocument, write-path, and non-group `RunQuery` receive zero code changes.
- No change to `embyr-pg-storage`'s SQL-building layer.

### Upstream Changes
- None — no DISCOVER/DIVERGE artifacts exist for this feature (same as all 3 prior epics); this DISCUSS is grounded directly in `security-rules-query-path`'s own shipped artifacts and ADR-031, direct reads of `crates/embyr-core/src/domain/query.rs`/`crates/embyr-core/src/domain/document.rs`/`crates/embyr-pg-storage/src/backend_adapter.rs`/`crates/embyr-server/src/grpc/handler.rs`/`crates/embyr-server/src/adapters/system_db.rs`/`crates/embyr-server/src/admin/handlers/access_rules.rs`, and `docs/product/jobs.yaml`.
