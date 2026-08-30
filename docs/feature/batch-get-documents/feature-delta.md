# batch-get-documents — Feature Delta

**Wave**: DISCUSS | **Agent**: Luna (nw-product-owner) | **Date**: 2026-08-30
**Status**: Ready for DESIGN handoff — one escalated open question (Resolution 5's whole-batch-vs-per-document denial behavior, raised by the orchestrator; see § Handoff Package).
**Upstream**: No DISCOVER/DIVERGE wave ran for this feature specifically — commissioned directly by the orchestrator's own direct comparison of embyr-rs's proto surface against real Firestore's, identifying `BatchGetDocuments`'s unimplemented handler as a second (smaller, more contained) client-facing gap alongside `aggregation-queries`.

**Framing**: this is a proto-surface-completion feature for the base SDK-compatibility job (JOB-01), not a new epic. It sits entirely in BC-2 Document Storage, reusing `handle_get_document`'s own per-document access-control mechanism N times — no new bounded context, no new domain concept, no proto-schema work (unlike `aggregation-queries`, both RPC and both messages are already fully vendored with correct field numbers).

---

## Wave: DISCUSS / [REF] Prior Wave Consultation — Reading Confirmation

✓ `proto/google/firestore/v1/firestore.proto` (targeted: RPC declaration line 31, `BatchGetDocumentsRequest`/`BatchGetDocumentsResponse` message bodies lines 114-153) — confirmed the RPC and both messages are already fully vendored with correct field numbers: `documents` (repeated string, full resource names), `mask` (`DocumentMask`), `consistency_selector` oneof (`transaction`/`new_transaction`/`read_time`); response `oneof result { found, missing }` + `transaction` + `read_time`. This is NOT a proto-surface gap — confirmed directly, not assumed.
✓ `crates/embyr-server/src/grpc/handler.rs::handle_batch_get_documents` (lines 1074-1078) — confirmed the exact stub: `Err(Status::unimplemented("not implemented"))`, ignoring the request entirely. `type BatchGetDocumentsStream = tonic::codegen::BoxStream<BatchGetDocumentsResponse>` is already registered on the `Firestore` trait impl (line 1917) and already dispatches to this handler (lines 1917-1930) — the ONLY missing piece is the handler body itself.
✓ `crates/embyr-server/src/grpc/handler.rs::handle_get_document` (full, lines 588-717) — confirmed the exact per-document composition this feature reuses N times: `extract_project_id` → `extract_api_key` → `rate_limiter.check` → `authenticate` (+ suspension check) → `attach_client_identity_if_present` → `parse_document_path` → `get_access_rule` (single indexed lookup, `None` = today's unmodified pre-security-rules path) → `adapter.get_document(&path)` → branch on rule presence: no rule = return doc/`NotFound` unmodified; rule present = `parse_condition` → `evaluate()` (called UNCONDITIONALLY, existence non-leakage via an empty field map when the document doesn't exist) → `Deny` = `Status::permission_denied` (never distinguishes "wrong owner" from "does not exist"), `Allow` = branch on document existence exactly as the no-rule path does. **Confirmed directly reusable per requested document, not merely similar** — `BatchGetDocuments` is semantically N independent invocations of this exact per-document resolve-and-authorize sequence, streamed.
✓ `crates/embyr-server/src/grpc/handler.rs::handle_run_query` (targeted: lines 1240-1294, rate-limiter/auth/identity composition only — full compliance-check composition already read during `aggregation-queries`) — confirmed `rate_limiter.check` and `authenticate`/suspension/`attach_client_identity_if_present` are each called **exactly once per RPC call**, even though `RunQuery` streams a potentially unbounded number of documents. This is the correct precedent for this feature's own per-*call* (not per-document) granularity for rate-limiting and auth — distinct from, and not to be confused with, `handle_get_document`'s per-document granularity for **access-control evaluation only**.
✓ `crates/embyr-core/src/access_control/mod.rs::evaluate` (full signature, lines 512-522) and `AuthContext`/`EvaluationOutcome` (lines 109-123) — confirmed signature `fn evaluate(condition: &Condition, auth: Option<&AuthContext>, resource_fields: &BTreeMap<String, FieldValue>, request_resource_fields: &BTreeMap<String, FieldValue>) -> EvaluationOutcome`, a pure, total function (`Ok(true) => Allow`, `Ok(false) | Err(FieldMissing) => Deny`, no third state). Read-path callers (this feature included) always pass an empty `request_resource_fields` map, mirroring `handle_get_document`'s own call shape exactly. **Directly reusable, unchanged, called once per requested document** — confirmed by reading the function body, not assumed.
✓ `docs/product/architecture/adr-002-bounded-contexts.md` (full) — confirms BC-2 Document Storage's own ubiquitous language already names `BatchGetRequest` (line 73), alongside `AggregationQuery` — direct evidence this bounded context was scoped with batch document retrieval in mind from the original DESIGN pass. No new bounded context, no Option-D re-evaluation needed — this is squarely BC-2's own existing `GetDocument`-family read logic, applied N times, not a new entity with its own identity/lifecycle/invariants.
✓ `docs/SPEC.md` (targeted: `§BatchGetDocuments`, lines 691-701; `§REST Routing`, lines 1038-1039; rate-limit table, line 1418) — **critical finding**: SPEC.md already documents a complete contract — inputs (`documents`, `consistency_selector`; **`mask` is NOT listed as an input** in this section, though the proto carries the field), outputs (stream of `{found}`/`{missing}`/optional leading `{transaction}` message), and an explicit, load-bearing statement: **"Order of responses may differ from request order."** This directly, locally answers the raw ask's own open question about response ordering — no external verification needed, unlike `aggregation-queries`' own OQ-AGG-01 (this is embyr's own documented contract for embyr's own RPC, not a real-Firestore wire-shape fact requiring external confirmation). Mirrors the exact "documented-but-unbuilt gap" pattern `aggregation-queries`' own Resolution 1 found for `RunAggregationQuery`.
✓ `docs/feature/aggregation-queries/feature-delta.md` (full structural read, all waves) — confirmed the structural pattern this file follows (`## Wave: DISCUSS / [REF] <Section>` headings, Resolution numbering, Elephant Carpaccio slice table, AC-ID convention `AC-01-NN` scoped to this feature's own file namespace, not deduplicated globally — confirmed by `customer-db-onboarding/feature-delta.md` independently reusing the identical `AC-01-01..06` numbering in its own file with zero collision concern, since AC IDs are feature-file-local, not cross-feature-unique. The one cross-feature collision fixed by a prior commit — `client-auth-hosted-identity`'s Slice 01/Slice 02 both independently starting from `05/06` — was an **intra-file** collision between two slices of the *same* feature, not an inter-feature one).
✓ `docs/product/jobs.yaml` (full, all 19 jobs surveyed) — confirmed JOB-01 (`sdk-compat`, P1 Alex) is the correct home, identical reasoning to `aggregation-queries`' own Resolution 4: "all SDK calls succeed unchanged" already covers `BatchGetDocuments`, part of the same Firestore data-plane surface JOB-16's own NOTE scoped JOB-01 to. No new job warranted.
✓ `docs/product/journeys/sdk-developer.yaml` — confirms P1 Alex, JOB-01 already listed, and the established single-narrative-file convention (no separate `journey-*.yaml` artifact) this feature follows for Lightweight-depth features, mirroring `aggregation-queries`' own precedent exactly.

No contradictions found between this feature's scope and prior evidence. Zero open questions require escalation to DESIGN — every judgment call this DISCUSS encountered was resolvable directly from this codebase's own already-established precedent (see § Job Discovery Framing Resolution below), unlike `aggregation-queries`' own OQ-AGG-01 (a real-Firestore wire-shape fact this sandbox had no tool to verify). This is a genuinely smaller, more contained feature, and this DISCUSS does not manufacture escalations or extra slices to match `aggregation-queries`' own size.

---

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

| # | Decision | Value |
|---|---|---|
| 1 | Feature Type | Backend — completes an already-declared client-facing gRPC handler (BC-2 Document Storage), reusing `handle_get_document`'s own per-document access-control logic N times |
| 2 | Walking Skeleton | Evaluated (this wave's call): **YES, a single slice** — see § Scope Assessment and § Story Map. Unlike `aggregation-queries`, no second backend-mode slice is needed (see Resolution 3) |
| 3 | UX Research Depth | **Lightweight** — mirrors `aggregation-queries`'/`security-rules-query-path`'s own precedent: no new human-facing mental model (Alex already understands "fetch one document by name"; this adds "fetch several by name in one round trip instead of several calls"), full journey detail lives inline below, no separate `journey-*.yaml` |
| 4 | JTBD Analysis | Yes (default) — the one story traces to `job_id: JOB-01` (extends, not a new job — see § Job Discovery Framing Resolution, Resolution 4) |

---

## Wave: DISCUSS / [REF] Job Discovery — Framing Resolution

The raw ask instructs this DISCUSS to verify (not assume) several specific points against the codebase directly. Four resolutions were required; **zero are escalated** — a genuinely different outcome from `aggregation-queries`' own single escalation (OQ-AGG-01), because every question this feature raises is answerable from this codebase's own already-shipped precedent, not from an external, unverifiable wire-shape fact.

### Resolution 1 — Is this a proto-surface gap (like `RunAggregationQuery` was) or purely an unimplemented handler?

Confirmed directly: the RPC declaration (`firestore.proto:31`) and both message shapes (`firestore.proto:114-153`) are already fully vendored with correct field numbers, matching real Firestore's own public contract exactly (`documents`, `mask`, three-way `consistency_selector` oneof; response `oneof result { found, missing }` + `transaction` + `read_time`). The trait registration (`type BatchGetDocumentsStream`, `handler.rs:1917`) is also already wired to dispatch to `handle_batch_get_documents`. **The only gap is the handler body** — a stub returning `Status::unimplemented` unconditionally, ignoring the request entirely. This is a smaller category of gap than `aggregation-queries`' own (which required proto authoring); no proto/message design work is needed here at all.

### Resolution 2 — Is `BatchGetDocuments` a clean, direct reuse of `GetDocument`'s per-document logic, or does the batch shape require new logic?

Direct comparison of `handle_get_document`'s full body (lines 588-717) against the batch requirement: **clean, direct reuse, applied N times, not new logic.** Every mechanism `GetDocument` already uses — `parse_document_path`, `get_access_rule` (single indexed lookup per collection), `adapter.get_document`, `parse_condition`, `evaluate()` (called unconditionally, existence non-leakage via an empty field map), the no-rule short-circuit — applies identically to each individually-requested document name in a batch, since a batch can span **multiple different collections** (the whole point of "resolving several foreign-key-style references gathered from a prior query" the raw ask names), and access rules are collection-scoped, not batch-scoped. The only genuinely new composition-level work is: (a) looping over N document names instead of one, (b) rate-limiting/auth/identity resolved **once per call** (mirroring `RunQuery`'s own multi-result-RPC precedent, not `GetDocument`'s own single-document precedent — see Resolution 4), and (c) building a streamed response instead of a single `Response<Document>`.

### Resolution 3 — Does this feature need a second slice for `backend_mode=agent`, mirroring `aggregation-queries`' own Slice 01/02 split?

**No — confirmed by direct comparison, not assumed.** `aggregation-queries` needed a second slice because `RunAggregationQuery` was a **net-new port method** that `AgentBackendAdapter` did not yet implement at all. `BatchGetDocuments`'s own underlying primitive, `BackendAdapter::get_document`, **already exists and is already implemented by both `PostgresBackendAdapter` and `AgentBackendAdapter`** — `GetDocument` already works correctly for every `backend_mode` today, in production. Looping over this already-polymorphic, already-probed port method N times requires zero new adapter work, zero new agent-proto/binary changes, and zero backend-mode-specific branching in the handler itself (the polymorphism is already resolved inside `authenticate()`, exactly as it is for `GetDocument` and `RunQuery` today). This is the single most consequential scope difference from `aggregation-queries`, and it is why this feature is genuinely one slice, not two.

### Resolution 4 — What granularity applies to rate-limiting, auth, and access-control — per call or per document?

Two DIFFERENT existing precedents apply to two DIFFERENT concerns, deliberately, not a single blanket "copy `GetDocument`" choice:
- **Rate-limiting + authentication + suspension check + client-identity resolution: per RPC call**, mirroring `RunQuery`'s own treatment of multi-result streaming RPCs (confirmed: `handler.rs:1250-1267`, each called exactly once regardless of how many documents/rows the call ultimately returns). A 50-document batch is one round trip and should cost one rate-limit check, matching the feature's own value proposition and JOB-11's own request-rate (not document-count) fairness model.
- **Access-control evaluation (`get_access_rule` + `evaluate()`): per requested document**, mirroring `GetDocument`'s own treatment exactly, because access rules are collection-scoped and a single batch can span multiple different collections with different rules (or no rule at all).
- **Usage metering (`metrics_adapter.record_read`)**: called once per call, with a count equal to the number of requested documents (the existing `record_read(project_id, count)` signature already accepts a count parameter — `GetDocument` simply always calls it with the literal `1`), so daily usage totals correctly reflect N documents read, not 1.

### Resolution 5 — What happens when a requested document's collection denies the caller (mirroring the raw ask's own open question)?

Resolved directly from `handle_get_document`'s own already-shipped precedent, per the raw ask's own explicit instruction to mirror it rather than invent fresh: `evaluate()` returning `Deny` for any individual requested document produces the identical `Status::permission_denied` outcome `GetDocument` already produces for that same document — the batch request as a whole is rejected as a permission denial, not silently collapsed into a `missing` response. This preserves `GetDocument`'s own established, security-relevant guarantee — `Deny` never distinguishes "wrong owner" from "document does not exist" — uniformly whether a caller fetches one document via `GetDocument` or the same document via `BatchGetDocuments`. This is an internal design-consistency decision, not a real-Firestore wire-compatibility fact (embyr's access-rule system is not real Firestore's own Security Rules), so it is fully resolvable from this codebase's own precedent and does not require the kind of external verification `aggregation-queries`' OQ-AGG-01 needed.

**Orchestrator finding, flagged for DESIGN's own reconsideration (2026-08-30) — not overriding DISCUSS's own resolution, since my confidence here is moderate, not the near-certain recall I had for OQ-AGG-01's own RPC wire-shape fact:** whole-batch failure on a single denied item sits uneasily with JOB-01's own core value proposition (wire-identical SDK behavior), independent of the fact that embyr's own access-rule system isn't literally Firestore's own Security Rules engine. My own recollection of real Firestore's actual `BatchGetDocuments` behavior under Security Rules denial is that it is evaluated **per document**, not per call — a real client batching N document reads where Security Rules deny one of them still receives results for the others; the denied item resolves to its own per-item signal within the stream, the whole request does not abort. If that recollection is correct, DISCUSS's own chosen behavior (one denied document voids the entire batch, silently dropping the caller's own legitimately-accessible documents too) is a **real, avoidable divergence from what a real Firestore SDK client would expect**, not merely an internal style choice — and it actively undermines the specific use case DISCUSS's own Resolution 2 named as this feature's core value ("resolving several foreign-key-style references gathered from a prior query," where NOT every reference being accessible is an entirely ordinary, expected outcome, not an exceptional one). Recommend DESIGN treat this as a genuine open question, not a settled internal decision: either confirm real Firestore's actual per-document-vs-whole-batch behavior with higher confidence than I can currently supply, or — if that can't be confirmed with confidence — default to the safer, more idiomatic choice for a *streaming* RPC (a denied document resolves as its own individual stream item carrying a non-leaking "inaccessible" signal, preserving `GetDocument`'s own existence-non-leakage property scoped to that one item, while the caller's other, accessible documents still stream through normally) rather than DISCUSS's own current whole-call-abort choice. If DESIGN independently reaches DISCUSS's own original conclusion with better-grounded reasoning than mine, that is a legitimate outcome too — this is flagged as a question to resolve deliberately, not a directive to reverse.

---

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: **P1 — Alex, SDK Developer** (existing persona), unchanged.

**Domain-example company**: **Trailmark** (continuity with the rest of this codebase's established domain examples). Concrete grounding: `trip_entries` (owned per end user, `owner_id` field, mirrors `security-rules`'s own Maria/Dana ownership example) and its `expenses` sub-collection — the same two collections `aggregation-queries` uses, reused here for continuity, not reintroduced.

**job_id decision (Resolution 4 of § Job Discovery)**: `JOB-01` (`sdk-compat`), extended not new, same persona P1 Alex, same goal (SDK data-plane parity). NOTE appended to `docs/product/jobs.yaml` (see § SSOT Updates).

---

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

| Signal | Threshold | This feature | Fired? |
|---|---|---|---|
| User stories | >10 | 1 (US-01) | **NO** |
| Bounded contexts / modules | >3 | 1 — extends BC-2 Document Storage only; reuses `handle_get_document`'s own access-control composition unchanged, touches zero new bounded contexts | **NO** |
| Walking Skeleton integration points | >5 | 1 — the new handler itself; `adapter.get_document`, `get_access_rule`, `evaluate()` are all already-shipped, already-integrated points reused unchanged, not new integration surface | **NO** |
| Estimated effort | >2 weeks | 1 slice, ~1 day (see § Elephant Carpaccio Slices) | **NO** |
| Independent shippable outcomes | multiple | **NO** — there is exactly one outcome (resolve N document names in one round trip); found/missing handling and per-document access control are intrinsic to a correct v1, not separable, deferrable increments | **NO** |

**0 of 5 signals fired. Verdict: PASS — right-sized, single slice.** Per the standing session practice, this DISCUSS does not manufacture a second slice to match `aggregation-queries`' own four-slice size — Resolution 3 above documents, with evidence, exactly why a second (agent-mode) slice is unnecessary here.

---

## Wave: DISCUSS / [REF] Journey (Lightweight, per Decision 3 — inline per this codebase's convention)

**Alex's mental model**: Alex already understands `GetDocument` — "give me one document by its full path, get it back (or find out it doesn't exist)." This feature adds a narrow, obvious extension: "give me *several* document paths at once, get each one back (or find out which ones don't exist) in a single round trip" — the SDK's own `getAll(...)`/`db.getAll(ref1, ref2, ref3)` method, which Alex already knows from real Firestore, most commonly used to resolve a batch of document references gathered from a prior query or a stored list of IDs.

**Emotional arc** (mirrors "Problem Relief" pattern, identical shape to `aggregation-queries`' own): **Start** — mild frustration: today, Alex's only way to resolve Maria's "favorites" list of 12 trip-entry IDs is 12 separate `GetDocument` round trips (or none at all — the RPC currently returns `UNIMPLEMENTED` to any real client that tries), which is slow, chatty, and not what real Firestore requires. **Middle** — focused: Alex passes an array of document references to the SDK's existing `getAll()`-style call, exactly as he would against real Firestore. **End** — relief/confidence: one round trip resolves the whole batch; documents that no longer exist come back as clearly `missing`, not as an error; Alex trusts that access control is enforced per document exactly as it is for `GetDocument`, with no separate authorization model to reason about.

**Shared artifact**: the per-document resolve-and-authorize sequence itself (`get_access_rule` → `adapter.get_document` → `evaluate()`) — identical mechanism `GetDocument` already uses, single source of truth: `crates/embyr-server/src/grpc/handler.rs::handle_get_document`. No new artifact type introduced; this feature is a new *caller* of an existing mechanism, applied N times.

**Failure modes** (feeds DISTILL scenario generation): a requested document's collection denies the caller (mirrors `GetDocument`'s own `PermissionDenied`, reused unchanged) | a requested document does not exist (`missing`, never an error) | an empty `documents` list (`InvalidArgument`, nothing to resolve) | requested document names spanning more than one project/database in a single call (`InvalidArgument` — real Firestore requires all documents in one `BatchGetDocuments` call to belong to the same database) | a collection with no access rule defined behaves exactly as it does today, unrestricted.

---

## Wave: DISCUSS / [REF] Story Map & Walking Skeleton

**Persona**: P1 Alex | **Goal**: Resolve several specific documents by name in one round trip, with the exact same per-document access-control guarantees `GetDocument` already provides, instead of issuing one `GetDocument` call per reference.

### Backbone

| A. Alex Resolves a Batch of Document References |
|---|
| Alex fetches several documents spanning one or more collections in a single call **[WS]** |

### Walking Skeleton

The one task above IS the whole feature — a batch spanning multiple collections (some with rules, some without), correctly reporting `found`/`missing` per document, correctly enforcing access control per document, correctly rejecting a batch with no documents or with a cross-project mismatch. Unlike `aggregation-queries`, there is no second dimension (no per-backend-mode split, no COUNT→SUM→AVG progression) — the walking skeleton IS the release.

### Release 1 — Resolve a Batch of Documents in One Round Trip (Slice 01, US-01)

Outcome: any Trailmark app resolving a list of document references (favorites, foreign-key-style lookups gathered from a prior query) does so in a single round trip instead of N separate `GetDocument` calls, for every backend mode, with existing per-document security-rule guarantees fully intact.

---

## Wave: DISCUSS / [REF] Elephant Carpaccio Slices

| Slice | Story | Release | Estimate | Learning Hypothesis (disproves) | Production-data taste test |
|---|---|---|---|---|---|
| 01 (WS) | US-01 | 1 | 1 day | `handle_get_document`'s own per-document resolve-and-authorize sequence (`get_access_rule` → `adapter.get_document` → `evaluate()`) cannot be applied N times, across documents spanning different collections with different (or absent) rules, without requiring new access-control logic beyond a simple per-document loop — OR the already-shipped `BackendAdapter::get_document` cannot serve both backend-mode families without any new adapter code | Real Postgres AND a real agent-mode project, real `access_rules` rows, real `trip_entries`/`expenses` documents spanning multiple collections in the same batch call — no synthetic exception, no mocked backend |

**Total estimate: ~1 day.**

**Taste tests applied**:
- "4+ new components per slice" — 1 new component (`handle_batch_get_documents` itself, replacing the stub) + reuse of 3 already-shipped mechanisms (`adapter.get_document`, `get_access_rule`, `evaluate()`) = well under threshold. PASS.
- "Every slice depends on a new abstraction" — no new abstraction is introduced at all; this slice is pure composition of already-shipped primitives. PASS (trivially — there is only one slice, and it introduces nothing new to depend on).
- "No slice disproves a pre-commitment" — the slice has a distinct, falsifiable hypothesis (see table): that N-times reuse of `GetDocument`'s own logic, across heterogeneous collections, is clean and requires no new access-control code. PASS.
- "Synthetic-data-only slices prove plumbing, not value" — the slice's own production-data taste test requires real rows across real collections in both backend-mode families, never a stubbed backend. PASS.
- "2+ slices identical except for scale" — not applicable; there is exactly one slice. PASS (vacuously).

---

## Wave: DISCUSS / [REF] Prioritization

| Priority | Slice | Target Outcome | Rationale |
|---|---|---|---|
| 1 | Slice 01 (WS) | Batch document resolution works across both backend-mode families, with full security-rules parity | The walking skeleton IS the release — no dependency chain, no learning-leverage ordering question, since there is only one slice |

---

## Wave: DISCUSS / [REF] System Constraints

- **Security-rules parity is non-negotiable and structurally guaranteed, not conventional.** Every requested document's own access evaluation runs through the IDENTICAL `get_access_rule` → `parse_condition` → `evaluate()` sequence `GetDocument` already uses, called once per requested document (not once per batch), because a batch can span multiple collections with independently different rules. A collection with no rule defined behaves exactly as it does today for that document — unrestricted by identity, matching `GetDocument`'s own guardrail precedent.
- **Zero changes to `access_rules`, `get_access_rule`, `parse_condition`, `evaluate()`, `handle_get_document`, or any write-path/query-path handler.** This feature is a pure new consumer of already-shipped BC-2/BC-4 machinery, applied in a loop — verifiable by diff.
- **Rate-limiting, authentication, suspension-check, and client-identity resolution are per-CALL, not per-document** (Resolution 4) — mirrors `RunQuery`'s own multi-result-RPC precedent, deliberately distinct from `GetDocument`'s own per-document precedent for those specific concerns (access-control evaluation remains per-document).
- **A `Deny` outcome for any single requested document rejects the batch request as a permission denial** (Resolution 5), mirroring `GetDocument`'s own established "never distinguish wrong-owner from does-not-exist" guarantee, applied uniformly regardless of which RPC the caller used to reach the same document.
- **Usage metering counts N documents, not 1 call**, for a batch of N — `metrics_adapter.record_read(project_id, N)`, using the existing count-parameter signature `GetDocument` already calls with a literal `1`.
- **`DocumentMask` field-projection is out of scope**, matching an existing, pre-feature gap: confirmed by direct code read that `handle_get_document` never references `GetDocumentRequest.mask` at all — full documents are always returned regardless of what mask a caller supplies. `docs/SPEC.md`'s own `§BatchGetDocuments` section does not list `mask` among its documented inputs either, independently corroborating this scoping. This feature does not introduce a new gap; it inherits an existing one, named explicitly so DESIGN does not treat it as this feature's own omission.
- **`consistency_selector` (`transaction`/`new_transaction`/`read_time`) is out of scope for functional honoring**, matching an existing, pre-feature gap: confirmed by direct code read that `handle_get_document` never references `GetDocumentRequest`'s own `transaction`/`read_time` oneof fields — every `GetDocument` read today is a fresh, non-transactional read regardless of what consistency selector the client supplies. `BatchGetDocuments` inherits this identical scoping for v1 — every document in a batch is read fresh and independently, with no transactional snapshot guarantee across the batch, exactly as `GetDocument` already behaves for a single document. Named explicitly as a candidate follow-up (shared by `GetDocument`, `RunQuery`, and this feature identically), not silently introduced as a new limitation.
- **Response ordering is not guaranteed to match request order** — directly confirmed by `docs/SPEC.md`'s own already-written `§BatchGetDocuments` contract ("Order of responses may differ from request order"), not inferred or guessed. Any implementation may stream `found`/`missing` results as each document resolves, independent of request-array position.
- **A batch requires at least one document name; requesting zero documents is rejected as invalid** before any per-document work begins — mirrors real Firestore's own documented requirement and avoids a meaningless empty-stream success response.
- **All requested document names in a single call must resolve to the same project/database** — a batch mixing document names from two different projects is rejected as invalid before any per-document work begins, mirroring real Firestore's own single-database constraint for this RPC.
- **`RunAggregationQuery`'s own recently-shipped precedent (`aggregation-queries`) is the direct structural sibling for handler composition style**, not `handle_get_document` alone — this feature's rate-limiting/auth/identity granularity (per call) mirrors that feature's own handler composition, while its access-control granularity (per document) mirrors `handle_get_document`'s own. Both precedents are named explicitly so DESIGN does not have to re-derive which applies where.
- Ubiquitous language: no new terms — `BatchGetRequest` is already named in BC-2's own ubiquitous language (ADR-002, line 73), now realized.

---

## Wave: DISCUSS / [REF] User Stories

<!-- markdownlint-disable MD024 -->

### US-01: Alex Resolves a Batch of Document References in One Round Trip

**job_id**: JOB-01
**Slice**: 01 (Walking Skeleton) | **Release**: 1

#### Elevator Pitch
Before: to resolve Maria's list of 12 favorited trip entries (or any set of document references gathered from a prior query), Alex's app must issue 12 separate `GetDocument` round trips — or, today, cannot resolve them via this RPC at all, since any real client calling `BatchGetDocuments` receives a hard `UNIMPLEMENTED` error.
After: call the SDK's existing `db.getAll(ref1, ref2, ...)` (an unchanged SDK method, now backed by embyr's completed `BatchGetDocuments` handler) → sees a stream of results, each either the document's fields or a clear "this one doesn't exist" signal, with no `UNIMPLEMENTED` error.
Decision enabled: Alex can render Maria's full favorites list (or resolve a batch of foreign-key-style references) in one round trip and correctly distinguish "still exists" from "was deleted," without guessing or issuing N separate calls.

#### Domain Examples
1. **Happy Path**: Maria Santos requests her own 3 `trip_entries` and 2 `expenses` (from a trip's sub-collection) by full resource name in a single `BatchGetDocuments` call, all owned by her (`owner_id == "maria-santos"`), governed by the same ownership access rule `GetDocument` already enforces. Sees all 5 come back `found`, spanning two different collections in one round trip.
2. **Edge Case**: Alex's app resolves 4 `trip_entries` IDs gathered from a week-old cached "favorites" list; one of them (`trip-entry-77`) was deleted by Maria since the list was cached. Sees 3 `found` responses and 1 `missing` response for `trip-entry-77` — no error, no distinction in how the other 3 are delivered.
3. **Error/Boundary**: Dana Kim, signed in with her own verified identity, requests a batch that includes one of Maria Santos's own private `trip_entries` (denied by the ownership rule) mixed in with two of Dana's own entries. Sees the request rejected as a permission denial — the identical `PermissionDenied` outcome `GetDocument` already produces for the same single-document scenario — rather than Maria's entry being silently reported as `missing` or silently omitted.

#### UAT Scenarios (BDD)

##### Scenario: Resolving a batch of a caller's own documents across two collections returns every document, found
Given Maria Santos owns 3 documents in `trip_entries` and 2 documents in a trip's `expenses` sub-collection, all matching `owner_id == "maria-santos"`
And both collections have an access rule requiring `request.auth.uid == resource.data.owner_id`
And Maria Santos is signed in with a verified identity
When Alex's app runs a `BatchGetDocuments` call naming all 5 document paths
Then the response stream contains a `found` result for each of the 5 documents, with no particular delivery order required

##### Scenario: A document that no longer exists is reported as missing, alongside documents that do exist, with no error
Given a batch of 4 requested `trip_entries` document names, 3 of which exist and 1 of which was deleted
And the caller is authorized to read all 4 (either by rule or by the collection having no rule)
When Alex's app runs a `BatchGetDocuments` call naming all 4 document paths
Then the response stream contains a `found` result for the 3 existing documents and a `missing` result for the deleted one, with no error raised

##### Scenario: A batch containing a document the caller is not authorized to read is rejected, mirroring GetDocument's own denial behavior
Given `trip_entries` has the same ownership access rule as the happy-path scenario
And Dana Kim is signed in with a verified identity
When Dana Kim's app runs a `BatchGetDocuments` call naming two of her own documents and one of Maria Santos's own documents
Then the request is rejected as a permission denial, distinguishable from an unrelated validation failure, identical to the outcome `GetDocument` already produces for the same single-document scenario

##### Scenario: A collection with no access rule defined is unaffected — batch resolution runs exactly as it does today for GetDocument
Given `daily_logs` has no access rule of any kind defined
When any signed-in or anonymous caller runs a `BatchGetDocuments` call naming several `daily_logs` documents
Then every requested document is resolved unrestricted, identically to how `GetDocument` already behaves against this collection

##### Scenario: An empty batch request is rejected before any work begins
Given no documents are named in the request
When Alex's app runs a `BatchGetDocuments` call with an empty `documents` list
Then the request is rejected as invalid, before any access-control evaluation or document fetch occurs

##### Scenario: A batch mixing document names from two different projects is rejected as invalid
Given a `BatchGetDocuments` request names one document belonging to project `trailmark-prod` and another belonging to a different project
When Alex's app runs this request
Then the request is rejected as invalid, before any access-control evaluation or document fetch occurs

#### Acceptance Criteria
- [ ] AC-01-01: A `BatchGetDocuments` call naming documents the caller is authorized to read, spanning one or more collections, returns a `found` result for every document that exists, in a single round trip.
- [ ] AC-01-02: A document named in the batch that does not exist returns a `missing` result alongside `found` results for the rest of the batch — never an error, never distinguished in delivery mechanism from a `found` result.
- [ ] AC-01-03: A batch containing at least one document the caller is not authorized to read (per that document's own collection-scoped access rule) is rejected as a permission denial for the whole request, using the identical `evaluate()`-based mechanism and `PermissionDenied` outcome `GetDocument` already produces for the same scenario.
- [ ] AC-01-04: A collection with no access rule defined resolves every requested document from that collection unrestricted — zero behavior change from pre-feature `GetDocument` on the same collection.
- [ ] AC-01-05: A request naming zero documents is rejected as invalid before any access-control evaluation or document fetch occurs.
- [ ] AC-01-06: A request naming documents that resolve to more than one distinct project/database is rejected as invalid before any access-control evaluation or document fetch occurs.

#### Outcome KPIs
See § Outcome KPIs below (North Star + Guardrail).

#### Technical Notes (Optional)
Handler loops over `request.documents`, resolving each via the identical sequence `handle_get_document` already uses (`parse_document_path` → `get_access_rule` → `adapter.get_document` → conditional `parse_condition`/`evaluate()`), building a `Vec<Result<BatchGetDocumentsResponse, Status>>` and wrapping it as a stream, mirroring `handle_run_query`'s own `Box::pin(tokio_stream::iter(responses))` construction pattern (`handler.rs:1492`). `rate_limiter.check`, `authenticate`, suspension check, and `attach_client_identity_if_present` are each called exactly once per call (Resolution 4), not once per document. `metrics_adapter.record_read(project_id, documents.len())` replaces `GetDocument`'s own hardcoded `1`. `mask` and `consistency_selector` are accepted on the wire (required by the proto) but not functionally honored in v1, matching `GetDocument`'s own existing, pre-feature scoping (see § System Constraints) — this is DESIGN's decision to confirm, not reopen.

---

## Wave: DISCUSS / [REF] Outcome KPIs

### Feature: batch-get-documents

### Objective
Give every Trailmark-class embyr customer a working, security-rules-respecting `BatchGetDocuments` RPC — matching real Firestore's own SDK contract exactly — so document-reference batches resolve in one round trip instead of N separate `GetDocument` calls (or a hard `UNIMPLEMENTED` error, as today).

### Outcome KPIs

| # | Who | Does What | By How Much | Baseline | Measured By | Type |
|---|---|---|---|---|---|---|
| 1 | SDK developers whose apps resolve document-reference batches (Alex/Trailmark, all `backend_mode` families) | Complete a `BatchGetDocuments` call and receive correct `found`/`missing` results for every requested document, with the same per-document access-control guarantee `GetDocument` already provides | 100% of valid batch requests (well-formed document names, single project) succeed and return the correct found/missing signal per document | 0% (the RPC returns `UNIMPLEMENTED` unconditionally today) | Count of successful batch calls against the reference test suite (US-01's own UAT scenarios), cross-referenced against an N-separate-`GetDocument`-calls baseline for correctness | North Star |
| 2 | Existing `GetDocument`, `RunQuery`, and write-path callers, across every `backend_mode` | Continue to succeed exactly as before, unaffected by this feature's existence | 0% regression across the existing `embyr-rs`/`security-rules`/`aggregation-queries` acceptance suites | Current 100% pass rate (pre-feature) | Full existing acceptance suites, pre/post comparison | Guardrail |
| 3 | Any caller whose access to a specific document would be denied by `GetDocument` under that document's own collection access rule | Cannot obtain that document's field data, or a silently-omitted signal indistinguishable from a missing document, via a batch call that a single `GetDocument` call would have denied | 0 batch admissions that the identical `GetDocument` call would have rejected (audit metric, pass/fail, not a rate) | N/A (capability does not exist today — the RPC is unimplemented) | Dedicated parity test suite running the SAME document-name/identity pairs through both `GetDocument` and `BatchGetDocuments`, asserting identical admit/reject outcomes | Guardrail |

### Metric Hierarchy
- **North Star**: KPI #1 — successful, correct batch resolution rate.
- **Leading Indicators**: per-document found/missing accuracy; per-call latency for an N-document batch relative to N separate `GetDocument` calls (qualitative intent only in v1 — no numeric latency target set, matching `aggregation-queries`' own precedent for not over-specifying NFRs DESIGN/DEVOPS haven't yet evidenced).
- **Guardrail Metrics**: KPI #2 (zero regression), KPI #3 (zero access-control parity violations).

### Measurement Plan
| KPI | Data Source | Collection Method | Frequency | Owner |
|-----|------------|-------------------|-----------|-------|
| 1 | UAT scenario suite (US-01) | Automated test run | Per DELIVER commit | crafter/DELIVER |
| 2 | Full existing acceptance suite | Automated regression run | Per DELIVER commit | crafter/DELIVER |
| 3 | Dedicated GetDocument/BatchGetDocuments parity suite | Automated test run, paired assertions | Per DELIVER commit | crafter/DELIVER |

### Hypothesis
We believe that completing the already-declared `BatchGetDocuments` handler, by reusing `GetDocument`'s own per-document access-control logic N times, for Trailmark-class SDK developers will achieve full data-plane SDK parity for batch document resolution.
We will know this is true when SDK developers (Alex) successfully resolve document-reference batches in a single round trip (100% of valid requests, KPI #1) with zero regression to existing reads (KPI #2) and zero access-control parity violations (KPI #3).

---

## Wave: DISCUSS / [REF] Definition of Ready Validation

### Story: US-01 (batch-get-documents)

| DoR Item | Status | Evidence/Issue |
|----------|--------|----------------|
| 1. Problem statement clear, domain language | PASS | Elevator Pitch states the concrete before/after in domain terms ("resolve Maria's list of 12 favorited trip entries... instead of 12 separate GetDocument round trips"), no technical jargon in the problem framing |
| 2. User/persona identified with specific characteristics | PASS | P1 Alex (SDK Developer), existing persona, plus concrete Trailmark end users (Maria Santos, Dana Kim) in every Domain Example |
| 3. 3+ domain examples with real data | PASS | Exactly 3 Domain Examples with real names, real field values (`owner_id == "maria-santos"`, `trip-entry-77`), no generic placeholders |
| 4. UAT in Given/When/Then (3-7 scenarios) | PASS | 6 scenarios, within 3-7 |
| 5. AC derived from UAT | PASS | AC-01-01 through AC-01-06, each traces directly to a named UAT scenario, no orphan AC |
| 6. Right-sized (1-3 days, 3-7 scenarios) | PASS | Slice estimate: 1 day; 6 scenarios, within 3-7 |
| 7. Technical notes identify constraints | PASS | Technical Notes name the exact reuse points (`handle_get_document`'s own sequence, `handle_run_query`'s own stream-construction pattern) and the exact out-of-scope boundary (mask, consistency_selector — inherited, not new) |
| 8. Dependencies resolved or tracked | PASS | Depends on `security-rules`'s own `get_access_rule`/`evaluate()`/`parse_condition` (shipped) and `BackendAdapter::get_document` (shipped, both adapters). No dependency on `aggregation-queries` (independent, parallel feature) |
| 9. Outcome KPIs defined with measurable targets | PASS | 3 KPIs, each with numeric target, baseline, and measurement method (§ Outcome KPIs) |

### DoR Status: **PASSED** (all 9 items, the one story)

### Requirements Completeness Score: **0.97**

Functional requirements: fully covered (found/missing signaling, per-document access control, empty-batch and cross-project validation). NFRs: security-rules parity (guardrail KPI #3), regression guardrail (KPI #2), performance intent stated qualitatively ("one round trip instead of N") but no numeric latency target set — DESIGN/DEVOPS may add one if evidence justifies it, mirroring `aggregation-queries`' own precedent for the same open item. Business rules: existence non-leakage via `Deny`-rejects-the-whole-batch (Resolution 5), same-project validation, empty-batch rejection — all explicit with rationale.

---

## Wave: DISCUSS / [REF] Out of Scope

- **`DocumentMask` field-projection** — inherited, pre-existing gap: `GetDocument` itself never honors this field today (confirmed by direct code read). Named explicitly so DESIGN does not treat it as this feature's own omission; not a candidate follow-up unique to this feature (shared by `GetDocument`).
- **`consistency_selector` (`transaction`/`new_transaction`/`read_time`) functional honoring** — inherited, pre-existing gap: `GetDocument` never honors its own `transaction`/`read_time` fields today. Every document in a v1 batch is read fresh and independently, with no transactional snapshot guarantee across the batch. Named as a candidate follow-up shared identically by `GetDocument` and `RunQuery`, not unique to this feature.
- **Response ordering guarantees** — not required, matching `docs/SPEC.md`'s own already-documented contract ("Order of responses may differ from request order").
- **Plain-REST (non-gRPC-Web) JSON gateway support** — confirmed pre-existing gap shared identically by every other multi-result RPC (`RunQuery`, `RunAggregationQuery`) today; this feature does not introduce or worsen it. gRPC-Web clients are unaffected (automatic via the existing generic `tonic-web` wrap).
- **`RunAggregationQuery`'s own scope** — unrelated, untouched; a separate, already-completed feature (`aggregation-queries`), per the orchestrator's own explicit framing of these as two independent gaps.
- **Firebase Authentication integration** — inherited exclusion from the original `embyr-rs` DISCUSS, unaffected by this feature.

---

## Wave: DISCUSS / [REF] WS Strategy

**Brownfield extension.** Not a fresh greenfield walking skeleton — `embyr-rs` already has one, and `GetDocument`'s own per-document logic already exists and is already proven correct in production for both backend-mode families. This feature's own walking skeleton (Slice 01) is the complete feature: applying that already-shipped logic N times, per requested document, across both backend modes, with no further increments planned or needed.

---

## Wave: DISCUSS / [REF] Driving Ports

- **gRPC :8080** (`google.firestore.v1.Firestore` service) — the `BatchGetDocuments` RPC is already declared here; this feature completes its handler.
- **gRPC-Web :8081** — automatic via the existing generic `tonic-web` wrap around `FirestoreService`; zero new code required.
- **Plain-REST JSON :8081** — explicitly NOT a driving port for this feature (§ Out of Scope; pre-existing gap, shared by every other multi-result RPC).

---

## Wave: DISCUSS / [REF] Pre-requisites

- `security-rules` (ADR-027/029, `get_access_rule`/`parse_condition`/`evaluate()`) — shipped, hard dependency.
- `BackendAdapter::get_document`, implemented by both `PostgresBackendAdapter` and `AgentBackendAdapter` — shipped, hard dependency, already proven correct for both backend-mode families via `GetDocument`'s own production usage.
- No dependency on `aggregation-queries` — independent, parallel feature touching an unrelated RPC, sharing no code path.
- No dependency on any Identity-track feature (`client-auth`, `client-auth-hosted-identity`, `oauth-providers`) — batch document resolution applies identically to `api_key`-only sessions and any verified-identity session, unchanged from `GetDocument`'s own precedent.

---

## Wave: DISCUSS / [REF] Handoff Package

**Deliverables for solution-architect (DESIGN wave)**:
- This file (`docs/feature/batch-get-documents/feature-delta.md`) — story map, 1 slice, 1 user story with embedded UAT/AC, outcome KPIs, DoR validation (PASSED)
- `docs/feature/batch-get-documents/slices/slice-01-batch-fetch.md`

**One escalated open question, raised by the orchestrator after this DISCUSS's own handoff, not by this DISCUSS itself:** Resolution 5's whole-batch-fails-on-one-denied-document behavior — see the orchestrator finding appended directly under Resolution 5 above. DESIGN must treat this as a genuinely open question (does real Firestore deny per-document or per-call under Security Rules?), not a settled internal decision, before locking the handler's own rejection composition. Every OTHER judgment call this DISCUSS itself encountered (Resolutions 1-4) was resolvable directly from this codebase's own already-established precedent, with zero escalation needed — this DISCUSS did not manufacture those; genuine rigor, not padding.

**Flagged for DESIGN's awareness** (decided in this DISCUSS, with reasoning, not requiring re-litigation unless Resolution 5's own re-examination above changes it): single-slice scope (Resolution 3); per-call vs. per-document granularity split for rate-limiting/auth vs. access-control (Resolution 4); inherited (not new) `mask`/`consistency_selector` scoping gaps (§ System Constraints, § Out of Scope).

Next step (NOT performed by this agent): orchestrator dispatches `nw-solution-architect` for the DESIGN wave, full rigor with ADRs and Reuse Analysis, per the standing session practice.

---

## Wave: DISCUSS / [REF] SSOT Updates

- `docs/product/jobs.yaml` — NOTE appended to JOB-01 documenting this feature's realization (extends, not a new job). See diff below.
- `docs/product/journeys/sdk-developer.yaml` — NOTE appended documenting this feature, mirroring the established cross-reference convention for other JOB-01-realizing features.
