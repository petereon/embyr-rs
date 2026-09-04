# Feature Delta: security-rules-cel-cross-document-reads

## Wave: DISCUSS / [REF] Prior Wave Consultation — Reading Confirmation

✓ `docs/product/jobs.yaml` — JOB-17 (`document-access-control`) read in full, including every NOTE
appended by 4a/4b/4b′/4c. Same job, same persona (P1 Alex) — no new job created.
✓ `docs/feature/security-rules-cel-parity/feature-delta.md` — this feature's own naming origin:
§ Scope Assessment "4d — candidate `security-rules-cel-cross-document-reads`" (rejected for that
feature, named/deferred) and, load-bearing for this DISCUSS's own central finding, § Scope
Assessment's Bounded-Contexts row: **"`get()`/`exists()` requires BC-4 to actively *call into*
BC-2's read path mid-evaluation, not merely consume already-fetched data — the read-only,
in-process-only dependency shape ADR-029 established would no longer hold."** That finding is
NOT rediscovered here — it is the prior wave's own already-recorded verdict, confirmed structurally
below (§ Resolution 1), not re-litigated.
✓ `docs/feature/security-rules-cel-recursive-wildcards/feature-delta.md`,
`docs/feature/security-rules-cel-expression-grammar/feature-delta.md` § Out of Scope — both
carry this feature's own naming forward unchanged, no new scope note added by either.
✓ `docs/product/architecture/adr-002-bounded-contexts.md` § BC-4 Access Control — direct read:
BC-4's own CURRENT context-map entry is `→ BC-2 Document Storage [read-only, non-transactional:
resource.data during GetDocument evaluation]` — a single, ALREADY-KNOWN document (the one being
evaluated), never an on-demand fetch of an ARBITRARY OTHER document mid-evaluation. Confirms the
prior finding by direct read, not merely by citation.
✓ `deny.toml` — direct read: `embyr-core` (where `evaluate()`/`resolve_field_value` live) is
IO-forbidden by CI enforcement. `evaluate()` cannot itself perform a network/DB read — this is a
hard, tooling-enforced constraint, not a style preference, and is THE central design pressure this
feature's own DESIGN wave must resolve.
✓ `crates/embyr-core/src/access_control/mod.rs` — full re-read: `evaluate()`'s current 7-parameter
signature (ADR-062/063/065's own precedent: every new external value is PRE-RESOLVED by the caller
and threaded in as a parameter, never fetched inside `evaluate()` itself) — the SAME pattern this
feature's own central finding requires extending, not a new kind of change.
✓ `tests/security_rules_cel_parity/acceptance/cp04_reject_out_of_scope_imports.rs` — the ONLY
existing `get()`-shaped text anywhere in this initiative's own accumulated fixtures:
`get(/databases/x/documents/users/y).data.admin` — a REJECTION fixture only (proving 4a's own
`detect_unsupported_construct` correctly names and rejects it), never a real, evaluated domain
example. Confirms directly: **no Trailmark domain example anywhere in this initiative has ever
needed cross-document reads evaluated for real** — this feature's own domain example (below) is
constructed by evidenced analogy to the single most common published real-Firestore idiom
(organization/role-membership lookup), the same discipline every prior CEL-parity epic has used
when the charter's own named capability lacked a pre-existing Trailmark story.

**Live web verification** (unlike `security-rules-cel-recursive-wildcards`'s own DISCUSS, which
had no web-tool access and flagged its own recollection as unverified — this dispatch DOES have
`WebSearch`/`WebFetch` access, used here to resolve real Firestore's own `get()`/`exists()`
semantics with direct evidence, not recollection):
- `firebase.google.com/docs/firestore/security/rules-conditions` (fetched directly): real
  Firestore's own document-access limits are **10 for single-document/query requests, 20 for
  multi-document reads/transactions/batched writes** — exceeding either is a `permission denied`
  error (Deny). **Cached calls (repeated `get()`/`exists()` on the SAME path within one
  evaluation) do not count toward the limit.** Both functions require fully-specified document
  paths, with `$(variable)` syntax for path-segment substitution (e.g.
  `/databases/$(database)/documents/users/$(request.auth.uid)`).
- Community-verified behavior (Google Groups, cross-referenced against the official docs' own
  "returns the document" framing): `get()` returns the document; accessing `.data.<field>` on a
  `get()` result for a NONEXISTENT document throws — which, composed with real Firestore's OWN
  documented fail-closed-on-error semantics (an error anywhere in a rule denies the whole rule),
  means a `get()`-then-`.data` chain on a missing document is functionally equivalent to a
  fail-closed `Deny`. `exists()` instead returns `false` cleanly for a nonexistent path — a
  deliberate, real semantic DIFFERENCE between the two functions this feature's own grammar must
  preserve (never collapse `get()` and `exists()` into the same construct).

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

- Feature type: **Backend** (Decision 1) — extends BC-4's own evaluation mechanism with a genuinely
  new I/O capability, zero new user-facing admin surface beyond what already exists
  (`define_access_rule`/`define_write_access_rule`/`simulate_access_rule` accept arbitrary
  condition TEXT already — only what `parse_condition` accepts, and what `evaluate()` can now do
  with that parsed tree, widens).
- JTBD: **reuse JOB-17** (Decision 4 = "Yes", existing job) — the 12th realization of the SAME job.
  A NOTE is appended to `jobs.yaml`'s JOB-17 entry (§ SSOT Updates).
- Walking Skeleton: **Yes** (Decision 2) — the smallest slice of this feature's own new
  capability: a single, non-chained `exists()` check gating a real `GetDocument` call.
- UX Research Depth: **Lightweight** (Decision 3) — a backend I/O-capability extension, one
  persona (Alex, fully profiled across 6 prior epics), no new emotional arc.

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: P1 Alex (SDK Developer), unchanged.

**Job**: JOB-17 `document-access-control`, unchanged job_story. This feature's own realization:
Alex's real Firestore `.rules` file references another document's own fields to decide access —
the canonical real-world idiom being an organization/team-membership role lookup
(`allow write: if get(/databases/$(database)/documents/organizations/$(request.auth.uid)).data.role
== 'admin';`) — today, EVERY `get()`/`exists()` call is rejected outright at import time, named
`CROSS_DOCUMENT_READ`, unconditionally. After this feature, a NARROWLY-SCOPED, evidenced subset of
this idiom imports and enforces correctly.

## Wave: DISCUSS / [REF] Job Discovery — Framing Resolution

### Resolution 1 (THE central architectural question, ALREADY PARTIALLY ANSWERED by the prior
wave's own finding — this Resolution's own job is to design the actual mechanism, not rediscover
the problem) — How does `evaluate()` (100% pure, zero-IO, `embyr-core`, `deny.toml`-enforced)
perform a cross-document read it cannot itself issue?

**Confirmed by direct code read (§ Reading Confirmation), not assumed**: `evaluate()` cannot
change this constraint — `embyr-core` is IO-forbidden by CI enforcement, a hard, tooling-level
boundary, not something this feature can loosen "just this once." The ONLY viable mechanism is the
SAME one every prior grammar-extension epic has already used for every other externally-resolved
value (`path_variable_value`, ADR-062; `ancestor_path_variable_values`, ADR-063;
`request_time`, ADR-065): **pre-resolve the value BEFORE calling `evaluate()`, thread it in as a
new caller-supplied parameter.** The genuinely NEW piece this feature must build (never needed
before, since every prior pre-resolved value had a FIXED, small, statically-known set of possible
sources) is a **pure function that inspects the ALREADY-PARSED `Condition` tree and reports which
concrete document paths its own `get()`/`exists()` operands need** — so the caller (`embyr-server`,
which HAS I/O) knows WHAT to fetch before it ever calls `evaluate()`.

**Two-phase evaluation, locked**:
1. **Path-discovery** (NEW, pure, `embyr-core`): given the parsed `Condition` tree and the SAME
   already-known bindings every other operand family already has at the call site
   (`path_variable_value`, `ancestor_path_variable_values`, the caller's own `AuthContext`),
   resolve every `CrossDocumentGet`/`CrossDocumentExists` operand's own path template into a
   concrete document path string. Returns the SET of distinct paths to fetch.
2. **Fetch** (`embyr-server`, real I/O, mirrors `resolve_access_rule_pattern`'s own "one indexed
   lookup on the hot path" discipline): issue a real `BC-2` read for each distinct path from step
   1 (deduplicated — mirrors real Firestore's own "cached calls don't count" behavior, and is free
   to implement since a path→result map is being built anyway).
3. **Evaluate** (unchanged mechanism, `evaluate()`'s own new 8th parameter): the pre-fetched
   `path → Option<FirestoreDocument>` map threads in exactly like `request_time`'s own `Option<
   &FieldValue>` did — `resolve_field_value`'s new `CrossDocumentGet`/`CrossDocumentExists` arms
   are pure map lookups, zero I/O inside `embyr-core`.

**Resolution**: **locked as designed above** — this is the ONLY mechanism compatible with
`embyr-core`'s own IO-forbidden boundary, and it is a direct, evidenced extension of an ALREADY
-established pattern (3 prior precedents), never a new kind of architecture.

### Resolution 2 — Chaining: does a `get()`'s own result feed into ANOTHER `get()`'s own path, within one condition?

Real Firestore's OWN documented limits (10/20 distinct reads) implicitly allow chaining (a
role-lookup path built from a FIRST `get()`'s own result is a common published pattern). But: (a)
chaining makes path-discovery (Resolution 1, step 1) NO LONGER a single pure pass — it becomes
iterative (discover paths → fetch → discover MORE paths from the just-fetched results → fetch
again → ...), a materially more complex mechanism; (b) **zero domain evidence** anywhere in this
initiative's own accumulated text needs chaining — the ONE evidenced-by-analogy domain example
(§ Persona & Job) needs exactly ONE level (a single `get()`, its own path built ONLY from
already-known bindings — `request.auth.uid`/a path variable — never from another `get()`'s own
result).

| Option | Description | Fit against evidence |
|---|---|---|
| **(A) Full chaining, N levels** | Matches real Firestore's own actual capability | **Rejected for this feature** — zero evidenced need, and the iterative discover-fetch-discover mechanism is a materially larger, riskier build than a single pass; repeats the unevidenced-scope mistake this initiative has consistently avoided |
| **(B) Single level only — every `get()`/`exists()` path is built ONLY from bindings already known before evaluation begins (auth, path variables) — never from another `get()`'s own result** | Directly covers the evidenced role-lookup idiom; path-discovery stays a single pure pass, structurally simple | **Strongest fit** |

**Resolution**: **(B) is locked.** A `get()`/`exists()` operand whose own path template
references another `get()`'s own result (i.e., an operand that would need `.data.<field>` chained
into a path position) is a NAMED, distinguishable rejection (`UNSUPPORTED_EXPRESSION_GRAMMAR` —
reuses 4c's own shared variant, never a new one for a shape this narrowly out-of-scope), not built.
Named, deferred (§ Out of Scope) — a future feature, evidenced need required.

### Resolution 3 — Path template grammar: full expression substitution inside `get()`'s own path argument, or a narrow, evidenced subset?

Real Firestore's own `$(expr)` substitution syntax technically allows ANY expression inside the
parens. This feature's own evidenced domain example needs exactly ONE substitution shape:
`$(request.auth.uid)` (the caller's own uid, building an organization/user-scoped lookup path) —
mirrors `rules_file.rs`'s own `PathSegment` design almost exactly (literal segments interspersed
with ONE substitution kind), a direct structural precedent already proven out twice
(`security-rules-cel-parity`'s own leaf-variable capture, `security-rules-cel-path-matching`'s own
multi-variable ancestor capture).

| Option | Description | Fit against evidence |
|---|---|---|
| **(A) Full expression substitution** (`$(<any expression>)`) | Real Firestore parity | **Rejected** — no domain example needs anything beyond `request.auth.uid`; a general expression-substitution grammar inside a STRING-typed path argument is a structurally novel, higher-risk parser surface (embedding the WHOLE condition grammar recursively inside itself) for zero evidenced payoff |
| **(B) Literal segments + exactly ONE substitution kind, `$(request.auth.uid)`, at any segment position** | Directly covers the evidenced role-lookup idiom; reuses the EXISTING literal/wildcard-segment parsing discipline `rules_file.rs` already has, just for a differently-shaped consumer (a runtime path-BUILDING template, not an import-time path-MATCHING pattern) | **Strongest fit** |

**Resolution**: **(B) is locked.** `$(request.path.<var>)` (a path-variable capture, reusing the
SAME name `request.path.<var>` the rest of this grammar already uses) is ALSO in scope — it composes
for free once `$(request.auth.uid)`'s own substitution mechanism exists (both resolve against
already-known bindings, zero new mechanism), and closes a real gap `security-rules-cel-parity`'s
own leaf-variable feature left open (a path-variable-scoped role lookup, e.g.
`get(/databases/$(database)/documents/expeditions/$(request.path.expeditionId)).data.owner_id`).
Any OTHER substitution shape (a literal document field, an arithmetic expression, a nested `get()`
— Resolution 2) is a NAMED rejection.

### Resolution 4 — Read budget: does this feature enforce an artificial cap on distinct cross-document reads per evaluation?

Real Firestore's own 10/20 limits exist because ITS OWN grammar allows chaining and looping
constructs that could otherwise produce unbounded reads. This feature's own Resolution 2 (single
-level only, no chaining) already makes the PRACTICAL ceiling per condition small and
STATICALLY bounded — it equals the number of distinct `get()`/`exists()` operand instances
literally written in the condition's own source text (typically 1–3), never something that can
grow at RUNTIME the way a chained/looping construct could.

**Resolution**: **no artificial cap is enforced this feature.** Named, not silently assumed:
if a FUTURE feature (chaining, Resolution 2's own deferred scope) reopens this question, that
future feature's own DESIGN must revisit it — a chained construct COULD produce an unbounded read
count, this one structurally cannot. Deduplication (Resolution 1, step 2) is still built —
free, and it directly mirrors real Firestore's own documented caching behavior.

### Resolution 5 — Read+write+Listen parity, or a narrower walking-skeleton-only scope?

Every prior CEL-parity epic locked read+write+Listen-per-event parity within its OWN single
feature (4a/4b/4b′'s own precedent). This feature's own genuinely NEW I/O step (Resolution 1)
changes the RISK calculus that precedent was built on: `Listen`'s own per-event re-check
(`realtime::listen_handler`) fires on EVERY document-change notification, for EVERY active
subscriber whose own rule might reference `get()`/`exists()` — a cross-document read on that path
is a REAL per-event I/O cost multiplier that `GetDocument`'s own single-request cost is not.

**Resolution**: **Read+write parity locked within this feature (mirrors every prior epic's own
precedent, zero new architectural risk beyond Resolution 1's own already-resolved mechanism —
write handlers already fetch the CURRENT document before evaluating, adding a cross-document
fetch is the identical shape, just for a DIFFERENT document). Listen's own per-event re-check
is explicitly OUT OF SCOPE** — unchanged from `security-rules-cel-parity`'s own original
`OQ-CP-04`-adjacent boundary (that boundary was about `PathVariable` specifically, but the SAME
"per-event I/O cost multiplier" caution applies with GREATER force here, since a per-event
cross-document READ is real, uncached-across-events I/O, not a zero-cost value already in scope).
Named, not silently built — a future feature's own evidenced need, if any ever arises.

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

**Signals checked**: >10 user stories? No (5). >3 bounded contexts/modules? Borderline —
`embyr-core` (new pure path-discovery function) + `embyr-server` (new fetch step at each call
site) = 2 crates, but this is the SAME "BC-4 calling into BC-2" dependency-shape change the prior
wave already flagged as its OWN separate risk axis (not a NEW bounded context, but a NEW KIND of
dependency between 2 already-existing ones) — tracked explicitly, not silently absorbed.
Walking skeleton >5 integration points? No (1: `exists()` + `GetDocument`). Estimated effort >2
weeks? No — 5 slices, each ≤1.5 days by slice estimate. Multiple independent user outcomes? No —
`get()`/`exists()` are two halves of one coherent capability (one returns a value, the other a
boolean; both need the identical path-discovery/fetch mechanism), never independently shippable.

**Scope Assessment: PASS** (0-1 signals fired) — right-sized as one feature.

## Wave: DISCUSS / [REF] Journey — Alex's "My Real Role-Lookup Rule Finally Works" Arc

### Mental model

Alex's real Firestore `.rules` file checks an ORGANIZATION-level role before allowing a write —
the single most common real-world Firestore idiom needing cross-document reads. Before this
feature: rejected outright, named `CROSS_DOCUMENT_READ`, no way around it. After: the narrowly
-evidenced subset (single-level, `$(request.auth.uid)`/`$(request.path.<var>)` substitution only)
imports and enforces correctly; anything beyond that subset (chaining, arbitrary expression
substitution) is STILL rejected, but distinguishably, never silently mis-evaluated.

### Failure modes (feeds DISTILL scenario generation)

- The referenced document does not exist: `exists()` must return `false` cleanly (a real
  boolean, participates in `&&`/`||` normally); `get()`'s own `.data.<field>` access on a
  nonexistent document must fail closed (`Deny`), mirroring the SAME `FieldMissing` mechanism
  every other missing-field case already uses — zero new control-flow shape.
- Two DIFFERENT `get()`/`exists()` calls in the SAME condition referencing the SAME path: must
  be deduplicated to ONE real fetch (Resolution 1, step 2) — proven by a real assertion count on
  the underlying adapter call, not merely by the OUTCOME being correct (a correct outcome could
  hide a redundant fetch).
- A candidate condition attempting chaining (Resolution 2) or non-`$(request.auth.uid)`/
  `$(request.path.<var>)` substitution (Resolution 3) must be a NAMED rejection at import time,
  never silently accepted-and-then-denied at evaluation time.

## Wave: DISCUSS / [REF] Story Map & Walking Skeleton

### Backbone

Alex writes/imports a `.rules` file containing a single-level `get()`/`exists()` role-lookup
clause → embyr pre-resolves the referenced path from already-known bindings → embyr fetches the
real document (deduplicated) → embyr evaluates the condition correctly on a real request →
(Release 2) the same mechanism gates writes → Alex simulates a candidate before publishing.

### Walking Skeleton

**Release 1, Slice 01**: a single `exists()` check (the simpler of the two — boolean-only, no
`.data.<field>` chain) gating a real `GetDocument` call, importing correctly and fetching the
real referenced document.

### Release 1 — Cross-Document Reads Work End-to-End on Reads (Slices 01–03, US-01 through US-03)

`exists()` (Walking Skeleton), then `get()`'s own `.data.<field>` access, then the deduplication
proof + nonexistent-document fail-closed proof — all on the read path (`GetDocument`).

### Release 2 — The Same Mechanism Gates Writes (Slices 04–05, US-04 through US-05)

Write-path parity (`CreateDocument`/`UpdateDocument`), then simulation parity.

## Wave: DISCUSS / [REF] Elephant Carpaccio Slices

| # | Story | Release | Estimate | Learning Hypothesis | Reference Class |
|---|---|---|---|---|---|
| 01 | US-01 | 1 (Walking Skeleton) | 1.5 days | Disproves: a two-phase (path-discovery → fetch → evaluate) mechanism cannot be built while keeping `embyr-core` genuinely IO-free, without either loosening the `deny.toml` boundary or duplicating the fetch logic per call site | New mechanism, nearest reference class: `resolve_access_rule_pattern`'s own "one indexed lookup on the hot miss path" precedent (ADR-063), generalized from a fixed lookup to a data-dependent one |
| 02 | US-02 | 1 | 1 day | Disproves: `get()`'s own `.data.<field>` access cannot reuse the EXISTING `FieldMissing` fail-closed mechanism for a nonexistent-document case without a new control-flow shape | Mirrors AC-17-09's own fail-closed precedent, reused unchanged |
| 03 | US-03 | 1 | 1 day | Disproves: two `get()`/`exists()` operand instances referencing the identical concrete path cannot be deduplicated to one real fetch without either a cache keyed wrong or a correctness regression | New primitive (a path→result dedup map), nearest reference class: `resolve_access_rule_pattern`'s own "cheap on the hot path, extended" discipline (ADR-064 § Decision Driver 2) |
| 04 | US-04 | 2 | 1 day | Disproves: write-path parity needs anything beyond the identical mechanical extension every prior epic's own write-parity slice already proved | Mirrors 4c's own Slices 02/06 |
| 05 | US-05 | 2 | 1 day | Disproves: `simulate_access_rule` cannot share the SAME two-phase mechanism real enforcement uses without either a second, independently-maintained fetch path or a synthetic-document escape hatch | New: simulation needs a CALLER-SUPPLIED synthetic document set (mirrors `request_time`'s own synthetic-input precedent), not a real fetch — the FIRST simulation slice in this whole initiative that cannot simply "share `evaluate()` unchanged" (every prior epic's simulation slice needed zero production code) |

## Wave: DISCUSS / [REF] Prioritization

Ordered by learning leverage AND genuine dependency: Slice 01 first (proves the CORE two-phase
mechanism — the highest-uncertainty, highest-consequence question this feature asks); Slice 02
(fail-closed `.data` access) and Slice 03 (dedup) are confirmatory extensions of Slice 01's own
mechanism, low incremental uncertainty; Slice 04 (write parity) mirrors a proven pattern; Slice 05
(simulation) comes LAST because it is, uniquely in this whole initiative, NOT purely confirmatory
— simulation needs its own new synthetic-document-supply mechanism, the highest remaining
uncertainty after Slice 01 itself.

## Wave: DISCUSS / [REF] System Constraints

- `embyr-core` stays zero-IO (`deny.toml`-enforced) — the path-discovery function is pure;
  `evaluate()`'s own new parameter is a pre-fetched, caller-supplied map, identical in kind to
  `request_time`.
- The new BC-4→BC-2 dependency (`resolve_access_rule_pattern`'s own sibling: a NEW
  `fetch_cross_document_reads` step, `embyr-server`-side) is READ-ONLY, non-transactional — the
  SAME shape ADR-029 already established for the CURRENT-document fetch, generalized to an
  ARBITRARY other document, never a write dependency (re-verify explicitly in DESIGN, per
  `security-rules-cel-parity`'s own flagged risk).
- Mutation-testing lesson from `security-rules-cel-expression-grammar`'s own QUALITY_GATE
  (`docs/evolution/2026-09-04-security-rules-cel-expression-grammar.md` § Lessons Learned): every
  new PURE function this feature adds (path-discovery, dedup-map construction) needs its OWN
  `embyr-core`-level unit tests from the start of DELIVER, not deferred to a post-hoc QUALITY_GATE
  fix — acceptance tests alone (however complete) do not substitute for unit-level mutation
  coverage.

## Wave: DISCUSS / [REF] User Stories

### US-01: Alex's Role-Lookup `exists()` Clause Parses and Enforces on Reads (Walking Skeleton)

**job_id**: JOB-17 | **Release**: 1 | **Persona**: P1 Alex

#### Elevator Pitch
Before: `allow read: if exists(/databases/$(database)/documents/organizations/$(request.auth.uid));`
is rejected outright, named `CROSS_DOCUMENT_READ`.
After: Alex imports that exact clause via `POST /admin/v1/projects/:project_id/access_rules`, sees
a 200 with the condition stored verbatim, then a real `GetDocument` call from a signed-in caller
whose own `organizations/<uid>` document exists succeeds; one whose own organization document
does NOT exist is denied.
Decision enabled: Alex confirms his real organization-membership gate now behaves identically to
real Firebase, before any real end user hits it.

#### Acceptance Criteria
- [ ] AC-CDR-01: `exists(/databases/$(database)/documents/<literal segments>/$(request.auth.uid))`
      parses into a new `Operand::CrossDocumentExists` carrying its own resolved path template.
- [ ] AC-CDR-02: a real `GetDocument` request from a caller whose own referenced organization
      document exists succeeds; one whose own referenced document does not exist is denied.
- [ ] AC-CDR-03: `$(request.path.<var>)` substitution (a path-variable capture) is ALSO accepted
      in the path template, resolving against the SAME already-known bindings
      `path_variable_value`/`ancestor_path_variable_values` already carry.
- [ ] AC-CDR-04: a candidate condition using ANY other substitution shape (a literal field, a
      nested `get()`, an arithmetic expression) inside `$(...)` is a NAMED, distinguishable
      rejection, never silently mis-parsed.

### US-02: `get()`'s Own `.data.<field>` Access Fails Closed on a Nonexistent Document

**job_id**: JOB-17 | **Release**: 1 | **Persona**: P1 Alex

#### Elevator Pitch
Before: `get(...)` is equally rejected outright.
After: `allow write: if get(/databases/$(database)/documents/organizations/$(request.auth.uid))
.data.role == "admin";` imports, and a real write from a caller whose organization document's
`role` field is `"admin"` succeeds; one whose own document is missing entirely (or whose `role`
field is absent/mismatched) is denied — the SAME fail-closed mechanism AC-17-09 already proves
for any other missing field, never a new control-flow shape or a crash.
Decision enabled: Alex confirms role-based cross-document authorization — the single most common
real-Firestore idiom needing this capability — works correctly.

#### Acceptance Criteria
- [ ] AC-CDR-05: `get(<path>).data.<field>` parses into a new `Operand::CrossDocumentGet`
      carrying its own resolved path template and the referenced field name.
- [ ] AC-CDR-06: a real read gated by a `get(...).data.<field> == <value>` condition succeeds
      when the referenced document exists with a matching field value.
- [ ] AC-CDR-07: the SAME condition denies (never panics, never a 500) when the referenced
      document does not exist at all — `get()` on a missing document fails closed via the
      existing `FieldMissing` mechanism, exactly as a missing `resource.data.<field>` already does.
- [ ] AC-CDR-08: the same condition denies when the referenced document exists but its own
      field value does not match — an ordinary comparison-mismatch `Deny`, not a special case.

### US-03: Two References to the Same Document Are Fetched Once

**job_id**: JOB-17 | **Release**: 1 | **Persona**: P1 Alex

#### Elevator Pitch
Before: no cross-document reads exist at all, so no dedup question exists either.
After: `allow read: if exists(/databases/$(database)/documents/organizations/$(request.auth.uid))
&& get(/databases/$(database)/documents/organizations/$(request.auth.uid)).data.role == "admin";`
(the SAME path referenced twice) fetches the real organization document exactly ONCE, proven by a
direct call-count assertion on the underlying adapter, not merely by the outcome being correct.
Decision enabled: Alex's rule can freely combine `exists()` and `get()` on the same document
without worrying about a doubled read cost — mirrors real Firestore's own documented caching
behavior.

#### Acceptance Criteria
- [ ] AC-CDR-09: a condition referencing the identical concrete document path via BOTH `exists()`
      and `get()` results in exactly ONE real backend fetch for that path.
- [ ] AC-CDR-10: two DIFFERENT concrete paths (e.g. two different callers' own organization
      documents in a hypothetical `&&`-combined condition) each get their own real fetch —
      dedup is per-PATH, never a blanket single-fetch-per-evaluation cap (Resolution 4).

### US-04: The Same Cross-Document Grammar Gates Writes

**job_id**: JOB-17 | **Release**: 2 | **Persona**: P1 Alex

#### Elevator Pitch
Before: same rejection as US-01/US-02, for the write-path surface.
After: a real `CreateDocument`/`UpdateDocument` call is gated correctly by a cross-document
`exists()`/`get()` condition, mirroring the read-path mechanism exactly.
Decision enabled: full read+write confidence for cross-document role-lookup rules.

#### Acceptance Criteria
- [ ] AC-CDR-11: a real `CreateDocument` call is gated correctly by a cross-document condition.
- [ ] AC-CDR-12: a real `UpdateDocument` call is gated correctly by a cross-document condition.

### US-05: Alex Simulates a Cross-Document Candidate Rule Before Publishing It

**job_id**: JOB-17 | **Release**: 2 | **Persona**: P1 Alex

#### Elevator Pitch
Before: no way to test a cross-document candidate condition without publishing it live (and
without a REAL other document already existing to reference).
After: Alex calls `simulate_access_rule` with a candidate cross-document condition and a
caller-supplied SYNTHETIC set of referenced documents (mirrors `resource`/`request_resource`'s own
existing synthetic-input discipline) — sees the resolved allow/deny outcome, with zero real fetch
issued.
Decision enabled: Alex catches a role-lookup mistake during his own testing, without needing a
real second document to exist first.

#### Acceptance Criteria
- [ ] AC-CDR-13: `simulate_access_rule` accepts a new synthetic `cross_document_reads` map
      (concrete path → synthetic field values, reusing the EXISTING `json_value_to_field_value`
      translation) and resolves `get()`/`exists()` operands against it — zero real backend fetch.
- [ ] AC-CDR-14: a path referenced by the candidate condition but ABSENT from the synthetic map
      simulates identically to a real nonexistent document (`exists()` → false; `get()`'s own
      `.data` access → fails closed).

## Wave: DISCUSS / [REF] Outcome KPIs

### Feature: security-rules-cel-cross-document-reads

### Objective
Close the single remaining named CEL-parity capability gap that requires I/O — the canonical
real-Firestore organization/role-membership lookup idiom.

### Outcome KPIs
| KPI | Target | Measurement |
|---|---|---|
| Real `.rules`-file constructs newly accepted | 1 idiom family (single-level role/membership lookup via `exists()`/`get()`) | Direct: this feature's own acceptance-test suite proves it imports and enforces correctly |
| Regression | 0 | Full `security_rules_*` baseline re-run clean after every slice |
| Cross-document fetch dedup | 100% (zero duplicate fetches for an identical path within one evaluation) | Direct adapter call-count assertion, US-03 |
| Mutation-testing kill rate on new `embyr-core` logic | >= 80% (per project CLAUDE.md), unit tests written DURING DELIVER, not deferred | `cargo-mutants`, `--in-place`, module-filtered |

## Wave: DISCUSS / [REF] Out of Scope

- **Chained/nested cross-document reads** (a `get()`'s own result feeding another `get()`'s own
  path) — zero domain evidence (Resolution 2). Named, deferred, no candidate feature id assigned.
- **Full expression substitution inside `$(...)`** — only `request.auth.uid`/`request.path.<var>`
  are supported (Resolution 3). Named, deferred.
- **An artificial read-budget cap** — not needed given Resolution 2's own structural bound
  (Resolution 4); revisit if/when chaining is ever built.
- **`RunQuery`/Listen enforcement under cross-document conditions** — read+write only this
  feature (Resolution 5); Listen's own per-event re-check is explicitly deferred given the real
  per-event I/O cost multiplier a cross-document read would introduce.
- **`OQ-CEG-01`** (nested map-field traversal on `resource.data`/`request.resource.data`) —
  unchanged, carried forward from 4c, not this feature's own concern.
- **Custom `function` definitions and invocation** — unchanged, candidate id
  `security-rules-cel-functions`, "Epic 4e." **Next after this feature.**
- **Re-opening any part of 4a/4b/4b′/4c's own already-shipped scope** — done, out of bounds.

## Wave: DISCUSS / [REF] WS Strategy

**Strategy A** (real, minimal, end-to-end) — Slice 01 is a real `exists()` check + a real fetch of
a real `organizations/<uid>` document + a real `GetDocument` enforcement proof, not a mock.

## Wave: DISCUSS / [REF] Driving Ports

Admin HTTP `:9090` (existing `define_access_rule`/`define_write_access_rule`/`simulate_access_rule`
routes, condition TEXT unchanged in shape) + gRPC `:8080` (`GetDocument`, `CreateDocument`,
`UpdateDocument` — existing routes, zero new RPCs). No new admin route, no new RPC.

## Wave: DISCUSS / [REF] Pre-requisites

- `security-rules-cel-parity` (4a), `security-rules-cel-path-matching` (4b),
  `security-rules-cel-recursive-wildcards` (4b′), `security-rules-cel-expression-grammar` (4c) —
  all finalized, provide the `Operand`/`Condition`/`evaluate()`/parser architecture this feature
  extends, and the `FieldValue`/fail-closed (`FieldMissing`) mechanism this feature reuses
  unchanged for a missing cross-document field.
- No new external dependency, no new bounded context (BC-4 already exists) — a NEW kind of
  dependency EDGE from BC-4 to BC-2 (on-demand, arbitrary-path, vs. the current single-known-doc
  shape), tracked explicitly in § System Constraints.

## Wave: DISCUSS / [REF] Handoff Package

Handed to `nw-solution-architect` (DESIGN): this feature-delta.md, all 5 Resolutions (especially
Resolution 1's own two-phase mechanism design and Resolution 5's own read+write-only,
Listen-excluded boundary), and the explicit instruction to re-verify the BC-4→BC-2 dependency
stays read-only/non-transactional (the risk `security-rules-cel-parity`'s own DISCUSS originally
flagged) as part of DESIGN's own architecture design, not merely asserted here.

## Wave: DISCUSS / [REF] SSOT Updates

`docs/product/jobs.yaml`, JOB-17 entry: append a new dated NOTE (mirroring 4b′'s/4c's own
identical NOTE-append convention) — "JOB-17 now also covers a narrowly-scoped, single-level
cross-document `get()`/`exists()` role-lookup idiom in a real Firestore `.rules` file, same job,
same persona, not a new job (12th realization). The FIRST realization of this job requiring
`evaluate()` itself to gain a genuinely new I/O-adjacent capability (pre-resolved, caller-supplied
cross-document reads), never I/O inside `embyr-core` itself."

## Wave: DISCUSS / [REF] Definition of Ready Validation

### Requirements Completeness Score: **0.96**

### DoR Checklist (9-item hard gate)
1. [x] Every story traces to a job_id (JOB-17, all 5 stories)
2. [x] Every story has a complete Elevator Pitch (Before/After/Decision enabled)
3. [x] Every AC is testable without ambiguity
4. [x] Walking Skeleton identified (US-01)
5. [x] Scope Assessment passed
6. [x] No slice contains only `@infrastructure` stories (every slice has a direct Alex-facing
   value story)
7. [x] Out of Scope explicitly named (6 items)
8. [x] Outcome KPIs have numeric targets and measurement methods
9. [x] Prior-wave artifacts read and reconciled (Resolution 1 confirms, does not contradict, the
   prior wave's own already-recorded finding)

### DoR Status: **PASSED**

## Wave: DISCUSS / [REF] Open Questions

None carried forward unresolved — every open question this DISCUSS raised (chaining, substitution
grammar, read budget, Listen scope) was independently resolved with a locked Resolution above,
unlike `security-rules-cel-recursive-wildcards`'s own `OQ-RW-01` (which needed DESIGN/DISTILL
follow-up); this DISCUSS's own live web verification (§ Reading Confirmation) resolved the
equivalent uncertainty in-wave instead.

## Wave: DISCUSS / [REF] Wave Decisions Summary

### Key Decisions
- [D1] Two-phase evaluation (path-discovery → fetch → evaluate), `evaluate()` gains an 8th
  parameter — the ONLY mechanism compatible with `embyr-core`'s own IO-forbidden boundary,
  confirming (not rediscovering) `security-rules-cel-parity`'s own prior finding.
- [D2] Single-level only, no chaining (Resolution 2) — the practical read count per condition is
  therefore statically bounded by the condition's own source text, never runtime-unbounded.
- [D3] `$(request.auth.uid)`/`$(request.path.<var>)` substitution only inside a `get()`/`exists()`
  path template (Resolution 3) — a strict, evidenced subset of real Firestore's own general
  expression-substitution grammar.
- [D4] No artificial read-budget cap (Resolution 4) — Decision 2's own structural bound already
  makes one unnecessary; revisit only if chaining is ever built.
- [D5] Read+write parity locked; Listen's own per-event re-check explicitly OUT of scope
  (Resolution 5) — a real per-event I/O cost multiplier a cross-document read would introduce,
  a materially different risk profile from every prior "just thread an already-known value"
  parity extension this initiative has made.

### Requirements Summary
- Primary need: real customer `.rules` files still get rejected on the single most common
  cross-document idiom (organization/role-membership lookup).
- Walking skeleton scope: a single `exists()` check, real end-to-end enforcement.
- Feature type: Backend.

### Constraints Established
- `embyr-core` stays zero-IO — the path-discovery function is pure, `evaluate()`'s new parameter
  is caller-supplied and pre-fetched, mirroring `request_time`'s own precedent exactly.
- The new BC-4→BC-2 dependency edge is read-only/non-transactional, re-verified (not merely
  asserted) in DESIGN.
- No artificial read-budget cap needed given the single-level-only structural bound.

### Upstream Changes
- None — no DISCOVER assumptions from a prior feature are contradicted; this feature is a named,
  evidenced continuation of `security-rules-cel-parity`'s own original Out-of-Scope entry, and its
  own central architectural question was already correctly flagged (not merely guessed at) by that
  prior wave.

## Wave: DISCUSS / [REF] Next Wave

**Handoff To**: nw-solution-architect (DESIGN wave)
**Deliverables**: this feature-delta.md, 5 locked Resolutions, 5-slice/2-release plan
