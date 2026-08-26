# security-rules-realtime — Feature Delta

**Wave**: DISCUSS | **Agent**: Luna (nw-product-owner) | **Date**: 2026-08-26
**Status**: Ready for DESIGN handoff (two flagged escalations — see § Handoff Package)
**Upstream**: `security-rules` (FINALIZED, `docs/evolution/2026-08-18-security-rules.md`) → `security-rules-write-path` (DISCUSS+DESIGN complete, not FINALIZED) → `security-rules-query-path` (DISCUSS+DESIGN complete, mid-DELIVER, not FINALIZED) → `security-rules-collection-group-rules` (DISCUSS complete, `docs/evolution/*.md` shows no evolution doc, not FINALIZED). This feature is Epic 2d of the Authorization initiative — `security-rules`' own `## Wave: DISCUSS / [REF] Out of Scope` section named it explicitly, quoted verbatim as this feature's own charter: **"Real-time Listen enforcement (`onSnapshot`/BC-3 fan-out gated by rules) — named, deferred follow-up epic (candidate id `security-rules-realtime`, 'Epic 2d'). BC-3's eventual-consistency, re-fetch-on-`DocChange` model is a different mechanism than a synchronous point-read check."** Direct investigation of `Listen`'s actual, shipped code (below) confirms that quote's own hedge was correct, and confirms two things the task's own framing suspected but did not assert: `Listen` currently supports only query-shaped targets (no `TargetType::Documents` handling exists at all), and `Listen`'s real-time fan-out mechanism is **more broken than merely "unprotected"** — it is not even scoped to the subscribed collection.

<!-- markdownlint-disable MD024 -->

---

## Wave: DISCUSS / [REF] Density Resolution

`~/.nwave/global-config.json` was not read — no Bash tool available to this agent invocation (per `nw-product-owner`'s tool grant: Read/Write/Edit/Glob/Grep/Task only) — falling back to the documented DISCUSS hard default: `mode=lean`, `expansion_prompt=ask-intelligent`, noted explicitly rather than silently assumed, mirroring all 4 prior sibling features' own precedent for this exact unavailability. Trigger check against this feature's own artifacts: "multi-stakeholder need" (Alex, Maria, Dana) fires but is answered by cross-reference per Decision 3 (Lightweight), exactly as 3 of the 4 priors did. "Cross-cutting complexity" (≥3 bounded contexts) **fires** — this feature genuinely touches BC-2 (query-filter translation reuse), BC-3 Real-Time Delivery (a genuinely new, BC-3-internal collection-scoping mechanism with no analog in any prior epic), and BC-4 Access Control (both `check_query_compliance()` and `evaluate()` reused) — see § Scope Assessment for why this does not, on its own, make the feature oversized. No other trigger fires. Tier-1 [REF] only, no Tier-2 expansions rendered — the cross-cutting-complexity trigger is answered inline in § Scope Assessment and § Job Discovery Framing Resolution rather than via a separate `alternatives-considered` expansion, consistent with Decision 3 (Lightweight).

---

## Wave: DISCUSS / [REF] Prior Wave Consultation — Reading Confirmation

✓ `docs/feature/security-rules-collection-group-rules/feature-delta.md` (full, 1053 lines, DISCUSS-only) — the most recent, most structurally relevant precedent: Reading Confirmation checklist format, Resolution-table methodology for the central architectural question, Elephant Carpaccio slicing, feature-delta.md single-narrative-file output, DoR/Outcome-KPI/Handoff-Package template all mirrored below. Confirms `check_query_compliance()`/`QueryComplianceOutcome`/`UnsatisfiedConjunct` (ADR-031) and the `group_access_rules` table (ADR-032) as the most recently-shipped machinery this feature might have reused — investigated below and found **not directly reusable for Listen's per-event problem**, a materially different finding from collection-group-rules' own "thin, low-risk extension" conclusion.
✓ `docs/feature/security-rules-query-path/feature-delta.md` (full DISCUSS section, 772 lines, plus targeted DESIGN-section reads for ADR-031's exact composition) — confirms the 5-shape decidable set, the `evaluate()`-not-reusable-for-RunQuery finding (no document fetched at query-planning time), and `handle_run_query`'s exact composition. This feature's own Resolution 2 below directly inverts query-path's own Resolution 2 finding for the per-event case — evidenced, not asserted, below.
✓ `docs/feature/security-rules-write-path/feature-delta.md` (full DISCUSS section, 912 lines) — confirms `evaluate()`'s two-resource-map signature (`resource_fields`, `request_resource_fields`) and its fail-closed-on-missing-field mechanism, reused unchanged for this feature's own per-event check (only `resource_fields` is meaningful for a Listen delivery; there is no "proposed new document" concept — see § System Constraints).
✓ `docs/feature/security-rules/feature-delta.md` (full DISCUSS section, ~500 lines read, plus targeted grep confirming § Out of Scope) — confirms `security-rules`'s own deferral of this feature, quoted verbatim above; confirms `handle_get_document` is BC-4's original, sole 2a call site and that `handle_listen` was explicitly named as one of the untouched handlers ("`RunQuery`, `CreateDocument`, `UpdateDocument`, `DeleteDocument`, `BeginTransaction`/`Commit`, and `Listen` are **not** gated by rules in this feature").
✓ `docs/product/architecture/adr-027-access-rule-grammar-and-evaluation.md` (full) — confirms `evaluate(condition, auth, resource_fields) -> EvaluationOutcome` is total, infallible, pure, zero-IO — the exact function this feature's own per-event check reuses unchanged.
✓ `docs/product/architecture/adr-028-access-rule-storage-and-lifecycle.md` (full) — confirms `access_rules`'s single-row-per-`(project_id, collection_path)` shape and `get_access_rule() -> None` short-circuit, the pattern this feature's own "no rule ⇒ unrestricted" guardrail (US-06) reuses.
✓ `docs/product/architecture/adr-029-access-control-composition-and-bounded-context.md` (full) — confirms BC-4's placement rationale and, critically, ADR-002's **original** BC-3 text this ADR quotes verbatim: *"BC-3's Listen handler re-fetches the full document via a BC-2 read (GetDocument) to work around the 8 KB NOTIFY payload cap"* — a "read-only, non-transactional dependency" ADR-002 already sanctioned as an accepted pattern. Direct code read below (`postgres_notify_listener.rs::fetch_event`) confirms the SHAPE holds (a document is genuinely re-fetched before fan-out) but the MECHANISM differs from the ADR's own paraphrase: the re-fetch is a raw SQL query inside BC-3's own adapter, not a literal call into BC-2's `handle_get_document` handler — noted as a discrepancy between the ADR's abstract description and the concrete implementation, not treated as a contradiction (the "read-only dependency on already-fetched data" property this feature needs is unaffected either way).
✓ `docs/product/architecture/adr-030-write-path-grammar-storage-and-composition.md` (full) — confirms `evaluate()`'s extended two-map signature and the "existence non-leakage extends via evaluate-unconditionally-against-real-or-empty-fields" mechanism this feature's own US-05 (delete-event non-leakage) reuses.
✓ `docs/product/architecture/adr-031-query-shape-compliance-check.md` (full) — confirms `check_query_compliance(condition, filter: Option<&QueryFilter>, auth) -> QueryComplianceOutcome` operates on a **query filter tree**, never a document — the exact evidence behind this feature's own Resolution 1, Option B rejection (it is not merely undesirable but structurally not well-typed to "run `check_query_compliance()` against a delivered document").
✓ `docs/product/architecture/adr-032-collection-group-rule-storage-and-composition.md` (full) — confirms `group_access_rules`, keyed `(project_id, collection_id)`, and confirms (§ Decision — Composition) that `handle_run_query`'s own `all_descendants` local is read directly from `sq_proto.from[0].all_descendants` — the proto field this feature's own investigation (below) confirms `Listen`'s own extraction path never reads at all.
✓ `docs/product/jobs.yaml` (full) — JOB-17's own `job_story` text ("enforce them on **every request**") and its 4 accumulated NOTEs (read/write/query/collection-group realizations) already frame this feature as JOB-17's fifth incremental realization, not a new goal — same evidence method all 4 priors used, confirmed below (§ Persona & Job), not merely asserted.
✓ `docs/product/journeys/sdk-developer.yaml` (full) — JOB-17 already listed for P1 Alex, extended with 4 prior NOTEs; extended below with a 5th, no new job id.
✓ `crates/embyr-core/src/domain/query.rs` (full, 71 lines) — confirms `StructuredQuery { collection_id, all_descendants, filter: Option<QueryFilter>, ..., since_update_time: Option<DateTime<Utc>> }`. `since_update_time` is a Listen-specific field already present in the domain model (used for resume-token delta delivery) — confirming the domain query type was already designed with Listen in mind, which makes `handle_add_target`'s own hardcoding of `filter: None`/`all_descendants: false` (below) a deliberate simplification of an already-Listen-aware type, not a type-level limitation.
✓ `crates/embyr-server/src/grpc/handler.rs` (targeted full reads: `handle_listen` lines 1366-1462, `translate_filter` lines 1670-1719, `extract_project_id_from_listen_request` lines 1589-1598) — **confirms directly**: `handle_listen` authenticates via `api_key`/checks suspension only; calls neither `attach_client_identity_if_present()` nor any `get_*_access_rule` method anywhere; spawns `crate::realtime::listen_handler::handle_add_target` and otherwise only drains subsequent stream messages. `translate_filter` (the exact function `handle_run_query` uses to build a domain `QueryFilter` from a proto `StructuredQuery.where_`) exists as a free function in this same file but is **never called anywhere in the `realtime` module** — confirmed by targeted search.
✓ `crates/embyr-server/src/realtime/listen_handler.rs` (full, 251 lines) — **confirms directly, the central evidence of this DISCUSS**: (a) `handle_add_target`'s target-type match (lines 53-60) handles ONLY `Some(TargetType::Query(qt))`; every other variant, including a direct document-path target, falls to `_ => return Err("target must have Query target type".into())` — `TargetType::Documents` is not handled at all today; (b) the constructed `domain_query` (lines 74-84) hardcodes `filter: None` and `all_descendants: false` — the client's `StructuredQuery.where_` clause and collection-group intent are both silently dropped, never extracted, never consulted; `collection_id_from_query_target` (lines 226-239) reads only `cs.collection_id`, nothing else from the `StructuredQuery`; (c) the main event loop (lines 142-220) forwards every `ListenEvent` received on `event_rx` directly to the client as a `DocumentChange`/`DocumentDelete`, with **no check anywhere that the event's own `collection_path` matches the subscriber's own subscribed `collection.collection_path`**.
✓ `crates/embyr-server/src/realtime/listen_registry.rs` (full, 109 lines) — **confirms directly**: `ListenRegistry` is keyed ONLY by the project-level NOTIFY channel name (`dc_<hex16>(project_id)`); `fan_out()` delivers every `ListenEvent` to **every** subscriber registered on that channel, with zero filtering by collection, document path, or query shape — the structural mechanism behind the cross-collection leak this feature's own US-01 closes.
✓ `crates/embyr-server/src/adapters/postgres_notify_listener.rs` (full, 154 lines) — **confirms directly**: `PostgresNotifyListener::start`'s background task calls `fetch_event()` — a real `SELECT fields, version, create_time, update_time FROM documents WHERE ... AND NOT deleted` — on **every** NOTIFY, before `registry.fan_out()` is ever called. This means a fully-fetched `FirestoreDocument` (with real field data) already exists in memory at the exact moment fan-out happens, for every `Changed` event — the opposite constraint from `RunQuery`'s own (query-path's Resolution 2: no document exists at query-planning time). For a `Removed` event, `fetch_event`'s own `AND NOT deleted` filter means no fields are fetched (confirmed: `documents` uses a soft-delete `deleted` boolean flag, `SET deleted = true` on delete, never a hard `DELETE` — confirmed via `crates/embyr-pg-storage/src/backend_adapter.rs:508,826` — meaning the pre-deletion fields technically still exist in storage and are fetchable by a query that does not filter `NOT deleted`, a DESIGN-level implementation option, not a DISCUSS-level blocker).
✓ `crates/embyr-server/src/realtime/mod.rs` (full, 6 lines) — module manifest only, no additional logic.
✓ `docs/evolution/2026-08-18-security-rules.md` (full) — confirms `security-rules` is the only FINALIZED prior epic; 133-scenario baseline this feature must not regress. `security-rules-write-path`, `security-rules-query-path`, and `security-rules-collection-group-rules` are NOT FINALIZED (no evolution docs exist for any of the three — confirmed via `docs/evolution/*.md` directory listing). This feature's dependency is on `security-rules-query-path`'s ADR-031 (`check_query_compliance()`) and `security-rules`/`security-rules-write-path`'s ADR-027/030 (`evaluate()`) being **stable, Accepted DESIGN decisions** — not on any of the three being FINALIZED or fully DELIVERed, mirroring `security-rules-collection-group-rules`'s own identical dependency framing.

No contradictions found between this feature's scope and any prior artifact's LOCKED decisions. This feature does not reopen any Resolution from `security-rules`, `security-rules-write-path`, `security-rules-query-path`, or `security-rules-collection-group-rules` — it composes two of their already-Accepted mechanisms in a new way, and additionally discovers and must resolve **two pre-existing, non-security-rules-caused structural gaps in `Listen`'s own delivery mechanism** (§ Job Discovery Framing Resolution, Findings 2 and 5) that no prior epic had occasion to discover, since none of `GetDocument`/writes/`RunQuery` has a fan-out-to-N-other-subscribers delivery model. Two genuinely new judgment calls are flagged, with their own confidence reasoning, not silently assumed — see § Handoff Package.

---

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

| # | Decision | Value |
|---|---|---|
| 1 | Feature Type | Cross-cutting — spans BC-2 (reuse of `translate_filter`), BC-3 Real-Time Delivery (a genuinely new, BC-3-internal collection-scoping mechanism — the first ADR to touch BC-3's own fan-out logic in this initiative), and BC-4 Access Control (both `check_query_compliance()` and `evaluate()` reused, composed together for the first time in one feature). **Evidence-based nuance**: unlike the 4 priors, this feature's "Cross-cutting" label is fully earned by direct code evidence, not merely a hedge — BC-3's own `ListenRegistry`/`PostgresNotifyListener` require real, new logic, not just a new call site into an already-existing mechanism. |
| 2 | Walking Skeleton | Evaluated (this wave's call): **YES** — see § Walking Skeleton Evaluation |
| 3 | UX Research Depth | Lightweight — backend/API extension of an already-researched feature, mirroring all 4 prior epics' own framing |
| 4 | JTBD Analysis | Yes (default) — extends `job_id: JOB-17` (see § Persona & Job) |

### Walking Skeleton Evaluation (Decision 2 = "Depends")

Three existing mechanisms were evaluated for reuse before concluding what this feature's walking skeleton actually needs to add:

1. **`embyr_core::access_control::check_query_compliance()` (ADR-031).** Reused **completely unchanged** for exactly one purpose: gating the initial snapshot at subscribe time, mirroring `RunQuery`'s own mechanism almost exactly — but ONLY once a real `QueryFilter` is actually being derived from the client's request (§ Job Discovery Framing Resolution, Finding 2). It is **not** reusable for the ongoing per-event delta stream — its signature takes a filter tree, never a document (confirmed directly, ADR-031).
2. **`embyr_core::access_control::evaluate()` (ADR-027/030).** Reused **completely unchanged** for exactly one purpose: re-checking each individually-delivered document-change event against the caller's rule, using the document `fetch_event()` already fetches before fan-out (Finding 6). This is the **inverse** of query-path's own finding: `evaluate()` was rejected for `RunQuery` because no document exists at query-planning time; here a document genuinely, cheaply exists at delivery time, by construction of how NOTIFY-driven fan-out already works.
3. **`ListenRegistry::fan_out()` / `PostgresNotifyListener`'s existing project-wide delivery model.** **Not reusable as-is for even the most basic correctness** — confirmed by direct code read (`listen_registry.rs`, `listen_handler.rs`): every subscriber on a project's NOTIFY channel receives every event for the ENTIRE project, with zero filtering by collection anywhere. This is not an access-control gap alone; it is a missing basic collection-scoping filter, a genuinely new mechanism this feature must add inside BC-3 itself.

**Verdict**: no existing mechanism (a) scopes Listen's live delivery to the subscriber's own collection, (b) derives a real query filter for Listen's initial snapshot, or (c) re-checks a delivered document's content against a rule using data Listen's own fan-out path already fetches. A walking skeleton is needed that touches genuinely more surface than any of the 4 priors: a new BC-3-internal collection-scoping filter (US-01), a fix to Listen's own filter-derivation (US-02), a subscribe-time compliance gate reusing `check_query_compliance()` unchanged (US-03), and a per-event content gate reusing `evaluate()` unchanged (US-04/05) — see § Story Map, Slices 01–07.

---

## Wave: DISCUSS / [REF] Job Discovery — Framing Resolution

This is the single most important output of this DISCUSS: the central architectural question the task asked to be surfaced, resolved with the same evidence discipline all 4 priors' own Resolutions established as this project's precedent — plus two additional findings this feature's own investigation surfaced that none of the 4 priors had occasion to discover.

### Foundational findings (evidence, not options — established before any Resolution can be evaluated)

| # | Finding | Evidence |
|---|---|---|
| **Finding 1** | `Listen` supports only query-shaped targets today. `TargetType::Documents` (a direct document-path target — real Firestore's OTHER target type) is not handled at all; `handle_add_target` returns an error for it. | `listen_handler.rs:53-60` |
| **Finding 2** | `Listen`'s initial snapshot **ignores the client's own query filter**. `domain_query.filter` is hardcoded `None` — `collection_id_from_query_target` never reads `sq.where_`. Every Listen subscription today returns the **entire, unfiltered collection** in its initial snapshot, regardless of what `.where()` clauses the client's SDK query specified. | `listen_handler.rs:74-84,226-239` |
| **Finding 3** | `Listen` ignores collection-group intent. `domain_query.all_descendants` is hardcoded `false` — `cs.all_descendants` (the same proto field `RunQuery`'s own `handle_run_query` reads, per ADR-032) is never consulted for Listen targets. | `listen_handler.rs:76`; cross-referenced against ADR-032's own confirmed `sq_proto.from[0].all_descendants` read in `handle_run_query` |
| **Finding 4** | `Listen` performs zero rule consultation of any kind. `handle_listen`/`handle_add_target` never call `attach_client_identity_if_present()`, `get_access_rule`, `get_write_access_rule`, or `get_group_access_rule`. | `handler.rs:1366-1462`; `listen_handler.rs` (full) |
| **Finding 5** | `Listen`'s ongoing live delivery is **not scoped to the subscribed collection at all**. `ListenRegistry::fan_out()` delivers every event for a project's NOTIFY channel to every subscriber on that channel; `listen_handler.rs`'s event loop forwards every received event without checking it against the subscriber's own `collection.collection_path`. A subscriber to `journal_entries` today receives live change events for `trip_photos`, `app_config`, or any other collection's writes in the same project too. | `listen_registry.rs:81-93`; `listen_handler.rs:163-203` |
| **Finding 6** | A fully-fetched `FirestoreDocument` (real field data) already exists in memory, before fan-out, for every `Changed` event — `fetch_event()` performs a real SQL fetch on every NOTIFY, before `registry.fan_out()` is ever called. This is the **opposite** constraint from `RunQuery`'s own (query-path's Resolution 2: no document exists at query-planning time). | `postgres_notify_listener.rs:57-75` |
| **Finding 7** | `documents` uses soft-delete (`SET deleted = true`, never a hard `DELETE`) — pre-deletion field data technically still exists in storage after a delete, fetchable by a query that omits the `NOT deleted` filter `fetch_event` currently applies for the removed case. | `backend_adapter.rs:508,826`; `postgres_notify_listener.rs:82-96` |

### Resolution 1 (THE central architectural question) — Does `Listen` reuse `check_query_compliance()` UNCHANGED, or does it need something structurally new?

| Option | Description | Fit against evidence |
|---|---|---|
| **(A) Reuse `check_query_compliance()` unchanged as `Listen`'s sole enforcement mechanism, applied once at subscribe time only** — mirrors `RunQuery`'s own mechanism exactly, no other change | **Rejected — insufficient by direct construction, not merely a smaller version of the correct answer.** Two independent reasons: (1) `check_query_compliance()` needs a real `QueryFilter` to check (Finding 2 — currently always `None`); applying it unchanged today would make EVERY Listen subscription against an ownership-equality rule fail closed, since the filter is structurally invisible, breaking legitimate use rather than fixing a bypass — Finding 2 must be fixed as a genuine prerequisite, not an optional nicety. (2) Even with Finding 2 fixed, a subscribe-time-only check does nothing for the ongoing NOTIFY-driven delta stream — Finding 5 means a compliant subscription's SUBSEQUENT live events are still delivered completely unscoped, from ANY collection in the project, rule or no rule. A subscribe-time-only gate would create a false sense of security: admitted at t=0, unprotected at every t>0. |
| **(B) Invent a wholly new, Listen-specific compliance function from scratch** — neither `check_query_compliance()` nor `evaluate()` reused | **Rejected — no evidence supports it, and it violates BC-4's own "extend, don't duplicate" discipline (ADR-029 DDD-SR-8, reused by every prior epic).** Listen's problem decomposes cleanly into two ALREADY-SOLVED sub-problems: (i) is a query's SHAPE compliant before it runs (exactly `RunQuery`'s own problem, once Finding 2 is fixed) and (ii) does a specific, already-fetched DOCUMENT satisfy a rule (exactly `GetDocument`'s own problem, and Finding 6 confirms a document genuinely, cheaply exists at Listen's own delivery time). Inventing a third, novel algorithm for either sub-problem, when two proven mechanisms already answer them, is unjustified complexity. |
| **(C) A composed mechanism: `check_query_compliance()` (ADR-031, UNCHANGED) gates the initial snapshot at subscribe time; `evaluate()` (ADR-027/030, UNCHANGED) re-checks each individually-delivered document at event time — PLUS a genuinely new, BC-3-internal collection-scoping filter with no analog in any prior epic (Finding 5's own fix)** | **Strongest fit — the only option that is (a) tractable given each existing function's actual signature, (b) closes ALL FOUR gaps (Findings 2, 4, 5, and the missing per-event re-check) rather than only the access-control gap, and (c) reuses, rather than duplicates, both of BC-4's existing compliance mechanisms.** This is genuinely "structurally new" relative to `RunQuery`'s own extension (a single new branch in one existing function) — it composes TWO existing BC-4 mechanisms in a way none of the 4 priors needed, and adds one wholly new BC-3-internal correctness mechanism the other three RPCs never needed because none of them has a fan-out-to-N-other-subscribers delivery model. |

**Resolution**: **(C) is locked.** This is **not** a thin, low-risk extension like `security-rules-collection-group-rules` turned out to be — the task's own framing anticipated this possibility explicitly, and direct investigation confirms it: `Listen`'s enforcement surface is structurally different from, and larger than, `RunQuery`'s own, because `Listen`'s underlying delivery mechanism (BC-3, project-wide NOTIFY fan-out) has correctness gaps `RunQuery`'s own request/response model never had reason to develop.

### Resolution 2 — Must the compliance decision be re-checked on every delivered event, or does a subscribe-time-only check suffice?

This is the SECOND, distinct question the task asked to be resolved — the "document's owner field could change after admission" hypothesis — resolved explicitly, not silently folded into Resolution 1.

| Option | Description | Fit against evidence |
|---|---|---|
| **(A) Once, at subscribe time only** | Compliance decided against the initial snapshot's query shape; every subsequent live-delivered event for that target is forwarded unconditionally | **Rejected.** Directly falsifiable by the task's own hypothesis, now confirmed structurally reachable: `journal_entries`'s `owner_id` field is a normal, mutable document field — nothing in `check_query_compliance()`'s own shape-only reasoning constrains what a LATER write (by a different caller, e.g. an admin correction or a data-migration script) does to that same document's `owner_id`. A subscribe-time-only check would continue streaming that document's post-change content to the ORIGINAL subscriber, who may no longer be entitled to see it — the exact silent-bypass failure mode this entire initiative exists to close, now recurring inside the one RPC most naturally suited to it (a long-lived stream, unlike a one-shot `RunQuery`). Real Firestore itself does not do subscribe-time-only enforcement either — Firestore re-evaluates Security Rules on every document a listener receives, not merely at listener creation. |
| **(B) Per-event only — skip the subscribe-time gate entirely, filter only the ongoing delta stream** | No `check_query_compliance()` call at all; every individual document (initial snapshot AND every subsequent event) is decided via `evaluate()` alone | **Rejected.** Gating the INITIAL SNAPSHOT via N individual `evaluate()` calls (one per document in the collection) reintroduces exactly the execute-then-filter antipattern `security-rules-query-path`'s own Resolution 1 Option B already rejected for `RunQuery` — fetching every document in a collection, including ones the caller has zero entitlement to see, before deciding to hide them, with the identical pagination/`COUNT()`-shape/wasted-I/O consequences that Resolution already enumerated. Rejecting a non-compliant SUBSCRIPTION outright before running any query at all — the cheap, fail-fast behavior every one of the 4 priors already established and real Firestore itself exhibits — would be abandoned for no evidenced benefit. |
| **(C) Both — subscribe-time `check_query_compliance()` gates initial admission (cheap, fail-fast); per-event `evaluate()` re-checks every subsequently delivered document (defense-in-depth, catches content drift)** | **Strongest fit, high confidence.** Directly evidenced by Finding 6: the per-event `evaluate()` re-check costs **zero additional I/O** beyond what `Listen`'s own NOTIFY-driven fan-out already performs (`fetch_event()` runs regardless of whether security rules exist) — this is not a new expensive mechanism bolted on, it is a pure, in-memory decision over data already resident in memory at exactly the point fan-out happens. This also matches real Firestore's own behavior precisely (rules re-evaluated on every listener delivery, not merely at subscribe time) and is the only option that correctly resolves the task's own owner-field-change hypothesis. |

**Resolution**: **(C) is locked.** Yes, per-event re-check is required, and it is cheap — the opposite conclusion from `RunQuery`'s own Resolution 2 (`evaluate()` NOT reusable, because no document exists at query time), evidenced here by the concrete, opposite structural fact (Finding 6) that a document IS already fetched at Listen's own delivery time, by construction.

**Confidence and escalation note**: HIGH confidence on both Resolutions — directly evidenced by reading the actual, shipped `listen_handler.rs`/`listen_registry.rs`/`postgres_notify_listener.rs` source (not inferred from the ADR's own abstract paraphrase alone), and independently cross-checked against real Firestore's own documented rule-re-evaluation behavior for listeners. **Two genuine judgment calls made without a live stakeholder are flagged, not silently asserted**: (1) whether Findings 2 and 5 (the filter-drop and cross-collection-leak bugs) belong INSIDE this feature's own scope, or should be split into a separate, prerequisite "Listen correctness" feature shipped first — this DISCUSS locks the "inside this feature" reading (§ Scope Assessment), reasoning that both are causally, not merely thematically, entangled with the enforcement mechanism this feature exists to build; (2) whether a `Removed` event for a document the subscriber's rule would have denied access to CAN be withheld given `fetch_event`'s own current `AND NOT deleted` filter, or whether DESIGN must additionally query the soft-deleted row's pre-deletion fields (Finding 7) to make that decision — this DISCUSS locks the OBSERVABLE requirement (existence non-leakage extends to Removed events) and leaves the mechanism to DESIGN, flagging Finding 7 as the concrete evidence such a mechanism is buildable. Recommend the orchestrator confirm judgment call (1) before DESIGN treats the feature's own boundary as unquestionably final (§ Handoff Package).

---

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: **P1 — Alex, SDK Developer** (existing persona, unchanged). No new persona work — Decision 3 (Lightweight) confirms Alex's authoring mental model is already established across 4 prior epics.

**Domain-example company**: **Trailmark**, continued. Collections: `journal_entries` (top-level — Maria's/Dana's private trip-journal entries, `owner_id` field, existing `security-rules` exact-path rule, continued as this feature's primary domain example); `trip_photos` (curator-owned shared galleries, AND-composed rule, from `security-rules-query-path`'s own US-04 — reused for this feature's AND-composition and content-drift domain examples); `trail_guides` (published content, `request.auth != null` rule); `app_config` (never protected by any rule).

**job_id decision (per Decision 4)**: **this feature extends JOB-17 (`document-access-control`) — it does not mint a new job.** Direct textual evidence: JOB-17's own `job_story` already says "enforce them **on every request**" — a `Listen` call is a request, and `onSnapshot()` subscriptions are the Firestore SDK surface that backs Trailmark's own real-time collaborative features (JOB-03, `live-sync`, an existing, unrelated job this feature does not touch or extend — JOB-17's own scope is authorization, not real-time delivery mechanics). JOB-17's own accumulated NOTEs (read-path, write-path, query-path, collection-group-query realizations) already establish the "make it real, one operation surface at a time" extension pattern this feature continues a fifth time. `security-rules`'s own § Out of Scope entry (quoted verbatim in this document's header) already reserved this feature's candidate id and framing — this is the strongest possible evidence of "same job, next increment," not a new goal.

**Opportunity scoring**: JOB-17's existing opportunity score (17, priority critical) is unchanged — this feature does not create a new job. The urgency case specifically for this feature is the highest of any of the 5 realizations to date: Alex who has already shipped a `journal_entries` read rule, a write rule, and confirmed both protect `GetDocument`/`CreateDocument`/`UpdateDocument`/`DeleteDocument`/`RunQuery` (via the 4 prior epics) has every reason to believe his real-time listeners are equally protected — nothing in any prior epic's own UX signals that `onSnapshot()`/`Listen` is a structurally different, currently-unprotected code path, and this feature's own investigation additionally reveals the unprotected path leaks MORE than just rule-gated content (it leaks across collections entirely, per Finding 5) — the single most severe finding of any of the 5 epics in this initiative.

---

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

Run before journey/story-map investment, per Phase 1.5, applied honestly given this feature's own genuinely larger footprint than any of its 4 predecessors.

| Signal | Threshold | This feature | Fired? |
|---|---|---|---|
| User stories | >10 | 8 (US-01 through US-08) | **NO** |
| Bounded contexts / modules | >3 | 3 — BC-2 (reuse of `translate_filter`, US-02), BC-3 Real-Time Delivery (a genuinely new collection-scoping mechanism inside `ListenRegistry`/`listen_handler.rs`, US-01/04/05 — the first ADR in this initiative to touch BC-3's own internals), BC-4 Access Control (both `check_query_compliance()` and `evaluate()` reused, US-03/04/05/06). **Confirmed by direct code read**: no new bounded context is needed — BC-3's existing "read-only, non-transactional dependency on already-fetched data" shape (ADR-002/029) already sanctions exactly this kind of extension. | **NO** (at 3, not exceeding) |
| Walking Skeleton integration points | >5 | 5 — fan-out collection filter (US-01) + initial-snapshot filter derivation (US-02) + subscribe-time compliance branch (US-03) + per-event `evaluate()` branch for `Changed` (US-04) + per-event non-leakage branch for `Removed` (US-05) | **NO** (at threshold, not exceeding — mirrors `security-rules-write-path`'s own identical "borderline, at threshold" precedent) |
| Estimated effort | >2 weeks | 8 slices, ~11 days total (§ Elephant Carpaccio Slices) — the largest of any of the 5 epics, but under the 10-business-day/2-week threshold | **NO** (at the edge, not exceeding — see rationale below) |
| Independent shippable outcomes | multiple | **NO** — Findings 2 and 5's own correctness fixes (US-01/02) are CAUSALLY, not merely thematically, prerequisite to Findings 3/4's enforcement mechanism (US-03/04/05) meaning anything: `check_query_compliance()` cannot decide compliance against a filter that is always `None` (US-02 must precede US-03), and per-event enforcement cannot mean anything if the delivered event might belong to an entirely different, unrelated collection (US-01 must precede US-04/05). This is a single walking skeleton with an unusually long dependency chain, not multiple parallel-shippable outcomes — mirrors all 4 priors' own "inseparable halves" reasoning, extended to a longer chain. |

**0 of 5 signals fired outright; 2 of 5 (bounded contexts, WS integration points) sit exactly at their threshold without exceeding it, and effort sits near but under the 2-week edge. Verdict: PASS — right-sized, but denser than any of the 4 predecessors.** This density is fully explained, not silently absorbed: this feature is the first in the initiative to discover that its own target RPC's underlying delivery mechanism (BC-3's project-wide fan-out) has structural correctness gaps unrelated to, but entangled with, access control — no prior epic's own target RPC (`GetDocument`, three write handlers, `RunQuery`) has a fan-out-to-N-subscribers delivery model, so none had occasion to discover a class of gap this feature's own investigation surfaced for the first time. The alternative — splitting Findings 2/5's correctness fixes into a separate, prerequisite feature — was considered and is flagged as a genuine judgment call for the orchestrator (§ Handoff Package), not silently decided; this DISCUSS's own reasoning for keeping them together is the causal (not merely sequential) dependency named above.

---

## Wave: DISCUSS / [REF] Journey — Real-Time Compliance Flow (Lightweight)

Per Decision 3, this is a short delta on the 4 priors' own journeys — not reproduced in full here.

### What's new in Alex's mental model

Alex now learns — via this feature's own investigation, not something any prior epic's UX ever suggested — that `onSnapshot()` is not merely "another RPC that happened to be unprotected until now" (the framing every prior epic used) but a **structurally different delivery mechanism** whose correctness (does my listener see only the collection I subscribed to? does it see only documents matching my own query filter?) was never fully guaranteed even before security rules existed. He learns that fixing this makes his existing, currently-shipped (but silently over-broad) Listen usage narrower, in addition to gating it by rule — a message this feature's own DISTILL-facing documentation should frame honestly as "your listeners get MORE correct, not just more restricted."

### Real-time compliance flow (extends `security-rules-query-path`'s own flow, composed with `security-rules`'s own per-document flow)

```
An AddTarget Listen message arrives for a collection
        │
   (authenticate()/attach_client_identity_if_present() — NEW call site
    for Listen, function itself unchanged, identical to RunQuery's own
    wiring)
        │
        ▼
   [US-02] Derive the real QueryFilter from the client's own StructuredQuery
   (reuses translate_filter(), previously never called from this module)
        │
        ▼
   Does this collection have a READ rule defined? (access_rules, unchanged
   from security-rules — the SAME row GetDocument/RunQuery already consult)
        │
   no ──────────────────────────┐                    yes
        │                        │                     │
        ▼                        │                     ▼
  [US-06] Subscription proceeds  │      [US-03] check_query_compliance()
  exactly as before this         │      (ADR-031, UNCHANGED) — is the
  feature shipped, EXCEPT the    │      initial snapshot's own filter shape
  Finding-2/5 correctness fixes  │      admitted?
  (US-01/02) still apply — no    │                     │
  rule ⇒ unrestricted CONTENT,   │        ┌────────────┴─────────────┐
  never unrestricted SCOPE       │       no                         yes
        │                        │        │                          │
        │                        │        ▼                          ▼
        │                        │  Subscription rejected    Initial snapshot runs,
        │                        │  outright, before any     [US-01] scoped to the
        │                        │  row is read               subscriber's own
        │                        │                            collection ONLY
        └────────────────────────┴──────────────────────────────────┬────────
                                                                       ▼
                                                    Live NOTIFY-driven delivery begins
                                                            │
                                                    [US-01] Is this event's own
                                                    collection_path the SAME
                                                    collection the subscriber
                                                    is subscribed to?
                                                            │
                                                   no ──────┴────── yes
                                                    │                │
                                                    ▼                ▼
                                              Event dropped,   [US-04/05] evaluate()
                                              never delivered  (ADR-027/030, UNCHANGED)
                                              (Finding 5 fix,  against the already-
                                              structural, not  fetched document — does
                                              rule-dependent)  THIS caller's rule admit
                                                                THIS document, right now?
                                                                        │
                                                              ┌─────────┴─────────┐
                                                             no                  yes
                                                              │                    │
                                                              ▼                    ▼
                                                        Event withheld,      Event delivered
                                                        never delivered
```

---

## Wave: DISCUSS / [REF] Story Map & Walking Skeleton

**Persona**: P1 Alex | **Goal**: Give Alex's real-time listeners the same two guarantees every other RPC in this initiative already has — genuinely scoped to what the caller asked for, genuinely gated by whatever rule the caller's collection carries — continuously, not just at subscribe time, while every existing GetDocument/write/query behavior remains provably byte-for-byte unaffected.

### Backbone

| A. Live Delivery Is Correctly Scoped | B. A Subscription Is Rule-Compliant | C. Every Delivered Event Stays Compliant |
|---|---|---|
| A subscriber's live delivery never crosses into another collection **[WS]** | A rule-protected collection's initial snapshot honors the caller's own query filter **[WS]** | Each individually-delivered change event is re-checked against the rule using the already-fetched document **[WS]** |
| | A compliant subscription is admitted; a non-compliant one is rejected before the initial snapshot runs **[WS]** | A delete event for a document the caller's rule would deny is not delivered **[WS]** |
| | A collection with no rule keeps subscribing exactly as before (content-wise), and GetDocument/writes/RunQuery remain unaffected **[WS]** | |

### Walking Skeleton

Maria subscribes to `journal_entries`, filtered by her own `owner_id` (Activity B): the initial snapshot honors that filter (Activity B, US-02) and, once `journal_entries` carries the existing ownership rule (`security-rules`), is admitted only because her filter satisfies it (Activity B, US-03). Her live stream never receives `trip_photos`/`app_config`/any other collection's changes (Activity A, US-01) — proven with a real concurrent write to an unrelated collection during her session. A later write (by Dana, or an admin script) that reassigns the `owner_id` of one of Maria's already-delivered documents to someone else causes the NEXT live event for that document to be withheld from Maria's stream, not delivered (Activity C, US-04); a delete of a document Maria's rule would deny access to is likewise withheld (Activity C, US-05). `trail_guides`/`app_config` (no rule) keep subscribing exactly as before, content-wise, while still benefiting from the Activity-A scoping fix; GetDocument/writes/RunQuery (group and non-group) remain provably unaffected (Activity B, US-06); the full 133+-scenario regression baseline is unaffected (US-07). No facade, real System DB rule state, real Maria/Dana signed-in sessions, real Postgres NOTIFY events — mirrors all 4 priors' own WS discipline exactly, extended across a longer dependency chain.

### Release 1 — Real-Time Delivery Is Honestly Scoped and Rule-Enforced (Slices 01–07, US-01 through US-07)

Outcome: any collection Alex protects with a rule genuinely gates `Listen`'s initial snapshot AND every subsequently delivered event, continuously; every listener — rule-protected or not — is correctly scoped to its own subscribed collection for the first time; every GetDocument/write/query behavior remains exactly as the 4 prior epics left it.

### Release 2 — Authoring Confidence, Extended to Listen (Slice 08, US-08)

Outcome: Alex can prove a candidate Listen subscription (rule + query filter) will be admitted before shipping it, extending all 4 priors' own simulation guarantee to the real-time case.

---

## Wave: DISCUSS / [REF] Elephant Carpaccio Slices

| Slice | Story | Release | Estimate | Learning Hypothesis (disproves) | Production-data taste test |
|---|---|---|---|---|---|
| 01 (WS) | US-01 | 1 | 1.5 days | A subscriber's live delivery cannot be scoped to their own subscribed collection without either introducing per-subscriber, not per-project, NOTIFY channels (a much larger BC-3 redesign) or requiring an expensive per-event DB round-trip beyond what `fetch_event()` already performs | Real Postgres NOTIFY events across ≥2 distinct collections in the same project, real Maria session subscribed to only one of them |
| 02 (WS) | US-02 | 1 | 1.5 days | `Listen`'s initial snapshot cannot honor the client's own query filter by reusing `translate_filter()` unchanged, without either duplicating filter-translation logic or requiring changes to the proto-to-domain translation layer itself | Real filtered `AddTarget` requests against a real multi-document collection, asserting the initial snapshot excludes non-matching documents |
| 03 (WS) | US-03 | 1 | 1.5 days | A Listen subscription cannot be proven compliant with a rule-protected collection's own rule, before the initial snapshot runs, by reusing `check_query_compliance()` completely unmodified | Real rule-protected `journal_entries`, real compliant and non-compliant `AddTarget` requests from real Maria/Dana sessions |
| 04 (WS) | US-04 | 1 | 2 days | An individually-delivered live change event cannot be re-checked against the caller's rule using the document `fetch_event()` already fetches, without either re-fetching a second time or requiring a new evaluation mechanism beyond `evaluate()` | Real NOTIFY-triggered write that changes a delivered document's `owner_id` mid-session, real assertion the next event for that document is withheld from the original subscriber |
| 05 (WS) | US-05 | 1 | 1.5 days | A delete event for a document the caller's rule would deny access to cannot be withheld without either leaking the document's prior existence or requiring a new non-leakage mechanism beyond the one `security-rules`/`security-rules-write-path` already established | Real delete of a document the subscriber's rule denies, real assertion no removal event reaches that subscriber |
| 06 (WS) | US-06 | 1 | 1.5 days | This feature's own enforcement mechanism cannot be proven structurally independent of GetDocument/writes/RunQuery (group and non-group), and cannot be proven to leave an unruled collection's CONTENT fully unrestricted (only its SCOPE newly correct), without exercising a collection carrying both an active rule and an active Listen subscription simultaneously | Real `journal_entries` with an active rule, real simultaneous GetDocument/write/RunQuery/Listen traffic against it |
| 07 (WS) | US-07 | 1 | 1.5 days | This feature's own new mechanism cannot be proven additive to the 133-scenario FINALIZED baseline plus the 3 non-finalized-but-DESIGN-stable priors' own delivered scenarios without actually re-running them unmodified | Real full regression suite, real multi-collection project state including collections with no rule and no active Listen subscription |
| 08 | US-08 | 2 | 1 day | A Listen-subscription simulation cannot share the exact same `check_query_compliance()` function real subscribe-time enforcement uses without duplicating (and risking drift in) the compliance logic a fifth time | Real candidate rules + real candidate query filters checked against the real compliance function, framed as a candidate Listen subscription |

**Total estimate: ~11 days.** The largest of any of the 5 epics in this initiative — explained transparently in § Scope Assessment, not silently absorbed: this feature uniquely required discovering and fixing two pre-existing, non-security-rules-caused structural gaps (Findings 2 and 5) in BC-3's own delivery mechanism, work no prior epic's own target RPC ever required.

**Taste tests applied**:
- "4+ new components per slice" — none exceeds 2 (Slice 01: new collection-match check inside the existing event loop, no new component; Slice 02: reuse of an existing free function [`translate_filter`] plus a new call site, no new component; Slice 03: new call-site branch reusing `check_query_compliance()` unchanged, no new component; Slice 04: new call-site branch reusing `evaluate()` unchanged, no new component; Slice 05: extends Slice 04's own branch, no new component; Slices 06–07: zero new components, pure proof obligations; Slice 08: thin wrapper over the existing simulation handler). PASS.
- "Every slice depends on a new abstraction" — no genuinely new abstraction is introduced anywhere in this feature (unlike `security-rules-collection-group-rules`'s own new table, or `security-rules-write-path`'s own new operand family) — every slice composes EXISTING BC-4 mechanisms (`check_query_compliance`, `evaluate`) with a structural fix inside BC-3's own already-existing `ListenRegistry`/`listen_handler.rs`. PASS — this feature is unusually free of new abstractions relative to its 4 predecessors, despite its larger footprint.
- "No slice disproves a pre-commitment" — each has a distinct, falsifiable hypothesis (see table). PASS.
- "Synthetic-data-only slices prove plumbing, not value" — N/A; all 8 slices require real System DB rule state, real signed-in sessions, and real Postgres NOTIFY events across genuinely multiple collections. PASS.
- "2+ slices identical except for scale" — none; each targets a distinct proof obligation (scope-fan-out / filter-honor / admit-subscription / re-check-changed-event / re-check-removed-event / prove-other-surfaces-unaffected / prove-regression-baseline-unaffected / simulate). PASS.

---

## Wave: DISCUSS / [REF] Prioritization

| Priority | Slice | Target Outcome | Rationale |
|---|---|---|---|
| 1 | Slice 01 (WS) | A subscriber's live delivery never crosses into another collection | Sequenced first deliberately — this is the single highest-consequence, most severe finding of this entire DISCUSS (a currently-shipped, project-wide cross-collection data leak, independent of whether any rule exists), and every later slice's own reasoning about "does this event belong to this subscriber" depends on this fix existing first |
| 2 | Slice 02 (WS) | `Listen`'s initial snapshot honors the caller's own query filter | Second-highest-consequence, and a genuine PREREQUISITE for Slice 03 — `check_query_compliance()` cannot decide anything meaningful against a filter that is always `None` |
| 3 | Slice 03 (WS) | A compliant subscription is admitted; a non-compliant one is rejected before the initial snapshot runs | Burns down the third-riskiest assumption — that `check_query_compliance()` composes cleanly into `handle_add_target`'s own async, streaming call shape, structurally different from `handle_run_query`'s request/response shape |
| 4 | Slice 04 (WS) | Each individually-delivered live change event is re-checked against the rule | The core, ongoing-enforcement half of this feature's own mechanism — sequenced after subscribe-time admission is proven, since it extends the same rule lookup already resolved for Slice 03 |
| 5 | Slice 05 (WS) | A delete event for a document the caller's rule would deny is not delivered | The other half of Slice 04's own mechanism, sequenced immediately after |
| 6 | Slice 06 (WS) | GetDocument/writes/RunQuery remain structurally unaffected; an unruled collection's content stays unrestricted | Proven directly against a collection carrying an active rule AND an active Listen subscription simultaneously — the strongest possible regression proof, sequenced once every enforcement mechanism (Slices 01–05) exists to make the contrast meaningful |
| 7 | Slice 07 (WS) | The full pre-existing regression baseline is unaffected | The single highest-consequence regression risk, sequenced last within the WS as a proof *over* Slices 01–06's real behavior |
| 8 | Slice 08 | Alex can pre-check a candidate Listen subscription | Highest-leverage for Alex's own confidence; depends on Slices 02–03's compliance mechanism existing to wrap |

---

## Wave: DISCUSS / [REF] System Constraints

- **`Listen`'s enforcement mechanism is LOCKED to Resolution 1's Option C** (§ Job Discovery Framing Resolution) — a composition of `check_query_compliance()` (subscribe-time, ADR-031, unchanged) and `evaluate()` (per-event, ADR-027/030, unchanged), plus a new, BC-3-internal collection-scoping filter. DESIGN must not implement Resolution 1's Option A (subscribe-time-only) under any framing, including as an "interim MVP" — Resolution 2 explicitly rejects subscribe-time-only as insufficient.
- **Per-event re-check is LOCKED to Resolution 2's Option C** — both a subscribe-time gate AND a per-event re-check are required. DESIGN must not omit either half.
- **`check_query_compliance()`, `QueryComplianceOutcome`, `UnsatisfiedConjunct`, and the 5-shape decidable set (ADR-031) are reused completely unchanged** for the subscribe-time gate. **`evaluate()` (ADR-027/030) is reused completely unchanged** for the per-event gate, called with `request_resource_fields` as an empty map (a Listen delivery has no "proposed new document" concept — mirrors `handle_get_document`'s own identical empty-map convention, ADR-030). No new decidable shape, no new evaluator branch, no modification to `embyr_core::access_control` anywhere in this feature.
- **This feature reads `access_rules` only — never `write_access_rules` or `group_access_rules`.** `Listen`'s v1 enforcement surface is non-group, query-shaped targets only (Finding 1: `TargetType::Documents` does not exist today; Finding 3: `all_descendants` is hardcoded `false` today). GetDocument's, write-path's, and RunQuery's (group and non-group) existing rule-lookup behavior receive ZERO code changes.
- **The collection-scoping fix (Finding 5, US-01) is structural, not rule-dependent** — it must hold for EVERY Listen subscription, including collections with no rule defined at all. This is the single highest-consequence design risk in this feature (worse than a false-allow on a ruled collection, since it currently affects every collection in every project, ruled or not) — designated mutation-testing surface (per-feature strategy, CLAUDE.md).
- **The second-highest-consequence risk**: the per-event `evaluate()` re-check (US-04) must fail closed identically to `GetDocument`'s own fail-closed-on-missing-field mechanism (ADR-027) — never crash, never silently deliver on an evaluation error.
- **No regression to `GetDocument`, writes, or `RunQuery` (group and non-group).** This feature's own new BC-3-internal mechanism and BC-4 call sites must have zero observable effect on any of the four existing surfaces — structurally, not just conventionally (a new, isolated call path inside `listen_handler.rs`/`listen_registry.rs`, never touching `handle_get_document`/`handle_create_document`/`handle_update_document`/`handle_delete_document`/`handle_run_query`'s own code).
- **The Finding-2/Finding-5 correctness fixes are IN SCOPE, LOCKED — not deferred to a separate feature.** § Job Discovery Framing Resolution's confidence note flags this as a genuine judgment call; this DISCUSS's own reasoning (causal, not merely sequential, dependency) is recorded and the orchestrator is asked to confirm before DESIGN treats it as unquestionably final (§ Handoff Package).
- **`TargetType::Documents` (direct document-path Listen targets) and collection-group Listen (`all_descendants = true`) remain out of scope** (Findings 1 and 3) — neither is reachable in shipped code today, so this feature's own rule-enforcement locks apply strictly to non-group, query-shaped targets, the entirety of what is mechanically reachable. Fixing Finding 3 (so a genuine `collectionGroup().onSnapshot()` becomes possible at all) is flagged as a separate, orthogonal correctness gap, out of scope here — mirroring `security-rules-collection-group-rules`'s own OQ-SRCG-03 precedent for an analogous, pre-existing, non-security correctness bug.
- Ubiquitous language introduced: **subscribe-time compliance** (whether a Listen subscription's initial snapshot query shape satisfies a rule, decided once, before any row is read), **per-event compliance** (whether an individually-delivered live document-change event still satisfies a rule, decided continuously, using the already-fetched document), **collection-scoped delivery** (a structural, rule-independent guarantee that a subscriber's live stream never contains another collection's events).

---

## Wave: DISCUSS / [REF] User Stories

### US-01: A Listen Subscriber's Live Delivery Never Crosses Into Another Collection

**job_id**: JOB-17
**Slice**: 01 (Walking Skeleton) | **Release**: 1

#### Elevator Pitch
Before: Maria's `onSnapshot()` subscription to `journal_entries` today, in shipped code, also receives live change events for `trip_photos`, `app_config`, or any other collection Trailmark's app happens to write to in the same project — a structural, project-wide leak with zero connection to any access rule.
After: call the SDK's existing `onSnapshot(collection(db,'journal_entries'), ...)` — an unchanged SDK method — during a session where a concurrent write happens to `trip_photos` in the same project → Maria's callback fires ONLY for `journal_entries` changes; the `trip_photos` write is never delivered to her stream.
Decision enabled: Alex can trust that a listener he scoped to one collection genuinely only sees that collection's own changes — the same basic scoping guarantee `RunQuery`/`GetDocument` have always structurally had, extended to Listen for the first time.

#### Domain Examples
1. **Happy Path**: Maria subscribes to `journal_entries`. While her session is open, a concurrent write creates a new `trip_photos` document in the same project. Her `onSnapshot()` callback never fires for it.
2. **Edge Case**: Maria and Dana both subscribe to `journal_entries` in the SAME project, at the same time. A write to Maria's own document delivers to both subscribers' streams (both are subscribed to `journal_entries` — this is correct, expected `journal_entries`-scoped delivery, not a leak); a write to `app_config` in the same project delivers to neither.
3. **Error/Boundary**: `app_config` (no rule of any kind) has an active Listen subscription open. A concurrent write to `journal_entries` (which DOES have a rule) never reaches the `app_config` subscriber, regardless of rule state on either side — the scoping fix is structural, not rule-dependent.

#### UAT Scenarios (BDD)

##### Scenario: A subscriber's live stream never receives another collection's change event
Given Maria Santos has an active `onSnapshot()` subscription to `journal_entries`
When a concurrent write creates a new document in `trip_photos` in the same project
Then Maria's stream never receives a `DocumentChange` event for that `trip_photos` write

##### Scenario: Two subscribers to the same collection both receive that collection's own events
Given Maria Santos and Dana Kim both hold active `onSnapshot()` subscriptions to `journal_entries`
When a write creates a new document in `journal_entries`
Then both Maria's and Dana's streams receive the corresponding `DocumentChange` event

##### Scenario: An unruled collection's subscriber never receives an unrelated ruled collection's events
Given a session holds an active `onSnapshot()` subscription to `app_config` (no rule defined)
When a concurrent write happens to `journal_entries` (an active rule defined) in the same project
Then the `app_config` subscriber's stream never receives an event for the `journal_entries` write

##### Scenario: A document-delete event is also correctly scoped
Given Maria Santos has an active `onSnapshot()` subscription to `journal_entries`
When a concurrent delete happens to a `trip_photos` document in the same project
Then Maria's stream never receives a `DocumentDelete` event for that `trip_photos` delete

#### Acceptance Criteria
- [ ] AC-17-105: A Listen subscriber's live stream never receives a `DocumentChange` event whose own `collection_path` differs from the subscriber's own subscribed collection.
- [ ] AC-17-106: A Listen subscriber's live stream never receives a `DocumentDelete` event whose own `collection_path` differs from the subscriber's own subscribed collection.
- [ ] AC-17-107: Two subscribers to the SAME collection both correctly receive that collection's own events — the fix narrows delivery to the right collection, it does not narrow it further than the subscriber's own scope.
- [ ] AC-17-108: The collection-scoping guarantee holds identically regardless of whether either collection involved has any access rule defined — this is a structural, rule-independent correctness fix.

#### Outcome KPIs
See § Outcome KPIs below (feeds KPI #1, North Star).

#### Technical Notes (Optional)
Suggested: compare the delivered `ListenEvent`'s own document path (`FirestoreDocument.path.collection_path` for `Changed`, `DocumentPath.collection_path` for `Removed`) against the subscriber's own `collection.collection_path` inside `listen_handler.rs`'s existing event-consumption loop, before forwarding to the client — no change to `ListenRegistry`'s own fan-out mechanism is required if the filter is applied on the CONSUMING side; whether the filter instead belongs inside `ListenRegistry::fan_out()` itself (per-subscriber filtering at the registry) or inside `listen_handler.rs`'s own loop (per-event filtering after receipt) is DESIGN's call.

---

### US-02: A Rule-Protected Collection's Initial Snapshot Honors the Caller's Own Query Filter

**job_id**: JOB-17
**Slice**: 02 (Walking Skeleton) | **Release**: 1

#### Elevator Pitch
Before: Maria's `onSnapshot(query(collection(db,'journal_entries'), where('owner_id','==',auth.currentUser.uid)))` today, in shipped code, ignores her own `.where()` clause entirely — the initial snapshot returns EVERY document in `journal_entries`, from every user, regardless of the filter she wrote.
After: call the identical, unchanged SDK query → the initial snapshot returns ONLY documents matching her own filter, exactly as `RunQuery`'s own filter-honoring behavior already works.
Decision enabled: Alex's real-time list screens finally behave the way his client-side query code has always claimed they would — a prerequisite this feature's own subscribe-time compliance check (US-03) structurally depends on.

#### Domain Examples
1. **Happy Path**: Maria subscribes to `journal_entries` filtered by `owner_id == "maria-santos"`. The initial snapshot returns only her own entries, not Dana's.
2. **Edge Case**: Maria's subscription adds a second, unrelated filter (`archived == false`) alongside the ownership one. The initial snapshot correctly narrows by BOTH filters.
3. **Error/Boundary**: Maria subscribes with NO filter at all to `journal_entries` (a collection with no rule defined). The initial snapshot returns the full, unfiltered collection — correct, since an absent filter on an unruled collection is not itself a bug, only a genuinely-specified filter being silently dropped is.

#### UAT Scenarios (BDD)

##### Scenario: The initial snapshot honors a single equality filter
Given Maria Santos subscribes to `journal_entries` filtered by `owner_id == "maria-santos"`
And `journal_entries` contains entries from both Maria and Dana
When the initial snapshot is delivered
Then it contains only Maria's own entries

##### Scenario: The initial snapshot honors a composite AND filter
Given Maria Santos subscribes to `journal_entries` filtered by `owner_id == "maria-santos"` AND `archived == false`
When the initial snapshot is delivered
Then it contains only Maria's own, non-archived entries

##### Scenario: An unfiltered subscription to an unruled collection returns the full collection, unchanged from today
Given a session subscribes to `app_config` (no rule, no filter specified)
When the initial snapshot is delivered
Then it contains every `app_config` document, exactly as before this feature shipped

##### Scenario: Filter translation reuses the identical function RunQuery already uses
Given a Listen subscription's `StructuredQuery.where_` clause is syntactically identical to a `RunQuery`'s own `where_` clause
When both are translated into a domain `QueryFilter`
Then they produce byte-identical `QueryFilter` values, confirming no second, independently-maintained translation path exists

#### Acceptance Criteria
- [ ] AC-17-109: A Listen subscription's initial snapshot narrows by every filter specified in the client's `StructuredQuery.where_`, matching `RunQuery`'s own filter-honoring behavior for an identical filter shape.
- [ ] AC-17-110: Composite AND filters are honored in full, not partially.
- [ ] AC-17-111: A subscription specifying no filter at all continues to return the full collection, unchanged from pre-feature behavior.
- [ ] AC-17-112: Filter translation reuses `translate_filter()` (the exact function `RunQuery` already uses) — no second, independently-maintained filter-translation path is introduced.

#### Outcome KPIs
See § Outcome KPIs below (feeds KPI #1, North Star, and KPI #2).

#### Technical Notes (Optional)
`handle_add_target` must extract the full `StructuredQuery` (not merely `sq.from[0].collection_id`) and call `translate_filter()` (currently a free function in `grpc/handler.rs`, may need visibility widened to `pub(crate)` for the `realtime` module to call it) to build `domain_query.filter`, replacing the current hardcoded `None`.

---

### US-03: A Listen Subscription Against a Rule-Protected Collection Is Admitted Only if the Initial Snapshot's Filter Is Compliant

**job_id**: JOB-17
**Slice**: 03 (Walking Skeleton) | **Release**: 1

#### Elevator Pitch
Before: nothing distinguishes "Maria subscribed correctly, filtered to her own entries" from "Maria subscribed to everyone's entries and the server quietly decided what to stream her" — `journal_entries`'s existing ownership rule (`security-rules`) has never had any bearing on `Listen` at all.
After: call `onSnapshot(query(collection(db,'journal_entries'), where('owner_id','==',auth.currentUser.uid)))` — now checked against the published rule before the initial snapshot ever runs → the subscription is admitted and the stream opens; the identical call with no filter, or a wrong-value filter, is rejected outright, before any row is read.
Decision enabled: Alex knows the exact rule that already protects `GetDocument`/`RunQuery` also protects `Listen` — the same "reuse the rule you already trust" guarantee this entire initiative has built toward, now extended to the real-time case.

#### Domain Examples
1. **Happy Path**: Maria subscribes to `journal_entries` filtered by `owner_id == "maria-santos"`, satisfying the collection's ownership rule. Admitted; the stream opens.
2. **Edge Case**: Maria subscribes with no filter at all to `journal_entries`. `check_query_compliance()` rejects the subscription outright, before the initial snapshot runs — identical mechanism to `RunQuery`'s own US-02.
3. **Error/Boundary**: Dana subscribes to `journal_entries` filtered by `owner_id == "maria-santos"` — a filter on the right field, bound to Maria's id, not her own. Rejected — the filter's value doesn't match Dana's own verified `request.auth.uid`, the identical load-bearing property `security-rules-query-path`'s own AC-17-51 established.

#### UAT Scenarios (BDD)

##### Scenario: A subscription with the required matching filter is admitted
Given `journal_entries` has an active rule requiring `request.auth.uid == resource.data.owner_id`
And Maria Santos holds a verified identity
When Maria subscribes to `journal_entries` filtered by `owner_id == "maria-santos"`
Then the subscription is admitted and the stream opens

##### Scenario: A subscription with no filter is rejected before the initial snapshot runs
Given the same rule as above
When Maria subscribes to `journal_entries` with no filter
Then the subscription is rejected, and no document row is read for the initial snapshot

##### Scenario: A filter bound to someone else's identity is rejected
Given the same rule as above
And Dana Kim holds a verified identity distinct from `"maria-santos"`
When Dana subscribes to `journal_entries` filtered by `owner_id == "maria-santos"`
Then the subscription is rejected, attributable to the rule

##### Scenario: check_query_compliance is reused completely unmodified for Listen subscriptions
Given a rule using any of the 5 decidable shapes `security-rules-query-path` already locked
When any caller subscribes with a matching or non-matching filter
Then the admit/reject outcome is identical in mechanism to `RunQuery`'s own equivalent case, with no new decidable shape introduced

#### Acceptance Criteria
- [ ] AC-17-113: A Listen subscription whose filter satisfies a rule-protected collection's own rule is admitted; the stream opens and the initial snapshot runs.
- [ ] AC-17-114: A Listen subscription whose filter does not satisfy the rule is rejected outright, before any row is read for the initial snapshot.
- [ ] AC-17-115: `check_query_compliance()`, `QueryComplianceOutcome`, and `UnsatisfiedConjunct` (ADR-031) are reused completely unmodified for Listen subscriptions — no new decidable shape, no new evaluator branch.
- [ ] AC-17-116: The rejection is delivered as a terminal stream error, distinguishable from `authenticate()`-level rejections.

#### Outcome KPIs
See § Outcome KPIs below (KPI #1, North Star).

#### Technical Notes (Optional)
`handle_add_target` gains a rule lookup (`get_access_rule`, unchanged) immediately after `domain_query`/`collection` are built (US-02), before the initial snapshot's `adapter.run_query()` call; `None` → proceed exactly as today (US-06). `Some` → call `check_query_compliance()` exactly as `handle_run_query` already does — same function, same types, a new call site inside an async streaming handler rather than a request/response one.

---

### US-04: Each Individually-Delivered Live Change Event Is Re-Checked Against the Rule Using the Already-Fetched Document

**job_id**: JOB-17
**Slice**: 04 (Walking Skeleton) | **Release**: 1

#### Elevator Pitch
Before: even a perfectly-scoped, perfectly-admitted subscription (US-01–03) has no ongoing protection — if a document's `owner_id` changes after Maria's subscription was admitted, her stream keeps receiving that document's updates forever, regardless of who owns it now.
After: a concurrent write reassigns `journal_entries/maria-trip-042`'s `owner_id` from `"maria-santos"` to `"dana-kim"` while Maria's stream is open → the NEXT live update for that document is evaluated against Maria's own rule using the document `fetch_event()` already fetched, and is withheld from her stream, not delivered.
Decision enabled: Alex knows his real-time listeners stay honest for the LIFETIME of the subscription, not just at the moment it opened — the same continuous guarantee real Firestore's own rule re-evaluation gives him.

#### Domain Examples
1. **Happy Path**: Maria's subscription is open and admitted (US-03). A write updates her own `journal_entries/maria-trip-042` (still `owner_id: "maria-santos"`, just a `title` change). `evaluate()` re-confirms the rule is satisfied; the update is delivered.
2. **Edge Case**: A write reassigns `journal_entries/maria-trip-042`'s `owner_id` to `"dana-kim"` while Maria's stream is open. `evaluate()` now denies (the document's own content no longer satisfies Maria's rule); the event is withheld from Maria's stream.
3. **Error/Boundary**: A `journal_entries` document is missing its `owner_id` field entirely (legacy data). Any live update to it fails closed via `evaluate()`'s own existing fail-closed-on-missing-field mechanism (ADR-027) — never a crash, never delivered.

#### UAT Scenarios (BDD)

##### Scenario: A live update to a document that still satisfies the caller's rule is delivered
Given `journal_entries` has an active rule requiring `request.auth.uid == resource.data.owner_id`
And Maria Santos has an admitted, open subscription to `journal_entries` filtered by `owner_id == "maria-santos"`
And `journal_entries/maria-trip-042` has `owner_id: "maria-santos"`
When a write updates that document's `title` field only
Then Maria's stream receives the corresponding `DocumentChange` event

##### Scenario: A live update that reassigns ownership is withheld from the now-unentitled original subscriber
Given the same rule and open subscription as above
When a write updates `journal_entries/maria-trip-042`'s `owner_id` to `"dana-kim"`
Then Maria's stream does not receive a `DocumentChange` event for that write

##### Scenario: A missing referenced field fails closed, never crashes
Given `journal_entries` has the same rule
And a `journal_entries` document exists with no `owner_id` field at all
When a write updates that document while a subscriber's stream is open
Then no `DocumentChange` event is delivered for it, and no internal error or crash occurs

##### Scenario: evaluate() is reused completely unmodified for Listen's per-event check
Given a rule using the same grammar `evaluate()` already supports for GetDocument/writes
When a live change event for a document governed by that rule is delivered or withheld
Then the decision mechanism is identical to `GetDocument`'s own equivalent evaluation, with `request_resource_fields` passed as an empty map

#### Acceptance Criteria
- [ ] AC-17-117: A live change event for a document that still satisfies the subscriber's own rule is delivered.
- [ ] AC-17-118: A live change event for a document that no longer satisfies the subscriber's own rule (content changed since admission) is withheld, not delivered.
- [ ] AC-17-119: A live change event referencing a document with a missing rule-referenced field fails closed (withheld), never crashes.
- [ ] AC-17-120: `evaluate()` (ADR-027/030) is reused completely unmodified for the per-event check, called with `request_resource_fields` as an empty map — no new evaluation function, no new grammar.
- [ ] AC-17-121: The per-event re-check adds zero additional I/O beyond what `fetch_event()` already performs for every NOTIFY-driven delivery.

#### Outcome KPIs
See § Outcome KPIs below (KPI #1, North Star — the per-event re-check is this story's entire purpose).

#### Technical Notes (Optional)
The per-event `evaluate()` call belongs inside `listen_handler.rs`'s own event-consumption loop (`Some(ListenEvent::Changed(doc)) => { ... }` arm), consuming the SAME `FirestoreDocument` already resident in memory from `fetch_event()` — no second fetch. This is this feature's single highest-consequence arm alongside US-01's own collection-scoping fix; designated mutation-testing surface (per-feature strategy, CLAUDE.md).

---

### US-05: A Delete Event for a Document the Subscriber's Rule Would Deny Is Not Delivered

**job_id**: JOB-17
**Slice**: 05 (Walking Skeleton) | **Release**: 1

#### Elevator Pitch
Before: even a correctly-scoped, correctly-admitted, per-event-re-checked subscription (US-01–04) has no defined behavior for the delete case — real-time delete notifications currently carry no field data at all, so nothing decides whether the caller was ever entitled to know that document existed.
After: a document Maria's own rule would deny her access to is deleted by another caller → Maria's stream never receives a `DocumentDelete` event for it — mirroring the same existence non-leakage guarantee `GetDocument`/writes already give her.
Decision enabled: Alex knows a real-time listener never learns about the lifecycle of a document — creation, change, OR deletion — that the caller was never entitled to see in the first place.

#### Domain Examples
1. **Happy Path**: Maria's subscription is open and admitted. A document she OWNS (`owner_id: "maria-santos"`) is deleted (by herself, via another session, or by an admin action). Her stream receives the `DocumentDelete` event.
2. **Edge Case**: A document owned by Dana (`owner_id: "dana-kim"`) — one Maria's own rule would never have admitted her to see — is deleted. Maria's stream does not receive a `DocumentDelete` event for it (it was never delivered to her in the first place, and its removal is likewise never delivered).
3. **Error/Boundary**: A document's `owner_id` was reassigned FROM Maria TO Dana (US-04's own scenario) and is subsequently deleted while Maria's stream is still open. Maria's stream does not receive the delete event — she lost entitlement before the deletion happened, and the deletion itself reveals nothing new to her.

#### UAT Scenarios (BDD)

##### Scenario: A delete of an owned document is delivered to the owning subscriber
Given `journal_entries` has an active rule requiring `request.auth.uid == resource.data.owner_id`
And Maria Santos has an admitted, open subscription to `journal_entries` filtered by `owner_id == "maria-santos"`
And `journal_entries/maria-trip-042` has `owner_id: "maria-santos"`
When that document is deleted
Then Maria's stream receives a `DocumentDelete` event for it

##### Scenario: A delete of a document the subscriber's rule would deny is not delivered
Given the same rule and subscription as above
And `journal_entries/dana-trip-007` has `owner_id: "dana-kim"`
When that document is deleted
Then Maria's stream does not receive a `DocumentDelete` event for it

##### Scenario: A document whose ownership was reassigned away from the subscriber does not deliver its later deletion
Given the same rule as above
And `journal_entries/maria-trip-042` had its `owner_id` reassigned to `"dana-kim"` while Maria's subscription remained open (US-04)
When that document is subsequently deleted
Then Maria's stream does not receive a `DocumentDelete` event for it

##### Scenario: A content-blind rule's delete events are delivered without needing field data
Given `trail_guides` has a rule requiring only `request.auth != null`
And Dana Kim, signed in, has an admitted, open subscription to `trail_guides`
When a `trail_guides` document is deleted
Then Dana's stream receives the `DocumentDelete` event, decided without needing the deleted document's own field data

#### Acceptance Criteria
- [ ] AC-17-122: A delete of a document the subscriber's own rule would have admitted is delivered as a `DocumentDelete` event.
- [ ] AC-17-123: A delete of a document the subscriber's own rule would have denied is not delivered.
- [ ] AC-17-124: A document that became non-compliant for a subscriber before deletion (US-04) does not deliver its subsequent deletion to that subscriber either.
- [ ] AC-17-125: A content-blind rule's (auth-presence-only, public) delete events are decided and delivered correctly without requiring the deleted document's own field data.

#### Outcome KPIs
See § Outcome KPIs below (KPI #1 North Star, KPI #3 Guardrail).

#### Technical Notes (Optional)
Given the `documents` table's own soft-delete mechanism (`deleted = true`, fields preserved — Finding 7), DESIGN's own implementation may fetch the pre-deletion field snapshot (a query omitting `fetch_event`'s current `AND NOT deleted` filter, specifically for the tombstone case) to run `evaluate()` against real content for a content-referencing rule; a content-blind rule (auth-presence-only, public) needs no such fetch at all, since `evaluate()`'s own decision does not depend on `resource.data` for those shapes. Exact mechanism is DESIGN's call — this story locks the OBSERVABLE requirement only.

---

### US-06: GetDocument, Writes, and RunQuery Remain Unaffected, and an Unruled Collection's Content Stays Unrestricted

**job_id**: JOB-17
**Slice**: 06 (Walking Skeleton) | **Release**: 1

#### Elevator Pitch
Before: Alex worries that adding real-time enforcement might change how `GetDocument`, writes, or `RunQuery` behave, or might make an unruled collection's Listen subscriptions newly content-restricted (rather than merely correctly-scoped, US-01).
After: call the existing SDK `getDoc()`, `updateDoc()`, `getDocs()` (group and non-group), and `onSnapshot()` on `journal_entries` (which carries an active rule) and on `app_config` (which never has) → all four surfaces on `journal_entries` continue to be governed exclusively by their own already-established mechanisms, unaffected by this feature's own new code; `app_config`'s Listen subscription is correctly scoped (US-01) but its CONTENT remains fully unrestricted, exactly as before.
Decision enabled: Alex can adopt real-time enforcement without re-verifying every other surface he already trusts, and without fearing an unruled collection's real-time behavior becomes newly restrictive.

#### Domain Examples
1. **Happy Path**: `journal_entries` has an active read rule. Maria's `getDoc()`, her non-group `RunQuery`, her `collectionGroup()` query (from `security-rules-collection-group-rules`, if a group rule exists), and her writes continue exactly as those 4 prior epics left them — unaffected by this feature's own new Listen-specific code.
2. **Edge Case**: `app_config` has never had any rule. A Listen subscription to `app_config` is correctly scoped (US-01 — no cross-collection leak) but its CONTENT remains fully unrestricted — every document is delivered, exactly as before this feature shipped.
3. **Error/Boundary**: This feature's own new call sites (`listen_handler.rs`'s rule lookups and evaluation calls) are structurally isolated from `handle_get_document`/`handle_create_document`/`handle_update_document`/`handle_delete_document`/`handle_run_query` — none of those five handlers is modified by this feature.

#### UAT Scenarios (BDD)

##### Scenario: GetDocument is unaffected by this feature's own new code
Given `journal_entries` has an active read rule, unaffected by this feature
When Maria calls `getDoc()` on her own entry
Then the read succeeds exactly as `security-rules` already left it

##### Scenario: Writes are unaffected by this feature's own new code
Given `journal_entries` has an active write rule, unaffected by this feature
When Maria creates a new entry with her own `owner_id`
Then the create succeeds exactly as `security-rules-write-path` already left it

##### Scenario: RunQuery (group and non-group) is unaffected by this feature's own new code
Given `journal_entries` has active read and group rules, unaffected by this feature
When Maria issues a non-group and a `collectionGroup()` `RunQuery`, each satisfying its own rule
Then both succeed exactly as `security-rules-query-path`/`security-rules-collection-group-rules` already left them

##### Scenario: An unruled collection's Listen content remains fully unrestricted
Given `app_config` has never had any rule
When any session subscribes to `app_config`
Then every `app_config` document is delivered in the initial snapshot and every subsequent live event, exactly as before this feature shipped — only the subscription's CROSS-COLLECTION scoping (US-01) is newly correct

#### Acceptance Criteria
- [ ] AC-17-126: `GetDocument`'s rule-lookup behavior is byte-for-byte unmodified by this feature.
- [ ] AC-17-127: Write-path's rule-lookup behavior (`write_access_rules`) is byte-for-byte unmodified by this feature.
- [ ] AC-17-128: `RunQuery`'s rule-lookup behavior (both `access_rules` and `group_access_rules`) is byte-for-byte unmodified by this feature.
- [ ] AC-17-129: An unruled collection's Listen subscription content remains fully unrestricted — this feature's Finding-5 scoping fix (US-01) is orthogonal to, and does not itself introduce, any content restriction.

#### Outcome KPIs
See § Outcome KPIs below (KPI #3 Guardrail).

#### Technical Notes (Optional)
Primarily a proof obligation over US-01–US-05's real behavior and the 4 prior epics' own shipped code — mirrors every prior epic's own AC-17-9x-style independence discipline, now proven against a collection carrying an active rule AND an active Listen subscription simultaneously.

---

### US-07: Untouched Collections and the Full Regression Baseline Are Unaffected

**job_id**: JOB-17
**Slice**: 07 (Walking Skeleton) | **Release**: 1

#### Elevator Pitch
Before: Alex worries that shipping real-time enforcement might silently break an existing, currently-passing Listen scenario, or perturb a collection that has never used Listen at all.
After: the full pre-existing regression baseline (133 FINALIZED scenarios plus the 3 non-finalized-but-DESIGN-stable priors' own delivered scenarios) is re-run unmodified → all pass exactly as before this feature shipped.
Decision enabled: Alex can adopt real-time enforcement with zero risk to any collection or surface he hasn't touched, and with confidence that this feature's own correctness fixes (US-01/02) did not silently change any currently-passing scenario's own expected behavior.

#### Domain Examples
1. **Happy Path**: The 133-scenario FINALIZED regression baseline (`security-rules`) is re-run unmodified. None of them exercises multi-collection Listen fan-out (Finding 5's own condition for a leak), so none is affected by this feature's own scoping fix.
2. **Edge Case**: Any existing Listen-related acceptance scenario that happens to have relied, even inadvertently, on the unfiltered-initial-snapshot behavior (Finding 2) is re-examined — none is found to exist (confirmed: no prior epic's own domain examples exercise Listen at all).
3. **Error/Boundary**: A project with no rule defined anywhere continues to have every Listen subscription's content fully unrestricted, and every subscription's scope newly correct (US-01), consistently across every collection id in that project.

#### UAT Scenarios (BDD)

##### Scenario: The full pre-existing regression baseline passes unmodified
Given the 133 FINALIZED `security-rules` scenarios and the 3 non-finalized priors' own delivered scenarios, none of which exercises multi-collection Listen fan-out
When the full baseline is re-run against a build that includes this feature
Then all scenarios pass exactly as they did before this feature was added

##### Scenario: A project with no rules anywhere is fully unaffected in content, correctly scoped in delivery
Given a project has never defined any rule of any kind
When any number of sessions hold open Listen subscriptions to different collections in that project
Then each subscription's content remains fully unrestricted and each subscription's scope is correctly limited to its own collection (US-01)

##### Scenario: Existing Listen-adjacent test fixtures are unaffected
Given the existing test suite's own Listen-related fixtures (resume-token delivery, keepalive, RESET) were written before this feature existed
When they are re-run against a build that includes this feature
Then all pass exactly as before, confirming this feature's own new code is additive, not a rewrite of existing Listen mechanics

#### Acceptance Criteria
- [ ] AC-17-130: The 133-scenario FINALIZED regression baseline passes unmodified.
- [ ] AC-17-131: The 3 non-finalized-but-DESIGN-stable priors' own delivered acceptance scenarios pass unmodified.
- [ ] AC-17-132: A project with no rules anywhere retains fully unrestricted Listen content on every collection, while gaining correctly-scoped delivery (US-01) on every collection, consistently.
- [ ] AC-17-133: Existing resume-token/keepalive/RESET Listen mechanics are unaffected by this feature's own new code.

#### Outcome KPIs
See § Outcome KPIs below (KPI #3 Guardrail).

#### Technical Notes (Optional)
Structural guardrail preferred: `get_access_rule` returning `None` is the ONLY path that reaches US-06's "content fully unrestricted" behavior — no other code touches `access_rules` to reach it, mirroring every prior epic's own identical discipline.

---

### US-08: Alex Can Pre-Check Whether a Candidate Listen Subscription Would Be Admitted

**job_id**: JOB-17
**Slice**: 08 | **Release**: 2

#### Elevator Pitch
Before: Alex's only way to find out whether his app's `onSnapshot()` query code will be accepted is to run the app and watch the stream fail to open — or, worse, discover only in production that a candidate query filter he assumed was compliant is not.
After: call the admin API's existing rule-simulation action (`security-rules-query-path`'s own `simulate_query_compliance`), framed as testing a candidate Listen subscription's own initial-snapshot filter shape → sees whether that exact subscription would be admitted or rejected, without opening any real stream.
Decision enabled: Alex can validate his SDK's actual `onSnapshot()` query code, and confirm a given collection's rule will admit it, before shipping.

#### Domain Examples
1. **Happy Path**: Alex simulates `journal_entries`'s ownership rule with candidate identity `end_user_id: test-user-001` and candidate filter `owner_id == "test-user-001"`. Sees "admitted" — matching what a real Listen subscription with that filter would produce.
2. **Edge Case**: Alex simulates the same rule with the same identity but an empty candidate filter (what his app's real-time list screen actually issues today, before he fixes it). Sees "rejected — missing required filter" — catching the bug before shipping.
3. **Error/Boundary**: Alex simulates a candidate collection id with no rule defined at all. Sees "admitted" (content unrestricted, matching US-06's own default) — confirming the difference between "admitted because compliant" and "admitted because unruled" is visible in the simulation response.

#### UAT Scenarios (BDD)

##### Scenario: Simulating a compliant candidate Listen subscription returns "admitted"
Given Alex holds a candidate rule and a candidate query filter that satisfies it for a given candidate identity
When Alex calls the simulation action with the rule, identity, and filter
Then the response shows "admitted," matching what a real Listen subscription would produce

##### Scenario: Simulation surfaces a missing-filter bug before publishing client code
Given Alex holds a published rule and a candidate filter that omits the required conjunct
When Alex simulates that filter
Then the response shows "rejected — missing required filter," matching US-03's real behavior

##### Scenario: Simulation reports an unruled collection as admitted, unrestricted
Given Alex holds no rule for a candidate collection id
When Alex simulates any candidate filter against it
Then the response shows "admitted," matching US-06's real default for an unruled collection

##### Scenario: Simulation has zero effect on live traffic
Given `journal_entries` has an active, published rule and an open Listen subscription
When Alex calls the simulation action with a different candidate rule and filter
Then the real, open subscription continues to be evaluated against the published rule, unaffected by the simulation

#### Acceptance Criteria
- [ ] AC-17-134: Simulating a candidate query filter against a candidate rule, framed as a Listen subscription, returns the same admit/reject outcome real subscribe-time enforcement would produce.
- [ ] AC-17-135: Simulation correctly reports "missing required filter" for candidate filters lacking a required conjunct.
- [ ] AC-17-136: Simulation correctly reports "admitted" for a candidate collection id with no rule, matching US-06's own unruled-content default.
- [ ] AC-17-137: Simulating a Listen-subscription shape has zero effect on live/open Listen traffic.

#### Outcome KPIs
See § Outcome KPIs below (KPI #4 Leading).

#### Technical Notes (Optional)
Reuses `security-rules-query-path`'s own `simulate_query_compliance` admin handler UNCHANGED — no new response contract is needed, since Listen's own subscribe-time gate reuses `check_query_compliance()` identically to `RunQuery`'s own. This is purely a framing/documentation addition (Alex may reasonably expect a Listen-specific simulation entry point), not a new mechanism — DESIGN's call whether a dedicated route alias is warranted or whether existing documentation suffices.

---

## Wave: DISCUSS / [REF] Outcome KPIs

### Feature: security-rules-realtime

### Objective
Give Alex's real-time listeners the same two guarantees every other RPC in this initiative already has — genuinely scoped to the collection the caller subscribed to, and genuinely, continuously gated by whatever rule that collection carries — closing both the access-control gap this initiative exists to close AND the more severe, pre-existing, project-wide cross-collection delivery leak this DISCUSS's own investigation discovered.

### Outcome KPIs

| # | Who | Does What | By How Much | Baseline | Measured By | Type |
|---|---|---|---|---|---|---|
| 1 | SDK developers with a rule-protected collection carrying an active Listen subscription (e.g. Alex/Trailmark) | Have `Listen` correctly admit compliant subscriptions, reject non-compliant ones, and continuously withhold individual events that no longer satisfy the rule — zero false-allows at subscribe time or at any point during the subscription's lifetime | 100% of Listen subscriptions and their subsequent delivered events produce the admit/reject/withhold decision this feature's locked mechanism implies; 0 false-allows | 0% (capability does not exist today — Listen performs zero rule consultation of any kind) | Acceptance-scenario pass rate against the real-time compliance truth table (admit/reject subscription × deliver/withhold per-event) | North Star |
| 2 | Every SDK developer with any active Listen subscription, ruled or not (e.g. Alex/Trailmark) | Have their listener's live delivery correctly scoped to the collection they subscribed to — zero cross-collection event leakage | 100% of delivered events belong to the subscriber's own subscribed collection; 0% cross-collection leakage | 0% (today, EVERY Listen subscription in a project receives every other collection's events too — confirmed by direct code read, Finding 5) | Acceptance-scenario pass rate against the collection-scoping truth table (same-collection delivered, cross-collection dropped) | North Star |
| 3 | Existing `embyr-rs`/`client-auth`/`security-rules`/`security-rules-write-path`/`security-rules-query-path`/`security-rules-collection-group-rules` customers and collections that never use Listen or never define a rule | Continue using `GetDocument`, writes, `RunQuery` (group and non-group), and unruled Listen subscriptions successfully, unaffected | 0% regression across the 133 FINALIZED regression scenarios plus the 3 non-finalized-but-DESIGN-stable priors' own delivered scenarios | Current 100% pass rate | Full regression suite, pre/post comparison | Guardrail |
| 4 | SDK developers authoring/testing rules and real-time client query code | Catch a non-compliant Listen subscription shape via simulation before it reaches a real end user | ≥1 caught per integration/testing cycle (qualitative, ramps with adoption); 0 confirmed data-exposure incidents traced to an un-simulated Listen subscription | N/A (capability does not exist today) | Simulation-action usage log cross-referenced with post-publish subscription-rejection-rate anomalies | Leading |

---

## Wave: DISCUSS / [REF] Out of Scope

- **`TargetType::Documents` (direct document-path Listen targets)** — not reachable in shipped code today (Finding 1); building support for it is a separate, orthogonal enhancement to `Listen`'s own target-type handling, not something this feature's own rule-enforcement work should silently add. If it is added by a future feature, its own enforcement (per-document `evaluate()`, likely simpler than the query-shaped case) would extend this feature's mechanism, not replace it.
- **Collection-group Listen (`all_descendants = true` targets, `collectionGroup().onSnapshot()`)** — not reachable in shipped code today (Finding 3: `all_descendants` is hardcoded `false`, `cs.all_descendants` never read). Fixing this so a genuine collection-group Listen becomes possible at all is a separate, pre-existing, non-security correctness gap, flagged (§ Open Questions, OQ-SRRT-03) explicitly out of scope — mirroring `security-rules-collection-group-rules`'s own OQ-SRCG-03 precedent for an analogous gap in non-group `RunQuery`. Should it ever be fixed, this feature's own `group_access_rules`-consulting mechanism (from `security-rules-collection-group-rules`) is the evidenced precedent for how Listen's own group-rule enforcement should then be composed — not designed here.
- **`security-rules-operations`** (rule history/versioning, richer condition grammar, audit logging, Epic 2e) — unchanged, still separately deferred.
- **Any change to `access_rules`, `write_access_rules`, `group_access_rules`, `GetDocument`'s enforcement, write-path's enforcement, or RunQuery's (group and non-group) enforcement** — this feature adds a new BC-3-internal collection-scoping mechanism and two new BC-4 call sites only; the five existing surfaces receive zero code changes.
- **Any change to `check_query_compliance()`, `evaluate()`, or any BC-4 grammar/type** — both functions are reused completely unmodified; no new decidable shape, no new operand family.
- **Fixing `Listen`'s own multi-target-per-stream and `RemoveTarget` handling** — confirmed pre-existing and unimplemented (`handle_listen` only processes the first `AddTarget` message, draining but not acting on subsequent messages). Orthogonal to access control, not fixed here.
- **Redesigning `ListenRegistry`/`PostgresNotifyListener` to use per-collection, rather than per-project, NOTIFY channels** — considered as a more thorough fix for Finding 5 (§ Handoff Package), but not required by any of this feature's own domain examples; the collection-scoping filter applied at delivery time (US-01) achieves the same observable guarantee without a channel-topology redesign — flagged as a possible future optimization, not built here.

---

## Wave: DISCUSS / [REF] WS Strategy

Walking Skeleton Strategy: **B — Thin End-to-End Slice** (unchanged convention). Slices 01–07 are real, narrow vertical slices against real Postgres NOTIFY events, real System DB rule state, and real Maria/Dana signed-in sessions (no facade, no mock) — Slice 01 proves the single highest-consequence, most severe finding of this DISCUSS (cross-collection leak) first, independent of every other mechanism; Slice 02 proves the prerequisite filter-honoring fix; Slice 03 proves subscribe-time admission reusing `check_query_compliance()` unmodified; Slices 04–05 prove the genuinely new per-event, continuous re-check reusing `evaluate()` unmodified; Slice 06 proves structural independence from the 4 prior epics' own surfaces with the strongest possible contrast (a collection carrying an active rule AND an active Listen subscription simultaneously); Slice 07 proves the whole thing is additive to the FINALIZED baseline.

---

## Wave: DISCUSS / [REF] Driving Ports

| Port | Protocol | Extension |
|---|---|---|
| Admin port `:9090` (existing, unchanged for Release 1) | HTTP/1.1 | `simulate_query_compliance` (Release 2 only, US-08) reused unchanged, documented as also applicable to a candidate Listen subscription's own initial-snapshot filter shape |
| Data ports `:8080` (gRPC) / `:8081` (REST/gRPC-Web) (existing, extended in observable behavior only) | gRPC / HTTP | `Listen`'s existing, unchanged call shape now additionally reflects collection-scoped delivery (US-01, unconditional) and rule compliance (US-02–06, when a rule is defined) — no new RPC or endpoint |

No new network-facing port introduced. Exact call-site/branch shapes are DESIGN's call.

---

## Wave: DISCUSS / [REF] Pre-requisites

- `docs/feature/security-rules-query-path/feature-delta.md` (full — the source of `check_query_compliance()`/`QueryComplianceOutcome`/`UnsatisfiedConjunct`, reused unchanged for subscribe-time admission). **Dependency is on ADR-031's DESIGN decisions being stable (Accepted), not on that feature being FINALIZED or fully DELIVERed** — confirmed not yet FINALIZED.
- `docs/feature/security-rules/feature-delta.md` and `docs/feature/security-rules-write-path/feature-delta.md` (full DISCUSS sections — the source of `evaluate()`, reused unchanged for the per-event re-check). Dependency is on ADR-027/030's DESIGN decisions being stable (both Accepted).
- `docs/product/architecture/adr-027-access-rule-grammar-and-evaluation.md`, `adr-028-access-rule-storage-and-lifecycle.md`, `adr-029-access-control-composition-and-bounded-context.md`, `adr-030-write-path-grammar-storage-and-composition.md`, `adr-031-query-shape-compliance-check.md` (full — the exact machinery this feature composes, not replaces).
- `crates/embyr-server/src/grpc/handler.rs`'s `handle_listen`/`translate_filter` (the exact wiring points this feature extends).
- `crates/embyr-server/src/realtime/listen_handler.rs`, `listen_registry.rs`, and `crates/embyr-server/src/adapters/postgres_notify_listener.rs` (the exact current shape of Findings 1–7, confirmed by direct code read).
- `crates/embyr-pg-storage/src/backend_adapter.rs` (confirms the `documents` table's soft-delete mechanism, `deleted` boolean flag — Finding 7).
- `docs/product/jobs.yaml` (JOB-17, extended via NOTE — see § SSOT Updates).

---

## Wave: DISCUSS / [REF] Handoff Package

**To DESIGN (solution-architect)**: this `feature-delta.md` (journey delta + story map + user stories + embedded AC), 8 slice briefs (`docs/feature/security-rules-realtime/slices/slice-01-*.md` through `slice-08-*.md`), `docs/product/jobs.yaml` (JOB-17, extended via NOTE).

**To DEVOPS (platform-architect)**: § Outcome KPIs above (4 KPIs — 2 North Star, 1 Guardrail, 1 Leading).

**Explicit flags for DESIGN**:
1. § Job Discovery Framing Resolution's Resolution 1 (Option C — a composition of `check_query_compliance()` for subscribe-time admission and `evaluate()` for per-event re-check, plus a new BC-3-internal collection-scoping filter) is LOCKED. Do NOT implement subscribe-time-only enforcement (Option A) under any framing, including as an "interim MVP."
2. Resolution 2 (Option C — both subscribe-time AND per-event compliance are required) is LOCKED. The per-event `evaluate()` re-check must consume the SAME document `fetch_event()` already fetches — no second fetch.
3. **Findings 2 and 5 (the filter-drop and cross-collection-leak bugs) are LOCKED as IN SCOPE for this feature (US-01/02)** — **this is the first of two genuine judgment calls made without a live stakeholder**, flagged explicitly in § Job Discovery Framing Resolution's own confidence note. DESIGN may proceed provisionally, but recommend the orchestrator confirm this scoping call (rather than splitting Findings 2/5 into a separate, prerequisite "Listen correctness" feature) before treating it as unquestionably final.
4. `check_query_compliance()`/`QueryComplianceOutcome`/`UnsatisfiedConjunct` (ADR-031) and `evaluate()` (ADR-027/030) are confirmed reusable completely UNCHANGED — no new decidable shape, no new evaluator branch, zero modification to `embyr_core::access_control` anywhere in this feature.
5. `access_rules`, `write_access_rules`, `group_access_rules`, and `GetDocument`'s/write-path's/`RunQuery`'s (group and non-group) enforcement receive ZERO code changes — confirmed by direct code read, not assumed.
6. US-01's collection-scoping fix is this feature's single highest-consequence design risk, alongside US-04's per-event fail-closed correctness — both designated mutation-testing surfaces (per-feature strategy, CLAUDE.md).
7. **The exact mechanism for US-05's `Removed`-event non-leakage (whether to fetch pre-deletion field data via a query that omits `fetch_event`'s own `AND NOT deleted` filter, or via some other mechanism) is DESIGN's call** — **this is the second genuine judgment call**, flagged explicitly. This DISCUSS locks the OBSERVABLE requirement (existence non-leakage extends to delete events) and confirms the mechanism is buildable (Finding 7: soft-delete preserves pre-deletion field data), but does not prescribe how.
8. Regression baseline is the 133-scenario FINALIZED suite plus the 3 non-finalized-but-DESIGN-stable priors' own delivered scenarios — confirmed via direct evidence that zero existing scenarios exercise multi-collection Listen fan-out or Listen-with-a-defined-rule.
9. A NEW ADR (033, next-free per `docs/product/architecture/adr-*.md` numbering) is expected for `Listen`'s own compliance-composition mechanism and its BC-3-internal collection-scoping fix, mirroring ADR-027/028/029/030/031/032's pattern.
10. Findings 1 and 3 (`TargetType::Documents` not handled; collection-group Listen not honored) are explicitly OUT OF SCOPE for this feature — flagged for separate tracks (§ Open Questions), not silently folded in or silently ignored.

**Escalation flagged for the orchestrator, not resolved here**: given (a) the genuine, stakeholder-unconfirmed scoping call in flag #3 (whether Findings 2/5's correctness fixes belong inside this feature or should be split into a prerequisite feature) and (b) this feature's own larger footprint and effort estimate relative to any of its 4 predecessors, **recommend the orchestrator either confirm flag #3's scoping call with the user before DESIGN treats it as unquestionably locked, or explicitly trigger `/nw-review nw-product-owner-reviewer` before DESIGN proceeds.** This mirrors `security-rules-query-path`'s and `security-rules-collection-group-rules`'s own precedent for deviating from the default per-wave-review skip when the architectural consequence of an unconfirmed judgment call is high — here, the consequence of NOT flagging it is that DESIGN might silently build a feature whose own walking skeleton depends on fixing two pre-existing bugs the orchestrator never explicitly asked for, however well-evidenced the causal-dependency reasoning above is.

---

## Wave: DISCUSS / [REF] SSOT Updates

- `docs/product/jobs.yaml` — added a NOTE under JOB-17 (`document-access-control`) documenting that this feature is JOB-17's own real-time-listen realization — not a new job, not a new operation surface — and documenting the two pre-existing, non-security-rules-caused structural gaps (Findings 2 and 5) this feature's own investigation discovered and locked as in-scope. JOB-17's `job_story`/dimensions text is unchanged (historically accurate as written).
- `docs/product/journeys/sdk-developer.yaml` — extended with a NOTE clarifying JOB-17 now also covers real-time Listen enforcement AND correctly-scoped real-time delivery via this feature; `updated` date bumped. No new job id added.
- No new persona file — Trailmark's end users remain domain-example data, consistent with prior precedent.

---

## Wave: DISCUSS / [REF] Definition of Ready Validation

### Requirements Completeness Score: **0.95** (at the > 0.95 gate — the tightest margin of any of the 5 epics, honestly reported)

Computed across the three requirement categories:
- **Functional**: all 8 stories have complete Given/When/Then coverage of happy path, at least one edge case, and at least one error/failure path; the central architectural question (Resolution 1: composed mechanism, not a single reused function) and the per-event-recheck question (Resolution 2) are both explicitly locked — though whether Findings 2/5's correctness fixes belong inside this feature's own scope is honestly flagged as a genuine judgment call, not silently asserted as fact (§ Handoff Package flag 3).
- **Non-functional**: security (collection-scoping as a structural, rule-independent guarantee; the per-event re-check's fail-closed semantics inherited unchanged from ADR-027; existence non-leakage extended to delete events) is explicit. A performance guardrail note (the per-event re-check adds zero I/O beyond `fetch_event()`'s own existing fetch, AC-17-121) is flagged, mirroring every prior epic's own precedent — not a hard-numeric SLA at DISCUSS time. The existing p99 write→onSnapshot ≤ 2s target (ADR-002 § BC-3) is unchanged by this feature and not re-verified here (out of this DISCUSS's own scope — a DEVOPS/DISTILL concern).
- **Business rules**: the composed-mechanism requirement (Resolution 1), the both-checks-required requirement (Resolution 2), the collection-scoping-is-structural-not-rule-dependent requirement, and the admit/reject/withhold truth table are all explicitly specified with examples.

**NFR note (carried forward)**: a collection with no rule must add effectively zero enforcement overhead beyond the existing unarmed Listen path — a single cheap existence-check against `access_rules`, mirroring every prior epic's own precedent — but this feature's own collection-scoping fix (US-01) is NOT similarly zero-cost-optional, since it must apply universally, rule or no rule; DESIGN should confirm the chosen mechanism (comparing collection paths inline vs. per-subscriber channel filtering) does not introduce measurable overhead on the existing p99 write→onSnapshot ≤ 2s target.

The remaining 0.05 gap is the two flagged judgment calls (§ Handoff Package flags 3 and 7) plus the Open Questions below — explicitly flagged, not hidden, and does not block this feature's own DoR, but is the specific reason this DISCUSS recommends an extra confirmation step before DESIGN (§ Handoff Package escalation).

### DoR Checklist (9-item hard gate)

| # | DoR Item | Status | Evidence |
|---|---|---|---|
| 1 | Problem statement clear, domain language | PASS | Every story's Elevator Pitch "Before" line is stated in Alex/Maria/Dana domain terms and grounded in direct code evidence (e.g. US-01: "Maria's `onSnapshot()` subscription to `journal_entries` today, in shipped code, also receives live change events for `trip_photos`... a structural, project-wide leak with zero connection to any access rule") |
| 2 | User/persona identified with specific characteristics | PASS | P1 Alex (unchanged); Maria Santos and Dana Kim as concrete rule-subject domain examples, continued |
| 3 | 3+ domain examples per story with real data | PASS | Every story has exactly 3 Domain Examples using `trailmark-prod`, real collection ids, `maria-santos`/`dana-kim`, real field names |
| 4 | UAT scenarios in Given/When/Then (3–7 per story) | PASS | US-01: 4, US-02: 4, US-03: 4, US-04: 4, US-05: 4, US-06: 4, US-07: 3, US-08: 4 — all within range |
| 5 | Acceptance criteria derived from UAT | PASS | Every AC (AC-17-105 through AC-17-137) traces 1:1 or 1:many to a specific scenario above it |
| 6 | Right-sized (1–3 days, 3–7 scenarios) | PASS | Largest slice (US-04) estimated 2 days / 4 scenarios; all others ≤4 scenarios, ≤1.5 days |
| 7 | Technical notes identify constraints | PASS | Every story's Technical Notes references the relevant locked constraint (reused compliance/evaluation functions, no new I/O, structural independence) without prescribing implementation |
| 8 | Dependencies resolved or tracked | PASS | Dependency is on `security-rules-query-path`'s ADR-031 and `security-rules`/`security-rules-write-path`'s ADR-027/030 DESIGN decisions being stable (all confirmed Accepted), not on any of the three being FINALIZED or fully DELIVERed — explicitly distinguished, not conflated |
| 9 | Outcome KPIs defined with measurable targets | PASS | 4 KPIs, each with a numeric or explicitly-qualitative-with-rationale target, baseline, and measurement method |

### DoR Status: **PASSED** — with two explicit escalation flags (§ Handoff Package) recommended before DESIGN treats this feature's own scope boundary as final.

---

## Wave: DISCUSS / [REF] Open Questions

| ID | Question | Impact | Resolution owner |
|---|---|---|---|
| OQ-SRRT-01 | Whether Findings 2 and 5's own correctness fixes should remain inside this feature's own scope, or be split into a separate, prerequisite "Listen correctness" feature shipped first | Does not block this feature's own DoR (the causal-dependency reasoning is explicit and auditable); flagged for orchestrator confirmation before DESIGN treats the current scope boundary as final | Orchestrator, with user confirmation |
| OQ-SRRT-02 | The exact mechanism for US-05's `Removed`-event non-leakage — fetch pre-deletion fields via a query omitting `AND NOT deleted` (Finding 7), or some other mechanism | Does not block this feature's own DoR (the observable requirement is locked, the mechanism is confirmed buildable); DESIGN's call | Solution-architect (DESIGN) |
| OQ-SRRT-03 | `TargetType::Documents` (direct document-path Listen targets) is not handled at all in shipped code today — orthogonal to this feature, a genuine pre-existing gap in `Listen`'s own target-type support | Does not block this feature's own DoR; flagged as a separate bug-fix/enhancement candidate | Solution-architect / troubleshooter, on a separate track |
| OQ-SRRT-04 | Collection-group Listen (`all_descendants = true`) is not honored at all in shipped code today (Finding 3) — orthogonal to this feature, a genuine pre-existing gap in `Listen`'s own query-shape fidelity | Does not block this feature's own DoR; flagged as a separate bug-fix/enhancement candidate, mirroring `security-rules-collection-group-rules`'s own OQ-SRCG-03 precedent | Solution-architect / troubleshooter, on a separate track |
| OQ-SRRT-05 | Whether `ListenRegistry`/`PostgresNotifyListener` should be redesigned to use per-collection, rather than per-project, NOTIFY channels — a more thorough (but larger) fix for Finding 5 than the delivery-time filter this DISCUSS locks | Does not block this feature; a candidate performance/architecture optimization if channel-level filtering proves insufficient under real production fan-out volume | Product Discovery / platform-architect, triggered by future evidence |
| OQ-SRQ-02 (carried, unrelated) | Whether OR-composed rules will need query support badly enough to justify UNION-of-queries execution machinery | Does not block this feature; unchanged from `security-rules-query-path` | Product Discovery |
| OQ-SR-04 (carried, unrelated) | String/number literal operands — confirmed booleans-only | Not reopened by this feature | Closed |

---

## Wave: DISCUSS / [REF] Wave Decisions Summary

### Key Decisions
- [D1] Locked a composed mechanism for `Listen`'s own enforcement (Resolution 1, Option C): `check_query_compliance()` (unchanged, subscribe-time) plus `evaluate()` (unchanged, per-event) plus a new BC-3-internal collection-scoping filter — rejecting subscribe-time-only enforcement (Option A, insufficient by direct construction) and a wholly-invented new function (Option B, unjustified given two proven mechanisms already answer Listen's two sub-problems) — see § Job Discovery Framing Resolution.
- [D2] Locked that per-event re-check IS required (Resolution 2, Option C) — not subscribe-time-only (Option A, falsifiable by the task's own owner-field-change hypothesis, now confirmed structurally reachable) and not per-event-only (Option B, reintroduces the execute-then-filter antipattern for the initial snapshot).
- [D3] Discovered and locked as in-scope two pre-existing, non-security-rules-caused structural gaps in `Listen`'s own delivery mechanism: Finding 2 (initial snapshot ignores the client's own query filter) and Finding 5 (live delivery is not scoped to the subscribed collection, a project-wide cross-collection leak) — both flagged as genuine judgment calls for orchestrator confirmation (§ Handoff Package flag 3), not silently absorbed.
- [D4] Confirmed `check_query_compliance()`/`QueryComplianceOutcome`/`UnsatisfiedConjunct` (ADR-031) and `evaluate()` (ADR-027/030) are both reusable completely unchanged — no new decidable shape, no new evaluator branch, no new grammar anywhere in this feature.
- [D5] Extended JOB-17 rather than minting a new job — this feature is JOB-17's own fifth "make it real" realization, explicitly pre-named by `security-rules`'s own § Out of Scope entry, quoted verbatim as this feature's charter — see § Persona & Job.
- [D6] Scope Assessment PASS, 0/5 signals fired outright (2 sit exactly at threshold without exceeding, 1 sits near-but-under the effort edge) — this feature's own larger footprint, relative to its 4 predecessors, is fully explained by Findings 2/5's own genuinely new discovery, not silently absorbed or hand-waved — see § Scope Assessment.
- [D7] Discovered and flagged, explicitly out of scope: two further pre-existing correctness gaps in `Listen`'s own target-type/query-shape fidelity (Finding 1: `TargetType::Documents` unsupported; Finding 3: collection-group Listen unhonored) — see § Open Questions, OQ-SRRT-03/04.
