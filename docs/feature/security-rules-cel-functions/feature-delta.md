# Feature Delta: security-rules-cel-functions

## Wave: DISCUSS / [REF] Prior Wave Consultation — Reading Confirmation

✓ `docs/product/jobs.yaml` — JOB-17 (`document-access-control`) read in full, including every NOTE
appended by 4a/4b/4b′/4c/4d. Same job, same persona (P1 Alex) — no new job created.
✓ `docs/feature/security-rules-cel-parity/feature-delta.md` — this feature's own naming origin:
§ Scope Assessment "4e — candidate `security-rules-cel-functions`: Custom `function` definitions
and invocation. Composes over 4b/4c's own grammar surface once they exist; lowest-risk of the four
deferred epics" and § Out of Scope's identical entry. Both 4b (`security-rules-cel-path-matching`)
and 4c (`security-rules-cel-expression-grammar`) are now FINALIZED, so this feature's own
prerequisite is satisfied.
✓ `docs/feature/security-rules-cel-cross-document-reads/feature-delta.md` § Out of Scope — carries
this feature's own naming forward unchanged: "Custom `function` definitions and invocation —
unchanged, candidate id `security-rules-cel-functions`, 'Epic 4e.' Next after this feature."
✓ `crates/embyr-core/src/access_control/mod.rs`, `detect_unsupported_construct` (line 346) — direct
re-read: **still unconditionally rejects any identifier immediately followed by `(` as
`UnsupportedConstruct::CustomFunction`, except the 3 exempted names `get`/`exists`/
`duration.value`** (4d's/4c's own precedent, unchanged by this feature). This is load-bearing for
this feature's own central architectural finding below (§ Resolution 1) — it means ANY call-shaped
identifier this feature's own mechanism fails to fully resolve before `parse_condition` runs is
caught, fail-closed, by an EXISTING, unmodified rejection — not a new hazard this feature
introduces, a safety net this feature's own design leans on deliberately.
✓ `crates/embyr-core/src/access_control/rules_file.rs` — full re-read of the outer-syntax parser:
`parse_rules_file` (validates the fixed `service cloud.firestore { match
/databases/{database}/documents { ... } }` shell, fails fast on the first structural problem),
`parse_match_blocks`/`parse_nested_match_blocks` (repeatedly scans for `match /<pattern> { ... }`
blocks via `find_matching_close`'s own brace-depth-counting helper), `parse_allow_clauses` (per
match-block: splits on `;`, extracts each `allow <verbs>: if <condition>;` clause's own RAW
condition TEXT, already threading `path_pattern` through for `OffendingBlock` naming). Confirms
directly: `MatchBlock.allow_clauses: Vec<(Vec<Verb>, String)>` stores condition TEXT, not a parsed
AST — the LAST point in the pipeline where a pure TEXT transform can intercept a condition before
`decompose()`/`parse_condition` ever see it.
✓ `crates/embyr-core/src/access_control/rules_file.rs`, `decompose`/`decompose_block`/
`DecomposedTarget` — confirms every DOWNSTREAM consumer (write-rule/read-rule/pattern decomposition,
`embyr-server`'s own upsert calls) operates on the condition STRING `MatchBlock` already carries —
if that string is already fully self-contained (no function-call syntax remaining) by the time
`MatchBlock` is built, **zero code downstream of `parse_allow_clauses` needs to know this feature
exists.**

**Live web verification** (Firebase's own official docs, fetched directly — grounding this
feature's own locked v1 scope in REAL semantics, not recollection, mirroring 4c's/4d's own
practice):
- `firebase.google.com/docs/rules/rules-language` (fetched directly): real syntax —
  `function <name>(<params>) { return <expr>; }`, declared as a SIBLING of `match` blocks inside
  the `service cloud.firestore { ... }` scope (never nested inside a `match` block). A function
  body may contain up to 10 `let` bindings before its single terminal `return` — no loops, no
  multiple returns, no external calls. Functions take zero or more POSITIONAL parameters; critically,
  **a function's own body ALSO sees `request`/`resource` from its defining (service-level) scope
  ambiently, regardless of its own parameter list** — parameters supplement, never replace, the
  ambient bindings.
- `firebase.google.com/docs/firestore/security/rules-conditions` (fetched directly): a function MAY
  call another already-defined function (nesting), but **may NOT recurse, directly or indirectly**;
  real Firestore's own compiler caps call-stack depth at 20.

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

- Feature type: **Backend** (Decision 1) — extends `rules_file.rs`'s own outer-syntax IMPORT-TIME
  parser only; zero new admin route, zero new RPC, zero change to any already-shipped runtime
  evaluation call site.
- JTBD: **reuse JOB-17** (Decision 4 = "Yes", existing job) — the 13th realization of the SAME job.
  A NOTE is appended to `jobs.yaml`'s JOB-17 entry (§ SSOT Updates).
- Walking Skeleton: **Yes** (Decision 2) — a single zero-parameter function definition, called once
  from one `match` block, real `GetDocument` enforcement proving the expanded condition evaluates
  correctly.
- UX Research Depth: **Lightweight** (Decision 3) — a backend import-time parsing extension, one
  persona (Alex, fully profiled across 7 prior epics), no new emotional arc.

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: P1 Alex (SDK Developer), unchanged.

**Job**: JOB-17 `document-access-control`, unchanged job_story. This feature's own realization:
Alex's real Firestore `.rules` file factors a repeated boolean check into a named helper —
`function isEditor() { return request.auth.uid == resource.data.editor_id; }` — and calls it from
one or more `match` blocks (`allow write: if isEditor();`). Today, EVERY `function` declaration and
EVERY call to an undeclared-looking name is rejected outright at import time, named
`CUSTOM_FUNCTION`/`SYNTAX_ERROR`, unconditionally. After this feature, a narrowly-scoped, evidenced
subset — zero-parameter functions, no nesting — imports and enforces correctly.

## Wave: DISCUSS / [REF] Job Discovery — Framing Resolution

### Resolution 1 (THE central architectural question) — Where does a function CALL get resolved:
a new runtime concept inside `evaluate()` (`embyr-core`), or a pure IMPORT-TIME text transform
(`rules_file.rs`) that never reaches `evaluate()` at all?

| Option | Description | Fit against evidence |
|---|---|---|
| **(A) New runtime concept** — `Operand`/`Condition` gain a `FunctionCall` variant; `evaluate()` resolves it against a caller-supplied function-definition map, mirroring `cross_document_reads`' own "pre-resolve, thread as a parameter" shape | Real-Firestore-accurate in principle | **Rejected** — every prior CEL-parity epic's own new capability needed a genuinely NEW piece of information at EVALUATION time (a fetched document, a wall-clock read). A function call needs NOTHING new at evaluation time — its own body, once resolved, is ordinary, already-supported condition grammar over the SAME `resource`/`request` bindings the calling clause already has. Building a runtime concept for a purely SYNTACTIC substitution is unjustified complexity: a new `Operand` variant, a new `evaluate()` parameter, new wiring at all 18 call sites (mirroring 4c's/4d's own rollout) — for a capability that resolves to nothing MORE than "different text, same grammar." |
| **(B) Pure import-time text transform** — `rules_file.rs`'s own `parse_allow_clauses` expands each `<function_name>()` call site into `(<function's own body text>)`, spliced in BEFORE the condition string is ever handed to `parse_condition`. `decompose()`, `evaluate()`, `embyr-server`'s every call site: **unchanged, zero lines touched.** | Directly covers the evidenced idiom; the STORED `condition_source` is already fully self-contained — read/write/simulation parity is free, not built | **Strongest fit** |

**Resolution**: **(B) is locked.** This is the lowest-risk mechanism of any CEL-parity epic in this
initiative (confirming, not merely repeating, `security-rules-cel-parity`'s own original "lowest
-risk of the four deferred epics" framing) — it touches exactly one file
(`crates/embyr-core/src/access_control/rules_file.rs`), adds zero new `Operand`/`Condition`
variant, zero new `evaluate()` parameter, and needs zero per-call-site rollout across
`embyr-server` (unlike EVERY prior CEL-parity epic). Read enforcement, write enforcement, and
simulation all work correctly the moment a function-call-bearing `.rules` file imports
successfully — the expanded text is indistinguishable, to every downstream consumer, from a
hand-authored condition that never used a function at all.

**A structural corollary, confirmed by Reading Confirmation, not incidental**:
`detect_unsupported_construct`'s own EXISTING, unmodified rejection of any un-exempted
call-shaped identifier is this design's own safety net — a function-call expansion bug that leaves
`someName(` unresolved in the final condition string is caught, fail-closed, as a plain
`CUSTOM_FUNCTION` rejection by code this feature does not touch, never silently mis-evaluated.

### Resolution 2 — Parameters: does a v1 function accept arguments?

Real Firestore functions accept zero or more POSITIONAL parameters (§ Reading Confirmation). This
feature's own single evidenced domain example (`isEditor()`) takes none — the ambient
`request`/`resource` bindings already carry everything a Trailmark trail-guide-editorship check
needs.

| Option | Description | Fit against evidence |
|---|---|---|
| **(A) Full positional-parameter support** | Real Firestore parity | **Rejected for v1** — zero domain evidence; the substitution mechanism widens materially (each parameter name must be identifier-substituted with its own call-site argument TEXT, itself needing the same quote-aware scanning discipline as the call-site expansion — a second, coupled text-substitution pass) for a capability nothing in this initiative's own accumulated text has ever needed |
| **(B) Zero-parameter functions only — `function <name>() { return <expr>; }`, called as `<name>()`** | Directly covers the evidenced idiom; the substitution mechanism is the simplest possible shape (find `name()`, splice in the body text verbatim, no argument-substitution pass at all) | **Strongest fit** |

**Resolution**: **(B) is locked.** A function DEFINITION with a non-empty parameter list, or a
CALL site with non-empty argument text, is a NAMED rejection at import time (`construct`:
`FUNCTION_PARAMETERS_UNSUPPORTED`), never silently accepted-and-mis-expanded. Named, deferred
(§ Out of Scope) — parameterized functions are real, evidenced-by-Firestore's-own-docs capability,
just not evidenced by any Trailmark domain example yet.

### Resolution 3 — Nesting: may a function's own body call another function?

Real Firestore explicitly permits function-to-function calls (never recursion). This feature's own
Resolution 1 design (§ text substitution) COULD support one level of nesting by expanding a
function's own body against the SAME functions map before storing it — but this makes the
expansion pass iterative (function bodies themselves need their own expansion, with a cycle/depth
guard to prevent runaway or mutually-recursive definitions), a materially more complex mechanism
than a single linear pass over each `match` block's own condition text.

| Option | Description | Fit against evidence |
|---|---|---|
| **(A) Multi-level nesting, non-recursive, depth-capped** | Real Firestore parity (own 20-deep call-stack cap) | **Rejected for v1** — zero domain evidence for a SECOND function ever calling a FIRST; adds a cycle-detection/depth-limiting mechanism this feature's own single evidenced idiom does not need |
| **(B) Flat only — a function body is validated via the UNMODIFIED `parse_condition` (never itself expanded against the functions map)** | A function body containing ANY call-shaped identifier is rejected the SAME way a hand-authored condition already is (`detect_unsupported_construct`'s own existing, unmodified rejection) — recursion and nesting are BOTH impossible by construction, zero new detection code | **Strongest fit** |

**Resolution**: **(B) is locked.** A function's own body is parsed via the plain, unmodified
`parse_condition` at DEFINITION time (to fail fast on a malformed body) — this is the SAME
mechanism that structurally forbids nesting AND recursion, for free, as a direct consequence of
Resolution 1's own design rather than a separately-built safeguard. Named, deferred
(§ Out of Scope) — nesting is real-Firestore-accurate, just not evidenced.

### Resolution 4 — `let` bindings inside a function body?

Real Firestore permits up to 10 `let` bindings before the terminal `return`. Zero domain evidence
needs even one.

**Resolution**: **out of scope, not built.** A function body is exactly `return <expr>;` — anything
else (including `let`) is a named rejection at definition time. Real-Firestore-accurate as a
DEFERRED capability, not as a v1 gap silently ignored.

### Resolution 5 — Read+write+simulation parity, or a narrower walking-skeleton-only scope?

Unlike EVERY prior CEL-parity epic (4a/4b/4b′/4c/4d), Resolution 1's own design makes this question
almost moot: the expanded condition text is stored identically regardless of WHICH surface
(`GetDocument`, `CreateDocument`/`UpdateDocument`, `simulate_access_rule`) later reads it — there is
no per-surface wiring decision to make, because there is no new runtime concept for any surface to
wire.

**Resolution**: **full parity is a structural consequence of Resolution 1, not a separately-built
slice.** Confirmed, not merely assumed: Slices 02/03 (§ Elephant Carpaccio Slices) exist to PROVE
this by direct test, mirroring 4c's own Slices 02/03 "confirmatory, zero production code" precedent
— but unlike 4c (where confirmation still required each new operand type to already be wired into
`evaluate()`'s existing rollout), this feature's own confirmation requires literally zero
production code across BOTH slices, the strongest form of this precedent yet.

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

**Signals checked**: >10 user stories? No (4). >3 bounded contexts/modules? No — ONE file
(`rules_file.rs`), zero new crates, zero new bounded context, zero new dependency edge (contrast
4d's own "BC-4 now calls into BC-2" new edge — this feature adds no edge at all). Walking skeleton
>5 integration points? No (1: import → real `GetDocument`). Estimated effort >2 weeks? No — 4
slices, each ≤1 day, two of which are pure confirmation. Multiple independent user outcomes? No —
function definition and function invocation are two halves of one coherent capability, never
independently shippable.

**Scope Assessment: PASS** (0 signals fired) — right-sized as one feature, the smallest of the 3
remaining CEL-parity epics by every measured signal.

## Wave: DISCUSS / [REF] Journey — Alex's "One Named Helper, Reused Everywhere" Arc

### Mental model

Alex's real `.rules` file repeats the SAME ownership check (`request.auth.uid ==
resource.data.editor_id`) across several `match` blocks. Before this feature: factoring it into a
named `function isEditor() { ... }` is rejected outright at import time. After: the function
imports, every call site expands correctly, and the resulting enforcement is IDENTICAL to what
Alex would get by hand-copying the check into each block — this feature changes AUTHORING
ergonomics only, never enforcement semantics.

### Failure modes (feeds DISTILL scenario generation)

- A function call references a name that was never defined anywhere in the file: named rejection,
  distinguishable from a plain syntax error.
- Two functions share the same name: named rejection at definition time (which one would even be
  called is undecidable, never silently "last one wins").
- A function is called WITH arguments, or defined WITH parameters: named rejection (Resolution 2).
- A function's own body itself calls another function (or itself): rejected the SAME way a
  hand-authored condition calling an undefined-looking name already is (Resolution 3) — no special
  "nesting detected" error, just the pre-existing `CUSTOM_FUNCTION` rejection, reused unchanged.
- A function is defined but never called anywhere: imports successfully, zero effect — real
  Firestore itself permits unused functions; no lint this feature does not need to build.

## Wave: DISCUSS / [REF] Story Map & Walking Skeleton

### Backbone

Alex imports a `.rules` file containing a zero-parameter `function` definition and one `match`
block calling it → embyr expands the call site into the function's own body text at import time →
embyr stores the fully-expanded condition exactly like any hand-authored one → real enforcement
(read, then write, then simulation) proves the expansion is correct, with zero new production code
required for the latter two.

### Walking Skeleton

**Release 1, Slice 01**: one function, one call site, real `GetDocument` enforcement proving both
the allow and deny paths of the expanded condition.

### Release 1 — The Mechanism Works End-to-End, Proven on Every Surface (Slices 01–03, US-01 through US-03)

Walking Skeleton (Slice 01), then two purely confirmatory slices proving write-path (Slice 02) and
simulation (Slice 03) parity — both, per Resolution 5, need zero production code.

### Release 2 — Multiple Functions, Multiple Call Sites (Slice 04, US-04)

A file with more than one function definition, called from more than one `match` block (including
the SAME function called from two DIFFERENT blocks) — proves the mechanism generalizes beyond the
single-function walking skeleton, and locks in the named-rejection behavior for every scoped-out
construct (Resolutions 2–4) in one slice.

## Wave: DISCUSS / [REF] Elephant Carpaccio Slices

| # | Story | Release | Estimate | Learning Hypothesis | Reference Class |
|---|---|---|---|---|---|
| 01 | US-01 | 1 (Walking Skeleton) | 1 day | Disproves: a pure import-time text-substitution mechanism cannot correctly splice a function's own body into a call site's condition text without either corrupting a string literal or breaking operator precedence | New mechanism (text-level AST-free substitution), nearest reference class: `rewrite_path_variable`'s own existing text-rewrite precedent in the SAME file, widened from a single-token rewrite to a whole-expression splice |
| 02 | US-02 | 1 | 0.5 day (confirmatory) | Disproves: write-path enforcement needs ANY production code change beyond Slice 01's own import-time expansion | Mirrors 4c's own Slices 02/03 "confirmatory, zero production code" precedent, the strongest form yet (§ Resolution 5) |
| 03 | US-03 | 1 | 0.5 day (confirmatory) | Disproves: `simulate_access_rule` needs ANY production code change beyond Slice 01's own import-time expansion | Same reference class as Slice 02 |
| 04 | US-04 | 2 | 1 day | Disproves: the substitution mechanism only works for the single-function walking-skeleton shape and cannot generalize to multiple definitions/call sites, or cannot correctly and distinguishably reject every scoped-out construct (parameters, nesting, undefined/duplicate names) in one pass | Mirrors `decompose`'s own existing "collect every offending block, never fail on just the first" discipline, applied here to definition-time validation |

## Wave: DISCUSS / [REF] Prioritization

Ordered by learning leverage AND genuine dependency: Slice 01 first (proves the CORE substitution
mechanism — the only genuinely new code this feature writes); Slices 02–03 are pure confirmation,
essentially free once Slice 01 lands (the fully-expanded condition text is ALREADY correct for
every surface); Slice 04 comes last because it is the only slice touching the FULL breadth of the
scoped-out-construct rejection surface (parameters, nesting, duplicates, undefined names) —
highest remaining uncertainty after Slice 01 itself, lowest urgency (the walking skeleton already
proves the mechanism works for the evidenced single-function case).

## Wave: DISCUSS / [REF] System Constraints

- `embyr-core`'s `access_control/mod.rs` (`Operand`/`Condition`/`evaluate()`) is NOT touched by this
  feature at all — confirmed as a locked constraint by Resolution 1, not merely a hoped-for outcome.
  `deny.toml`'s IO-forbidden boundary is trivially satisfied (this feature performs zero I/O of any
  kind, at import time or evaluation time).
- The substitution mechanism must be quote-aware (never substitute inside a `"..."` string literal
  or `'...'` single-quoted unit) — mirrors the tokenizer's own existing quote-handling discipline
  (`'"' =>` / `'\'' =>` branches, `tokenize`), reused as a DESIGN PRINCIPLE, not literally shared
  code (this feature's own scanner operates on raw pre-tokenized text, a structurally different
  layer).
- Mutation-testing lesson from `security-rules-cel-cross-document-reads`'s own QUALITY_GATE
  (`docs/evolution/2026-09-04-security-rules-cel-cross-document-reads.md` § Lessons Learned): even
  disciplined during-slice unit testing leaves narrow boundary/exemption gaps — write unit tests for
  every new pure function (the substitution scanner, the function-block outer-syntax parser) from
  the start of DELIVER, and budget for a dedicated `cargo-mutants --in-diff` pass regardless.

## Wave: DISCUSS / [REF] User Stories

### US-01: Alex's Named Helper Function Parses and Enforces on Reads (Walking Skeleton)

**job_id**: JOB-17 | **Release**: 1 | **Persona**: P1 Alex

#### Elevator Pitch
Before: `function isEditor() { return request.auth.uid == resource.data.editor_id; }` combined with
`allow read: if isEditor();` is rejected outright — the function declaration itself is unrecognized
outer-shell content, and the bare call `isEditor()` is separately rejected as `CUSTOM_FUNCTION`.
After: Alex imports that exact file via `POST /admin/v1/projects/:project_id/access_rules/import`,
sees a 200, and a real `GetDocument` call from the document's own editor succeeds; one from a
different signed-in caller is denied.
Decision enabled: Alex factors a repeated ownership check into one named, reusable helper, exactly
as he would in real Firebase.

#### Acceptance Criteria
- [ ] AC-CF-01: a `function <name>() { return <expr>; }` block, declared as a sibling of the
      top-level `match /databases/{database}/documents { ... }` block, parses into a name → body
      -text mapping.
- [ ] AC-CF-02: a call site `<name>()` inside an `allow` clause's condition expands, before
      `parse_condition` ever runs, into the function's own body text wrapped in parens — preserving
      surrounding operator precedence (e.g. `!isEditor()`, `isEditor() && <other>`).
- [ ] AC-CF-03: a real `GetDocument` call gated by the expanded condition allows the document's own
      editor (per the walking-skeleton domain example) and denies a different signed-in caller.
- [ ] AC-CF-04: a call to a name that matches no defined function anywhere in the file is a NAMED
      rejection (`UNDEFINED_FUNCTION`), distinguishable from a plain syntax error.

### US-02: The Same Expanded Condition Gates Writes — Zero New Production Code

**job_id**: JOB-17 | **Release**: 1 | **Persona**: P1 Alex

#### Elevator Pitch
Before: same rejection as US-01, for the write-path surface.
After: a real `CreateDocument`/`UpdateDocument` call is gated correctly by the SAME expanded
condition — proven without any production code change beyond Slice 01's own import-time expansion.
Decision enabled: full read+write confidence for function-authored rules, at zero extra
implementation cost.

#### Acceptance Criteria
- [ ] AC-CF-05: a real `CreateDocument`/`UpdateDocument` call is gated correctly by a
      function-call-bearing condition, using the identical import from Slice 01 — no new handler
      wiring.

### US-03: Alex Simulates a Function-Call-Bearing Candidate Rule

**job_id**: JOB-17 | **Release**: 1 | **Persona**: P1 Alex

#### Elevator Pitch
Before: same rejection as US-01/US-02, for `simulate_access_rule`.
After: Alex simulates the ALREADY-STORED, already-expanded condition (imported via Slice 01) —
`simulate_access_rule` behaves identically to how it already does for any hand-authored condition,
since the stored text carries no trace of the function-call syntax that produced it.
Decision enabled: Alex tests a function-authored rule before publishing, exactly like any other.

#### Acceptance Criteria
- [ ] AC-CF-06: `simulate_access_rule`, given the stored (already-expanded) condition from a
      function-authored rule, produces the correct allow/deny outcome — zero new
      `simulate_access_rule` code path.

### US-04: Multiple Functions, Multiple Call Sites, Every Scoped-Out Construct Named

**job_id**: JOB-17 | **Release**: 2 | **Persona**: P1 Alex

#### Elevator Pitch
Before: no functions exist at all, so none of these questions are reachable.
After: a file with 2+ function definitions, one function called from 2 different `match` blocks,
and — separately — a file exercising each scoped-out construct (a parameterized function/call,
duplicate function names, a function whose body calls another function) each produce their own
distinguishable, named rejection.
Decision enabled: Alex trusts the mechanism generalizes beyond a toy single-function example, and
trusts every rejection message tells him exactly what to fix.

#### Acceptance Criteria
- [ ] AC-CF-07: a file with 2 distinct function definitions, each called from a different `match`
      block, imports and enforces both correctly.
- [ ] AC-CF-08: the SAME function called from 2 different `match` blocks expands correctly at both
      call sites independently.
- [ ] AC-CF-09: a function definition or call site with a non-empty parameter/argument list is a
      NAMED rejection (`FUNCTION_PARAMETERS_UNSUPPORTED`).
- [ ] AC-CF-10: two function definitions sharing the same name is a NAMED rejection
      (`DUPLICATE_FUNCTION`).
- [ ] AC-CF-11: a function whose own body calls another function (nesting) is a NAMED rejection —
      reusing the pre-existing `CUSTOM_FUNCTION` construct tag unchanged (Resolution 3's own "no
      new detection code" guarantee, proven directly here).

## Wave: DISCUSS / [REF] Outcome KPIs

### Feature: security-rules-cel-functions

### Objective
Close the LAST remaining named CEL-parity capability gap — reusable, named boolean helpers in a
real `.rules` file — completing the 3-epic sequence `security-rules-cel-parity`'s own original
DISCUSS split named (4c, 4d, 4e).

### Outcome KPIs
| KPI | Target | Measurement |
|---|---|---|
| Real `.rules`-file constructs newly accepted | 1 idiom family (zero-parameter, non-nested named helper functions) | Direct: this feature's own acceptance-test suite proves it imports and enforces correctly |
| Regression | 0 | Full `security_rules_*` baseline re-run clean after every slice |
| Production code needed for write/simulation parity beyond Slice 01 | 0 lines (Resolution 5) | Slices 02–03 add zero non-test files |
| Mutation-testing kill rate on new `embyr-core` logic | 100% effective (per this initiative's own established bar), unit tests written DURING DELIVER | `cargo-mutants --in-diff` against this feature's own diff |

## Wave: DISCUSS / [REF] Out of Scope

- **Parameterized functions** — real-Firestore-accurate, zero domain evidence (Resolution 2). Named,
  deferred, no candidate feature id assigned.
- **Function nesting (a function calling another function)** — real-Firestore-accurate, zero domain
  evidence (Resolution 3). Named, deferred.
- **`let` bindings inside a function body** — real-Firestore-accurate, zero domain evidence
  (Resolution 4). Named, deferred.
- **Recursion** — real Firestore itself forbids this; not a v1 gap, a permanent non-goal matching
  real Firestore's own semantics.
- **`OQ-CEG-01`** (nested map-field traversal), chained/nested cross-document reads, an artificial
  read-budget cap, full expression substitution inside `$(...)` — all unchanged, carried forward
  from 4c/4d, not this feature's own concern.
- **Re-opening any part of 4a/4b/4b′/4c/4d's own already-shipped scope** — done, out of bounds.

## Wave: DISCUSS / [REF] WS Strategy

**Strategy A** (real, minimal, end-to-end) — Slice 01 is a real function import + a real
`GetDocument` enforcement proof against a real seeded document, not a mock.

## Wave: DISCUSS / [REF] Driving Ports

Admin HTTP `:9090` (existing `POST .../access_rules/import` route, `.rules`-file TEXT unchanged in
its OWN outer shape — this feature widens what text inside it is accepted) + gRPC `:8080`
(`GetDocument`, `CreateDocument`, `UpdateDocument` — existing routes, zero new RPCs, zero new
handler code per Resolution 1/5). No new admin route, no new RPC.

## Wave: DISCUSS / [REF] Pre-requisites

- `security-rules-cel-path-matching` (4b) — provides the `match`-block-decomposition pipeline
  (`parse_rules_file`/`parse_match_blocks`/`decompose`) this feature extends.
- `security-rules-cel-expression-grammar` (4c) — provides the richer condition grammar a real
  function body typically needs (though this feature's own evidenced example needs only the
  ownership-equality shape already proven by `security-rules-cel-parity`'s own original walking
  skeleton).
- No new external dependency, no new bounded context, no new dependency edge between any two
  existing ones (contrast 4d's own new BC-4→BC-2 edge) — the smallest architectural footprint of
  any CEL-parity epic in this initiative.

## Wave: DISCUSS / [REF] Handoff Package

Handed to `nw-solution-architect` (DESIGN): this feature-delta.md, all 5 Resolutions (especially
Resolution 1's own "why a runtime concept is unjustified complexity here" finding — the central
thing that makes this feature different in kind from every prior CEL-parity epic), and the explicit
instruction to design the exact quote-aware text-substitution algorithm in
`crates/embyr-core/src/access_control/rules_file.rs` as part of DESIGN's own architecture design,
not merely assert the shape here.

## Wave: DISCUSS / [REF] SSOT Updates

`docs/product/jobs.yaml`, JOB-17 entry: append a new dated NOTE (mirroring 4b′/4c/4d's own
identical NOTE-append convention) — "JOB-17 now also covers zero-parameter, non-nested named helper
`function` definitions/invocation in a real Firestore `.rules` file, same job, same persona, not a
new job (13th realization). The ONLY CEL-parity epic in this initiative that touches zero lines of
`embyr-core`'s own runtime evaluation logic (`Operand`/`Condition`/`evaluate()`) — a pure
import-time text transform confined to `rules_file.rs`, proven by Resolution 1. This completes the
3-epic sequence `security-rules-cel-parity`'s own original DISCUSS named (4c, 4d, 4e) — no further
CEL-parity epic is currently named or deferred."

## Wave: DISCUSS / [REF] Definition of Ready Validation

### Requirements Completeness Score: **0.97**

### DoR Checklist (9-item hard gate)
1. [x] Every story traces to a job_id (JOB-17, all 4 stories)
2. [x] Every story has a complete Elevator Pitch (Before/After/Decision enabled)
3. [x] Every AC is testable without ambiguity
4. [x] Walking Skeleton identified (US-01)
5. [x] Scope Assessment passed
6. [x] No slice contains only `@infrastructure` stories (every slice has a direct Alex-facing
   value story)
7. [x] Out of Scope explicitly named (6 items)
8. [x] Outcome KPIs have numeric targets and measurement methods
9. [x] Prior-wave artifacts read and reconciled (this feature's own naming/scope note from
   `security-rules-cel-parity`'s original DISCUSS is confirmed, not contradicted)

### DoR Status: **PASSED**

## Wave: DISCUSS / [REF] Open Questions

None carried forward unresolved — every open question this DISCUSS raised (runtime-vs-import-time
mechanism, parameters, nesting, `let` bindings, cross-surface parity) was independently resolved
with a locked Resolution above, grounded in live web verification of real Firestore's own
documented semantics.

## Wave: DISCUSS / [REF] Wave Decisions Summary

### Key Decisions
- [D1] Pure import-time text substitution (`rules_file.rs` only) — zero new `embyr-core` runtime
  concept, the smallest-footprint mechanism of any CEL-parity epic in this initiative.
- [D2] Zero-parameter functions only (Resolution 2) — the simplest possible substitution shape,
  matching the sole evidenced domain example exactly.
- [D3] No nesting (Resolution 3) — a function body is validated via the UNMODIFIED
  `parse_condition`, which structurally forbids both nesting and recursion with zero new detection
  code, as a direct consequence of D1's own design.
- [D4] No `let` bindings (Resolution 4) — a function body is exactly one `return <expr>;`.
- [D5] Read+write+simulation parity is a structural CONSEQUENCE of D1, not a separately-built
  slice — Slices 02/03 exist to prove this by direct test, needing zero production code.

### Requirements Summary
- Primary need: real customer `.rules` files still get rejected on the last remaining named
  CEL-parity gap — reusable, named boolean helper functions.
- Walking skeleton scope: one function, one call site, real end-to-end enforcement.
- Feature type: Backend.

### Constraints Established
- `embyr-core`'s `access_control/mod.rs` is untouched by this feature — confirmed as a locked
  constraint, not a hoped-for outcome.
- The substitution scanner must be quote-aware, mirroring the tokenizer's own existing discipline
  as a design principle (not shared code — a structurally different layer).
- Unit tests for every new pure function (the substitution scanner, the function-block outer-syntax
  parser) are written DURING DELIVER, and a dedicated `cargo-mutants --in-diff` pass is still
  budgeted regardless (4d's own QUALITY_GATE lesson).

### Upstream Changes
- None — this feature is a named, evidenced continuation of `security-rules-cel-parity`'s own
  original Out-of-Scope entry, and completes that DISCUSS's own originally-named 3-epic sequence
  (4c, 4d, 4e) in full.

## Wave: DISCUSS / [REF] Next Wave

**Handoff To**: nw-solution-architect (DESIGN wave)
**Deliverables**: this feature-delta.md, 5 locked Resolutions, 4-slice/2-release plan

---

## Wave: DESIGN / [REF] Prior Wave Consultation — Reading Confirmation

✓ This feature-delta.md's own DISCUSS sections in full, all 5 Resolutions.
✓ `crates/embyr-core/src/access_control/rules_file.rs`, `parse_allow_clauses` (line 442) — direct
re-read confirming the EXACT splice point: `condition_text` (the raw, trimmed string after `if`,
before the trailing `;`) is computed at line ~469–476, then pushed into `clauses` at line 481 —
substitution must happen strictly BETWEEN those two points, operating on the same `&str` slice
`parse_allow_clauses` already has in scope, before it is ever converted to an owned `String` and
stored.
✓ `crates/embyr-core/src/access_control/rules_file.rs`, `find_matching_close` (line 261) — confirms
the EXACT brace-depth-counting helper this feature's own `function` block scanner reuses unchanged
for its own `{...}` body — zero new brace-matching code needed.
✓ `crates/embyr-core/src/access_control/rules_file.rs`, `parse_rules_file` (line 219) — confirms the
exact insertion point: `service_body` (the content directly inside `service cloud.firestore { ... }`,
before the `match /databases/{database}/documents { ... }` sub-block is stripped off) is where
`function` blocks, as SIBLINGS of that sub-block, must be scanned — a NEW leading-blocks scan,
mirroring `parse_nested_match_blocks`'s own "repeatedly scan for a keyword-prefixed block until
exhausted" loop shape exactly, just for a different keyword and a flat (non-nesting) result shape
(`BTreeMap<String, String>`, not `Vec<MatchBlock>`).
✓ `crates/embyr-core/src/access_control/mod.rs`, `tokenize`'s own `'"' =>`/`'\'' =>` branches — the
EXACT quote-handling discipline (track whether inside a literal, skip scanning its own content)
this feature's own call-site scanner mirrors as a design principle.

## Wave: DESIGN / [REF] Reuse Analysis

| Existing mechanism | Reused unchanged for this feature? |
|---|---|
| `find_matching_close` (brace-depth counting) | Yes — the function-block scanner's own `{...}` body extraction reuses it unchanged |
| `RulesFileError::single`/`shell_syntax_error` (error shape, `OffendingBlock` naming) | Yes — every new rejection this feature adds (`UNDEFINED_FUNCTION`, `FUNCTION_PARAMETERS_UNSUPPORTED`, `DUPLICATE_FUNCTION`) uses the identical shape, new `construct` tags only |
| `parse_condition`/`detect_unsupported_construct` | Yes, UNCHANGED (not even narrowed, unlike 4c's/4d's own `get`/`exists`/`duration.value` exemptions) — reused as both the function-BODY validator (at definition time) AND the safety net catching any un-expanded call site (Resolution 1's own corollary) |
| `evaluate()`, `Operand`, `Condition` | Not touched at all — the first CEL-parity epic with zero `embyr-core` runtime changes |
| `parse_allow_clauses`'s own existing `path_pattern` threading (for `OffendingBlock` naming) | Yes — call-site expansion errors (`UNDEFINED_FUNCTION`, `FUNCTION_PARAMETERS_UNSUPPORTED`) reuse the SAME per-match-block naming already threaded through this exact function |

**Nothing in this feature requires a new bounded context, a new admin route, a new port/adapter
trait method, a new `Operand`/`Condition` variant, or a new `evaluate()` parameter** — confirmed by
direct code read before locking ADR-067, not assumed from the epic's own naming.

## Wave: DESIGN / [REF] Architecture Design

See ADR-067 (`docs/product/architecture/adr-067-custom-functions-import-time-text-substitution.md`)
for the full mechanism design. Summary of the 3 extension points, each additive and confined to
`crates/embyr-core/src/access_control/rules_file.rs`:

1. **Function-block scanning** (`parse_function_blocks`, NEW): mirrors
   `parse_nested_match_blocks`'s own "repeatedly scan a keyword-prefixed block" loop, scanning the
   LEADING portion of `service_body` for zero or more `function <name>() { return <expr>; }`
   blocks, reusing `find_matching_close` unchanged. Validates each body via the UNCHANGED
   `parse_condition` (Resolution 3's own "no nesting/recursion" guarantee) and returns a
   `BTreeMap<String, String>` (function name → body text, ALREADY stripped of `return`/`;`). A
   duplicate name is a NAMED rejection (`DUPLICATE_FUNCTION`) at THIS scan step, before any `match`
   block is even reached.
2. **Call-site expansion** (`expand_function_calls`, NEW): a single quote-aware linear scan over
   `parse_allow_clauses`'s own `condition_text`, splicing `(<body text>)` in place of each
   `<name>()` occurrence found OUTSIDE a string/single-quoted literal. A name matching no defined
   function is `UNDEFINED_FUNCTION`; a name found immediately followed by `(<non-empty>)` — a
   call WITH arguments — is `FUNCTION_PARAMETERS_UNSUPPORTED`. Applied exactly once per condition
   (never recursively — function bodies are pre-validated flat, never themselves containing an
   unexpanded call, so one linear pass is provably sufficient).
3. **Threading**: `parse_rules_file` calls `parse_function_blocks` on the leading portion of
   `service_body` before stripping the `match /databases/{database}/documents { ... }` sub-block;
   the resulting `BTreeMap<String, String>` threads down through
   `parse_match_blocks`→`parse_nested_match_blocks`→`parse_block_body`→`parse_allow_clauses`
   (mirroring how `full_path_pattern`/`full_segments` are already threaded down the IDENTICAL call
   chain), applied inside `parse_allow_clauses` at the exact splice point confirmed above.

## Wave: DESIGN / [REF] Wave Decisions Summary

### Key Decisions
- [D1] Zero new `embyr-core` runtime concept — confirmed by direct code read (`detect_unsupported_
  construct` already fail-closed rejects any un-expanded call site) BEFORE locking the mechanism,
  never assumed from the epic's own "composes over the grammar surface" framing.
- [D2] `parse_function_blocks` returns a FLAT `BTreeMap<String, String>` (name → body text), never
  a richer AST — the body text is spliced VERBATIM at each call site, never re-parsed-and
  -re-serialized (no AST-to-string serializer exists in this codebase, and building one purely for
  this feature would be new machinery this feature's own narrow scope does not justify).
- [D3] A function body is validated via `parse_condition` ONCE, at definition time (fail-fast on a
  malformed body before any `match` block is even scanned) — accepted, named trade-off: the SAME
  body text is parsed AGAIN, as part of each call site's own fully-expanded condition, when THAT
  condition is later parsed by `decompose_block`. Mirrors ADR-066's own accepted "path template
  resolved twice" precedent (D3) — pure computation, not I/O, non-load-bearing.
- [D4] The call-site scanner is quote-aware by DESIGN PRINCIPLE (mirrors the tokenizer's own
  discipline) but is NOT shared code with `tokenize()` — it operates on raw, pre-tokenized text at
  a structurally earlier pipeline stage, a deliberately separate, simpler scanner (it only needs to
  find `name(` / `name()` shapes and track quote state, not build a full token stream).

### Constraints Established
- No new dependency, no new bounded context, no new port/adapter trait method, no new `Operand`/
  `Condition` variant, no new `evaluate()` parameter.
- `embyr-core`'s `access_control/mod.rs` stays byte-for-byte unchanged by this feature — the ONLY
  CEL-parity epic in this initiative with that property, confirmed by construction.
- Unit tests for both new pure functions (`parse_function_blocks`, `expand_function_calls`) are
  written DURING DELIVER (ADR-067 § Enforcement), and a dedicated `cargo-mutants --in-diff` pass is
  still run regardless of during-slice test discipline (4d's own QUALITY_GATE lesson, applied
  proactively).

## Wave: DESIGN / [REF] Next Wave

**Handoff To**: nw-software-crafter (DELIVER wave, per this project's own established convention —
DISTILL folds into per-slice TDD, not a separate artifact, matching how the entire CEL family has
actually been built)
**Deliverables**: this feature-delta.md's DESIGN section, ADR-067
