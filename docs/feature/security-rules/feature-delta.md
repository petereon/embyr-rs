# security-rules — Feature Delta

**Wave**: DISCUSS | **Agent**: Luna (nw-product-owner) | **Date**: 2026-08-17
**Status**: Ready for DESIGN handoff
**Upstream**: `client-auth` (DONE, merged to `master`) — this feature is epic 2 of the two-epic Firebase-security-model initiative; epic 1 (`client-auth`) built caller identity, this epic builds the authorization that consumes it.

---

## Wave: DISCUSS / [REF] Prior Wave Consultation — Reading Confirmation

✓ `docs/product/jobs.yaml` (all 16 existing jobs read in full; JOB-16 `client-identity-verification` is the direct precedent and dependency — its own "pull" four-force text names this epic by name; JOB-01 `sdk-compat` confirmed still scoped to the Firestore data-plane surface only)
✓ `docs/product/journeys/sdk-developer.yaml` (pointer file, P1 Alex, JOB-01 + JOB-03 + JOB-16 — extended with JOB-17 as part of this wave, see § SSOT Updates)
✓ `docs/product/architecture/adr-002-bounded-contexts.md` (full — confirms 3 bounded contexts: BC-1 Tenant Management, BC-2 Document Storage, BC-3 Real-Time Delivery; Option D's rejection reasoning — "credential resolution has no entities, no aggregate roots, no lifecycle ... that is a domain service within BC-1, not a separate context" — does **not** straightforwardly apply to a rule-definition subsystem, which *does* have an aggregate with identity and lifecycle [define → redefine → (future) version history]; flagged for DESIGN, not resolved here — see § System Constraints)
✓ `docs/feature/client-auth/feature-delta.md` (full, 973 lines, all four waves — DISCUSS's Framing Resolution methodology is the direct structural precedent this DISCUSS reuses twice over; DESIGN's Component Decomposition confirms the exact shape of `VerifiedEndUserIdentity { end_user_id, project_id, expires_at_unix }` and that it is currently discarded — bound to `_verified_identity` — in `handle_get_document`; § Out of Scope's "Firestore Security Rules ... is a separate, dependent follow-up epic" is this feature's own charter, quoted verbatim)
✓ `docs/evolution/2026-08-17-client-auth.md` (full — confirms `client-auth` shipped clean: 3 independent security-review checkpoints all APPROVED, 0 blocking findings; § Follow-Up Work confirms "02-02's gRPC extension is wired only into `handle_get_document`" — the other 8 RPC methods do not yet carry the additive identity-attach step — and explicitly names this epic as the next consumer; OQ-CA-01 [real Firebase SDK wire-fidelity spike] and OQ-CA-02 [verification-result caching] are unrelated to rules and not reopened here)
✓ `crates/embyr-server/src/grpc/handler.rs` (full, 1369 lines — `extract_api_key`/`authenticate` [lines 129–333, unchanged, three-role `api_key` check this feature must not restructure], `extract_client_identity_token`/`attach_client_identity_if_present` [lines 144–383, the additive, structurally-optional client-identity step], and `handle_get_document` [lines 506–551, where `_verified_identity` is computed but never consumed] read in full — this is the exact, single wiring point this feature's walking skeleton extends)
✓ `crates/embyr-core/src/client_identity/mod.rs` (full — confirms `VerifiedEndUserIdentity { end_user_id: String, project_id: String, expires_at_unix: i64 }` is the complete shape available to rules; no custom-claims map exists on this type today — a rule referencing `request.auth.token.<claim>` is not buildable in v1 without a separate, cross-epic extension to `client-auth`'s own token contract, ADR-024)
✓ `docs/product/architecture/adr-002-bounded-contexts.md` § Context Map Summary (BC-2 Document Storage's storage boundary is Customer DB only, OCC-consistency; BC-2 never reads the System DB — a new rule-definition subsystem, which is naturally System-DB-scoped [it belongs to the *project*, not to any one customer document], sits awkwardly against this boundary — flagged for DESIGN, see § System Constraints)

No contradictions found between this feature's scope and prior evidence. This feature reverses no founding decision of its own — `client-auth` already reversed the relevant `embyr-rs` founding exclusion — but it is the feature that gives the optional identity `client-auth` established real behavioral consequence for the first time. One deliberate scope narrowing (Elephant Carpaccio split of the full read/write/query/listen ambition into five ordered epics, this DISCUSS covering only the first) is documented explicitly below (§ Scope Assessment), not silently introduced.

---

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

| # | Decision | Value |
|---|---|---|
| 1 | Feature Type | Cross-cutting — spans the existing gRPC/REST data-plane handler, a new rule-definition/storage subsystem, and (in this pass) BC-2 Document Storage's read path |
| 2 | Walking Skeleton | Evaluated (this wave's call): **YES** — see § Walking Skeleton Evaluation and § Story Map |
| 3 | UX Research Depth | Comprehensive — full experience mapping with emotional arcs for **both** Alex (authoring/testing) and the downstream consequence for Trailmark's end users (Maria Santos, Dana Kim), per explicit user instruction that this is not a backend-only happy-path feature |
| 4 | JTBD Analysis | Yes (default) — every story traces to `job_id: JOB-17` |

### Walking Skeleton Evaluation (Decision 2 = "Depends")

Two existing mechanisms were evaluated for reuse before concluding what the walking skeleton actually needs to add:

1. **`client-auth`'s `VerifiedEndUserIdentity` / `attach_client_identity_if_present`.** This is *reused*, not replaced — it is the exact input a rule's `request.auth` reference needs. Nothing about it needs to change: absent header → `None` (rules see "anonymous"); present-and-valid → `Some(VerifiedEndUserIdentity)`; present-but-invalid (malformed/expired/wrong-project) → also `None`, per ADR-026 DDD-CA-5's existing "attach nothing, never reject the call" semantics — this feature does not need, and must not invent, a second rejection channel for identity itself. See § Shared Artifact.
2. **The existing `authenticate()` three-role `api_key` check.** Structurally unchanged and unextended. A rule denial is a *new*, independent outcome layered strictly after `authenticate()` succeeds — mirroring exactly how `client-auth`'s own identity-attach step was layered after `authenticate()`, not merged into it.

**Verdict**: no existing mechanism evaluates a boolean condition over identity + document data — that computation does not exist anywhere in the codebase today. A walking skeleton is needed: define a rule for a collection, then have a `GetDocument` call whose caller/document satisfies the rule succeed while one that does not is denied, and a collection with no rule defined is provably unaffected (§ Story Map, Slices 01–04).

---

## Wave: DISCUSS / [REF] Job Discovery — Framing Resolution

This feature has **three** central scoping questions, not one — each evaluated with the same rigor `client-auth`'s own Framing Resolution established as this project's precedent. None was decided silently.

### Resolution 1 — Rule-expressiveness scope (how much of real Firestore Rules to replicate)

| Option | Description | Fit against evidence |
|---|---|---|
| **(A) Full Firestore Rules Language parity** | A complete CEL-like expression grammar: custom functions, recursive/wildcard path matching, cross-document reads (`get()`/`exists()`), list-comprehension-like constructs — matching real Firestore's actual rules engine surface. | **Rejected — unbounded scope, no evidenced need.** No job, persona, or domain example in this codebase requires a rule to read a *different* document to decide access to the one being requested (the concrete example — Maria reading her own trip-journal entry — needs only the target document's own fields). Building a general-purpose expression interpreter with cross-document I/O is a multi-week undertaking with a materially different risk profile (a rule that calls `get()` on another document turns a pure computation into an I/O-bound one, inside the hot request path) — a decision this scale deserves its own DISCUSS/DESIGN pass if evidence for it ever appears, not a default. |
| **(B) A fixed enum of named rule "shapes"** | No free-form condition syntax — Alex picks from a small closed set of pre-built patterns: "owner field equals `request.auth.uid`," "authenticated-only," "public read," "deny all." | **Rejected — real, avoidable migration friction for no cost saving.** Alex is migrating an existing Firebase app and, per JOB-17's own "habit" four-force, already has a real `firestore.rules` file expressed as boolean conditions over `request.auth`/`resource.data` — Firebase's actual authoring mental model. Forcing him to re-express existing boolean logic into a fixed enum of named shapes is friction with no implementation-cost saving over Option (C): both require *some* structured condition representation; only (B) also requires Alex to learn a new vocabulary. |
| **(C) A constrained boolean-expression grammar** | Comparison (`==`, `!=`) and boolean combinators (`&&`, `||`, `!`) over `request.auth` (`null`, or an object exposing `.uid`) and `resource.data.<field>` (the target document's fields), with literal `true`/`false`. Explicitly **excludes** cross-document reads, custom functions, and recursive/wildcard path matching. | **Strongest fit.** Satisfies the concrete domain example (owner-field equality) and JOB-17's "habit" four-force (Firestore-shaped syntax, not an unfamiliar one) without Option (A)'s unbounded scope or I/O-in-hot-path risk. Composable enough to also express "authenticated-only" (`request.auth != null`) and "public read" (`true`) as special cases of the *same* grammar, rather than Option (B)'s separate enum — proven necessary by US-02/US-03's own domain examples below, which need more than pure owner-equality. |

**Resolution**: **(C) is the locked v1 rule-expressiveness scope.** Option (A) is a named candidate follow-up ("Rules Language Expansion") triggered only by future evidence that a rule genuinely needs to read another document or invoke a function — not built here. Option (B) is rejected outright, not deferred — it does not represent a smaller version of the same capability, it represents a *different, worse-fitting* one.

**Confidence and escalation note**: high confidence, independently supported by both the concrete domain example and JOB-17's habit four-force. The one open edge this could not fully close from repository evidence alone: whether Alex will ever need `resource.data` on the *previous* version of a document during an update (real Firestore's `resource` vs. a hypothetical `request.resource` distinction) — not applicable to this feature's read-only v1 scope, but flagged for whichever future epic (Epic 2b, write-path) picks this grammar back up, since write rules commonly need both old and new document state.

### Resolution 2 — Default behavior when no rule is defined (backward-compatibility guardrail)

Real Firebase defaults a **new** project to either fully-locked (`allow read, write: if false`) or fully-open ("test mode," time-limited) rules — it does not default to "no rules file exists, therefore unrestricted." embyr-rs cannot adopt either of those defaults without breaking its own existing regression suite: 72 `embyr-rs` acceptance scenarios plus 41 `client-auth` tests (113 total) all assume unrestricted, identity-independent access gated only by the project `api_key` — none of them define any rule, because rules did not exist when they were written.

**Resolution**: **a collection with no rule defined behaves exactly as it does today — unrestricted by identity, gated only by `api_key`.** This is a deliberate, evidence-forced divergence from real Firebase's own default (which is locked-by-default), not an oversight — flagged explicitly here so DESIGN does not "correct" it toward Firebase parity, and so DISTILL frames Alex-facing messaging (documentation, admin-API response text) to set the correct expectation: unlike a brand-new Firebase project, an embyr-rs project starts **open**, and Alex must deliberately close each collection he wants protected, one rule at a time. This mirrors `client-auth`'s AC-16-08 regression discipline exactly — see § System Constraints.

### Resolution 3 — Rule lifecycle shape: register-then-rotate, or idempotent redefine?

`client-auth`'s US-01/US-03 modeled credential lifecycle as **register once, then a separate, deliberate rotate action** with a two-generation overlap window — appropriate for a security-sensitive secret that changes rarely. A rule is different: JOB-17's own "habit" four-force establishes that Alex expects to *redeploy* rules repeatedly while iterating (exactly how `firebase deploy --only firestore:rules` works today) — treating each edit as a rare, deliberate "rotation" with an overlap window mismatches that expectation and adds needless process friction to an activity Alex will do dozens of times while testing one collection.

**Resolution**: **rule definition is an idempotent upsert (define/replace).** The same admin action that defines a rule for the first time also redefines (fully replaces, not merges) it later — no separate "rotate" action, no overlap window, no register-then-conflict-then-rotate three-step dance. This is locked as an *observable behavior* decision (DISCUSS's job); the exact persistence/versioning mechanism DESIGN chooses to implement "replace" is DESIGN's call.

---

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: **P1 — Alex, SDK Developer** (existing persona, same as `client-auth`; no dedicated persona file exists yet — defined inline per this project's established convention). Alex is the rule *author*; he is also, functionally, Trailmark's own security reviewer — nobody else checks his rules before they affect real end users.

**Domain-example company**: **Trailmark**, continued from `client-auth`. Collections used consistently across this feature's domain examples: `journal_entries` (Maria's/Dana's private trip-journal documents, each carrying an `owner_id` field), `trail_guides` (Trailmark's own published content, intended to be publicly readable), `app_config` (an internal collection nobody has ever needed to protect).

**The rule *subject***: Trailmark's own end users — **Maria Santos** and **Dana Kim**, the same two end users established in `client-auth`. They never interact with embyr directly (same as `client-auth`) and remain domain-example data, not a formal persona. Per the Comprehensive research-depth decision, their downstream experience receives real narrative weight below (§ Journey — Consequence Arc), not just a passing mention.

**job_id decision (per Decision 4)**: this feature creates one new job, **JOB-17 (`document-access-control`)**, rather than extending JOB-16. JOB-16 is scoped to *establishing* identity (see its own Framing Resolution); JOB-17 is a distinct goal — *consuming* that identity to gate access — for the same persona. This mirrors the project's precedent for "same persona, different goal ⇒ new job" (JOB-11 vs. JOB-06; JOB-14 vs. JOB-10; JOB-16 vs. JOB-01). JOB-16 receives a cross-reference NOTE (not a rewrite); JOB-17 carries a NOTE pointing back — see `docs/product/jobs.yaml`.

**Opportunity scoring**: Importance = 9 (without this, `client-auth`'s entire value is inert — a verified identity that nothing ever checks is not meaningfully different from no identity at all; every multi-user Firestore app migrating to embyr needs this to have real per-user data protection, not just per-user *identification*). Satisfaction = 1 (zero support exists; a named, deliberate, already-queued follow-up, not an unaddressed gap — `client-auth`'s own Out of Scope section names it explicitly). Opportunity = 9 + (9−1) = **17**. Priority: **critical** — comparable magnitude to JOB-16 (15) and JOB-01/JOB-03 (18/15), since an app that migrates identity but never gets real authorization has a worse security posture post-migration than it started with (every user is now *identifiable* but still not *protected*).

---

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

Run before journey/story-map investment, per Phase 1.5. This gate is evaluated **twice**: once against the full ambition stated in the raw ask ("enforce them on every read/write/query/listen"), and once against the narrowed scope this DISCUSS actually commits to.

### Pass 1 — full ambition (read + write + query + listen enforcement, in one feature)

| Signal | Threshold | This scope | Fired? |
|---|---|---|---|
| User stories | >10 | ~10–12 (define, read-gate × 2–3, write-gate × 3 [create/update/delete], query-gate, listen-gate, anonymous handling, no-rule guardrail, simulate/test, versioning) | Borderline |
| Bounded contexts / modules | >3 | 3 — new rule subsystem + BC-2 (read/write/query) + BC-3 (listen, different consistency model and re-fetch mechanism) | Borderline (not strictly >3, but touches every context BC-2/BC-3 in the system) |
| Walking Skeleton integration points | >5 | 5 — define + read-gate + write-gate + query-gate + listen-gate | Borderline (at threshold) |
| Estimated effort | >2 weeks | Query-path enforcement alone requires a structurally different mechanism (query-shape validation before execution, not point-read post-check — a query must be *provably* rule-compliant, unlike a single document fetch); listen-path enforcement requires new BC-3 fan-out logic under an eventual-consistency model. Combined with read+write, this is credibly 3+ weeks. | **YES** |
| Independent shippable outcomes | multiple | **YES** — "rules gate reads" is independently valuable and demoable without query or listen enforcement also existing (real Firebase apps commonly ship read rules before write rules); query-path and listen-path are structurally distinct enforcement mechanisms serving genuinely separable outcomes, not two halves of one outcome the way `client-auth`'s register/verify pair were | **YES** |

**2 of 5 signals clearly fire** (threshold is 2+). **Verdict: OVERSIZED at the full-ambition scope.**

### Proposed split (Auto Mode — no interactive confirmation available; reasoning made fully explicit and auditable for redirect if wrong, same discipline `client-auth`'s own Framing Resolution used)

| Epic | Scope | Status |
|---|---|---|
| **2a — `security-rules` (this feature)** | Rule authoring (define/redefine) + rule simulation/testing + **read-path** (`GetDocument`) enforcement, including the anonymous-session and no-rule-defined guardrails | **This DISCUSS pass** |
| 2b — candidate `security-rules-write-path` | Extend the same rule grammar/evaluation function to `create`/`update`/`delete` | Named, deferred — not started |
| 2c — candidate `security-rules-query-path` | `RunQuery` enforcement — flagged as needing its own DISCUSS/DESIGN, since query-time rule compliance is a structurally different mechanism (reject a non-compliant query shape before execution) than a point-read post-check | Named, deferred — not started |
| 2d — candidate `security-rules-realtime` | `Listen`/`onSnapshot` enforcement — BC-3, eventual-consistency, re-fetch-on-`DocChange` mechanism | Named, deferred — not started |
| 2e — candidate `security-rules-operations` | Rule history/versioning/rollback, richer condition grammar (if Resolution 1's Option A trigger ever fires), audit logging | Named, deferred — not started |

This exactly mirrors `client-auth`'s own narrowing discipline (JOB-16 was framed at full ambition; the feature under discussion locked a narrower v1). JOB-17's `job_story` above is written at the full, honest ambition; **this feature's story map and slices below implement only Epic 2a.**

### Pass 2 — narrowed scope (Epic 2a only, the actual scope of this feature)

| Signal | Threshold | This feature | Fired? |
|---|---|---|---|
| User stories | >10 | 5 (US-01 through US-05) | **NO** |
| Bounded contexts / modules | >3 | 2 — new rule-definition/storage subsystem + BC-2's existing read path (`GetDocument`, already the exact wiring point `client-auth` left the identity-attach step in) | **NO** |
| Walking Skeleton integration points | >5 | 2 — admin-port rule definition (US-01) + data-port read-path evaluation (US-02/03/04, all the same wiring point) | **NO** |
| Estimated effort | >2 weeks | 5 slices × ~1.3 days average ≈ 6.5 days | **NO** |
| Independent shippable outcomes | multiple | **NO** — US-01 (define) and US-02/03/04 (evaluate-on-read, including both guardrails) are inseparable halves of one outcome, exactly like `client-auth`'s US-01/US-02 pairing; US-05 (simulate) is a normal release-2 enhancement, not a second walking-skeleton-level outcome | **NO** |

**0 of 5 signals fired. Verdict: PASS — right-sized.** No further split needed within Epic 2a. Decomposed into 5 elephant-carpaccio slices below (normal thin-slicing, not oversized-triggered splitting).

---

## Wave: DISCUSS / [REF] Journey — Alex's Authoring/Testing Arc and Maria's Consequence Arc

Per Decision 3 (Comprehensive), both sides of this feature receive full narrative weight — not just Alex's API-call-level flow (as `client-auth`'s Lightweight journey was), but the downstream stakes for the people whose data the rule actually protects or fails to protect.

### Mental model

Alex's mental model, carried over from real Firebase and confirmed by JOB-17's "habit" four-force: a rule is a boolean condition, scoped to a collection, that the server evaluates on every relevant request using two things — who is asking (`request.auth`) and what they're asking about (`resource.data`). He expects to write it, redeploy it as often as he likes while testing, and trust that a collection he hasn't touched is untouched by anything he did to a different one.

### Alex's authoring/testing emotional arc

```
Start                    Middle                        Peak tension              End
Anxious                  Focused                       "Did I just get this      Confident / relieved
                                                          backwards?"
   |                        |                                |                        |
"If I get this wrong,   Writes a condition,        The realistic failure    Sees the exact
 I could leak every      runs it against            modes: swapped ==        scenarios he was
 user's data to every    concrete synthetic          direction (over-        worried about pass
 other user — or lock    examples via the            permissive), forgot     via simulation; then
 every user out of       simulation action           the null-auth check     publishes and watches
 their own"               (US-05) before              (crashes/leaks on      existing, untouched
                          publishing                   anonymous callers)     collections not even
                                                                               notice anything changed
```

### Rule-authoring flow (Alex's side, Slice 01 + Slice 05)

```
Alex calls the admin API to define/redefine a rule for a collection
        │
        ▼
   Is the admin credential valid and does the project exist?
        │
   no / invalid ──────────────┐                 yes
        │                      │                  │
        ▼                      │                  ▼
  401 (bad admin credential)   │       Is the condition syntax valid, and
  or 404 (no such project)     │       does it stay within the v1 grammar
        │                      │       (no get()/exists()/functions)?
        │                      │                  │
        │                      │        ┌─────────┼─────────┐
        │                      │      invalid                valid
        │                      │        │                     │
        │                      │        ▼                     ▼
        │                      │   400, naming what's    Rule stored active;
        │                      │   wrong (plain syntax    fully replaces any
        │                      │   error vs. explicitly   prior rule for this
        │                      │   out-of-v1-scope)        collection — no merge,
        │                      │        │                  no overlap window
        └──────────────────────┴────────┴────────────────────────┬────────
                                                                    ▼
                                                    Alex, before trusting it,
                                                    calls the simulation action
                                                    (US-05) with a synthetic
                                                    identity + document pair
                                                            │
                                                    ┌───────┴────────┐
                                                  matches           surprises
                                                  expectation       Alex — bug
                                                    │                caught here,
                                                    ▼                not in prod
                                              Alex trusts the           │
                                              rule for real            ▼
                                              traffic              Alex fixes the
                                                                    condition and
                                                                    redefines (loop)
```

### Read-evaluation flow (Maria's / Dana's / an anonymous session's side, Slices 02–04)

```
A GetDocument call arrives for a document in a collection with a rule defined
        │
   (the existing, unchanged authenticate() + attach_client_identity_if_present
    steps already ran — request.auth is Some(VerifiedEndUserIdentity) or None)
        │
        ▼
   Evaluate the rule's condition against request.auth (Some/None) and
   resource.data (the target document's fields)
        │
   ┌────┴─────┬────────────────────┬─────────────────────────┐
   │           │                    │                          │
condition   condition false     referenced field         no rule defined
true        (wrong owner, or     absent on the doc        for this collection
            unauthenticated on   (fails closed)            at all
            an auth-required     │                          │
            rule)                │                          │
   │           │                 │                          │
   ▼           ▼                 ▼                          ▼
Read         Denied,           Denied — never a          Read succeeds exactly
succeeds,    PermissionDenied,  500/crash; and never      as it did before this
identical    same response      distinguishes "field      feature shipped —
to before    whether or not     absent" from "field       gated only by the
this          the document      present but wrong" in     unchanged existing
feature      exists (no         the response (no          api_key check
existed      existence leak)    schema-shape leak)
```

### Maria's / Dana's consequence arc (Comprehensive depth — narrated, not first-person, since they never call embyr directly)

Unlike Alex, Maria and Dana never see an error message, a status code, or a rule. Their entire experience of this feature is *whether the Trailmark app behaves the way they expect it to* — which makes the stakes higher, not lower, than a feature with a visible UI:

- **If Alex's rule is correct**: Maria notices nothing has changed at all. Her trip journal keeps working exactly as it always has. This *is* the intended experience — correctness here is invisible by design, which is precisely why Alex's own confidence (via US-05) matters so much: there is no user-facing signal to tell him he got it right.
- **If Alex's rule is over-permissive** (e.g., `||` where he meant `&&`, or the equality direction reversed): Dana's `getDoc()` call on Maria's private trip-journal entry silently succeeds. Maria has no way to detect this happened — no notification, no log she can see, nothing in the Trailmark UI. This is the scenario the existence-non-leakage and simulation (US-05) requirements exist to prevent from ever reaching production undetected.
- **If Alex's rule is over-restrictive** (e.g., referencing a field name that doesn't match what's actually stored, or gating a collection Alex didn't intend to touch): Maria, who did nothing wrong, suddenly gets a permission error inside an app that worked yesterday. From her perspective the app is simply broken; she has no way to know a rule denied her, and the only signal that reaches Alex is a support ticket or a spike in failed reads — which is exactly what US-04's per-collection-isolation guardrail and US-05's pre-publish testing both exist to prevent.

### Shared artifact

| Artifact | Source of truth | Consumers | Integration risk |
|---|---|---|---|
| `VerifiedEndUserIdentity` / `None` (reused from `client-auth`) | `attach_client_identity_if_present()` in `crates/embyr-server/src/grpc/handler.rs` (existing, currently discarded as `_verified_identity` in `handle_get_document`) | The new rule-evaluation step (US-02/03) reads `request.auth` from this exact value | **HIGH** — if the rule-evaluation step re-derives or re-verifies identity through a second, independent code path instead of consuming this already-computed value, the two can drift (exactly the class of risk `client-auth`'s own US-02/US-04 pairing flagged for its verification routine) |
| The per-collection rule definition | New rule-definition subsystem (exact storage shape DESIGN's call) | Rule-evaluation on `GetDocument` (US-02/03/04); rule-simulation action (US-05) | **HIGH** — mirrors `client-auth`'s own flagged risk: if simulation (US-05) evaluates against a separately-maintained copy of the evaluation logic instead of the exact same function real enforcement uses, Alex's pre-publish confidence is a false signal |

### Failure modes (feeds DISTILL scenario generation)

- Alex writes a condition with the equality direction reversed (`resource.data.owner_id == request.auth.uid` typo'd as `!=`) — over-permissive, must be catchable via simulation (US-05) before it reaches Maria/Dana.
- Alex references a field name that doesn't exist on the actual stored documents (typo, or schema drift since the rule was written) — must fail closed (deny), never crash, and must be distinguishable in simulation from "the condition is syntactically fine but evaluates false."
- Alex defines a rule for the wrong collection name entirely (typo) — the collection he meant to protect stays open (no rule defined = unrestricted, per Resolution 2), and a DIFFERENT collection he didn't intend to touch becomes gated. Per-collection isolation (US-04) must make this observable/testable, not silently absorbed.
- A session presents an identity header that is malformed/expired/wrong-project — must be evaluated identically to "no identity at all" (`request.auth == null`), not as a distinct rejection class, reusing `client-auth`'s existing ADR-026 semantics unchanged (US-03).
- The existing 113-scenario regression suite (72 `embyr-rs` + 41 `client-auth`) must re-run unmodified — none of those scenarios define a rule, so none should observe any behavior change (US-04).
- A denied read must not reveal, via any observable difference in its response, whether the target document exists at all (US-02) — an information-disclosure side channel a naive implementation could easily introduce.

---

## Wave: DISCUSS / [REF] Story Map & Walking Skeleton

**Persona**: P1 Alex | **Goal**: Give each collection Alex chooses real per-user read protection, gated on the identity `client-auth` already establishes, without affecting any collection he hasn't touched.

### Backbone

| A. Alex Defines Protection | B. A Read Is Evaluated Against the Rule | C. Alex Builds Confidence Before Publishing |
|---|---|---|
| Alex defines a rule for a collection **[WS]** | A signed-in end user's own-document read is allowed; another signed-in user's is denied **[WS]** | Alex simulates a candidate rule against synthetic identity/document pairs |
| Alex redefines (replaces) an existing rule, same action **[WS]** | An anonymous (never-signed-in) session is evaluated against the rule as `request.auth == null` **[WS]** | |
| | A collection with no rule defined keeps reading exactly as before **[WS]** | |

### Walking Skeleton

One task from each activity, thinnest end-to-end happy path plus its two load-bearing guardrails: Alex defines a rule for `journal_entries` requiring the caller to be the document's owner (Activity A); Maria Santos's `getDoc()` on her own entry succeeds while Dana Kim's `getDoc()` on that same entry is denied, an anonymous session is likewise denied (or allowed, on a separately-configured public-read collection), and `trail_guides`/`app_config` — collections with no rule defined — continue reading exactly as before this feature shipped (Activity B). This is exactly Slices 01–04's combined happy-path-plus-both-guardrails scenario set — no facade, real project/rule/identity state, mirroring `client-auth`'s own WS discipline.

### Release 1 — Read-Path Protection Works End-to-End (Slices 01–04, US-01 through US-04)

Outcome: any collection Alex protects with a rule genuinely gates reads by identity — for signed-in and anonymous callers alike — while every other collection remains exactly as it was.

### Release 2 — Authoring Confidence (Slice 05, US-05)

Outcome: Alex can prove a rule does what he intended before any real end user is affected by it, instead of discovering a bug from a support ticket or a security incident.

---

## Wave: DISCUSS / [REF] Elephant Carpaccio Slices

| Slice | Story | Release | Estimate | Learning Hypothesis (disproves) | Production-data taste test |
|---|---|---|---|---|---|
| 01 (WS) | US-01 | 1 | 1.5 days | A per-collection rule cannot be defined and idempotently redefined (replace, not register+rotate), with syntax validated against the v1 grammar, using the existing admin-API auth/response conventions without inventing a new pattern | Real System DB row, real admin Bearer credential — no synthetic exception |
| 02 (WS) | US-02 | 1 | 2 days | A rule condition over `request.auth`/`resource.data` cannot be evaluated against a real `GetDocument` call without either re-verifying identity through a second, drift-prone code path, or requiring more expression-language investment than the constrained v1 grammar (Resolution 1, Option C) actually provides | Real registered rule + real Maria/Dana signed-in sessions + real `journal_entries` documents (with and without the referenced field) |
| 03 (WS) | US-03 | 1 | 1 day | Rules cannot meaningfully give `client-auth`'s optional identity real teeth without also handling the anonymous (`request.auth == null`) case explicitly, reusing `client-auth`'s existing invalid-token-attaches-nothing semantics unchanged rather than inventing a new rejection class | Real un-signed-in session + real malformed/expired/wrong-project identity header, both against real rule-gated and real rule-public collections |
| 04 (WS) | US-04 | 1 | 1 day | A rule defined for one collection cannot be proven not to silently affect a different collection or the existing regression suite without actually re-running all 113 pre-existing scenarios unmodified | Real full regression suite (72 `embyr-rs` + 41 `client-auth`), real multi-collection project state |
| 05 | US-05 | 2 | 1 day | A rule-simulation action cannot share the exact same evaluation routine as real enforcement without duplicating (and risking drift in) the evaluation logic | Real candidate rules + real synthetic identity/document pairs checked against the real evaluation path, not a hand-rolled test double |

**Total estimate: ~6.5 days.**

**Taste tests applied**:
- "4+ new components per slice" — none exceeds 2 (Slice 01: admin handler + rule storage/validation; Slice 02: evaluation function + `GetDocument` wiring; Slice 03: extends Slice 02's evaluator with null-auth handling, no new component; Slice 04: zero new components, pure regression proof; Slice 05: thin wrapper over Slice 02's evaluator). PASS.
- "Every slice depends on a new abstraction" — Slice 01 is the one genuinely new abstraction (the rule/condition grammar); Slices 02–05 build on it but do not each introduce a new one. PASS — natural sequencing (01 before 02) mirrors the Walking-Skeleton pairing, not forced dependency inflation.
- "No slice disproves a pre-commitment" — each has a distinct, falsifiable hypothesis (see table). PASS.
- "Synthetic-data-only slices prove plumbing, not value" — N/A; all 5 slices require real System DB state, real signed-in/anonymous sessions, and (Slice 04) the real existing regression suite. PASS.
- "2+ slices identical except for scale" — none; each targets a distinct mechanism (define vs. evaluate-identified vs. evaluate-anonymous vs. prove-unaffected vs. simulate). PASS.

---

## Wave: DISCUSS / [REF] Prioritization

| Priority | Slice | Target Outcome | Rationale |
|---|---|---|---|
| 1 | Slice 01 (WS) | A collection can have a rule defined | Walking Skeleton first — without a defined rule, Slices 02–04 have nothing to evaluate against |
| 2 | Slice 02 (WS) | A signed-in end user's own-document read is allowed; another's is denied | Closes the loop the raw ask requires; burns down the riskiest new assumption (a boolean condition over identity + document data can be evaluated correctly on the hot read path) immediately after Slice 01 |
| 3 | Slice 03 (WS) | An anonymous session is evaluated correctly | Gives `client-auth`'s optional identity real behavioral consequence for the first time — the single most load-bearing continuity claim with the prior epic |
| 4 | Slice 04 (WS) | Untouched collections and the existing regression suite are provably unaffected | The single highest-consequence regression risk in this feature (mirrors `client-auth`'s AC-16-08); sequenced last within the WS because it is a proof *over* Slices 01–03's real behavior, not a standalone mechanism |
| 5 | Slice 05 | Alex can test a rule before publishing it | Highest-leverage for Alex's own confidence and for preventing the failure modes narrated in § Journey, but depends conceptually on Slice 02's evaluation routine existing to wrap — correctly sequenced after the Walking Skeleton, exactly mirroring `client-auth`'s own debug-verify (US-04) sequencing |

---

## Wave: DISCUSS / [REF] System Constraints

- **Rule definition is idempotent upsert, not register-then-rotate.** Per Resolution 3, the same action defines and redefines (fully replaces) a rule. DESIGN must not import `client-auth`'s two-generation rotation-window pattern here — it solves a different problem (safe transition for a rarely-changed secret) than this one (frequent, deliberate iteration during authoring).
- **Critical regression guardrail.** A collection with no rule defined **must** continue to behave exactly as it does today, gated only by the existing `api_key` (Resolution 2). A rule defined for one collection must not observably affect any other collection in the same project, nor any of the 113 pre-existing `embyr-rs`/`client-auth` regression scenarios (none of which define a rule). This is this feature's AC-16-08 equivalent — the single highest-consequence design risk.
- **Existence non-leakage.** A read denied by a rule must not reveal, via any difference in its response, whether the target document actually exists. This is a locked, testable security-observable behavior, not an implementation nicety.
- **Shared artifact — reuse, do not re-derive, identity.** The rule-evaluation step's `request.auth` **must** consume the exact `VerifiedEndUserIdentity`/`None` value `attach_client_identity_if_present()` already computes (currently discarded as `_verified_identity` in `handle_get_document`) — not a second, independently-verified copy. See § Shared Artifact above for the integration risk this mitigates.
- **v1 enforcement surface is `GetDocument` only.** `RunQuery`, `CreateDocument`, `UpdateDocument`, `DeleteDocument`, `BeginTransaction`/`Commit`, and `Listen` are **not** gated by rules in this feature — see § Scope Assessment's split and § Out of Scope. DESIGN must not silently widen enforcement to any of these; each is a named, separately-scoped follow-up epic.
- **Rule-expressiveness ceiling.** Per Resolution 1, conditions may reference only `request.auth` (`null` or `{uid}`) and `resource.data.<field>` via `==`/`!=`/`&&`/`||`/`!`/literals. No cross-document reads (`get()`/`exists()`), no custom functions, no wildcard/recursive path matching, no `request.auth.token.<claim>` (no custom-claims map exists on `VerifiedEndUserIdentity` today — extending that is a `client-auth`/ADR-024 concern, not this feature's).
- **Bounded-context placement — flagged, not locked.** Unlike `client-auth`'s credential resolution (ADR-002 Option D — correctly folded into BC-1, since it has no entities/lifecycle of its own), a per-collection rule **does** have an aggregate with identity and a define→redefine lifecycle. It is also naturally System-DB-scoped (owned by the project, like BC-1's other concerns) while its *evaluation* consumes BC-2 Document Storage's `resource.data` (Customer DB-scoped) — straddling the exact storage-boundary line ADR-002 uses as its primary bounded-context signal. DESIGN (with `ddd-architect`, if invoked) should evaluate whether this warrants a fourth bounded context (candidate name: Access Control) rather than defaulting the new subsystem into BC-1 or BC-2 by inertia.
- Ubiquitous language introduced: **access-control rule** / **rule** (the condition Alex defines per collection), **rule evaluation** (checking a `request`/`resource` pair against a rule), **anonymous** (a session with no verified end-user identity — `request.auth == null`), **rule simulation** (testing a candidate rule with synthetic data, with zero effect on live traffic). These terms should carry forward into DESIGN's naming, not be silently renamed.

---

## Wave: DISCUSS / [REF] User Stories

<!-- markdownlint-disable MD024 -->

### US-01: Alex Defines (and Redefines) an Access-Control Rule for a Collection

**job_id**: JOB-17
**Slice**: 01 (Walking Skeleton) | **Release**: 1

#### Elevator Pitch
Before: Alex has no way to tell embyr which of Trailmark's collections need per-user protection — every one of Trailmark's signed-in end users, once `client-auth` verifies them, still has identical read access to every document in every collection.
After: call the admin API's project-scoped rule-definition action for a named collection (exact endpoint shape DESIGN's call) with a condition such as `request.auth.uid == resource.data.owner_id` → sees confirmation the rule is stored and active for that collection; calling the same action again with a different condition immediately replaces it.
Decision enabled: Alex knows a specific collection is now protected by the exact condition he wrote, and can move on to verifying it behaves the way he expects (US-05) before any of Trailmark's real users are affected.

#### Domain Examples
1. **Happy Path**: Alex defines, for the first time, a rule on `journal_entries` in `trailmark-prod`: `request.auth != null && request.auth.uid == resource.data.owner_id`. Sees confirmation the rule is stored and active.
2. **Edge Case**: An hour later, Alex redefines the same collection's rule to also require a second field check. The new condition fully replaces the old one immediately — no blend of old and new, no separate "old rule still valid for a window" behavior (unlike credential rotation).
3. **Error/Boundary**: Alex, copy-pasting from an old Firebase project, submits a condition calling `get(/databases/(default)/documents/users/$(request.auth.uid))` — a cross-document read outside the v1 grammar (Resolution 1). Sees 400 naming specifically that cross-document reads are not supported in v1, not a generic parse error.

#### UAT Scenarios (BDD)

##### Scenario: First-time rule definition succeeds and is immediately active
Given project `trailmark-prod` exists and `journal_entries` has no rule defined yet
When Alex defines a rule with a valid v1-grammar condition, using a valid admin Bearer credential
Then the rule is stored and active for `journal_entries`

##### Scenario: Redefining an existing rule fully replaces it, with no overlap window
Given `journal_entries` already has an active rule
When Alex submits a new condition for the same collection
Then the new condition is immediately and fully active, and the previous condition no longer applies to any subsequent read

##### Scenario: A condition using an out-of-v1-scope construct is rejected, naming what's unsupported
Given project `trailmark-prod` exists
When Alex submits a condition that calls `get()` or `exists()` on another document
Then the request is rejected with a message naming that cross-document reads are not supported in v1, distinguishable from a plain syntax error

##### Scenario: A condition with invalid syntax is rejected with a specific reason
Given project `trailmark-prod` exists
When Alex submits a condition with unbalanced parentheses or an unrecognized operator
Then the request is rejected with a message naming what specifically is invalid

##### Scenario: Rule definition without valid admin credentials is rejected
Given project `trailmark-prod` exists
When Alex submits a rule-definition request with a missing or invalid admin Bearer credential
Then the request is rejected the same way any other admin endpoint rejects missing/invalid credentials

#### Acceptance Criteria
- [ ] AC-17-01: A valid first-time rule definition is stored and active for the named collection.
- [ ] AC-17-02: Redefining a collection's rule fully and immediately replaces the prior condition — no merge, no overlap window.
- [ ] AC-17-03: A condition using a construct outside the v1 grammar (cross-document reads, functions, wildcard paths) is rejected with a message naming that specifically, distinguishable from a plain syntax error.
- [ ] AC-17-04: A condition with invalid syntax is rejected with a message naming what specifically is invalid.
- [ ] AC-17-05: Missing or invalid admin Bearer credential is rejected 401, consistent with existing admin-endpoint behavior.

#### Outcome KPIs
See § Outcome KPIs below (feeds KPI #1, North Star).

#### Technical Notes (Optional)
Exact endpoint path, persistence shape, and how "replace" is implemented (versioned rows vs. in-place update) are DESIGN's call — this story locks observable behavior only. Per Resolution 3, do not model this as register-then-rotate; per § System Constraints, the v1 grammar ceiling (no `get()`/`exists()`/functions/wildcards) is locked, not DESIGN's call to widen.

---

### US-02: A Signed-In End User's Read Is Gated by Their Own Rule

**job_id**: JOB-17
**Slice**: 02 (Walking Skeleton) | **Release**: 1

#### Elevator Pitch
Before: Maria's verified identity exists on her session (`client-auth`), but nothing checks it — her `getDoc()` on her own trip-journal entry succeeds identically to Dana's `getDoc()` on that same entry.
After: call the SDK's existing `getDoc()` on `journal_entries/maria-trip-042` — an unchanged SDK method — now evaluated against Trailmark's published rule → Maria sees the document; Dana, calling the identical unchanged method on the identical document, sees a permission-denied error instead.
Decision enabled: Alex knows his rule is doing the one thing he wrote it to do — separating "my own document" from "someone else's" — the exact protection Maria's data now genuinely has.

#### Domain Examples
1. **Happy Path**: Maria Santos (`end_user_id: maria-santos`), signed in, calls `getDoc()` on `journal_entries/maria-trip-042` (`owner_id: maria-santos`). The rule's condition evaluates true. She sees the document, exactly as before this feature shipped.
2. **Edge Case**: Dana Kim (`end_user_id: dana-kim`), signed in, calls `getDoc()` on that same `journal_entries/maria-trip-042`. The rule's condition evaluates false. She sees a permission-denied error, attributable to the rule (not to project suspension or any other existing rejection cause).
3. **Error/Boundary**: A `journal_entries` document written before this feature existed is missing the `owner_id` field entirely. Any caller's read against it fails closed (denied), never a crash — and the denial response is identical to what a caller would see if the document didn't exist at all.

#### UAT Scenarios (BDD)

##### Scenario: A signed-in end user reading their own document succeeds unchanged
Given `journal_entries` has a rule requiring `request.auth.uid == resource.data.owner_id`
And Maria Santos holds a verified identity and `journal_entries/maria-trip-042` has `owner_id: "maria-santos"`
When Maria calls `getDoc()` on that document
Then the read succeeds and returns the document exactly as it would have before this feature shipped

##### Scenario: A different signed-in end user's read of the same document is denied
Given `journal_entries` has a rule requiring `request.auth.uid == resource.data.owner_id`
And Dana Kim holds a verified identity distinct from the document's owner
When Dana calls `getDoc()` on `journal_entries/maria-trip-042`
Then the read is denied with PermissionDenied, attributable to the rule

##### Scenario: A rule not based on ownership allows any signed-in caller
Given `trail_guides` has a rule requiring only `request.auth != null` (no ownership check)
When Dana Kim, signed in, calls `getDoc()` on a `trail_guides` document she does not own
Then the read succeeds

##### Scenario: A condition referencing a missing field fails closed, not with an error
Given `journal_entries` has a rule requiring `request.auth.uid == resource.data.owner_id`
And a `journal_entries` document exists with no `owner_id` field at all
When any signed-in end user calls `getDoc()` on that document
Then the read is denied, and no internal error or crash occurs

##### Scenario: A denied read never reveals whether the target document exists
Given `journal_entries` has an ownership rule
When Dana Kim calls `getDoc()` on a document she doesn't own, and separately on a document ID that doesn't exist at all
Then both calls return the identical PermissionDenied response, with no distinguishing detail

#### Acceptance Criteria
- [ ] AC-17-06: A signed-in end user reading a document that satisfies the rule's condition succeeds, unchanged from pre-feature behavior.
- [ ] AC-17-07: A signed-in end user reading a document that fails the rule's condition is denied with PermissionDenied, attributable to the rule and distinguishable from project-suspension or any other existing rejection cause.
- [ ] AC-17-08: A rule not based on document ownership (e.g., "any signed-in caller") correctly allows callers who are not the document's owner — proving the grammar is not limited to owner-equality.
- [ ] AC-17-09: A condition referencing a document field that is absent evaluates to denied (fails closed), never an internal error.
- [ ] AC-17-10: A denied read's response is identical whether or not the target document actually exists — document existence is never leaked to a caller the rule has denied.

#### Outcome KPIs
See § Outcome KPIs below (KPI #1 North Star, KPI #3 Guardrail).

#### Technical Notes (Optional)
Must consume the existing `VerifiedEndUserIdentity`/`None` value already computed by `attach_client_identity_if_present()` in `handler.rs` — do not re-verify identity through a second path (§ System Constraints, Shared Artifact). Exact evaluation-order internals (fetch document then evaluate, vs. some other order) are DESIGN's call; only the externally observable non-leakage behavior (AC-17-10) is locked.

---

### US-03: A Session With No Verified Identity Is Evaluated as Anonymous

**job_id**: JOB-17
**Slice**: 03 (Walking Skeleton) | **Release**: 1

#### Elevator Pitch
Before: a Trailmark session that never signs in reads every document identically to one that did — `client-auth`'s optional identity has no observable consequence for anyone who skips it.
After: an unsigned-in session calls the existing `getDoc()` on a collection whose published rule requires `request.auth != null` → sees a permission-denied error, distinguishable from a rule denial caused by a wrong owner; the same session calling `getDoc()` on a collection whose rule allows public read still succeeds.
Decision enabled: Alex knows whether to require sign-in for a given collection is now genuinely his own per-collection call to make, not an all-or-nothing property of the whole project.

#### Domain Examples
1. **Happy Path (expected deny)**: A Trailmark session that never signed in at all attempts `getDoc()` on `journal_entries/maria-trip-042`, whose rule requires `request.auth != null`. The read is denied — `request.auth` is `null`, so the condition is false.
2. **Edge Case**: That same never-signed-in session calls `getDoc()` on a `trail_guides` document, whose rule is `allow read: if true`. The read succeeds — anonymous access is not blanket-denied, only denied where the rule itself requires identity.
3. **Error/Boundary**: A session presents a client-identity header that is malformed, expired, or minted for a different project (per `client-auth`'s existing ADR-026 semantics, `attach_client_identity_if_present()` already attaches nothing in all three cases). Its `getDoc()` on the `request.auth != null`-gated `journal_entries` collection is denied — evaluated identically to the fully-absent-header case, not as a distinct rejection reason.

#### UAT Scenarios (BDD)

##### Scenario: A never-signed-in session is denied by a rule requiring identity
Given `journal_entries` has a rule requiring `request.auth != null`
When a session that never presented a client-identity token calls `getDoc()` on a `journal_entries` document
Then the read is denied, attributable to the rule

##### Scenario: A never-signed-in session succeeds against a rule allowing public read
Given `trail_guides` has a rule of `allow read: if true`
When a session that never presented a client-identity token calls `getDoc()` on a `trail_guides` document
Then the read succeeds

##### Scenario: An invalid client-identity header is evaluated identically to no header at all
Given `journal_entries` has a rule requiring `request.auth != null`
When a session presents a malformed, expired, or wrong-project client-identity header and calls `getDoc()` on a `journal_entries` document
Then the read is denied identically to how a session presenting no header at all would be denied — no distinct rejection reason is introduced for this feature

#### Acceptance Criteria
- [ ] AC-17-11: A session with no verified identity is denied by a rule requiring `request.auth != null`, attributable to the rule.
- [ ] AC-17-12: A session with no verified identity succeeds against a rule that explicitly allows unauthenticated access (e.g., `allow read: if true`).
- [ ] AC-17-13: A session presenting an invalid (malformed/expired/wrong-project) client-identity header is evaluated identically, for rule purposes, to a session presenting no header at all — reusing `client-auth`'s existing ADR-026 DDD-CA-5 "attach nothing" semantics unchanged, introducing no new rejection class.

#### Outcome KPIs
See § Outcome KPIs below (KPI #1 North Star, KPI #4).

#### Technical Notes (Optional)
This story adds no new logic to `attach_client_identity_if_present()` — it only requires the rule-evaluation step (US-02) to treat its `None` result as `request.auth == null` in the v1 grammar. No changes to `client-auth`'s own code are needed or in scope.

---

### US-04: A Collection With No Rule Defined Keeps Reading Exactly as Before

**job_id**: JOB-17
**Slice**: 04 (Walking Skeleton) | **Release**: 1

#### Elevator Pitch
Before: Alex worries that defining a rule for one collection might silently start restricting every other collection in the same project too.
After: call the existing SDK `getDoc()` on a collection that has never had a rule defined (e.g. `app_config`) → sees the read succeed exactly as it did before this feature shipped, regardless of how many rules exist on other collections in the same project.
Decision enabled: Alex can adopt rules one collection at a time, at his own pace, with zero risk to collections he hasn't touched yet.

#### Domain Examples
1. **Happy Path**: Trailmark's `app_config` collection has never had a rule defined. Any caller's `getDoc()` call succeeds exactly as it did before this feature shipped, gated only by the existing project `api_key`.
2. **Edge Case**: `trailmark-prod` has a rule defined for `journal_entries` but never for `trail_guides`. Reads on `trail_guides` remain fully unrestricted while `journal_entries` is now gated — the two collections do not interact.
3. **Error/Boundary**: The full pre-existing 113-scenario regression suite (72 `embyr-rs` + 41 `client-auth`) is re-run, unmodified, against a system where this feature is deployed but no project in the suite has ever defined a rule. All 113 pass exactly as before.

#### UAT Scenarios (BDD)

##### Scenario: A collection that has never had a rule defined is unaffected by this feature
Given `app_config` has never had a rule defined
When any caller calls `getDoc()` on an `app_config` document, using only the existing project `api_key`
Then the read succeeds exactly as it did before this feature shipped

##### Scenario: A rule on one collection does not affect a sibling collection without its own rule
Given `journal_entries` has an active rule and `trail_guides` in the same project has none
When a caller with no matching rule-satisfying identity calls `getDoc()` on a `trail_guides` document
Then the read succeeds unaffected by `journal_entries`'s rule

##### Scenario: The full pre-existing regression suite passes unmodified
Given none of the projects exercised by the 72 `embyr-rs` and 41 `client-auth` regression scenarios has ever defined a rule
When the full 113-scenario suite is re-run against a build that includes this feature
Then all 113 scenarios pass exactly as they did before this feature was added

#### Acceptance Criteria
- [ ] AC-17-14: A collection with no rule defined behaves identically to pre-feature behavior — unrestricted by identity, gated only by the existing `api_key`.
- [ ] AC-17-15: A rule defined for one collection has zero observable effect on any other collection in the same project that has no rule of its own.
- [ ] AC-17-16: The full 113-scenario pre-existing regression suite (72 `embyr-rs` + 41 `client-auth`) passes unmodified.

#### Outcome KPIs
See § Outcome KPIs below (KPI #3 Guardrail).

#### Technical Notes (Optional)
This story is primarily a proof obligation over US-01–US-03's real behavior, not new production logic — mirrors `client-auth`'s own AC-16-08 discipline (a structural, not just tested, non-regression claim is preferred if achievable without over-scoping DISCUSS's own remit; the exact structural-unreachability mechanism, if any, is DESIGN's call).

---

### US-05: Alex Tests a Rule Against Concrete Examples Before Publishing It

**job_id**: JOB-17
**Slice**: 05 | **Release**: 2

#### Elevator Pitch
Before: Alex's only way to find out whether a rule does what he intended is to publish it and watch real Trailmark users' calls succeed or fail.
After: call the admin API's rule-simulation action (exact endpoint shape DESIGN's call) with a candidate condition plus a synthetic identity and document payload → sees the resolved allow/deny outcome, without the simulation touching any live document or affecting real traffic.
Decision enabled: Alex catches an over-permissive or over-restrictive rule bug during his own testing, before it reaches Maria or Dana in production.

#### Domain Examples
1. **Happy Path**: Alex, testing locally, simulates his candidate `journal_entries` rule with a synthetic identity `end_user_id: test-user-001` and a synthetic document `{owner_id: "test-user-001"}`. Sees "allow" — matching what he expected.
2. **Edge Case**: Alex simulates the same rule with a mismatched pair (`end_user_id: test-user-002`, document `{owner_id: "test-user-001"}`) that he mistakenly expected to be denied but the rule was written with the equality reversed. Sees "allow" instead of the expected "deny" — catching the exact over-permissive bug before publishing.
3. **Error/Boundary**: Alex simulates the same rule with no synthetic identity at all (representing an anonymous caller). Sees "deny" (per the rule's `request.auth != null` clause), matching what a real anonymous caller would get — without any live document or session being created or touched.

#### UAT Scenarios (BDD)

##### Scenario: Simulating a valid candidate rule against a matching identity/document pair returns the correct outcome
Given Alex holds a candidate rule and a synthetic identity/document pair that should satisfy it
When Alex calls the simulation action with the candidate rule and the synthetic pair
Then the response shows "allow," matching what real evaluation would produce for that pair

##### Scenario: Simulation surfaces an over-permissive rule bug before publishing
Given Alex holds a candidate rule he believes denies a mismatched identity/document pair
When Alex calls the simulation action with that pair and the candidate rule instead allows it
Then the response shows "allow," surfacing the discrepancy from Alex's expectation before the rule is ever published

##### Scenario: Simulation has zero effect on live traffic
Given `journal_entries` has an active, published rule
When Alex calls the simulation action with a different candidate rule and synthetic data
Then real callers' `getDoc()` calls against `journal_entries` continue to be evaluated against the published rule, unaffected by the simulation

##### Scenario: Simulation supports the anonymous case identically to real evaluation
Given Alex holds a candidate rule requiring `request.auth != null`
When Alex calls the simulation action with no synthetic identity, representing an anonymous caller
Then the response shows "deny," matching what a real anonymous caller would receive under US-03

#### Acceptance Criteria
- [ ] AC-17-17: Simulating a candidate rule against a synthetic identity/document pair returns the same allow/deny outcome real evaluation would produce for that exact pair.
- [ ] AC-17-18: Simulating a rule has zero effect on live/published traffic — a live collection's real reads are evaluated only against its published rule, never a simulated one.
- [ ] AC-17-19: Simulation supports the anonymous (no synthetic identity) case, matching US-03's real anonymous-evaluation semantics exactly.

#### Outcome KPIs
See § Outcome KPIs below (KPI #4).

#### Technical Notes (Optional)
Strongly encouraged to be implemented as a thin wrapper over the exact same evaluation routine used by real enforcement (US-02/US-03), not a duplicate — see § System Constraints' shared-artifact integration risk, mirroring `client-auth`'s own US-04 debug-verify precedent.

---

## Wave: DISCUSS / [REF] Outcome KPIs

### Feature: security-rules

### Objective
Give every rule Alex defines real, correct per-user read protection — gated on the identity `client-auth` already establishes, for both signed-in and anonymous callers — while every collection he hasn't touched remains provably unaffected, closing the gap that today leaves `client-auth`'s identity establishing *who* is calling without anything checking *what they may access*.

### Outcome KPIs

| # | Who | Does What | By How Much | Baseline | Measured By | Type |
|---|---|---|---|---|---|---|
| 1 | SDK developers who define a read rule for a collection (e.g. Alex/Trailmark) | Have that rule correctly allow/deny `GetDocument` reads for both signed-in and anonymous callers, matching the rule's logical evaluation | 100% of reads against a rule-configured collection produce the result the rule's condition logically implies (no false-allow, no false-deny) | 0% (capability does not exist today — no read is evaluated against identity at all) | Acceptance-scenario pass rate against the rule-evaluation truth table (own-doc allow, other-doc deny, anonymous per-rule, missing-field fail-closed) | North Star |
| 2 | SDK developers whose reads are denied by a rule | Identify that the denial is rule-caused (not project-suspension or another existing rejection class) from the response alone | ≥90% of rule-caused denials are self-attributed without an internal support ticket | 0% (no rule-denial rejection class exists today) | Support-ticket tagging cross-referenced with denial-response inspection | Leading |
| 3 | Existing embyr-rs/client-auth customers and collections that never define a rule | Continue to read successfully, unaffected | 0% regression across the 113 pre-existing regression scenarios (72 `embyr-rs` + 41 `client-auth`) | Current 100% pass rate (pre-feature) | Full regression suite, pre/post comparison | Guardrail |
| 4 | SDK developers authoring and testing rules before publishing them | Catch an incorrect rule (over-permissive or over-restrictive) via simulation before it reaches a real end user | ≥1 rule bug caught pre-publish per integration/testing cycle (qualitative, ramps with adoption); 0 confirmed data-leak or lockout incidents traced to an un-simulated rule | N/A (capability does not exist today) | Simulation-action usage log cross-referenced with post-publish denial/allow-rate anomalies | Leading |

---

## Wave: DISCUSS / [REF] Out of Scope

- **Write-path enforcement** (`CreateDocument`/`UpdateDocument`/`DeleteDocument` gated by rules) — named, deferred follow-up epic (candidate id `security-rules-write-path`, "Epic 2b"). The same rule grammar is expected to extend here, but write rules commonly need both old (`resource`) and new (`request.resource`) document state — a genuinely separate DISCUSS/DESIGN concern flagged in Resolution 1.
- **Query-path enforcement** (`RunQuery`/list gated by rules) — named, deferred follow-up epic (candidate id `security-rules-query-path`, "Epic 2c"). Structurally different mechanism: a query must be provably rule-compliant *before* execution, not filtered after — real Firestore rejects non-compliant query shapes outright rather than silently filtering results, and this feature does not attempt that.
- **Real-time Listen enforcement** (`onSnapshot`/BC-3 fan-out gated by rules) — named, deferred follow-up epic (candidate id `security-rules-realtime`, "Epic 2d"). BC-3's eventual-consistency, re-fetch-on-`DocChange` model is a different mechanism than a synchronous point-read check.
- **Rule history, versioning, and rollback; richer condition grammar (Resolution 1's Option A trigger); audit logging** — named, deferred follow-up epic (candidate id `security-rules-operations`, "Epic 2e"), mirroring `client-auth`'s own Release-2 operational-maturity pattern.
- **Full Firestore Rules Language parity** (cross-document `get()`/`exists()` reads, custom functions, recursive/wildcard path matching) — explicitly rejected for v1 per Resolution 1, Option A. A candidate follow-up ("Rules Language Expansion") only if concrete evidence for cross-document rule reads emerges.
- **Role-based / custom-claims authorization** (`request.auth.token.<claim>`) — `VerifiedEndUserIdentity` carries no custom-claims map today; adding one is a cross-epic change to `client-auth`'s own token contract (ADR-024), not this feature's call.
- **Extending `attach_client_identity_if_present`'s wiring beyond `GetDocument`** — inherited open item from `client-auth`'s own Follow-Up Work; relevant to whichever of Epics 2b/2c/2d picks up write/query/listen path next, not this feature's job.
- **Re-opening any part of `client-auth`'s already-shipped scope** (custom-token verification, credential rotation, the debug-verify endpoint) — done, merged, out of bounds per this DISCUSS's explicit scope boundary.
- **embyr-hosted email/password or any other identity provider** — remains out of scope per `client-auth`'s own still-standing exclusion; unrelated to rules, not reopened here.

---

## Wave: DISCUSS / [REF] WS Strategy

Walking Skeleton Strategy: **B — Thin End-to-End Slice**. Slices 01–04 are real, narrow vertical slices against real System DB rule state, real Maria/Dana signed-in sessions, and real anonymous sessions (no facade, no mock) — Slice 01 proves the riskiest new assumption (a rule can be defined/redefined and syntax-validated using existing admin-API conventions); Slice 02 proves the second riskiest assumption (a boolean condition over identity + document data can be correctly evaluated on the hot `GetDocument` path); Slice 03 proves the anonymous case gives the optional identity real teeth; Slice 04 proves the whole thing is additive, not a regression. Together they form the thinnest end-to-end flow: define → evaluate (identified, anonymous) → prove untouched collections are unaffected.

---

## Wave: DISCUSS / [REF] Driving Ports

| Port | Protocol | Extension |
|---|---|---|
| Admin port `:9090` (existing, extended) | HTTP/1.1 | New rule-definition/redefinition action (US-01) and rule-simulation action (US-05), alongside existing project-lifecycle and `client-auth` credential actions |
| Data ports `:8080` (gRPC) / `:8081` (REST/gRPC-Web) (existing, extended in observable behavior only) | gRPC / HTTP | `GetDocument`'s existing, unchanged call shape now additionally reflects rule evaluation when a rule is defined for the target collection (US-02/03/04) — no new RPC or endpoint added on the data plane itself |

No new network-facing port introduced. Exact endpoint/action shapes are DESIGN's call.

---

## Wave: DISCUSS / [REF] Pre-requisites

- `docs/feature/client-auth/feature-delta.md` (full — `VerifiedEndUserIdentity` shape, ADR-024/025/026, the discarded `_verified_identity` in `handle_get_document`, and this feature's own charter quoted from `client-auth`'s Out of Scope).
- `crates/embyr-server/src/grpc/handler.rs`'s `attach_client_identity_if_present`/`authenticate`/`handle_get_document` (the exact wiring point this feature extends; must not restructure `authenticate()`'s existing three-role `api_key` check).
- `crates/embyr-core/src/client_identity/mod.rs` (`VerifiedEndUserIdentity { end_user_id, project_id, expires_at_unix }` — the complete identity shape available to rules; no custom claims).
- `docs/product/architecture/adr-002-bounded-contexts.md` (BC-2 Document Storage's storage boundary; Option D's reasoning, which does not cleanly cover a rule-definition subsystem with its own aggregate/lifecycle — flagged for DESIGN, not resolved here).
- `docs/product/jobs.yaml` (JOB-16 the evidentiary and dependency basis; JOB-17 new).
- `docs/evolution/2026-08-17-client-auth.md` (§ Follow-Up Work — confirms this epic is the named next consumer of the identity-attach step, and that it is wired only into `handle_get_document` today).

---

## Wave: DISCUSS / [REF] Handoff Package

**To DESIGN (solution-architect)**: this `feature-delta.md` (journey + story map + user stories + embedded AC), 5 slice briefs (`docs/feature/security-rules/slices/slice-01-define-and-redefine-rule.md` through `slice-05-simulate-rule-before-publish.md`), `docs/product/jobs.yaml` (JOB-17, new; JOB-16 cross-reference NOTE), `docs/product/journeys/sdk-developer.yaml` (extended with JOB-17).

**To DEVOPS (platform-architect)**: § Outcome KPIs above (4 KPIs — 1 North Star, 2 Leading, 1 Guardrail — for instrumentation planning).

**Explicit flags for DESIGN**:
1. § Job Discovery Framing Resolution's Resolution 1 (Option C, constrained boolean grammar) is the locked v1 rule-expressiveness scope — do not silently expand toward full Firestore Rules Language parity (Option A) or narrow to a fixed rule-shape enum (Option B).
2. Resolution 3 (idempotent upsert, not register-then-rotate) is locked — do not import `client-auth`'s credential-rotation pattern here; it solves a different problem.
3. § System Constraints' critical regression guardrail (AC-17-14/15/16) is this feature's single highest-consequence design risk — a collection with no rule defined, and every one of the 113 pre-existing `embyr-rs`/`client-auth` regression scenarios, must keep working exactly as today.
4. § System Constraints' existence-non-leakage constraint (AC-17-10) is a locked security-observable behavior, not an implementation nicety.
5. § Shared Artifact's identity-reuse constraint: the rule-evaluation step's `request.auth` must consume `attach_client_identity_if_present()`'s existing (currently-discarded) result — not re-derive or re-verify identity through a second path.
6. v1 enforcement surface is `GetDocument` only — do not silently widen to `RunQuery`/writes/`Listen`; each is a named, deferred follow-up epic (§ Scope Assessment, § Out of Scope).
7. Bounded-context placement for the new rule-definition/storage subsystem is flagged, not locked — evaluate against ADR-002's own Option C/D reasoning (a rule has entities/lifecycle, unlike Option D's rejected credential-resolution-as-context) rather than defaulting it into BC-1 or BC-2 by inertia.
8. Simulation (US-05) must share the exact evaluation routine real enforcement (US-02/03) uses — see § Shared Artifact's second flagged HIGH integration risk.

Peer review: not invoked per-wave (default skip per SKILL Phase 3 step 6 — this DISCUSS's three genuine ambiguities [rule-expressiveness ceiling, default-when-unconfigured, upsert-vs-rotate lifecycle] are each resolved with fully explicit and auditable reasoning above, mirroring `client-auth`'s own precedent; no JTBD assumptions inherited from elsewhere requiring re-validation beyond JOB-16, already validated; no vendor-neutrality risk, since no technology was selected). Mandatory consolidated review fires at end of DISTILL.

---

## Wave: DISCUSS / [REF] SSOT Updates

- `docs/product/jobs.yaml` — added JOB-17 (`document-access-control`, P1 Alex). JOB-16 receives a cross-reference NOTE (not a rewrite), mirroring the project's established cross-reference pattern (JOB-02 → JOB-15, JOB-10 → JOB-14, JOB-01/JOB-16 → JOB-17).
- `docs/product/journeys/sdk-developer.yaml` — extended with JOB-17 in its `jobs` list (same persona, P1 Alex, new goal). No separate visual/YAML journey artifact produced — per the current single-narrative-file convention, full Comprehensive-depth journey detail (emotional arc, consequence arc, flows) lives inline in this file.
- No new persona file — Trailmark's end users (Maria Santos, Dana Kim) remain domain-example data within Alex's stories, not a formal persona, consistent with `client-auth`'s precedent.

---

## Wave: DISCUSS / [REF] Definition of Ready Validation

### Requirements Completeness Score: **0.97** (> 0.95 gate)

Computed across the three requirement categories (per `nw-bdd-requirements`'s Requirements Completeness Check):
- **Functional**: all 5 stories have complete Given/When/Then coverage of their happy path, at least one edge case, and at least one error/failure path; the rule-expressiveness ceiling and default-when-unconfigured behavior are both explicitly locked (Resolutions 1–2), not left ambiguous.
- **Non-functional**: security (existence non-leakage AC-17-10; fail-closed-on-missing-field AC-17-09; identity-reuse-not-re-derivation constraint) and a performance guardrail are both explicit — see NFR note below. Accessibility/usability NFRs are not applicable (no UI surface; this is an API-only feature, consistent with `client-auth`'s own Lightweight-depth precedent for NFR applicability).
- **Business rules**: idempotent-upsert semantics (Resolution 3), per-collection isolation (AC-17-15), and the four-way rejection/allow taxonomy (own-doc allow / other-doc deny / anonymous-per-rule / missing-field-fail-closed) are all explicitly specified with examples.

**NFR note (performance guardrail, flagged for DESIGN)**: a collection with no rule defined must add effectively zero overhead to the existing unarmed read path — a cheap existence-check against the rule store, not a full evaluation — mirroring the existing "fast-path status checks before the expensive Argon2id verification" precedent already established in `authenticate()` (`handler.rs`). Not a hard-numeric SLA at DISCUSS time (no baseline exists yet); DESIGN/DEVOPS should establish one once real traffic volume is known, consistent with `client-auth`'s own OQ-CA-02 precedent for deferring performance-tuning questions until profiling data exists.

The remaining 0.03 gap is OQ-SR-01 below (query-path scoping detail for a future epic) — explicitly flagged, not hidden, and does not block this feature's own DoR.

### DoR Checklist (9-item hard gate)

| # | DoR Item | Status | Evidence |
|---|---|---|---|
| 1 | Problem statement clear, domain language | PASS | Every story's Elevator Pitch "Before" line is stated in Alex/Maria/Dana domain terms, no technical jargon (e.g. US-02: "nothing checks it — her getDoc() succeeds identically to Dana's") |
| 2 | User/persona identified with specific characteristics | PASS | P1 Alex (SDK developer migrating an existing Firebase app, same specificity as `client-auth`); Maria Santos and Dana Kim as concrete rule-subject domain examples |
| 3 | 3+ domain examples per story with real data | PASS | Every story has exactly 3 Domain Examples using `trailmark-prod`, `journal_entries`, `maria-santos`/`dana-kim`, real field names — no generic placeholders |
| 4 | UAT scenarios in Given/When/Then (3–7 per story) | PASS | US-01: 5, US-02: 5, US-03: 3, US-04: 3, US-05: 4 — all within range |
| 5 | Acceptance criteria derived from UAT | PASS | Every AC (AC-17-01 through AC-17-19) traces 1:1 or 1:many to a specific scenario above it |
| 6 | Right-sized (1–3 days, 3–7 scenarios) | PASS | Largest slice (US-02) estimated 2 days / 5 scenarios; all others ≤5 scenarios, ≤1.5 days |
| 7 | Technical notes identify constraints | PASS | Every story's Technical Notes references the relevant locked constraint (grammar ceiling, identity-reuse, no-rotation-pattern) without prescribing implementation |
| 8 | Dependencies resolved or tracked | PASS | Sole dependency, `client-auth`, is DONE and merged to `master` (confirmed via `docs/evolution/2026-08-17-client-auth.md`); `VerifiedEndUserIdentity` exists and is readable today |
| 9 | Outcome KPIs defined with measurable targets | PASS | 4 KPIs, each with a numeric or explicitly-qualitative-with-rationale target, baseline, and measurement method (§ Outcome KPIs) |

### DoR Status: **PASSED**

---

## Wave: DISCUSS / [REF] Open Questions

| ID | Question | Impact | Resolution owner |
|---|---|---|---|
| OQ-SR-01 | Exact bounded-context placement for the new rule-definition/storage subsystem (extend BC-1, extend BC-2, or a new 4th context) — flagged in § System Constraints, not resolved here | Affects DESIGN's Component Decomposition and possibly a future ADR-002 amendment; does not block this feature's observable-behavior contract, which is bounded-context-agnostic | Solution-architect (DESIGN), optionally with `ddd-architect` consultation |
| OQ-SR-02 | Whether Epic 2b (write-path) will need `resource`/`request.resource` (old vs. new document state) as two distinct grammar symbols, extending Resolution 1's Option C grammar | Does not block this feature (read-only); flagged for whoever picks up Epic 2b | Product Discovery / DISCUSS, triggered when Epic 2b starts |
| OQ-SR-03 | Whether custom claims on `VerifiedEndUserIdentity` (role-based rules, e.g. `request.auth.token.admin`) will ever be needed — would require a cross-epic extension to `client-auth`'s own token contract (ADR-024) | Does not block this feature; triggered by future customer-segment evidence, not scheduled work | Product Discovery, cross-referenced with `client-auth` |

---

## Wave: DISCUSS / [REF] Wave Decisions Summary

### Key Decisions
- [D1] Locked v1 rule-expressiveness scope to Resolution 1's Option C (constrained boolean grammar over `request.auth`/`resource.data`, no cross-document reads/functions/wildcards): strongest evidentiary fit against the concrete domain example and JOB-17's habit four-force, without Option A's unbounded scope or Option B's needless migration friction (see § Job Discovery Framing Resolution, Resolution 1)
- [D2] Locked default-when-unconfigured to "unrestricted, unchanged" (Resolution 2): the only option that does not break the 113-scenario pre-existing regression suite; a deliberate, flagged divergence from real Firebase's own locked-by-default posture (see Resolution 2)
- [D3] Locked rule lifecycle to idempotent upsert, not register-then-rotate (Resolution 3): matches Alex's actual Firebase-rules authoring habit of frequent redeploys, avoiding needless rotation-window process for a non-secret, frequently-edited artifact (see Resolution 3)
- [D4] Split the full read/write/query/listen ambition into 5 ordered epics via the Elephant Carpaccio gate; this DISCUSS covers Epic 2a (read-path) only, 2+ oversized signals fired at full-ambition scope (effort >2 weeks, multiple independent shippable outcomes) — see § Scope Assessment

### Requirements Summary
- Primary jobs/user needs: Alex needs a rule he defines for a collection to genuinely gate `GetDocument` reads by the identity `client-auth` already establishes — for signed-in callers (own-document vs. others') and anonymous callers alike — while every collection he hasn't touched, and the entire pre-existing regression suite, remain provably unaffected.
- Walking skeleton scope: define a rule (US-01) → evaluate it correctly for an identified owner, an identified non-owner, and an anonymous caller (US-02/03) → prove untouched collections and the existing suite are unaffected (US-04). Simulation (US-05) is Release 2.
- Feature type: Cross-cutting (spans the existing gRPC handler, a new rule-definition subsystem, and BC-2's read path).

### Constraints Established
- v1 enforcement surface is `GetDocument` only; write/query/listen are named, deferred follow-up epics.
- Rule evaluation must consume `client-auth`'s existing `VerifiedEndUserIdentity`/`None` value, not re-derive it.
- A denied read must never leak document existence.
- A collection with no rule defined is provably unaffected — the single highest-consequence guardrail.

### Upstream Changes
- None — no DISCOVER/DIVERGE artifacts exist for this feature (same as `client-auth`); this DISCUSS is grounded directly in `client-auth`'s own shipped artifacts and `docs/product/jobs.yaml`, per the task's own instruction that this is greenfield for the rules-evaluation piece landing on a mature existing system.

---

## Wave: DESIGN / [REF] Prior Wave Consultation — Reading Confirmation

**Wave**: DESIGN | **Agent**: Morgan (nw-solution-architect) | **Date**: 2026-08-17 | **Mode**: Propose (autonomous analysis, no live user available — options presented with trade-offs below, self-selected with rationale, exactly as a propose-mode session would resolve them)

✓ `docs/product/architecture/brief.md` § System Architecture, § Domain Model (Bounded Contexts, Aggregates, Context Map), § Application Architecture (Development Paradigm, Architectural Pattern, Component Decomposition, Driving/Driven Ports, Technology Choices, Reuse Analysis, Application-Level Decisions Table) — read in full
✓ `docs/product/architecture/brief.md` § Application Architecture — client-auth (lines 3316-3478) — read in full; direct structural precedent for this feature's own section
✓ `docs/product/architecture/adr-002-bounded-contexts.md` — read in full; Option D's rejection reasoning quoted verbatim and re-applied below (§ Bounded-Context Placement)
✓ `docs/feature/security-rules/feature-delta.md` (this file, DISCUSS sections, full) — 5 user stories, 19 ACs, 3 Framing Resolutions, System Constraints, Journey, Story Map, Outcome KPIs, Open Questions, and the 8-item Explicit-flags-for-DESIGN list, all consulted directly below
✓ `docs/feature/client-auth/feature-delta.md` (DESIGN wave section, full) — `VerifiedEndUserIdentity` shape, ADR-024/025/026's structure and shape, the admin-handler pattern in `client_identity.rs`, the register-then-rotate lifecycle this feature deliberately does NOT reuse (Resolution 3)
✓ `crates/embyr-server/src/grpc/handler.rs` (full, 1369 lines) — `authenticate()` (129-333, unchanged, three-role `api_key` check confirmed not restructured), `extract_client_identity_token`/`attach_client_identity_if_present` (144-383), and `handle_get_document` (506-551) — confirmed the exact current shape of the discarded `_verified_identity` binding
✓ `crates/embyr-core/src/client_identity/mod.rs` (full) — confirms `VerifiedEndUserIdentity { end_user_id, project_id, expires_at_unix }` is the complete identity shape available; no custom-claims map
✓ `crates/embyr-server/src/admin/handlers/client_identity.rs` (full) — register/rotate/verify handler pattern, direct precedent for the new rule-definition/simulation handler
✓ `crates/embyr-server/src/admin/handlers/shared.rs` (full) — `verify_project_ownership`, reused directly (not re-implemented)
✓ `crates/embyr-server/src/adapters/system_db.rs` (targeted read: `ClientIdentityCredentialRow`, `insert_client_identity_credential`, `get_client_identity_credential`, `rotate_client_identity_credential`, lines 9-282) — direct adapter-method-shape precedent
✓ Migration directory listing (`crates/embyr-server/migrations/*.sql`) — confirms 0021 is the highest existing migration; this feature adds 0022
✓ ADR directory listing (`docs/product/architecture/adr-*.md`) — confirms 026 is the highest existing ADR; this feature adds 027, 028, 029, and amends 002

No contradictions found between DISCUSS's requirements and existing architecture. `nwave-ai outcomes check-delta` was **skipped** — the CLI is not available in this environment; noted here rather than silently omitted, per the task's explicit instruction not to block on it.

---

## Wave: DESIGN / [REF] Quality Attribute Priorities

See `docs/product/architecture/brief.md` § Application Architecture — security-rules § Quality Attribute Priorities for the full ranked table (reproduced there for SSOT completeness). Summary, ranked: (1) no regression to collections with no rule defined — structurally enforced, not just tested; (2) existence non-leakage (AC-17-10); (3) fail-closed correctness (AC-17-09); (4) shared-artifact integrity between simulation and real enforcement (HIGH integration risk, § Handoff Package flag 8); (5) identity-reuse integrity (HIGH integration risk, flag 5); (6) grammar containment (flag 1).

---

## Wave: DESIGN / [REF] Existing System Analysis

Confirmed by direct code read (not assumed): `handle_get_document` (grpc/handler.rs:506-551) is the exact, single call site DISCUSS's Pre-requisites section names. `_verified_identity` (line 530) is genuinely computed and genuinely discarded today — the binding's underscore prefix is not decorative, `cargo build` would warn on an unused binding without it, confirming no other code path already consumes this value. `client_identity.rs`'s three handlers (register/rotate/verify) and `system_db.rs`'s matching adapter methods (169-281) are the closest available precedent for both the admin-handler shape and the storage-adapter shape a new rule subsystem needs — reused directly, not reimplemented (see § Reuse Analysis below). No existing code anywhere evaluates a boolean condition over identity + document data — confirmed independently during DESIGN (not just carried from DISCUSS's own Walking Skeleton Evaluation) by reading `grpc/handler.rs` in full and finding no such logic in any `handle_*` method.

---

## Wave: DESIGN / [REF] Constraint and Priority Analysis

The single highest-consequence constraint (§ System Constraints, "critical regression guardrail") is quantified precisely, not asserted: 113 pre-existing scenarios (72 `embyr-rs` + 41 `client-auth`), 0% of which define a rule today. Any design that adds even a single unconditional branch of new logic to the no-rule-defined path risks all 113. This is why § Composition (ADR-029) treats the `get_access_rule` → `None` short-circuit as the load-bearing structural decision of the whole feature, not an incidental implementation detail — it is the single point where 100% of the regression risk is retired by construction. Constraint-free opportunity: the parser/evaluator (ADR-027) has zero interaction with existing code paths at all — it is pure, additive, new code with no regression surface of its own; effort there is unconstrained by the regression risk and was prioritized on correctness/simplicity grounds instead (hand-rolled parser, Decision Driver 4 in ADR-027).

---

## Wave: DESIGN / [REF] Architecture Design

**Pattern**: No change to the project's Hexagonal (ports-and-adapters) architecture or Cargo-workspace enforcement mechanism (brief.md § Architectural Pattern). This feature adds one new inner hexagon (BC-4) using the identical mechanism BC-1/BC-2/BC-3 already use.

**The 8 Explicit Flags for DESIGN — resolved in order:**

**1. Grammar scope — confirmed NOT widened or narrowed.** Resolution 1's Option C
(comparison + boolean combinators over `request.auth`/`resource.data.<field>`/
`true`/`false`, no cross-document reads/functions/wildcards) is implemented
exactly as locked. ADR-027's grammar (EBNF, § Decision) has no production for
`get()`/`exists()`, function calls, or wildcard paths — they are explicitly
*recognized and rejected* (`ConditionParseError::UnsupportedConstruct`), not
silently accepted or silently absent. One scoped clarification is flagged, not
silently resolved: the locked grammar's literal-operand set (`true`/`false` only,
no string/number literals) is narrower than some real-world Firestore rules would
need — recorded as **OQ-SR-04** for DISTILL, not resolved unilaterally by DESIGN
in either direction (see ADR-027 § Grammar Gap Flagged for DISTILL).

**2. Rule lifecycle — confirmed idempotent upsert, not register-then-rotate.**
ADR-028's schema (`access_rules`, composite PK `(project_id, collection_path)`,
single `upsert_access_rule` method using `INSERT ... ON CONFLICT ... DO UPDATE`)
makes "define" and "redefine" the *same SQL statement* — there is no code branch
distinguishing first-time definition from replacement, unlike `client_identity_credentials`'s
deliberate `insert_*`/`rotate_*` split. Persistence shape decided: single row per
collection, in-place UPDATE (not versioned rows) — see ADR-028 § Considered
Options for the two versioned-row/audit-log alternatives considered and rejected
(both explicitly deferred to Epic 2e, not silently dropped).

**3. Structural regression guardrail — designed as a structural guarantee, not
just a tested one.** `get_access_rule(project_id, collection_path)` returning
`None` short-circuits `handle_get_document` before `embyr_core::access_control::evaluate()`
is ever called — the exact code-path shape flagged by the task's own hint,
mirroring `extract_client_identity_token` returning `None` short-circuiting
`attach_client_identity_if_present`. Every one of the 113 pre-existing regression
scenarios exercises a project/collection with zero rows in `access_rules`, so
every one of them takes this identical, unmodified branch — the guarantee is a
property of that branch containing no new code (ADR-029 § Structural
no-rule-defined guardrail), not merely a property re-verified by running the
suite (though AC-17-16 does that too, independently).

**4. Existence non-leakage — designed structurally.** Document existence is
checked (`adapter.get_document`) *before* the allow/deny decision, exactly as
today; the rule-lookup existence-check is independent of document existence
(keyed only on project+collection). When a rule is defined, `evaluate()` runs
unconditionally against either the real fetched fields or an empty field map (for
a non-existent document) — `Deny` always produces the identical `PermissionDenied`
response regardless of which. This structurally collapses "denied" and "not
found" into one indistinguishable wire-level outcome **for any rule that
references `resource.data`** (ADR-029 § Existence non-leakage) — the exact class
AC-17-10's own UAT scenario is written against. A scoped clarification for
content-blind rules (never referencing `resource.data`) is flagged as **OQ-SR-06**
rather than silently resolved.

**5. Identity reuse — exact mechanism designed.** In `handle_get_document`,
`_verified_identity` (line 530, currently discarded) is renamed to
`verified_identity` and threaded into the new evaluation step via a pure, local
translation (`verified_identity.as_ref().map(|v| AuthContext { uid: v.end_user_id.clone() })`)
performed at the call site — `attach_client_identity_if_present()` itself is not
modified in any way (ADR-029 § Identity reuse). No second verification path is
introduced anywhere.

**6. `GetDocument`-only enforcement surface — confirmed.** The single call site
touched is `crates/embyr-server/src/grpc/handler.rs::handle_get_document`. No
other RPC handler (`handle_create_document`, `handle_update_document`,
`handle_delete_document`, `handle_batch_get_documents`, `handle_begin_transaction`,
`handle_commit`, `handle_rollback`, `handle_run_query`, `handle_listen`) is
modified by this design — confirmed by direct enumeration against the full file
read during Prior Wave Consultation.

**7. Bounded-context placement — resolved: a new 4th bounded context, BC-4
Access Control.** See § Bounded-Context Placement below for the full alternatives
analysis against ADR-002's own five decision drivers, and
`docs/product/architecture/adr-002-bounded-contexts.md` § Changed Assumptions for
the formal amendment.

**8. Simulation shares the exact evaluation routine — designed and named.**
`embyr_core::access_control::evaluate()` (ADR-027) is called from exactly two
production sites: (a) `handle_get_document` (real enforcement, US-02/03/04), auth
sourced from `attach_client_identity_if_present()`'s result, resource fields from
`adapter.get_document()`'s result; (b) `admin::handlers::access_rules::simulate_access_rule`
(US-05, new), auth sourced from the caller-supplied synthetic identity in the
request body (or `None`), resource fields from the caller-supplied synthetic
document payload. `parse_condition()` is likewise shared across three call sites
(define/redefine's validation, real enforcement's re-parse, simulation's
candidate-condition parse). No third, independently-maintained evaluation
implementation exists anywhere (ADR-029 § Simulation shares the exact evaluation
routine).

---

## Wave: DESIGN / [REF] Bounded-Context Placement

**Options presented (propose-mode, self-selected with rationale — no live user available):**

| Option | Description | Fit against ADR-002's 5 decision drivers |
|---|---|---|
| **(A) Extend BC-1 Tenant Management** | Add `AccessRule` to BC-1's ubiquitous language alongside `Project`/`AuthKey`/`BackendConfig`/`ClientIdentityCredential`. | **Rejected.** Storage locality (System DB) superficially fits, but a `Rule` has an entity, a lifecycle, and invariants — exactly the three properties Option D's *rejection* of a BC-1 fold-in turned on being *absent* for credential resolution. Applying Option D's conclusion to a case that fails Option D's own test would be inconsistent. Continues a "BC-1 as junk drawer" drift. |
| **(B) Extend BC-2 Document Storage** | Add `AccessRule` alongside `Document`/`Transaction`/`Index`, since evaluation reads `resource.data`. | **Rejected — weakest fit.** Rule *storage* is System-DB-scoped; BC-2's own defining boundary (ADR-002's "primary signal") is "Customer DB only... BC-2 never reads the System DB." Folding rule storage into BC-2 breaks that boundary directly. BC-2's OCC consistency model also has no analog for a rule (idempotent replace, not version-conflict-detected). |
| **(C) A fourth bounded context, BC-4 Access Control** | New context owns the `AccessRule` aggregate; read-only, non-transactional dependency on BC-2's already-fetched `resource.data`, mirroring BC-3's own established read-only dependency on BC-2. | **Accepted — strongest fit on all 5 drivers**, most decisively on language divergence (Rule/Condition/Evaluation vocabulary exists nowhere else) and codebase isolation (independently unit-testable with no `Project` or `Document` aggregate at all). |

**Selected: Option C.** Full driver-by-driver analysis, the BC-3 precedent this
option directly mirrors, and the formal ADR-002 amendment are in
`docs/product/architecture/adr-029-access-control-composition-and-bounded-context.md`
§ Considered Options — Bounded-Context Placement and
`docs/product/architecture/adr-002-bounded-contexts.md` § Changed Assumptions
(Option D's original text quoted verbatim, new assumption stated, Context Map
addition appended — ADR-002's existing content is not deleted or rewritten).

---

## Wave: DESIGN / [REF] Component Decomposition

See `docs/product/architecture/brief.md` § Application Architecture —
security-rules § Component Decomposition for the full table (reproduced there for
SSOT completeness). Summary: `embyr-core::access_control` (new, pure, BC-4),
`embyr-server::admin::handlers::access_rules` (new, BC-4 driving adapter),
`embyr-server::adapters::system_db` (extended, BC-4 driven adapter),
`embyr-server::grpc::handler::handle_get_document` (extended, single call site),
`access_rules` table (new, migration 0022).

---

## Wave: DESIGN / [REF] Driving Ports

See `docs/product/architecture/brief.md` § Application Architecture —
security-rules § Driving Ports (Inbound) for the full table. Summary:
`AccessRuleAdminPort` (`POST /admin/v1/projects/:project_id/access_rules`,
define/redefine, US-01, Owner/Admin), `AccessRuleSimulationPort`
(`POST .../access_rules/simulate`, US-05, any role, zero writes), and
`FirestoreGrpcPort`/`RestPort` extended additively (no new RPC, no new endpoint —
`GetDocument`'s existing shape now additionally reflects rule evaluation).

---

## Wave: DESIGN / [REF] Driven Ports + Adapters

No new driven port. `upsert_access_rule`/`get_access_rule` reuse the existing,
already-probed `SystemDb` connection pool. No new adapter, no new `probe()` — see
`docs/product/architecture/brief.md` § Driven Ports + Adapters — security-rules
additions for the full Earned Trust (Principle 12) reasoning: `embyr_core::access_control`'s
`evaluate()`/`parse_condition()` are pure, deterministic CPU computation with no
partial-trust surface, identical in kind to `verify_client_identity_token()`'s own
"no new probe needed" justification (ADR-024).

---

## Wave: DESIGN / [REF] Technology Choices

No new workspace dependency. Condition parser: hand-rolled recursive-descent
(zero new crate) — `pest`/`nom` parser-generator crates were considered and
rejected (ADR-027 § Considered Options) specifically because their expressiveness
invites silently widening the locked grammar (Decision Driver 1), and because a
generator is disproportionate tooling for a grammar this small and deliberately
closed. `resource.data.<field>` values reuse the existing `embyr_core::domain::field_value::FieldValue`
type unchanged — no new value-representation type.

---

## Wave: DESIGN / [REF] Decisions Table

See `docs/product/architecture/brief.md` § Application Architecture —
security-rules § Decisions Table (DDD-SR-1 through DDD-SR-9) for the full table.

---

## Wave: DESIGN / [REF] Reuse Analysis (hard gate)

See `docs/product/architecture/brief.md` § Application Architecture —
security-rules § Reuse Analysis for the full 12-row table (10 EXTEND, 2 justified
CREATE NEW, 0 unjustified CREATE NEW). The two CREATE NEW rows: the condition
parser/evaluator (no existing mechanism evaluates a boolean condition over
identity + document data — confirmed independently by DESIGN's own code read, not
just carried from DISCUSS) and the `access_rules` table (no existing table stores
per-collection conditions; its schema shape mirrors an existing precedent, but the
data itself is new).

---

## Wave: DESIGN / [REF] C4 Diagrams

See `docs/product/architecture/brief.md` § Application Architecture —
security-rules for the full C4 System Context, Container, and Component (BC-4
Access Control) diagrams in Mermaid. No new external system is introduced;
Trailmark's end users (Maria, Dana) and Alex's admin credential are the same
actors `client-auth` already established. A Component diagram is included
(warranted: 5 separable pieces — parser, evaluator, storage adapter, 2 admin
handlers, 1 composition point — whose call-graph is exactly what makes flags 3 and
8 above verifiable at a glance).

---

## Wave: DESIGN / [REF] Architecture Enforcement

Style: Hexagonal (ports-and-adapters), unchanged. `embyr-core::access_control` has
zero IO imports, enforced by the existing `cargo-deny`/`deny.toml` rule already
covering all of `embyr-core` — no new crate-specific configuration needed (new
submodule, not a new crate). No new adapter, no new `probe()`, so no new
enforcement-tooling requirement (unlike `client-auth`, which needed none either,
for the identical "no new substrate" reason).

---

## Wave: DESIGN / [REF] Open Questions

See `docs/product/architecture/brief.md` § Application Architecture —
security-rules § Open Questions for the full table. New this wave: **OQ-SR-04**
(literal-operand grammar gap — string/number literals not expressible in v1,
flagged for DISTILL to confirm scope), **OQ-SR-05** (parsed-AST caching, deferred
pending profiling, mirrors OQ-CA-02), **OQ-SR-06** (content-blind-rule existence
-leak scoped clarification, flagged for DISTILL). Carried and resolved this wave:
**OQ-SR-01** (bounded-context placement — now closed, BC-4). Carried and still
open, out of this feature's scope: OQ-SR-02 (Epic 2b's `resource`/`request.resource`
need), OQ-SR-03 (custom claims / role-based rules).

---

## Wave: DESIGN / [REF] Handoff Package

**To DISTILL (acceptance-designer)**: this `feature-delta.md` (DISCUSS + DESIGN
sections), `docs/product/architecture/adr-027-access-rule-grammar-and-evaluation.md`,
`adr-028-access-rule-storage-and-lifecycle.md`,
`adr-029-access-control-composition-and-bounded-context.md`,
`docs/product/architecture/adr-002-bounded-contexts.md` § Changed Assumptions,
`docs/product/architecture/brief.md` § Application Architecture — security-rules.

**To DEVOPS (platform-architect)**: § Outcome KPIs (DISCUSS section, unchanged)
for instrumentation planning; § External Integrations — security-rules confirms
zero new outbound network dependency, no contract-testing surface introduced.

**Explicit flags for DISTILL/DELIVER**:
1. **ADR-029's structural regression argument is the acceptance-test design
   center of gravity** — mirrors `client-auth`'s ADR-026 precedent exactly.
   DISTILL should design the "collection with no rule defined is unaffected"
   scenario as a direct re-run of the existing 113-scenario suite, unmodified, not
   just one more happy-path test.
2. **OQ-SR-04 (literal-operand grammar gap) should be confirmed before DELIVER
   locks the parser's literal-operand support** — DESIGN implemented the literal
   locked-grammar reading (booleans only), not a guess in either direction.
3. **OQ-SR-06 (content-blind-rule existence-leak nuance) is a genuine, scoped
   edge case, not an oversight** — DISTILL should decide explicitly whether to
   write an acceptance scenario for it.
4. **Write/query/listen enforcement remains explicitly out of scope** —
   `embyr_core::access_control`'s types are read-path-shaped; Epic 2b/2c/2d will
   need to extend, not replace, `Condition`/`Operand` if/when they start.
5. **No new Earned Trust probe was added** — flagged explicitly (not silently
   skipped) per Principle 12 discipline; the reasoning (pure computation, reused
   already-probed `SystemDb`) is in ADR-029 § Enforcement.

Peer review: not invoked per-wave (default skip; reviewing against the four named
triggers — contested ADR: no, all three ADRs have single accepted options with
documented, evidence-driven alternatives; novel pattern: no, BC-4 deliberately
mirrors BC-3's already-established read-only-dependency precedent rather than
inventing a new relationship kind; performance-budget unverified by spike: no
explicit performance budget was set, and the grammar's CPU cost is
well-characterized as negligible relative to the existing Postgres round-trip
already on this hot path; security boundary change: arguably yes, in the sense
that a new authorization dimension is introduced, but the boundary itself
[fail-closed, non-leaking, identity-reuse-only] is the entire subject of
ADR-027/028/029's Alternatives analysis, already peer-reviewable from the
documents as written — mirrors `client-auth`'s own identical reasoning for
skipping per-wave review). Mandatory consolidated review fires at end of DISTILL
covering all 4 waves in parallel, per standard process.

---

## Wave: DESIGN / [REF] SSOT Updates

- `docs/product/architecture/brief.md` — new `## Application Architecture — security-rules` section added.
- `docs/product/architecture/adr-027-access-rule-grammar-and-evaluation.md` — new.
- `docs/product/architecture/adr-028-access-rule-storage-and-lifecycle.md` — new.
- `docs/product/architecture/adr-029-access-control-composition-and-bounded-context.md` — new.
- `docs/product/architecture/adr-002-bounded-contexts.md` — **amended** (§ Changed
  Assumptions appended; no existing content deleted or rewritten). This is the
  one deliberate, documented reversal this wave makes: Option D's fold-into-BC-1
  conclusion is confirmed correct for credential resolution and confirmed *not*
  transferable to this feature's rule subsystem.

---

## Wave: DESIGN / [REF] Wave Decisions Summary

### Key Decisions
- [D1] Grammar implemented exactly as Resolution 1's Option C locks it — hand-rolled recursive-descent parser, zero new dependency, structurally incapable of silently widening (see: ADR-027)
- [D2] Rule lifecycle is a single idempotent-upsert SQL statement (`ON CONFLICT DO UPDATE`), not a register/rotate split — matches Resolution 3's lock structurally, not just observably (see: ADR-028)
- [D3] The no-rule-defined guardrail (AC-17-14/15/16) is structural: `get_access_rule` returning `None` short-circuits before any evaluation logic runs, mirroring `attach_client_identity_if_present`'s own AC-16-08(c) precedent (see: ADR-029)
- [D4] Existence non-leakage (AC-17-10) is structural for any rule referencing `resource.data`: `Deny` always produces an identical `PermissionDenied` response regardless of document existence, via a shared fail-closed/empty-field-map mechanism (see: ADR-029)
- [D5] A 4th bounded context, BC-4 Access Control, is added — evaluated against ADR-002's own 5 decision drivers, not defaulted by inertia; amends ADR-002 via an appended, non-destructive § Changed Assumptions (see: ADR-029, adr-002 amendment)

### Architecture Summary
- Pattern: Hexagonal (ports-and-adapters), modular monolith — unchanged project-wide pattern, one new inner hexagon (BC-4)
- Paradigm: Functional-where-practical Rust — unchanged; `evaluate()`/`parse_condition()` are pure, total/infallible-by-design functions
- Key components: `embyr-core::access_control` (parser + evaluator, pure), `embyr-server::admin::handlers::access_rules` (define/redefine + simulate), `embyr-server::adapters::system_db` (extended: `upsert_access_rule`/`get_access_rule`), `embyr-server::grpc::handler::handle_get_document` (extended, single call site), `access_rules` table (migration 0022)

### Reuse Analysis
10 EXTEND, 2 justified CREATE NEW (condition parser/evaluator; `access_rules` table), 0 unjustified CREATE NEW. Full table: `docs/product/architecture/brief.md` § Application Architecture — security-rules § Reuse Analysis.

### Technology Stack
- No new workspace dependency. Hand-rolled recursive-descent parser (rejected `pest`/`nom` — grammar-widening risk, disproportionate tooling for a deliberately small closed grammar). Reuses `embyr_core::domain::field_value::FieldValue`, existing `SystemDb`/`sqlx`, existing `SessionContext`/Axum session sub-router.

### Constraints Established
- v1 enforcement surface is `GetDocument` only — single call site, confirmed by full-file enumeration.
- Rule evaluation consumes `client-auth`'s existing `VerifiedEndUserIdentity`/`None`, never re-derives it.
- A denied read never reveals document existence, for any rule referencing `resource.data` (scoped clarification for content-blind rules: OQ-SR-06).
- A collection with no rule defined is provably unaffected by construction, not just by test.
- Literal-operand grammar is booleans-only in v1 (OQ-SR-04, flagged not silently decided).

### Upstream Changes
- `docs/product/architecture/adr-002-bounded-contexts.md` amended (§ Changed Assumptions, appended) — a new 4th bounded context, BC-4 Access Control, is established. This is the one architecture-driven reversal this wave makes to a prior-wave (DESIGN-owned, cross-feature) artifact; it does not change any DISCUSS-locked observable behavior or user story.
