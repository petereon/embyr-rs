# Evolution: security-rules-cel-functions

**Date:** 2026-09-04
**Feature:** Custom `function` definitions and invocation in the security-rules condition grammar
— zero-parameter, non-nested named helper functions (e.g. `function isEditor() { return
request.auth.uid == resource.data.editor_id; }`), callable as `<name>()` from any `match` block's
own `allow` clause.
**Job:** JOB-17 (`document-access-control`) — 13th realization, "Epic 4e" (the LAST of the 3
remaining CEL-parity epics named by `security-rules-cel-parity`'s original DISCUSS split).
**ADRs:** ADR-067 (`docs/product/architecture/adr-067-custom-functions-import-time-text-substitution.md`)

## Business Context

Closes the LAST remaining named CEL-parity capability gap: reusable, named boolean helpers in a
real `.rules` file. Real Firestore's own documented syntax (`function <name>(<params>) { return
<expr>; }`, declared as a sibling of `match` blocks inside `service cloud.firestore { ... }`,
independently web-verified this DISCUSS) was ground truth for this feature's own locked v1 scope:
zero-parameter functions only, no nesting, no `let` bindings — a narrow, evidenced subset of a
real capability real Firestore supports more broadly.

The central architectural finding (DISCUSS Resolution 1), genuinely different in kind from every
prior CEL-parity epic: **a function call needs NOTHING new at evaluation time.** Every prior epic
(4a through 4d) needed a genuinely new piece of information at evaluation time — a fetched
cross-document result, a wall-clock read, a numeric/timestamp operand — resolved via
`evaluate()`'s own "pre-resolve, thread as a parameter" pattern. A function call's own body, once
resolved, is ordinary, already-supported condition grammar over the SAME `resource`/`request`
bindings the calling clause already has — there is nothing to evaluate differently, only something
to EXPAND, once, before evaluation ever begins. This makes the feature's own mechanism a pure
IMPORT-TIME text substitution confined entirely to `rules_file.rs`, touching ZERO lines of
`embyr-core`'s own runtime evaluation logic (`Operand`/`Condition`/`evaluate()`) — the only
CEL-parity epic in the whole initiative with that property.

## Key Decisions

| Decision | Verdict |
|---|---|
| Pure import-time text substitution (`rules_file.rs` only), never a new `Operand`/`Condition` variant or `evaluate()` parameter (Resolution 1) — the smallest-footprint mechanism of any CEL-parity epic in this initiative | ADR-067 |
| Zero-parameter functions only (Resolution 2) — real-Firestore-accurate as a deferred capability, matching the sole evidenced domain example exactly | ADR-067 |
| No nesting — a function body is validated via the UNMODIFIED `parse_condition`, which structurally forbids both nesting and recursion with zero new detection code, as a direct consequence of the substitution design (Resolution 3) | ADR-067 |
| No `let` bindings — a function body is exactly one `return <expr>;` (Resolution 4) | ADR-067 |
| Read+write+simulation parity is a structural CONSEQUENCE of Resolution 1, not a separately-built slice (Resolution 5) — proven directly by 2 slices needing zero production code, the strongest form of this initiative's own "confirmatory slice" precedent | ADR-067 |
| An undefined function call gets its own named `UNDEFINED_FUNCTION` rejection, distinguishable from the pre-existing generic `CUSTOM_FUNCTION` tag a bare, non-imported condition still gets | ADR-067 |
| A function named after a reserved word (`get`/`exists`/`duration`/`true`/`false`/`in`) is rejected at definition time — prevents silently shadowing an existing grammar construct | ADR-067 (implementation-level, not a DISCUSS Resolution) |

## Steps Completed

1. **Slice 01** (US-01, Walking Skeleton) — `parse_function_blocks` (scans `function <name>() {
   return <expr>; }` blocks, siblings of the `match`/documents block) and `expand_function_calls`
   (a single quote-aware linear scan splicing `(<body>)` into each call site), threaded through
   `parse_rules_file` → `parse_match_blocks` → `parse_nested_match_blocks` → `parse_block_body` →
   `parse_allow_clauses`. Real `GetDocument` enforcement proof.
2. **Slice 02** (US-02) — write-path parity (`CreateDocument`/`UpdateDocument`), confirmatory: zero
   production code change needed, exactly as ADR-067's own central claim predicted.
3. **Slice 03** (US-03, Release 1 complete) — simulation parity, confirmatory: zero production code
   change needed.
4. **Slice 04** (US-04, LAST slice) — multiple functions across multiple call sites (including the
   SAME function called from 2 independent `match` blocks, proven to resolve independently per
   call site), plus real-import proofs for every scoped-out construct's own named rejection
   (parameterized definition/call, duplicate name, nested function call — the last of which reuses
   the pre-existing `CUSTOM_FUNCTION` tag unchanged, proving Resolution 3's own "no new detection
   code" guarantee directly).

**QUALITY_GATE** — `cargo-mutants --in-diff` against this feature's own full `rules_file.rs` diff
(90 mutants — this feature's ENTIRE `embyr-core` footprint, since `access_control/mod.rs` stays
untouched). First run found 4 narrow function-name character-validation gaps (two `||`→`&&`
junction mutants in a 3-way disjunction no test exercised with exactly one disjunct true; two
`==`→`!=` mutants on `c == '_'` checks no test proved a leading/mid-name underscore is actually
accepted). Added 3 targeted unit tests; final run: 0 missed, 72 caught, 6 unviable, 12 timeouts
(infinite-loop mutants against the 30s harness timeout in the char-scanning loop — observably
broken, not silently passing). Full details:
`docs/feature/security-rules-cel-functions/deliver/mutation/mutation-report.md`.

**Full regression**: `security_rules_*`-filtered test run, 165 target binaries, 384 tests, 0
failures (re-run clean after every slice and again after the mutation-testing fix).

## Lessons Learned

1. **Not every capability extension needs a new runtime concept — some need only a new SYNTACTIC
   transform.** Every prior CEL-parity epic assumed (correctly, for those epics) that a new
   grammar capability meant widening `evaluate()`'s own signature. This feature's own central
   insight — a function call resolves to nothing more than "different text, same grammar" — meant
   the lowest-risk design was to NOT build a runtime concept at all, even though a `FunctionCall`
   `Operand` variant was the more "obvious" design by analogy to every prior epic's own shape. The
   right architectural question is "does this NEED new information at evaluation time," not
   "did the last several features all add an `Operand` variant."
2. **A structural constraint can BE the safety mechanism, not merely be satisfied by one.**
   Resolution 3 (no nesting/recursion) was not implemented as a separately-built cycle detector —
   it falls out for free from validating a function body via the UNMODIFIED `parse_condition`
   (which already rejects any call-shaped identifier). Choosing NOT to apply the expansion pass to
   function bodies was simultaneously the simplest implementation AND the correctness guarantee,
   rather than two separate concerns.
3. **A capability that touches zero production code in 2 of its own 4 slices is still worth
   PROVING by direct test, not merely asserting from the design.** Slices 02–03 could have been
   skipped as "obviously true given Resolution 1" — but running them as real acceptance tests
   confirmed the design's own central claim empirically, catching what would otherwise have been
   an unverified assumption carried all the way to FINALIZE.
4. **Character-validation boundary conditions in a hand-rolled name-syntax check need explicit
   positive-and-negative pairs, not just negative "this is invalid" tests.** Every existing test
   for this feature's own function-name validation was a REJECTION case; none proved a legitimate
   edge case (a leading or mid-name underscore) was actually ACCEPTED — exactly the kind of gap a
   `==`/`!=` mutation surfaces and "does this look right" review does not.

## Key Files

- `crates/embyr-core/src/access_control/rules_file.rs` — `parse_function_blocks`,
  `expand_function_calls`, `find_char_paren_close`; `parse_rules_file`/`parse_match_blocks`/
  `parse_nested_match_blocks`/`parse_block_body`/`parse_allow_clauses` all gained a threaded
  `&BTreeMap<String, String>` (function name → body text) parameter. New `construct` tags:
  `UNDEFINED_FUNCTION`, `DUPLICATE_FUNCTION`, `FUNCTION_PARAMETERS_UNSUPPORTED`.
- `crates/embyr-core/src/access_control/mod.rs` — **untouched by this feature**, confirmed by
  construction, the only CEL-parity epic with that property.
- `crates/embyr-server/` — **untouched by this feature** (Slices 02–03's own confirmatory proof).
- `docs/product/architecture/adr-067-custom-functions-import-time-text-substitution.md`
- `tests/security_rules_cel_functions/acceptance/` — cf01 through cf04, 4 acceptance targets,
  shared `common/mod.rs` (re-exports `security_rules_cel_cross_document_reads`'s own harness)
- `docs/feature/security-rules-cel-functions/feature-delta.md` — full DISCUSS/DESIGN narrative
  (retained in place, this project's established SSOT convention)
- `docs/feature/security-rules-cel-functions/slices/` — 4 elephant-carpaccio slice briefs
- `docs/feature/security-rules-cel-functions/deliver/mutation/mutation-report.md`

## Follow-Up Work

**This FINALIZE completes the 3-epic CEL-parity sequence `security-rules-cel-parity`'s original
DISCUSS split named and deferred (4c, 4d, 4e) — no further CEL-parity epic is currently named or
deferred.**

- **Parameterized functions** — real-Firestore-accurate, zero domain evidence (Resolution 2).
  Named, deferred, no candidate feature id assigned.
- **Function nesting** (a function calling another function, non-recursively — real Firestore's
  own actual capability) — zero domain evidence (Resolution 3). Named, deferred.
- **`let` bindings inside a function body** — zero domain evidence (Resolution 4). Named, deferred.
- Carried forward, still unbuilt, from prior CEL-parity epics: `OQ-CEG-01` (nested map-field
  traversal), chained/nested cross-document reads, an artificial cross-document read-budget cap,
  full expression substitution inside `$(...)`, map literals, `*`/`/`/`%` arithmetic, `path[0]`
  segment-indexing.
