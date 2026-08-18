# security-rules-write-path — Feature Delta

**Wave**: DISCUSS | **Agent**: Luna (nw-product-owner) | **Date**: 2026-08-18
**Status**: Ready for DESIGN handoff
**Upstream**: `security-rules` (FINALIZED 2026-08-18, `docs/evolution/2026-08-18-security-rules.md`) — this feature is Epic 2b of the 5-epic access-control initiative Epic 2a's own Elephant Carpaccio split named (`docs/feature/security-rules/feature-delta.md` § Scope Assessment). Epic 2a shipped rule authoring + read-path (`GetDocument`) enforcement only, deliberately. Right now, in production, a rule defined for a collection gates reads but `CreateDocument`/`UpdateDocument`/`DeleteDocument` never consult `access_rules` at all — this feature closes that gap.

<!-- markdownlint-disable MD024 -->

---

## Wave: DISCUSS / [REF] Density Resolution

`~/.nwave/global-config.json` was not read (no Bash tool available to this agent invocation to run `resolve_density`) — falling back to the documented DISCUSS hard default: `mode=lean`, `expansion_prompt=ask-intelligent`, noted explicitly rather than silently assumed, mirroring `security-rules`' own precedent for `nwave-ai outcomes check-delta` unavailability. Trigger check against this feature's own artifacts: the "multi-stakeholder need" trigger (≥3 personas: Alex, Maria, Dana) technically fires, but Decision 3 (Lightweight) and this task's own instruction that Alex's authoring mental model and Maria/Dana's stakes are already comprehensively established in `security-rules`' own Comprehensive-depth journey mean the correct response is a cross-reference, not a duplicated `persona-narrative` expansion — see § Journey below. No other trigger fires (2 bounded contexts touched, not ≥3; no compliance/regulatory terms; WS strategy is B, not D). Tier-1 [REF] only, no Tier-2 expansions rendered.

---

## Wave: DISCUSS / [REF] Prior Wave Consultation — Reading Confirmation

✓ `docs/feature/security-rules/feature-delta.md` (full, 1302 lines, all DISCUSS+DESIGN+DISTILL sections — the direct structural and methodological precedent for this entire feature; Resolution 1's confidence/escalation note and § Open Questions OQ-SR-02 are the exact seeds this DISCUSS resolves; § Out of Scope's write-path entry confirms this feature's own id was already reserved)
✓ `docs/product/architecture/adr-027-access-rule-grammar-and-evaluation.md` (full — locked v1 grammar EBNF, `Condition`/`Operand`/`CompareOp` AST shape, fail-closed-on-missing-field semantics, the OQ-SR-04 literal-operand gap — the exact machinery this feature must compose with, not replace)
✓ `docs/product/architecture/adr-028-access-rule-storage-and-lifecycle.md` (full — confirms `access_rules` table has exactly ONE row per `(project_id, collection_path)` with a single `condition_source` column; this ADR's own Resolution-3-driven idempotent-upsert reasoning did not anticipate a second, write-specific condition — read-only implication, unstated because 2a only had reads to worry about, now directly relevant)
✓ `docs/product/architecture/adr-029-access-control-composition-and-bounded-context.md` (full — confirms `handle_get_document` is the ONLY call site touched by 2a; confirms BC-4 Access Control's placement reasoning and its read-only dependency on BC-2; confirms identity reuse via `attach_client_identity_if_present()`)
✓ `docs/evolution/2026-08-18-security-rules.md` (full — confirms 2a shipped clean: 6/6 roadmap steps PASS, adversarial review APPROVED zero blocking findings, mutation testing closed 2 genuine gaps; confirms 20 acceptance + 13 unit/property = 33 new tests, full 113-scenario prior regression suite reconfirmed 0 regressions [133 total scenarios this feature must not regress]; § Follow-Up Work explicitly names this feature next and flags Resolution 1's write-path edge as unresolved)
✓ `docs/product/jobs.yaml` (full — JOB-17's own functional-dimension text already names "have embyr enforce them on every request" and its own NOTE already anticipates write-path as this job's own full ambition, not a different goal; JOB-16's cross-reference NOTE re-confirmed unrelated to this feature)
✓ `docs/product/journeys/sdk-developer.yaml` (full — JOB-17 already listed for P1 Alex; extended below with a NOTE, no new job id added)
✓ `crates/embyr-core/src/access_control/mod.rs` (full — `parse_condition()`/`evaluate()`, the `Condition`/`Operand`/`AuthContext`/`EvaluationOutcome` types, the fail-closed-on-missing-field mechanism this feature's Resolution 2 reuses rather than reinvents, and the existing 13 unit/property tests establishing the exact behavioral contract any grammar extension must not break)
✓ `crates/embyr-server/src/admin/handlers/access_rules.rs` (full — `define_access_rule`/`simulate_access_rule`, the exact admin-handler shape and rejection-taxonomy convention [`SYNTAX_ERROR`/`UNSUPPORTED_CONSTRUCT`] this feature's write-rule actions extend)
✓ `crates/embyr-server/src/adapters/system_db.rs` (targeted read: `AccessRuleRow`, `upsert_access_rule`, `get_access_rule`, lines 34, 306-360 — confirms the single-condition-column schema ADR-028 describes, direct evidence for this feature's own schema-extension need)
✓ `crates/embyr-server/src/grpc/handler.rs` (full read of `handle_get_document` [506-616], `handle_create_document` [618-679], `handle_update_document` [681-728], `handle_delete_document` [730-760] — confirms `handle_get_document` is the ONLY handler that calls `attach_client_identity_if_present()` or `get_access_rule()` today; the three write handlers do NOT resolve client identity at all and do NOT consult `access_rules` at all — direct evidence for both central scoping questions below)

No contradictions found between this feature's scope and prior evidence. This feature reverses no founding decision of its own. Two central scoping questions — one named upstream (OQ-SR-02) and one discovered independently by re-reading `security-rules`' own shipped schema (ADR-028) against the raw ask's most common real-world Firestore pattern — are resolved explicitly below (§ Job Discovery Framing Resolution), not silently assumed either direction.

---

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

| # | Decision | Value |
|---|---|---|
| 1 | Feature Type | Backend — extends 3 existing gRPC write handlers with the same `evaluate()` call the read-path already uses; no new bounded context (BC-4 already exists) |
| 2 | Walking Skeleton | Evaluated (this wave's call): **YES** — see § Walking Skeleton Evaluation |
| 3 | UX Research Depth | Lightweight — Alex's authoring mental model and Maria/Dana's stakes are already comprehensively established in `security-rules`' own DISCUSS; this journey section is a short delta, not a re-derivation |
| 4 | JTBD Analysis | Yes (default) — every story traces to `job_id: JOB-17` (extended, not a new job — see § Persona & Job) |

### Walking Skeleton Evaluation (Decision 2 = "Depends")

Two existing mechanisms were evaluated for reuse before concluding what this feature's walking skeleton actually needs to add:

1. **`embyr_core::access_control::{parse_condition, evaluate}` (ADR-027).** Reused, extended, not replaced. The existing `Condition`/`Operand`/`CompareOp` AST and the fail-closed-on-missing-field mechanism compose directly with a new operand family for `request.resource.data.<field>` (§ Resolution 2) — no new evaluator, no new parser generation strategy, no second evaluation routine.
2. **`attach_client_identity_if_present()` (`client-auth`, unchanged).** Reused, but via THREE NEW call sites. Direct evidence (`handler.rs` read in full): today it is called only inside `handle_get_document`. `handle_create_document`/`handle_update_document`/`handle_delete_document` never resolve client identity at all — a session's write request carries no `request.auth` concept whatsoever today, signed-in or not. This is not a hypothetical gap: `security-rules`' own § Out of Scope named it explicitly ("Extending `attach_client_identity_if_present`'s wiring beyond `GetDocument`... relevant to whichever of Epics 2b/2c/2d picks up write/query/listen path next") and `client-auth`'s own evolution doc flagged the same thing from the other direction.

**Verdict**: no existing mechanism (a) stores more than one condition per collection, (b) lets a condition reference two different document states in the same evaluation, or (c) resolves caller identity on any write call. All three are new. A walking skeleton is needed: define an independent write rule for a collection, then have `CreateDocument`/`UpdateDocument`/`DeleteDocument` calls whose caller/proposed-data/existing-data satisfy the rule succeed while ones that do not are denied — while the collection's existing 2a read rule (if any) and every other collection remain provably unaffected (§ Story Map, Slices 01–06).

---

## Wave: DISCUSS / [REF] Job Discovery — Framing Resolution

This feature has **two** central scoping questions. Both are evaluated with the same rigor `security-rules`' own three Resolutions established as this project's precedent for this initiative. Neither is decided silently.

### Resolution 1 — Per-operation condition structure (the more load-bearing question: does one condition gate both read and write, or does write need its own?)

| Option | Description | Fit against evidence |
|---|---|---|
| **(A) Reuse the existing single `condition_source` column for both read and write** | No schema change. The read condition `security-rules` already stores per `(project_id, collection_path)` is also consulted on `CreateDocument`/`UpdateDocument`/`DeleteDocument`. | **Rejected — a silent, retroactive behavior change to every already-shipped `security-rules` rule.** Every collection Alex protected under 2a (e.g. `journal_entries`'s `request.auth.uid == resource.data.owner_id` read rule) would, the instant this feature ships, ALSO start gating writes with that same condition — with zero opt-in, zero migration step, and zero mention in `security-rules`' own DoR-passed, review-approved artifact. It also cannot express the single most common real Firestore pattern (task framing, independently confirmed against real-world usage): "anyone can read, only the owner can write" (a public blog post, a product catalog with owner-only edits) — impossible if read and write share one condition. Real Firestore's own rules syntax (`allow read: if X; allow write: if Y;`) already differentiates the two; Alex's own migration habit (JOB-17's four-force) expects that differentiation to carry over. |
| **(B) A fully separate condition per exact CRUD verb** | Four independent condition slots per collection: read, create, update, delete — matching Firestore's most granular `allow create, update, delete: if ...` capability. | **Rejected for v1 — no evidenced need beyond what Option C already provides.** Every one of this feature's own domain examples (owner-only create validating the new `owner_id`, owner-only update preventing `owner_id` mutation, owner-only delete) is expressible as ONE write condition, differentiated internally via the new `request.resource`/`resource` presence semantics Resolution 2 establishes — a create naturally has no `resource.data` (fails closed if referenced) and a delete naturally has no `request.resource.data` (fails closed if referenced). Splitting create/update/delete into three independently-stored conditions is more storage surface, more admin-API surface, and more DoR/AC combinatorics than any evidenced example requires — over-engineering relative to Principle 8. Named as a candidate follow-up only if a genuine create-vs-update-vs-delete divergent-condition need emerges that Option C's composition cannot express. |
| **(C) Two independent condition slots per collection — the existing read condition (unchanged, from `security-rules`) plus one new write condition gating create/update/delete uniformly** | The write condition is evaluated with `resource`/`request.resource` populated per the actual operation (`resource` absent on create, `request.resource` absent on delete, both present on update — Resolution 2). Read and write conditions are independently defined, redefined, and defaulted; neither ever influences the other. | **Strongest fit.** Matches the raw ask's own most common real-world pattern (public-read/owner-write) without Option A's silent retroactive risk or Option B's unevidenced granularity. A schema EXTENSION (one additional column or a structured condition set), not a redesign — directly satisfies the constraint that the `access_rules` upsert pattern (ADR-028) is a strong precedent to extend. |

**Resolution**: **(C) is locked.** A collection has (up to) two independent conditions: the existing read condition from `security-rules` (unchanged), and a new write condition this feature introduces. **The single most important lock in this feature**: a collection's existing read rule — including every rule already shipped and active in production today — has **zero** effect on writes unless Alex explicitly defines a separate write rule for that same collection. Redefining one condition never touches the other. This is this feature's own regression-safety guardrail, mirroring `security-rules`' AC-17-14/15/16 shape exactly, and is the AC this feature treats as its single highest-consequence design risk (§ System Constraints, AC-17-43).

**Confidence and escalation note**: high confidence — independently supported by (a) the raw ask's own most-common-pattern framing, (b) the retroactive-behavior-change risk Option A would silently introduce to an already-shipped, already-review-approved feature, and (c) real Firestore's own syntax already drawing this exact distinction. No repository evidence contradicts it.

### Resolution 2 — OQ-SR-02: does write-path need `resource`/`request.resource` as two distinct grammar symbols?

**Resolution**: **Yes.** Direct evidence from this feature's own three domain-example shapes (task framing, independently re-derived from the grammar's existing fail-closed mechanism):

- **Create**: only the proposed new data exists. A condition referencing `request.resource.data.<field>` (new operand family) works normally; a condition referencing the EXISTING `resource.data.<field>` operand (unchanged from `security-rules`) evaluates against an **empty field map** — reusing ADR-027's fail-closed-on-missing-field mechanism verbatim, not a new mechanism. No document exists yet, so any reference to it is "missing," which already denies by construction.
- **Update**: both `resource.data.<field>` (the document's state immediately before this write) and `request.resource.data.<field>` (the proposed new state) are populated. A condition may reference both in the same expression — e.g. `resource.data.owner_id == request.auth.uid && request.resource.data.owner_id == resource.data.owner_id` (only the owner may update, and may not change the owner). This is the two-value comparison OQ-SR-02 named, and it composes with the EXISTING `&&`/`||`/`!`/`==`/`!=` combinators without needing new operators.
- **Delete**: only `resource.data.<field>` is populated; `request.resource.data.<field>` evaluates against an empty field map (fails closed if referenced — correctly denying any condition that tries to validate "new" data during a delete, since there is none).

**What this does NOT require, and is explicitly not adding**: a new "is this document present at all" sentinel operand (e.g. a `resource == null` whole-object null check). No domain example in this feature needs one — every example references specific fields, and the existing fail-closed-on-missing-field mechanism already produces the correct `Deny` outcome for any condition that assumes a document exists when it doesn't (or vice versa), exactly the way AC-17-09/AC-17-10 already work in `security-rules`. Inventing a whole-object presence sentinel now would be scope not evidenced by any story below — flagged as **OQ-SRW-01** (§ Open Questions) rather than built speculatively.

**What DESIGN must do with this (not DISCUSS's job)**: extend ADR-027's `Operand` enum with a new `RequestResourceField(String)` variant and extend `parse_condition`'s tokenizer to recognize `request.resource.data.<field>` as a distinct operand from `resource.data.<field>` (currently only `resource.data.` is recognized — `word_to_operand` would need a second `starts_with` branch). This is a composition, not a redesign, per the constraint that `embyr_core::access_control`'s existing AST shape must be extended, not replaced.

---

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: **P1 — Alex, SDK Developer** (existing persona, unchanged from `security-rules`/`client-auth`). No new persona work — Decision 3 (Lightweight) plus this task's own framing confirm Alex's authoring mental model is already established.

**Domain-example company**: **Trailmark**, continued. Collections: `journal_entries` (Maria's/Dana's private trip-journal documents, `owner_id` field — the established example, continued per instruction), `trail_guides` (Trailmark's published content, publicly readable per its existing 2a rule, `request.auth != null`-gated in some examples), `app_config` (never protected by any rule, read or write).

**job_id decision (per Decision 4)**: **this feature extends JOB-17 (`document-access-control`) — it does not mint a new job.** Reasoning, made explicit per this task's instruction:

JOB-16→JOB-17 was a "same persona, different goal ⇒ new job" call: JOB-16 is about *establishing* identity, JOB-17 is about *consuming* it to gate access — functionally, emotionally, and socially distinct goals for the same persona (`jobs.yaml` JOB-17's own NOTE quotes this precedent explicitly: JOB-11 vs. JOB-06, JOB-14 vs. JOB-10).

This feature is not that pattern. It is the **same** goal — per-collection access-control enforcement — extended to a new operation type. Direct textual evidence from JOB-17's own already-written `job_story` (`jobs.yaml`, written 2026-08-17, before this feature existed): *"I want to define access-control rules per collection and have embyr enforce them **on every request**"* — "every request" already named writes. JOB-17's own functional-dimension NOTE, also written before this feature: *"v1 (this feature, `security-rules`) implements read-path (`GetDocument`) enforcement only — write-path, query-path, and real-time-listen enforcement are named, deferred follow-up epics."* JOB-17 declared its own full ambition at the time it was created and named `security-rules-write-path` as the mechanism that would realize more of it — this is structurally identical to the project's own "make it real" extension pattern (JOB-14 → `card-payments-backend`, JOB-10 → `admin-api-v2`), where a feature realizes more of an ALREADY-STATED job's scope, not the "different goal" pattern that produced JOB-17 itself. See `docs/product/jobs.yaml` for the NOTE added under JOB-17 documenting this extension (§ SSOT Updates).

**Opportunity scoring**: JOB-17's existing opportunity score (17, priority critical) is unchanged — this feature does not create a new job, so it inherits JOB-17's existing score rather than computing a fresh one. The urgency case for THIS feature specifically: per the task's own framing, a state where reads look protected and writes are wide open is arguably worse than no rules at all, since it is actively misleading to Alex, who reasonably assumes "I defined a rule for `journal_entries`" means the collection is protected, full stop.

---

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

Run before story-map investment, per Phase 1.5, checked honestly against the same 5-signal gate `security-rules` used for its own Pass 2 (within-epic) assessment — this feature is itself already a Carpaccio slice of a larger ambition, so this is a re-check, not a first check.

| Signal | Threshold | This feature | Fired? |
|---|---|---|---|
| User stories | >10 | 7 (US-01 through US-07) | **NO** |
| Bounded contexts / modules | >3 | 2 — BC-4 Access Control (extended: new write-condition slot, new grammar operand) + BC-2 Document Storage's existing write handlers (`CreateDocument`/`UpdateDocument`/`DeleteDocument`); incidental reuse of BC-1's already-existing `attach_client_identity_if_present()` is not a new dependency shape (2a's `GetDocument` already has the identical dependency) | **NO** |
| Walking Skeleton integration points | >5 | 6 — define-write-rule + create-eval + update-eval + delete-eval + anonymous-write-eval + read/write-independence-and-regression-proof | **Borderline (at 6, just over)** |
| Estimated effort | >2 weeks | 7 slices, ~9.5 days total (§ Elephant Carpaccio Slices) | **NO** (at the edge, not exceeding) |
| Independent shippable outcomes | multiple | **NO** — create/update/delete gating and the anonymous/isolation guardrails are inseparable halves of one outcome ("writes are no longer wide open"), mirroring `security-rules`' own US-02/03/04 "inseparable halves" reasoning; simulation (US-07) is a normal Release 2 enhancement, not a second walking-skeleton-level outcome | **NO** |

**1 of 5 signals fires** (threshold for oversized is 2+). **Verdict: PASS — right-sized.** The one borderline signal (6 WS integration points, just over the >5 threshold) reflects that three genuinely distinct operation shapes (create/update/delete) each need their own real-data proof — collapsing any two into one slice would violate the "no facade, real state" taste test `security-rules` itself established, not reduce genuine complexity. No further split proposed; Decision 1 (Backend, narrower framing) and Decision 3 (Lightweight) already kept this feature's scope tighter than `security-rules`' own Comprehensive-depth first pass.

---

## Wave: DISCUSS / [REF] Journey — Write-Path Evaluation Flow (Lightweight)

Per Decision 3, this is a short delta on `security-rules`' own Comprehensive-depth journey, not a re-derivation. Alex's authoring mental model, his anxious→confident emotional arc, and Maria/Dana's consequence arc are all established in full there (`docs/feature/security-rules/feature-delta.md` § Journey) and are not reproduced here — only what changes for write-path is captured below.

### What's new in Alex's mental model

Alex now expects two things real Firestore also gives him: (1) a rule for `allow write` is a **separate** thing from `allow read` — he does not expect defining one to touch the other; (2) inside a write condition, he can reference `resource` (the document as it exists right now, before this write) and `request.resource` (the document as it would exist if this write is allowed) as two different things — exactly the `resource`/`request.resource` distinction his existing `firestore.rules` file already uses for update rules.

### Write-evaluation flow (extends `security-rules`' own read-evaluation flow)

```
A CreateDocument/UpdateDocument/DeleteDocument call arrives
        │
   (authenticate() unchanged; attach_client_identity_if_present() now ALSO
    called here — NEW wiring, function itself unchanged — request.auth is
    Some(VerifiedEndUserIdentity) or None, exactly as GetDocument already has)
        │
        ▼
   Does this collection have a WRITE rule defined? (independent of any
   READ rule the same collection may have — Resolution 1)
        │
   no ──────────────────────────────┐              yes
        │                            │               │
        ▼                            │               ▼
  Write proceeds exactly as          │     Fetch resource (pre-write state,
  before this feature shipped        │     empty on create) — request.resource
  (AC-17-42) — including if the      │     is the proposed new data (empty on
  same collection HAS a read rule    │     delete)
  (AC-17-43, the central lock)       │               │
        │                            │               ▼
        │                            │     evaluate(condition, auth, resource,
        │                            │     request.resource) — same function,
        │                            │     new operand family (Resolution 2)
        │                            │      ┌────────┴─────────┐
        │                            │    Allow               Deny
        │                            │      │                  │
        │                            │      ▼                  ▼
        │                            │   Write proceeds    PermissionDenied,
        │                            │   exactly as        identical whether
        │                            │   requested         or not the target
        └────────────────────────────┴──────────────────  document exists
```

---

## Wave: DISCUSS / [REF] Story Map & Walking Skeleton

**Persona**: P1 Alex | **Goal**: Give each collection Alex chooses real per-operation write protection — independent of any read protection the same collection has — without affecting any collection or operation he hasn't touched.

### Backbone

| A. Alex Defines Write Protection | B. A Write Is Evaluated Against the Rule | C. Alex Builds Confidence Before Publishing |
|---|---|---|
| Alex defines a write rule for a collection, independent of its read rule **[WS]** | A create satisfying the write rule succeeds; one that doesn't is denied **[WS]** | Alex simulates a candidate write rule against synthetic old/new document pairs |
| | An update honoring both old and new document state succeeds or is denied correctly **[WS]** | |
| | A delete satisfying the write rule succeeds; one that doesn't is denied **[WS]** | |
| | An anonymous write session is evaluated as `request.auth == null` **[WS]** | |
| | A collection with no write rule keeps writing exactly as before — including one with only a read rule **[WS]** | |

### Walking Skeleton

Alex defines a write rule for `journal_entries` requiring the caller to own the document and forbidding changing its `owner_id` (Activity A). Maria's create of her own new journal entry succeeds; her update preserving `owner_id` succeeds; her attempt to change `owner_id` on update is denied; Dana's update/delete of Maria's entry is denied; an anonymous session's write is denied (Activity B, Slices 02–05). `trail_guides` — which already has a 2a READ rule but no write rule of its own — keeps accepting writes from anyone exactly as before, and `app_config` keeps working exactly as before (Activity B, Slice 06 — the central regression/independence proof). This mirrors `security-rules`' own WS discipline: no facade, real project/rule/identity/document state.

### Release 1 — Write-Path Protection Works End-to-End (Slices 01–06, US-01 through US-06)

Outcome: any collection Alex protects with a write rule genuinely gates create/update/delete by identity and by old/new document state, for signed-in and anonymous callers alike, while collections he hasn't touched — and collections protected only on read — remain exactly as they were.

### Release 2 — Authoring Confidence, Extended (Slice 07, US-07)

Outcome: Alex can prove a write rule does what he intended, across all three operation shapes, before any real end user is affected — extending `security-rules`' own US-05 guarantee to writes.

---

## Wave: DISCUSS / [REF] Elephant Carpaccio Slices

| Slice | Story | Release | Estimate | Learning Hypothesis (disproves) | Production-data taste test |
|---|---|---|---|---|---|
| 01 (WS) | US-01 | 1 | 1.5 days | An independent write condition cannot be added to the existing `access_rules` schema, defined/redefined idempotently, without either colliding with the existing read condition or requiring a wholesale schema redesign | Real System DB row extension, real admin Bearer credential, real existing 2a read rule left provably untouched |
| 02 (WS) | US-02 | 1 | 1.5 days | A write condition referencing only `request.resource` cannot be evaluated for a real `CreateDocument` call without the existing fail-closed-on-missing-field mechanism (ADR-027) needing a new, second mechanism to handle the "no document exists yet" case | Real registered write rule + real Maria/Dana signed-in sessions + real `journal_entries` create calls |
| 03 (WS) | US-03 | 1 | 2 days | A condition cannot meaningfully compare a document's pre-write state against its proposed new state (the two-value comparison OQ-SR-02 named) without either fetching the pre-write document via a new, separate mechanism from `GetDocument`'s own fetch-then-decide pattern, or requiring more grammar investment than one new operand family provides | Real owner-preserving and owner-changing update payloads against real `journal_entries` documents |
| 04 (WS) | US-04 | 1 | 1 day | A condition referencing only `resource` (delete) cannot reuse the exact same fail-closed mechanism that already handles `request.resource`'s absence on delete, without asymmetric special-casing | Real Maria/Dana delete calls against real owned/non-owned `journal_entries` documents |
| 05 (WS) | US-05 | 1 | 1 day | Extending `attach_client_identity_if_present()` to three new call sites cannot be done without either modifying the function itself (risking `client-auth`'s own regression suite) or inventing a second identity-resolution path for writes | Real never-signed-in and real invalid-header sessions against real write-rule-gated collections |
| 06 (WS) | US-06 | 1 | 1.5 days | A collection's existing read rule cannot be proven to have zero effect on that same collection's writes, and untouched collections/the full prior regression suite cannot be proven unaffected, without actually re-running all 133 pre-existing scenarios (113 `embyr-rs`/`client-auth` + 20 `security-rules`) unmodified | Real full regression suite, real multi-collection project state with mixed read-only/write-only/both/neither rule configurations |
| 07 | US-07 | 2 | 1 day | A write-rule simulation action cannot share the exact same evaluation routine as real write enforcement without duplicating (and risking drift in) the extended evaluation logic | Real candidate write rules + real synthetic old/new document pairs checked against the real evaluation path |

**Total estimate: ~9.5 days.**

**Taste tests applied**:
- "4+ new components per slice" — none exceeds 2 (Slice 01: schema extension + admin handler extension; Slice 02: new operand family in the existing evaluator + `CreateDocument` wiring; Slice 03: extends Slice 02's evaluator, no new component, but adds a document-fetch step to `handle_update_document`; Slice 04: extends Slice 03, no new component; Slice 05: extends `attach_client_identity_if_present()`'s call sites, no new component; Slice 06: zero new components, pure regression/independence proof; Slice 07: thin wrapper over Slices 02–04's evaluator). PASS.
- "Every slice depends on a new abstraction" — Slice 01 (the write-condition schema slot) and Slice 02 (the `request.resource` operand family) are the two genuinely new abstractions; Slices 03–07 build on them but do not each introduce a new one. PASS — natural sequencing, not forced dependency inflation.
- "No slice disproves a pre-commitment" — each has a distinct, falsifiable hypothesis (see table). PASS.
- "Synthetic-data-only slices prove plumbing, not value" — N/A; all 7 slices require real System DB state, real signed-in/anonymous sessions, and (Slice 06) the real 133-scenario regression suite. PASS.
- "2+ slices identical except for scale" — none; each targets a distinct mechanism (define vs. create-eval vs. update-eval vs. delete-eval vs. anonymous vs. independence-proof vs. simulate). PASS.

---

## Wave: DISCUSS / [REF] Prioritization

| Priority | Slice | Target Outcome | Rationale |
|---|---|---|---|
| 1 | Slice 01 (WS) | A collection can have an independent write rule defined | Prerequisite — without a stored write condition, Slices 02–06 have nothing to evaluate against |
| 2 | Slice 03 (WS) | An update correctly compares old vs. new document state | Burns down the riskiest new assumption FIRST (the two-value comparison OQ-SR-02 named, and the fetch-before-write plumbing it requires) — sequenced ahead of the simpler create/delete cases so the hardest mechanism is proven before building on it |
| 3 | Slice 02 (WS) | A create correctly evaluates against only the proposed new document | Simpler subset of Slice 03's mechanism (only `request.resource`, no fetch needed — resource is naturally absent) |
| 4 | Slice 04 (WS) | A delete correctly evaluates against only the existing document | Simplest of the three operation shapes — mirrors Slice 03's fetch-before-decide pattern without the two-value comparison |
| 5 | Slice 05 (WS) | An anonymous write session is evaluated correctly | Extends identity-attach wiring across all three newly-gated call sites at once, now that Slices 02–04 prove the evaluation mechanism itself works |
| 6 | Slice 06 (WS) | Untouched collections, read-only-protected collections, and the full 133-scenario regression suite are provably unaffected | The single highest-consequence regression risk in this feature (Resolution 1's central lock); sequenced last within the WS because it is a proof *over* Slices 01–05's real behavior |
| 7 | Slice 07 | Alex can test a write rule before publishing it | Highest-leverage for Alex's own confidence; depends conceptually on Slices 02–04's evaluation mechanism existing to wrap, correctly sequenced after the Walking Skeleton |

---

## Wave: DISCUSS / [REF] System Constraints

- **Two independent condition slots per collection (Resolution 1).** A collection's read condition (`security-rules`, unchanged) and write condition (this feature, new) are defined, redefined, and defaulted completely independently. Redefining one MUST NOT touch the other. This is a schema EXTENSION to `access_rules` (ADR-028) — DESIGN's call whether that is an additional nullable column or a structured condition set — not a redesign of the existing read-condition mechanism.
- **The single highest-consequence regression lock**: a collection's existing `security-rules` read rule — including every rule already active in production today — has **zero** effect on writes unless Alex explicitly defines a separate write rule for that exact collection (AC-17-43). DESIGN must not implement this feature in any way that makes an existing read rule implicitly govern writes, even as a convenience default.
- **No write rule defined ⇒ unrestricted writes**, gated only by the existing `api_key` check — mirrors `security-rules`' own Resolution 2 discipline, now applied to the new write-condition slot specifically.
- **Grammar extension (Resolution 2, resolves OQ-SR-02): add `request.resource.data.<field>` as a new `Operand` variant.** The existing `resource.data.<field>` operand's semantics are UNCHANGED — for writes it means the document's state immediately before this write (naturally empty/absent on create). `request.resource.data.<field>` is naturally empty/absent on delete. Both reuse ADR-027's existing fail-closed-on-missing-field mechanism verbatim — no new null-sentinel operand, no new evaluator branch type, no per-operator null-propagation semantics invented.
- **Identity-attach wiring gap must be closed.** `attach_client_identity_if_present()` (unchanged) must be called from `handle_create_document`, `handle_update_document`, and `handle_delete_document` — three new call sites, zero modification to the function itself. This was an explicitly anticipated, named open item from both `client-auth`'s own Follow-Up Work and `security-rules`' own § Out of Scope.
- **Evaluating a write rule that references `resource.data.<field>` on update or delete requires reading the pre-write document state before the write executes** — a fetch step analogous to `GetDocument`'s own fetch-then-decide pattern. Observable requirement only; exact implementation (a dedicated fetch, reuse of an existing read path, or something else) is DESIGN's call. No such fetch is needed for create (resource is naturally empty, no I/O required).
- **Existence non-leakage extends to update/delete**, scoped identically to `security-rules`' own OQ-SR-06 precedent: a denied update/delete must not reveal, via any difference in its response, whether the target document existed, for a rule that references document content. A content-blind write rule (e.g. `request.auth != null`) may still incidentally reveal existence via the underlying write operation's own not-found/already-exists error — this feature does not attempt to mask that, mirroring `security-rules`' own honest scoping.
- **v1 write-enforcement surface is `CreateDocument`/`UpdateDocument`/`DeleteDocument` only.** `RunQuery`, `BeginTransaction`/`Commit`, and `Listen` remain out of scope — named, deferred follow-up epics (2c, 2d). `handle_get_document` itself is untouched by this feature beyond continuing to work exactly as `security-rules` left it.
- **Rule-expressiveness ceiling otherwise unchanged.** `security-rules`' Resolution 1 (no cross-document reads, custom functions, wildcard paths, custom claims) remains locked; only the new `request.resource.data.<field>` operand family is added. The OQ-SR-04 literal-operand gap (booleans only, no arbitrary string/number literals) is NOT reopened or resolved by this feature.
- Ubiquitous language extended: **write rule** / **write condition** (the new, independent condition gating create/update/delete for a collection), **request.resource** (the proposed new document state — present on create/update, absent on delete), distinguishing it from the existing **resource** (the document's state immediately before this write — absent on create, present on update/delete).

---

## Wave: DISCUSS / [REF] User Stories

### US-01: Alex Defines (and Redefines) an Independent Write Rule for a Collection

**job_id**: JOB-17
**Slice**: 01 (Walking Skeleton) | **Release**: 1

#### Elevator Pitch
Before: Alex protected `journal_entries`' reads with a `security-rules` rule, but every one of Trailmark's signed-in end users can still create, update, or delete any document in that same collection — the read rule has nothing to say about writes.
After: call the admin API's write-rule-definition action (exact endpoint shape DESIGN's call) for `journal_entries` with a condition such as `resource.data.owner_id == request.auth.uid && request.resource.data.owner_id == resource.data.owner_id` → sees confirmation the write rule is stored and active, entirely independent of the collection's existing read rule; calling the same action again with a different condition immediately replaces only the write rule.
Decision enabled: Alex knows a specific collection's writes are now protected by the exact condition he wrote, distinct from and unaffected by whatever read protection the same collection already has, and can move on to verifying it (US-07) before real end users are affected.

#### Domain Examples
1. **Happy Path**: Alex defines, for the first time, a write rule on `journal_entries` in `trailmark-prod`: `resource.data.owner_id == request.auth.uid && request.resource.data.owner_id == resource.data.owner_id`. Sees confirmation the write rule is stored and active; `journal_entries`' existing 2a read rule (`request.auth.uid == resource.data.owner_id`) is unchanged.
2. **Edge Case**: An hour later, Alex redefines `journal_entries`' write rule to also forbid changing a second field. The new write condition fully replaces the old one immediately; the collection's read rule remains exactly as it was throughout — untouched by either write-rule change.
3. **Error/Boundary**: Alex submits a write condition referencing `request.resource.data.owner_id.token` — a nested-path shape outside the v1 grammar. Sees the same distinguishable rejection taxonomy (`SYNTAX_ERROR` vs. `UNSUPPORTED_CONSTRUCT`) `security-rules`' read-rule definition already uses.

#### UAT Scenarios (BDD)

##### Scenario: First-time write-rule definition succeeds and is independent of the read rule
Given project `trailmark-prod` exists, `journal_entries` has an active read rule from `security-rules`, and no write rule yet
When Alex defines a write rule with a valid condition referencing both `resource.data` and `request.resource.data`, using a valid admin Bearer credential
Then the write rule is stored and active for `journal_entries`, and the existing read rule is unchanged

##### Scenario: Redefining a write rule fully replaces it, with no overlap window, and never touches the read rule
Given `journal_entries` already has an active write rule and an active read rule
When Alex submits a new write condition for the same collection
Then the new write condition is immediately and fully active, the previous write condition no longer applies, and the read rule remains exactly as it was

##### Scenario: A write condition using the new request.resource operand parses successfully
Given project `trailmark-prod` exists
When Alex submits a write condition referencing `request.resource.data.owner_id`
Then the condition is accepted and stored

##### Scenario: A write condition using an out-of-grammar construct is rejected, naming what's unsupported
Given project `trailmark-prod` exists
When Alex submits a write condition that calls `get()` on another document
Then the request is rejected with a message naming that cross-document reads are not supported, distinguishable from a plain syntax error

##### Scenario: Write-rule definition without valid admin credentials is rejected
Given project `trailmark-prod` exists
When Alex submits a write-rule-definition request with a missing or invalid admin Bearer credential
Then the request is rejected the same way any other admin endpoint rejects missing/invalid credentials

#### Acceptance Criteria
- [ ] AC-17-20: A valid first-time write-rule definition is stored and active for the named collection.
- [ ] AC-17-21: Redefining a collection's write rule fully and immediately replaces the prior condition — no merge, no overlap window.
- [ ] AC-17-22: Defining or redefining a WRITE rule has zero observable effect on that same collection's READ rule, and vice versa — independent conditions, independently upsertable.
- [ ] AC-17-23: A condition using the new `request.resource.data.<field>` operand parses successfully.
- [ ] AC-17-24: A condition using a construct outside the grammar is rejected with the same distinguishable `UNSUPPORTED_CONSTRUCT`/`SYNTAX_ERROR` taxonomy `security-rules` already established.
- [ ] AC-17-25: Missing or invalid admin Bearer credential is rejected, consistent with existing admin-endpoint behavior.

#### Outcome KPIs
See § Outcome KPIs below (feeds KPI #1, North Star, and KPI #2, Guardrail).

#### Technical Notes (Optional)
Exact endpoint shape and schema-extension mechanism (additional column vs. structured condition set) are DESIGN's call — this story locks observable behavior only. Reuse `parse_condition`'s existing validation-at-write-time pattern (`access_rules.rs::define_access_rule`) rather than inventing a second validation path.

---

### US-02: A Create Is Gated by the Write Rule Using Only the Proposed New Document

**job_id**: JOB-17
**Slice**: 02 (Walking Skeleton) | **Release**: 1

#### Elevator Pitch
Before: any of Trailmark's signed-in end users can create a `journal_entries` document with any `owner_id` they like, including someone else's — nothing validates the proposed data against who's creating it.
After: call the SDK's existing `addDoc()`/`setDoc()` on `journal_entries` — an unchanged SDK method — now evaluated against Trailmark's published write rule → Maria's creation of a new entry with `owner_id: "maria-santos"` succeeds; her attempt to create one with `owner_id: "dana-kim"` is denied.
Decision enabled: Alex knows new documents entering a protected collection are validated against the caller's own identity from the moment they're created, not just after the fact on read.

#### Domain Examples
1. **Happy Path**: Maria Santos (`end_user_id: maria-santos`), signed in, creates a new `journal_entries` document with `owner_id: "maria-santos"`. The write rule's condition evaluates true against the proposed new document. The create succeeds.
2. **Edge Case**: Maria attempts to create a `journal_entries` document with `owner_id: "dana-kim"` (someone else's id). The condition evaluates false against the proposed document. The create is denied, attributable to the rule.
3. **Error/Boundary**: `journal_entries`' write rule additionally references `resource.data.owner_id` (the pre-existing document, which cannot exist yet during a create). That reference evaluates as fail-closed/absent — reusing the exact mechanism AC-17-09 already established — never a crash, and the overall condition denies unless composed with an explicit create-permitting branch.

#### UAT Scenarios (BDD)

##### Scenario: A create whose proposed document satisfies the write rule succeeds
Given `journal_entries` has a write rule requiring `request.resource.data.owner_id == request.auth.uid`
And Maria Santos holds a verified identity
When Maria creates a new `journal_entries` document with `owner_id: "maria-santos"`
Then the create succeeds

##### Scenario: A create whose proposed document fails the write rule is denied
Given `journal_entries` has a write rule requiring `request.resource.data.owner_id == request.auth.uid`
And Maria Santos holds a verified identity
When Maria attempts to create a `journal_entries` document with `owner_id: "dana-kim"`
Then the create is denied with PermissionDenied, attributable to the rule

##### Scenario: A write rule referencing the pre-existing document fails closed during create
Given `journal_entries` has a write rule that also references `resource.data.owner_id`
When any signed-in end user creates a new `journal_entries` document
Then the reference to `resource.data.owner_id` evaluates as absent (fail-closed), and no internal error or crash occurs

##### Scenario: A write rule not based on ownership allows any signed-in caller to create
Given `trail_guides` has a write rule requiring only `request.auth != null`
When Dana Kim, signed in, creates a new `trail_guides` document
Then the create succeeds

#### Acceptance Criteria
- [ ] AC-17-26: A create whose proposed new document satisfies the write rule succeeds.
- [ ] AC-17-27: A create whose proposed new document fails the write rule is denied, attributable to the rule.
- [ ] AC-17-28: A write rule that references `resource.data.<field>` evaluates that reference as fail-closed/absent during create, since no document exists yet — never a crash.
- [ ] AC-17-29: A write rule not based on ownership correctly allows a signed-in caller to create a document.

#### Outcome KPIs
See § Outcome KPIs below (KPI #1 North Star).

#### Technical Notes (Optional)
No document fetch is required to evaluate `request.resource.data.<field>` on create — the proposed data is already in the request body. `resource.data.<field>` references use an empty field map (no I/O), matching this feature's own § System Constraints note that no fetch is needed for create.

---

### US-03: An Update Is Gated by the Write Rule Using Both the Existing and Proposed Document

**job_id**: JOB-17
**Slice**: 03 (Walking Skeleton) | **Release**: 1

#### Elevator Pitch
Before: Maria's `journal_entries` document can be updated by anyone, including changing its `owner_id` to someone else entirely — nothing compares what the document looked like before against what it's being changed to.
After: call the SDK's existing `updateDoc()` on `journal_entries/maria-trip-042` → Maria's update preserving `owner_id` succeeds; her attempt to change `owner_id` to `dana-kim` is denied; Dana's attempt to update Maria's document at all is denied.
Decision enabled: Alex knows a rule can protect not just who may touch a document, but what they're allowed to change about it — the exact "immutable field" pattern real Firestore apps rely on.

#### Domain Examples
1. **Happy Path**: Maria Santos updates `journal_entries/maria-trip-042` (`owner_id: "maria-santos"`), changing only its `title` field, leaving `owner_id` unchanged. The condition (`resource.data.owner_id == request.auth.uid && request.resource.data.owner_id == resource.data.owner_id`) evaluates true against both the existing and proposed document. The update succeeds.
2. **Edge Case**: Maria attempts to update the same document, changing `owner_id` to `"dana-kim"`. The second half of the condition evaluates false (`request.resource.data.owner_id` no longer equals `resource.data.owner_id`). The update is denied — the immutable-field protection this story exists to prove.
3. **Error/Boundary**: Dana Kim, signed in, attempts to update `journal_entries/maria-trip-042`, a document she doesn't own. The first half of the condition evaluates false regardless of what she's proposing to change. The update is denied, with a response identical to what she'd see if the document didn't exist at all.

#### UAT Scenarios (BDD)

##### Scenario: An update preserving the owner field succeeds
Given `journal_entries` has a write rule requiring `resource.data.owner_id == request.auth.uid && request.resource.data.owner_id == resource.data.owner_id`
And `journal_entries/maria-trip-042` has `owner_id: "maria-santos"`
When Maria Santos updates that document, changing only its `title` field
Then the update succeeds

##### Scenario: An update by a non-owner is denied
Given the same write rule and document as above
When Dana Kim, signed in, attempts to update `journal_entries/maria-trip-042`
Then the update is denied with PermissionDenied

##### Scenario: An update attempting to change the protected field is denied
Given the same write rule and document as above
When Maria Santos updates `journal_entries/maria-trip-042`, changing `owner_id` to `"dana-kim"`
Then the update is denied — the proposed new `owner_id` no longer matches the existing one

##### Scenario: An update evaluation fails closed if a referenced field is missing from either document state
Given `journal_entries` has the same write rule
And a `journal_entries` document exists with no `owner_id` field at all
When any signed-in end user attempts to update that document
Then the update is denied, and no internal error or crash occurs

##### Scenario: A denied update never reveals whether the target document existed
Given `journal_entries` has an ownership-based write rule
When Dana Kim attempts to update a document she doesn't own, and separately a document ID that doesn't exist at all
Then both attempts return the identical PermissionDenied response, with no distinguishing detail

#### Acceptance Criteria
- [ ] AC-17-30: An update where the caller owns the existing document and the proposed new document does not change the protected field succeeds.
- [ ] AC-17-31: An update where the caller does not own the existing document is denied.
- [ ] AC-17-32: An update where the caller owns the existing document but the proposed new document changes the protected field is denied — the two-value (old vs. new) comparison this feature exists to prove.
- [ ] AC-17-33: An update evaluation fails closed if the referenced field is missing from either the existing or the proposed document.
- [ ] AC-17-34: A denied update's response does not reveal whether the target document existed, for a rule referencing document content.

#### Outcome KPIs
See § Outcome KPIs below (KPI #1 North Star, KPI #3 Guardrail).

#### Technical Notes (Optional)
Requires fetching the document's pre-write state before evaluating (§ System Constraints) — analogous to `GetDocument`'s existing fetch-then-decide pattern, but inside `handle_update_document`. Must consume the existing `VerifiedEndUserIdentity`/`None` value from the newly-wired `attach_client_identity_if_present()` call (US-05), not re-verify identity separately.

---

### US-04: A Delete Is Gated by the Write Rule Using Only the Existing Document

**job_id**: JOB-17
**Slice**: 04 (Walking Skeleton) | **Release**: 1

#### Elevator Pitch
Before: any of Trailmark's signed-in end users can delete any `journal_entries` document, including one they don't own.
After: call the SDK's existing `deleteDoc()` on `journal_entries/maria-trip-042` → Maria's delete of her own entry succeeds; Dana's delete of that same entry is denied.
Decision enabled: Alex knows a document can't be permanently removed by anyone other than who the rule says may remove it — the simplest and highest-stakes of the three write shapes.

#### Domain Examples
1. **Happy Path**: Maria Santos deletes `journal_entries/maria-trip-042` (`owner_id: "maria-santos"`). The write rule's ownership half evaluates true against the existing document (`request.resource.data` is naturally absent on delete, so only the `resource.data` half of the condition is meaningful here). The delete succeeds.
2. **Edge Case**: Dana Kim attempts to delete that same document. The ownership check evaluates false. The delete is denied.
3. **Error/Boundary**: A `journal_entries` document written before `owner_id` existed as a field is missing it entirely. Any caller's delete attempt against it fails closed (denied), never a crash.

#### UAT Scenarios (BDD)

##### Scenario: A delete by the owning end user succeeds
Given `journal_entries` has a write rule requiring `resource.data.owner_id == request.auth.uid`
And `journal_entries/maria-trip-042` has `owner_id: "maria-santos"`
When Maria Santos deletes that document
Then the delete succeeds

##### Scenario: A delete by a non-owning end user is denied
Given the same write rule and document as above
When Dana Kim, signed in, attempts to delete `journal_entries/maria-trip-042`
Then the delete is denied with PermissionDenied

##### Scenario: A delete evaluation fails closed on a missing referenced field
Given `journal_entries` has the same write rule
And a `journal_entries` document exists with no `owner_id` field at all
When any signed-in end user attempts to delete that document
Then the delete is denied, and no internal error or crash occurs

##### Scenario: A denied delete never reveals whether the target document existed
Given `journal_entries` has an ownership-based write rule
When Dana Kim attempts to delete a document she doesn't own, and separately a document ID that doesn't exist at all
Then both attempts return the identical PermissionDenied response, with no distinguishing detail

#### Acceptance Criteria
- [ ] AC-17-35: A delete where the caller owns the existing document succeeds.
- [ ] AC-17-36: A delete where the caller does not own the existing document is denied.
- [ ] AC-17-37: A delete evaluation fails closed on a missing referenced field, never crashes.
- [ ] AC-17-38: A denied delete's response does not reveal whether the target document existed, for a rule referencing document content.

#### Outcome KPIs
See § Outcome KPIs below (KPI #1 North Star, KPI #3 Guardrail).

#### Technical Notes (Optional)
Same fetch-before-decide requirement as US-03, without the two-value comparison — `request.resource.data.<field>` references evaluate against an empty field map (no proposed new data exists on delete).

---

### US-05: A Session With No Verified Identity Is Evaluated as Anonymous on Writes

**job_id**: JOB-17
**Slice**: 05 (Walking Skeleton) | **Release**: 1

#### Elevator Pitch
Before: a Trailmark session that never signs in can create, update, or delete documents identically to one that did — writes have no concept of `request.auth` at all today, unlike reads.
After: an unsigned-in session calls the existing `addDoc()`/`updateDoc()`/`deleteDoc()` on a collection whose write rule requires `request.auth != null` → sees a permission-denied error, identical in shape to the wrong-owner case but attributable to missing identity; the same session against a write rule that explicitly allows unauthenticated writes still succeeds.
Decision enabled: Alex knows whether to require sign-in for writing to a given collection is his own per-collection call, exactly as it already is for reads.

#### Domain Examples
1. **Happy Path (expected deny)**: A Trailmark session that never signed in attempts to create a `journal_entries` document, whose write rule requires `request.auth != null`. The create is denied — `request.auth` is `null`, so the condition is false.
2. **Edge Case**: The same never-signed-in session updates a `trail_guides` document whose write rule is `true` (public write, an unusual but grammar-legal case). The update succeeds — anonymous write access is not blanket-denied, only denied where the rule itself requires identity.
3. **Error/Boundary**: A session presents a malformed, expired, or wrong-project client-identity header. Its create attempt on the `request.auth != null`-gated `journal_entries` collection is denied — evaluated identically to the fully-absent-header case, reusing `client-auth`'s existing ADR-026 "attach nothing" semantics unchanged.

#### UAT Scenarios (BDD)

##### Scenario: A never-signed-in session is denied by a write rule requiring identity
Given `journal_entries` has a write rule requiring `request.auth != null`
When a session that never presented a client-identity token attempts to create a `journal_entries` document
Then the create is denied, attributable to the rule

##### Scenario: A never-signed-in session succeeds against a write rule allowing public write
Given `trail_guides` has a write rule of `true`
When a session that never presented a client-identity token updates a `trail_guides` document
Then the update succeeds

##### Scenario: An invalid client-identity header on a write call is evaluated identically to no header at all
Given `journal_entries` has a write rule requiring `request.auth != null`
When a session presents a malformed, expired, or wrong-project client-identity header and attempts to create a `journal_entries` document
Then the create is denied identically to how a session presenting no header at all would be denied

#### Acceptance Criteria
- [ ] AC-17-39: A never-signed-in session's create/update/delete is denied by a write rule requiring `request.auth != null`.
- [ ] AC-17-40: A never-signed-in session succeeds against a write rule that explicitly allows unauthenticated writes.
- [ ] AC-17-41: An invalid client-identity header on a write call is evaluated identically to no header at all — no new rejection class introduced.

#### Outcome KPIs
See § Outcome KPIs below (KPI #1 North Star, KPI #4).

#### Technical Notes (Optional)
This story adds no new logic to `attach_client_identity_if_present()` itself — it requires that function to be CALLED from `handle_create_document`/`handle_update_document`/`handle_delete_document` (currently it is not, confirmed by direct code read), and requires the write-evaluation step (US-02/03/04) to treat its `None` result as `request.auth == null`, exactly mirroring `security-rules`' own US-03.

---

### US-06: Untouched Collections and the Read/Write Independence Guardrail Hold

**job_id**: JOB-17
**Slice**: 06 (Walking Skeleton) | **Release**: 1

#### Elevator Pitch
Before: Alex worries that adding write-rule enforcement might silently start restricting collections he hasn't defined a write rule for — including collections that already have a READ rule from `security-rules`.
After: call the existing SDK write methods on `app_config` (never had any rule) and on `trail_guides` (has a read rule but no write rule) → both continue to succeed exactly as they did before this feature shipped, regardless of what rules exist elsewhere in the same project.
Decision enabled: Alex can adopt write protection one collection at a time, with zero risk to collections — or operations — he hasn't touched, including collections he's already protected on read only.

#### Domain Examples
1. **Happy Path**: `app_config` has never had any rule, read or write. Any caller's write succeeds exactly as it did before this feature shipped, gated only by the existing project `api_key`.
2. **Edge Case (the central lock)**: `trail_guides` has an active READ rule (`request.auth != null`, from `security-rules`) but no write rule of its own. Any caller — signed in or not — can still create/update/delete `trail_guides` documents exactly as before this feature shipped; the existing read rule has zero effect on writes.
3. **Error/Boundary**: The full pre-existing regression suite (113 `embyr-rs`/`client-auth` scenarios + 20 `security-rules` acceptance scenarios = 133) is re-run, unmodified, against a build that includes this feature, where no project in the suite has ever defined a write rule. All 133 pass exactly as before.

#### UAT Scenarios (BDD)

##### Scenario: A collection that has never had any rule is unaffected by this feature
Given `app_config` has never had a read or write rule defined
When any caller creates, updates, or deletes an `app_config` document using only the existing project `api_key`
Then the operation succeeds exactly as it did before this feature shipped

##### Scenario: A collection's existing read rule never governs its writes
Given `trail_guides` has an active read rule requiring `request.auth != null` and no write rule of its own
When a caller — signed in or not — creates, updates, or deletes a `trail_guides` document
Then the operation succeeds exactly as it did before this feature shipped, unaffected by the read rule

##### Scenario: A write rule on one collection does not affect a sibling collection without its own write rule
Given `journal_entries` has an active write rule and `trail_guides` in the same project has none
When a caller with no matching rule-satisfying identity writes to a `trail_guides` document
Then the write succeeds, unaffected by `journal_entries`'s rule

##### Scenario: The full pre-existing regression suite passes unmodified
Given none of the projects exercised by the 133 pre-existing `embyr-rs`/`client-auth`/`security-rules` scenarios has ever defined a write rule
When the full 133-scenario suite is re-run against a build that includes this feature
Then all 133 scenarios pass exactly as they did before this feature was added

#### Acceptance Criteria
- [ ] AC-17-42: A collection with no write rule defined continues writing exactly as before this feature shipped.
- [ ] AC-17-43: A collection with a READ rule (from `security-rules`) but no WRITE rule of its own has fully unrestricted writes — the read rule never silently governs writes.
- [ ] AC-17-44: A write rule defined for one collection has zero observable effect on any other collection's writes.
- [ ] AC-17-45: The full pre-existing regression suite (113 prior + 20 `security-rules` acceptance = 133 scenarios) passes unmodified.

#### Outcome KPIs
See § Outcome KPIs below (KPI #3 Guardrail).

#### Technical Notes (Optional)
This story is primarily a proof obligation over US-01–US-05's real behavior, not new production logic — mirrors `security-rules`' own US-04/AC-17-14/15/16 discipline. AC-17-43 specifically is this feature's single highest-consequence claim (Resolution 1) — a structural, not merely tested, mechanism (e.g., the write-condition lookup and read-condition lookup being genuinely separate queries/columns with no shared code path) is preferred if achievable without over-scoping DISCUSS's own remit; DESIGN's call.

---

### US-07: Alex Tests a Write Rule Against Concrete Old/New Examples Before Publishing It

**job_id**: JOB-17
**Slice**: 07 | **Release**: 2

#### Elevator Pitch
Before: Alex's only way to find out whether a write rule does what he intended — especially an update rule comparing old vs. new document state — is to publish it and watch real Trailmark users' writes succeed or fail.
After: call the admin API's rule-simulation action, extended to accept an operation type plus synthetic old and/or new document payloads → sees the resolved allow/deny outcome for a simulated create, update, or delete, without touching any live document.
Decision enabled: Alex catches an over-permissive or over-restrictive write-rule bug — especially one involving the old-vs-new comparison — during his own testing, before it reaches Maria or Dana in production.

#### Domain Examples
1. **Happy Path**: Alex simulates a candidate update on `journal_entries` with synthetic identity `end_user_id: test-user-001`, existing document `{owner_id: "test-user-001"}`, and proposed document `{owner_id: "test-user-001", title: "updated"}`. Sees "allow" — matching what he expected.
2. **Edge Case**: Alex simulates the same rule with a proposed document that changes `owner_id` to `"test-user-002"`. Sees "allow" instead of the expected "deny," because he wrote the immutable-field check backwards — catching the exact bug before publishing.
3. **Error/Boundary**: Alex simulates a candidate create with no synthetic identity at all (anonymous). Sees "deny" (per the rule's `request.auth != null` clause), matching what a real anonymous caller would get under US-05 — without any live document or session being created.

#### UAT Scenarios (BDD)

##### Scenario: Simulating a valid candidate write rule against a matching old/new pair returns the correct outcome
Given Alex holds a candidate write rule and synthetic old/new document payloads that should satisfy it
When Alex calls the simulation action with the candidate rule, the operation type, and the synthetic payloads
Then the response shows "allow," matching what real write evaluation would produce for that triple

##### Scenario: Simulation surfaces an over-permissive update-rule bug before publishing
Given Alex holds a candidate rule he believes denies a changed-owner update
When Alex simulates that update and the candidate rule instead allows it
Then the response shows "allow," surfacing the discrepancy from Alex's expectation before the rule is ever published

##### Scenario: Simulation has zero effect on live traffic
Given `journal_entries` has an active, published write rule
When Alex calls the simulation action with a different candidate write rule and synthetic data
Then real callers' create/update/delete calls continue to be evaluated against the published write rule, unaffected by the simulation

##### Scenario: Simulation supports the anonymous case for write rules
Given Alex holds a candidate write rule requiring `request.auth != null`
When Alex simulates a create with no synthetic identity, representing an anonymous caller
Then the response shows "deny," matching what a real anonymous caller would receive under US-05

#### Acceptance Criteria
- [ ] AC-17-46: Simulating a candidate write rule against synthetic old/new document payloads returns the same allow/deny outcome real write-evaluation would produce, across create/update/delete shapes.
- [ ] AC-17-47: Simulating a write rule has zero effect on live/published traffic.
- [ ] AC-17-48: Simulation supports the anonymous (no synthetic identity) case for write rules, matching real anonymous-write evaluation.

#### Outcome KPIs
See § Outcome KPIs below (KPI #4).

#### Technical Notes (Optional)
Strongly encouraged to extend `security-rules`' existing `simulate_access_rule` handler (accepting an operation type plus optional old/new payloads) rather than a separate endpoint — must share the exact same evaluation routine real write enforcement (US-02/03/04) uses, mirroring ADR-029's existing shared-routine guarantee.

---

## Wave: DISCUSS / [REF] Outcome KPIs

### Feature: security-rules-write-path

### Objective
Give every write rule Alex defines real, correct per-operation protection — differentiated across create/update/delete and independent of any read protection the same collection already has — closing the "reads look protected, writes are wide open" gap `security-rules` deliberately left open, without retroactively changing the behavior of any collection already protected under `security-rules`.

### Outcome KPIs

| # | Who | Does What | By How Much | Baseline | Measured By | Type |
|---|---|---|---|---|---|---|
| 1 | SDK developers who define a write rule for a collection (e.g. Alex/Trailmark) | Have that rule correctly allow/deny `CreateDocument`/`UpdateDocument`/`DeleteDocument` calls, matching the rule's logical evaluation across all three operation shapes | 100% of writes against a rule-configured collection produce the result the rule's condition logically implies (no false-allow, no false-deny) | 0% (capability does not exist today — no write is evaluated against identity or old/new document state at all) | Acceptance-scenario pass rate against the write-evaluation truth table (create-allow/deny, update-allow/deny/field-immutability, delete-allow/deny, anonymous, missing-field fail-closed) | North Star |
| 2 | SDK developers with an existing `security-rules` read-only rule who never define a write rule | Have that collection's writes remain fully unrestricted, unaffected by the existing read rule | 0% of read-only-protected collections observe any change in write behavior after this feature ships | 100% (current, pre-feature, unrestricted-write state) | Regression comparison across every collection with an active read rule and no write rule | Guardrail |
| 3 | Existing `embyr-rs`/`client-auth`/`security-rules` customers and collections that never define a write rule | Continue to write successfully, unaffected | 0% regression across the 133 pre-existing regression scenarios (113 prior + 20 `security-rules` acceptance) | Current 100% pass rate (pre-feature) | Full regression suite, pre/post comparison | Guardrail |
| 4 | SDK developers authoring and testing write rules before publishing them | Catch an incorrect write rule (over-permissive or over-restrictive, including old-vs-new comparison bugs) via simulation before it reaches a real end user | ≥1 write-rule bug caught pre-publish per integration/testing cycle (qualitative, ramps with adoption); 0 confirmed data-corruption or lockout incidents traced to an un-simulated write rule | N/A (capability does not exist today) | Simulation-action usage log cross-referenced with post-publish denial/allow-rate anomalies | Leading |

---

## Wave: DISCUSS / [REF] Out of Scope

- **Query-path enforcement** (`RunQuery`/list gated by rules) — named, deferred follow-up epic (candidate id `security-rules-query-path`, "Epic 2c"). Unchanged from `security-rules`' own framing.
- **Real-time Listen enforcement** (`onSnapshot`/BC-3 fan-out gated by rules) — named, deferred follow-up epic (candidate id `security-rules-realtime`, "Epic 2d"). Unchanged.
- **Rule history, versioning, and rollback; richer condition grammar beyond `request.resource`; audit logging** — named, deferred follow-up epic (candidate id `security-rules-operations`, "Epic 2e"). Unchanged.
- **A fully separate condition per exact CRUD verb** (independent create/update/delete conditions) — explicitly rejected for v1 per Resolution 1, Option B. A candidate follow-up only if a genuine create-vs-update-vs-delete divergent-condition need emerges that Resolution 1's Option C composition cannot express.
- **A whole-object presence sentinel** (`resource == null` / `request.resource == null` as literal comparable operands) — not evidenced by any story here; the existing fail-closed-on-missing-field mechanism already produces correct behavior for every domain example. Flagged as OQ-SRW-01, not built speculatively.
- **`BeginTransaction`/`Commit`/`Rollback` enforcement** — transactional writes are not gated by rules in this feature; a transaction's individual reads/writes are not evaluated against rules mid-transaction. Named, implicitly deferred to whichever future epic addresses transactional consistency with rule evaluation, if evidence emerges.
- **String/number literal operands** (OQ-SR-04, `security-rules`' own still-open gap) — not reopened or resolved by this feature.
- **Full Firestore Rules Language parity** (cross-document reads, custom functions, wildcard paths) — still explicitly rejected, unchanged from `security-rules`' Resolution 1.
- **Role-based / custom-claims authorization** — still out of scope, unchanged.
- **Re-opening any part of `security-rules`' or `client-auth`'s already-shipped read-path/identity scope** beyond the additive `request.resource` grammar extension and the three new `attach_client_identity_if_present()` call sites explicitly locked here.

---

## Wave: DISCUSS / [REF] WS Strategy

Walking Skeleton Strategy: **B — Thin End-to-End Slice** (unchanged from `security-rules`). Slices 01–06 are real, narrow vertical slices against real System DB rule state, real Maria/Dana signed-in sessions, and real anonymous sessions (no facade, no mock) — Slice 01 proves the schema/admin-API extension; Slice 03 proves the riskiest new assumption (a condition can correctly compare a document's pre-write and proposed-new state); Slices 02/04 prove the simpler single-sided cases; Slice 05 proves the newly-wired identity-attach call sites; Slice 06 proves the whole thing is additive to both untouched collections AND collections already protected on read only.

---

## Wave: DISCUSS / [REF] Driving Ports

| Port | Protocol | Extension |
|---|---|---|
| Admin port `:9090` (existing, extended) | HTTP/1.1 | Write-rule-definition/redefinition action (US-01, likely extending or sitting alongside `security-rules`' existing rule-definition action) and write-rule-simulation extension (US-07, extending `simulate_access_rule`) |
| Data ports `:8080` (gRPC) / `:8081` (REST/gRPC-Web) (existing, extended in observable behavior only) | gRPC / HTTP | `CreateDocument`/`UpdateDocument`/`DeleteDocument`'s existing, unchanged call shapes now additionally reflect write-rule evaluation when a write rule is defined for the target collection (US-02/03/04/05/06) — no new RPC or endpoint added on the data plane itself |

No new network-facing port introduced. Exact endpoint/action shapes are DESIGN's call.

---

## Wave: DISCUSS / [REF] Pre-requisites

- `docs/feature/security-rules/feature-delta.md` (full — the direct methodological and structural precedent; Resolution 1's confidence/escalation note and § Open Questions OQ-SR-02 are the exact seeds this DISCUSS resolves).
- `docs/product/architecture/adr-027-access-rule-grammar-and-evaluation.md`, `adr-028-access-rule-storage-and-lifecycle.md`, `adr-029-access-control-composition-and-bounded-context.md` (full — the exact machinery this feature composes with, not replaces).
- `docs/evolution/2026-08-18-security-rules.md` (full — confirms `security-rules` is DONE/FINALIZED, 133-scenario regression baseline this feature must not break).
- `crates/embyr-core/src/access_control/mod.rs` (`parse_condition`/`evaluate`, `Condition`/`Operand` AST — must be extended, not replaced).
- `crates/embyr-server/src/admin/handlers/access_rules.rs` (`define_access_rule`/`simulate_access_rule` — direct handler-shape precedent).
- `crates/embyr-server/src/adapters/system_db.rs` (`AccessRuleRow`, `upsert_access_rule`, `get_access_rule` — the single-condition-column schema this feature must extend).
- `crates/embyr-server/src/grpc/handler.rs`'s `handle_get_document`/`handle_create_document`/`handle_update_document`/`handle_delete_document` and `attach_client_identity_if_present` (the exact wiring points this feature extends; confirmed by direct read that the three write handlers call neither `attach_client_identity_if_present` nor `get_access_rule` today).
- `docs/product/jobs.yaml` (JOB-17, extended — see § SSOT Updates).

---

## Wave: DISCUSS / [REF] Handoff Package

**To DESIGN (solution-architect)**: this `feature-delta.md` (journey delta + story map + user stories + embedded AC), 7 slice briefs (`docs/feature/security-rules-write-path/slices/slice-01-define-write-rule.md` through `slice-07-simulate-write-rule-before-publish.md`), `docs/product/jobs.yaml` (JOB-17, extended via NOTE).

**To DEVOPS (platform-architect)**: § Outcome KPIs above (4 KPIs — 1 North Star, 2 Guardrail, 1 Leading — for instrumentation planning).

**Explicit flags for DESIGN**:
1. **Resolution 1 is LOCKED and is this feature's single most important flag**: two independent condition slots per collection (existing read, new write). Do NOT silently reuse the existing `condition_source` column for both read and write — that would retroactively change the behavior of every already-shipped `security-rules` rule. Do NOT expand to four independent per-CRUD-verb conditions without new evidence (Option B, rejected).
2. **Resolution 2 (resolves OQ-SR-02) is LOCKED**: add a `RequestResourceField(String)` operand variant to ADR-027's existing `Operand` enum; reuse the existing fail-closed-on-missing-field mechanism for both `resource.data.<field>` (empty on create) and `request.resource.data.<field>` (empty on delete) — do NOT invent a new null-sentinel operand or per-operator null-propagation semantics.
3. **AC-17-43 (a collection's existing read rule never governs its writes) is this feature's single highest-consequence design risk** — treat with the same weight `security-rules`' own AC-17-14/15/16 received.
4. **Identity-attach wiring gap**: `attach_client_identity_if_present()` must be called from all three write handlers (currently called from none) — confirmed by direct code read, not assumed. The function itself is unchanged; only its call sites grow.
5. Evaluating a write rule referencing `resource.data.<field>` on update/delete requires the pre-write document state — a fetch-before-decide step DESIGN must design, analogous to `GetDocument`'s existing pattern. No fetch is needed for create.
6. Existence non-leakage extends to update/delete, scoped identically to OQ-SR-06's precedent (content-referencing write rules only).
7. v1 write-enforcement surface is `CreateDocument`/`UpdateDocument`/`DeleteDocument` only — do not silently widen to `RunQuery`/`Listen`/`BeginTransaction`/`Commit`; each is a named, deferred follow-up epic.
8. Simulation (US-07) must share the exact evaluation routine real write-enforcement (US-02/03/04) uses, extending ADR-029's existing shared-routine guarantee — not a second, independently-maintained copy.

Peer review: not invoked per-wave (default skip per SKILL Phase 3 step 6 — this DISCUSS's two genuine ambiguities [per-operation condition structure, `resource`/`request.resource` grammar need] are each resolved with fully explicit and auditable reasoning above, mirroring `security-rules`' own precedent; JTBD assumptions inherited from JOB-17, already validated; no vendor-neutrality risk, no technology selected). Mandatory consolidated review fires at end of DISTILL.

---

## Wave: DISCUSS / [REF] SSOT Updates

- `docs/product/jobs.yaml` — added a NOTE under JOB-17 (`document-access-control`) documenting that this feature is JOB-17's own write-path realization (not a new job) — mirroring the project's "make it real" extension pattern (JOB-14 → `card-payments-backend`, JOB-10 → `admin-api-v2`). JOB-17's own `job_story`/dimensions text is unchanged (historically accurate as written).
- `docs/product/journeys/sdk-developer.yaml` — extended with a NOTE clarifying JOB-17 now also covers write-path enforcement via this feature; `updated` date bumped. No new job id added (JOB-17 already listed).
- No new persona file — Trailmark's end users (Maria Santos, Dana Kim) remain domain-example data within Alex's stories, consistent with `security-rules`' precedent.

---

## Wave: DISCUSS / [REF] Definition of Ready Validation

### Requirements Completeness Score: **0.97** (> 0.95 gate)

Computed across the three requirement categories:
- **Functional**: all 7 stories have complete Given/When/Then coverage of their happy path, at least one edge case, and at least one error/failure path; both central scoping questions (per-operation condition structure, `resource`/`request.resource` grammar need) are explicitly locked (Resolutions 1–2), not left ambiguous.
- **Non-functional**: security (existence non-leakage extended to writes, AC-17-34/38; fail-closed-on-missing-field extended, AC-17-28/33/37; identity-reuse-not-re-derivation constraint) is explicit. A performance guardrail note carries forward from `security-rules`' own NFR precedent (the pre-write document fetch on update/delete should mirror the existing "cheap existence-check before expensive work" pattern, not a hard-numeric SLA at DISCUSS time). Accessibility/usability NFRs are not applicable (API-only feature, consistent with prior precedent).
- **Business rules**: read/write independence (Resolution 1, AC-17-22/43), per-collection isolation (AC-17-44), and the write-side four-way taxonomy (owner-allow / non-owner-deny / anonymous-per-rule / missing-field-fail-closed) are all explicitly specified with examples, for each of create/update/delete.

**NFR note (carried forward)**: a collection with no write rule defined must add effectively zero overhead to the existing unarmed write path — a cheap existence-check against the write-condition store, mirroring `security-rules`' own precedent for reads. Not a hard-numeric SLA at DISCUSS time; DESIGN/DEVOPS should establish one once real traffic volume is known.

The remaining 0.03 gap is OQ-SRW-01 below (whole-object presence sentinel, deferred pending evidence) — explicitly flagged, not hidden, and does not block this feature's own DoR.

### DoR Checklist (9-item hard gate)

| # | DoR Item | Status | Evidence |
|---|---|---|---|
| 1 | Problem statement clear, domain language | PASS | Every story's Elevator Pitch "Before" line is stated in Alex/Maria/Dana domain terms (e.g. US-03: "nothing compares what the document looked like before against what it's being changed to") |
| 2 | User/persona identified with specific characteristics | PASS | P1 Alex (unchanged from `security-rules`); Maria Santos and Dana Kim as concrete rule-subject domain examples, continued |
| 3 | 3+ domain examples per story with real data | PASS | Every story has exactly 3 Domain Examples using `trailmark-prod`, `journal_entries`/`trail_guides`/`app_config`, `maria-santos`/`dana-kim`, real field names |
| 4 | UAT scenarios in Given/When/Then (3–7 per story) | PASS | US-01: 5, US-02: 4, US-03: 5, US-04: 4, US-05: 3, US-06: 4, US-07: 4 — all within range |
| 5 | Acceptance criteria derived from UAT | PASS | Every AC (AC-17-20 through AC-17-48) traces 1:1 or 1:many to a specific scenario above it |
| 6 | Right-sized (1–3 days, 3–7 scenarios) | PASS | Largest slice (US-03) estimated 2 days / 5 scenarios; all others ≤5 scenarios, ≤1.5 days |
| 7 | Technical notes identify constraints | PASS | Every story's Technical Notes references the relevant locked constraint (schema-independence, grammar-extension mechanism, fetch-before-decide, identity-reuse) without prescribing implementation |
| 8 | Dependencies resolved or tracked | PASS | Sole dependency, `security-rules`, is FINALIZED (`docs/evolution/2026-08-18-security-rules.md`); `access_control::{parse_condition,evaluate}`, `access_rules` table, and `attach_client_identity_if_present` all exist and are readable today |
| 9 | Outcome KPIs defined with measurable targets | PASS | 4 KPIs, each with a numeric or explicitly-qualitative-with-rationale target, baseline, and measurement method (§ Outcome KPIs) |

### DoR Status: **PASSED**

---

## Wave: DISCUSS / [REF] Open Questions

| ID | Question | Impact | Resolution owner |
|---|---|---|---|
| OQ-SRW-01 | Whether a whole-object presence sentinel (`resource == null` / `request.resource == null` as literal comparable operands) will ever be needed beyond what the fail-closed-on-missing-field mechanism already provides | Does not block this feature — no domain example here requires it | Product Discovery, triggered by future evidence |
| OQ-SR-02 | (Carried forward from `security-rules`, now CLOSED by this DISCUSS) — resolved by Resolution 2: yes, `request.resource` is needed, added as a new operand family reusing the existing fail-closed mechanism | N/A — resolved | Closed, this DISCUSS |
| OQ-SR-01 | (Carried forward, unrelated) — bounded-context placement, already resolved by `security-rules`' own DESIGN as BC-4 Access Control | N/A — not reopened by this feature | Closed, `security-rules` DESIGN |
| OQ-SR-03 | (Carried forward, unrelated) — whether custom claims on `VerifiedEndUserIdentity` will ever be needed | Does not block this feature | Product Discovery, cross-referenced with `client-auth` |
| OQ-SR-04 | (Carried forward, unrelated) — string/number literal operands, confirmed booleans-only during `security-rules`' own DISTILL | Not reopened by this feature — the new `request.resource.data.<field>` operand inherits the same literal-comparand restriction | Closed, `security-rules` DISTILL |

---

## Wave: DISCUSS / [REF] Wave Decisions Summary

### Key Decisions
- [D1] Locked two independent condition slots per collection (read, unchanged from `security-rules`; write, new) rather than sharing one condition across both operations: the only option that does not retroactively change already-shipped `security-rules` rule behavior and the only option that can express the most common real-world Firestore pattern (public-read/owner-write) — see § Job Discovery Framing Resolution, Resolution 1.
- [D2] Locked the grammar extension needed to resolve OQ-SR-02: add `request.resource.data.<field>` as a new operand family, reusing the existing fail-closed-on-missing-field mechanism for both operand families rather than inventing new null-propagation or presence-sentinel semantics — see Resolution 2.
- [D3] Extended JOB-17 rather than minting a new job: this feature realizes more of JOB-17's own already-stated full ambition ("enforce them on every request"), not a functionally/emotionally/socially distinct goal — see § Persona & Job.
- [D4] Scope Assessment PASS at 1 of 5 signals fired (WS integration points, borderline) — no further split proposed; Decision 1 (Backend) and Decision 3 (Lightweight) already kept this feature tighter than `security-rules`' own first pass.

### Requirements Summary
- Primary jobs/user needs: Alex needs a write rule he defines for a collection to genuinely gate `CreateDocument`/`UpdateDocument`/`DeleteDocument` — differentiated by operation type via `resource`/`request.resource` — while being completely independent of whatever read protection the same collection already has, and while every collection and every already-shipped `security-rules` rule remains provably unaffected.
- Walking skeleton scope: define an independent write rule (US-01) → evaluate it correctly for create (US-02), update comparing old vs. new (US-03), delete (US-04), and anonymous callers (US-05) → prove untouched collections and read-only-protected collections remain unaffected, alongside the full 133-scenario regression suite (US-06). Simulation (US-07) is Release 2.
- Feature type: Backend (extends 3 existing gRPC write handlers with the same `evaluate()` call the read-path already uses).

### Constraints Established
- Two independent condition slots per collection; a collection's existing read rule never governs its writes.
- `resource.data.<field>` (unchanged semantics, now also readable during writes) and the new `request.resource.data.<field>` both reuse the existing fail-closed-on-missing-field mechanism — no new null-sentinel operand.
- `attach_client_identity_if_present()` must be called from all three write handlers (currently called from none).
- v1 write-enforcement surface is `CreateDocument`/`UpdateDocument`/`DeleteDocument` only; query/listen/transactional enforcement remain out of scope.

### Upstream Changes
- None — no DISCOVER/DIVERGE artifacts exist for this feature (same as `security-rules`/`client-auth`); this DISCUSS is grounded directly in `security-rules`' own shipped artifacts, its own ADRs, and `docs/product/jobs.yaml`.
