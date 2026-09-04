# Evolution: security-rules-cel-cross-document-reads

**Date:** 2026-09-04
**Feature:** `get()`/`exists()` cross-document reads in the security-rules condition grammar — a
single-level (no chaining) role-lookup idiom (e.g.
`get(/databases/$(database)/documents/organizations/$(request.auth.uid)).data.role == "admin"`),
across read enforcement, write enforcement, and simulation.
**Job:** JOB-17 (`document-access-control`) — 12th realization, "Epic 4d" (the 3rd of the 3
remaining CEL-parity epics named by `security-rules-cel-parity`'s original DISCUSS split).
**ADRs:** ADR-066 (`docs/product/architecture/adr-066-cross-document-reads-two-phase-evaluation.md`)

## Business Context

Closes the single remaining named CEL-parity capability gap that requires I/O — the canonical
real-Firestore organization/role-membership lookup idiom, evidenced by analogy to the widely
published pattern (no prior Trailmark domain example existed; real Firestore's own documented
`get()`/`exists()` semantics were independently web-verified this DISCUSS: 10/20-read ceiling,
cached-call exemption, `$(variable)` path substitution syntax, `exists()` returns a clean `false`
on a missing document, `get()`'s own `.data` access throws — denies — on one).

The central architectural question this feature's DESIGN wave resolved (flagged, unresolved, by
`security-rules-cel-parity`'s own DISCUSS): `get()`/`exists()` require `embyr-server` (BC-4) to
actively call into the backend's read path MID-EVALUATION — something `evaluate()` itself
structurally cannot do, since `embyr-core` is IO-forbidden (`deny.toml`-enforced, a hard
CI-tooling boundary). ADR-066's answer: two-phase evaluation — pure path-discovery (embyr-core) →
real I/O fetch (embyr-server) → pure `evaluate()` with the pre-fetched results threaded in as a
parameter. This is the FIRST epic in the whole CEL-parity initiative whose own simulation slice
genuinely needed new production code (every prior epic's simulation slice needed zero — this one's
`get()`/`exists()` resolution has no real backend to simulate against, so `simulate_access_rule`
needed a caller-supplied SYNTHETIC document map instead).

## Key Decisions

| Decision | Verdict |
|---|---|
| Two-phase evaluation: pure path-discovery (`discover_cross_document_paths`, embyr-core) → real fetch (`embyr-server`) → pure `evaluate()` with results threaded in as its new 8th parameter — never I/O inside `embyr-core` | ADR-066 |
| Single-level only, no chaining (`$(get(...))` nested inside another `get()`/`exists()` path is a NAMED `UnsupportedExpressionGrammar` rejection, never silently mis-parsed) | ADR-066 |
| `$(request.auth.uid)` / `$(request.path.<var>)` are the ONLY legal path substitutions; anything else inside `$(...)` is a named rejection | ADR-066 |
| No artificial read-budget cap — the single-level-only structural bound already keeps counts small; real Firestore's own 10/20-read ceiling is not reimplemented | ADR-066 |
| Deduplication is free: path-discovery returns a `BTreeSet<String>`, not a list — the SAME concrete path referenced by both `exists()` and `get()` in one condition fetches once | ADR-066 |
| Reuse the EXISTING `BackendAdapter::get_document` port method unchanged — no new trait method, no new adapter capability, no batched-fetch optimization | ADR-066 |
| A condition with zero cross-document operands costs ZERO extra I/O and ZERO extra allocation beyond an empty-map construction — path-discovery walks the `Condition` tree and returns an empty set immediately | ADR-066 |
| Simulation cannot share the real fetch step (no real backend to simulate against) — `simulate_access_rule` gains a caller-supplied SYNTHETIC `cross_document_reads` map instead, translated via the SAME `json_value_to_field_value` helper every other synthetic input already uses | ADR-066 |

## Steps Completed

1. **Slice 01** (US-01, Walking Skeleton) — `exists()` parses and enforces on real `GetDocument`
   reads. New types: `PathTemplate`/`PathTemplateSegment`, `Operand::CrossDocumentExists`,
   tokenizer `exists(` scan, `discover_cross_document_paths` (pure, `pub`), `evaluate()`'s new 8th
   parameter (`cross_document_reads`).
2. **Slices 02–03** (US-02/03) — `get().data.<field>` fail-closed access, and the SAME path
   referenced by both `exists()` and `get()` in one condition fetching exactly once (dedup proof).
3. **Slice 04** (US-04) — write-path parity: `CreateDocument`/`UpdateDocument`'s exact-match
   branches wired with the same discover→fetch→evaluate sequence (routed-pattern branches left
   mechanical — no domain example, out of scope).
4. **Slice 05** (US-05, LAST slice) — simulation parity. `SimulateAccessRuleBody` gains
   `cross_document_reads: BTreeMap<String, BTreeMap<String, serde_json::Value>>`; a path referenced
   by the candidate condition but absent from the synthetic map falls through `evaluate()`'s
   existing `FieldMissing` fail-closed mechanism identically to a real nonexistent document — no
   separate "absent" code path.

**QUALITY_GATE** — `cargo-mutants --in-diff` against this feature's own full `embyr-core` diff (95
mutants, precise to this feature's own new/changed lines only). Unlike 4c, unit tests were written
DURING each slice rather than deferred — first mutation run still found 8 narrow boundary/exemption
gaps (an exemption's own scope never proven narrow, several exact-loop-bounds edge cases, one
error-message-text distinction, the `Arithmetic`/`ListLiteral` recursion arms of
`walk_operand_for_cross_document_paths`). Added 9 targeted unit tests; final run: 0 missed, 86
caught, 7 unviable, 2 timeouts (infinite-loop mutants against the 30s harness timeout — observably
broken, not silently passing). Full details:
`docs/feature/security-rules-cel-cross-document-reads/deliver/mutation/mutation-report.md`.

**Full regression**: `security_rules_*`-filtered test run, 161 target binaries, 368 tests, 0
failures (re-run clean after every slice and again after the mutation-testing fix).

## Lessons Learned

1. **Writing unit tests during each slice (not deferred to a post-hoc QUALITY_GATE pass) still
   leaves boundary/exemption gaps — but far fewer, and narrower, than deferring entirely.** 4c's
   post-hoc approach found 45/53 missed (near-total absence of unit coverage); 4d's
   during-slice approach found 8/95 missed, every one an exact boundary (an index at the very last
   character, an exemption's untested scope, a message-text distinction) rather than a whole
   untested code path. The proactive lesson from 4c reduced the gap by an order of magnitude, but
   did not eliminate the need for a dedicated mutation-testing pass — boundary conditions are
   exactly the class of gap a developer's own "does this work" testing during implementation is
   least likely to think to probe, even when testing diligently.
2. **An exemption added to an existing rejection scan (`detect_unsupported_construct`'s new
   `get`/`exists` allow-list) needs its own test proving the exemption is narrow, not just a test
   proving the exempted case now succeeds.** Slice 01's own tests proved `exists(...)`/`get(...)`
   now parse; nothing proved a THIRD, non-exempted function-call-shaped identifier (`foo(...)`)
   still gets rejected — the exemption could have silently widened into a blanket bypass of the
   custom-function-call check without any existing test noticing.
3. **A recursive helper's every match arm needs its own direct test, even arms that "obviously"
   just recurse.** `walk_operand_for_cross_document_paths`'s `Arithmetic`/`ListLiteral` arms were
   written correctly on the first pass (mirroring the existing `Condition`-tree walker's own
   recursion shape) but had zero test coverage proving a cross-document operand nested inside
   either shape is actually discovered — an easy gap to leave open when the recursive case "looks
   obviously right" by inspection.
4. **`--in-diff` scoping (new to this feature; 4c used a hand-picked function-name list) is more
   precise AND less brittle to file growth.** As the same `access_control/mod.rs` file accumulates
   more prior-epic logic across CEL-parity epics, hand-picking "the functions this feature added"
   risks either omitting a genuinely-new function or including a function shared with a prior
   epic. `--in-diff` against `git diff <feature-start-commit>~1..HEAD` scopes automatically to
   exactly the lines this feature's own commits touched — the diff must be regenerated against the
   CURRENT working tree (not a stale earlier commit) if unit tests are added after the diff was
   first captured, or `cargo-mutants` rejects it with a line-mismatch error.

## Key Files

- `crates/embyr-core/src/access_control/mod.rs` — `PathTemplate`/`PathTemplateSegment`,
  `Operand::{CrossDocumentExists,CrossDocumentGet}`, `Token::{ExistsCall,GetCall}`,
  `starts_with_at`, `find_matching_paren`, `parse_path_template`, `discover_cross_document_paths`
  (pub), `resolve_path_template`, `evaluate()`'s 8th parameter (`cross_document_reads`)
- `crates/embyr-server/src/grpc/handler.rs` — `fetch_cross_document_reads` (real I/O), wired into
  `handle_get_document` (both branches, Slice 01) and `handle_create_document`/
  `handle_update_document`'s exact-match branches (Slice 04); mechanical empty map at every other
  call site (Commit write-path helper, `DeleteDocument` x2, `ListDocuments`, `BatchGetDocuments`,
  Listen's 2 per-event sites — out of this feature's own locked scope)
- `crates/embyr-server/src/admin/handlers/access_rules.rs` —
  `SimulateAccessRuleBody.cross_document_reads`, translated via `json_value_to_field_value` into
  synthetic `FirestoreDocument`s
- `docs/product/architecture/adr-066-cross-document-reads-two-phase-evaluation.md`
- `tests/security_rules_cel_cross_document_reads/acceptance/` — cdr01 through cdr05, 5 acceptance
  targets, shared `common/mod.rs` (re-exports `security_rules_cel_expression_grammar`'s own
  harness)
- `docs/feature/security-rules-cel-cross-document-reads/feature-delta.md` — full DISCUSS/DESIGN
  narrative (retained in place, this project's established SSOT convention)
- `docs/feature/security-rules-cel-cross-document-reads/slices/` — 5 elephant-carpaccio slice
  briefs
- `docs/feature/security-rules-cel-cross-document-reads/deliver/mutation/mutation-report.md`

## Follow-Up Work

- **Epic 4e — `security-rules-cel-functions`** — custom `function` definitions and invocation.
  Composes over both 4c's grammar surface and 4d's cross-document-read mechanism once built.
  **Next in sequence — the last of the 3 CEL-parity epics named by `security-rules-cel-parity`'s
  original DISCUSS split.**
- **Chained/nested `get()`/`exists()`** (a `get()` result feeding another `get()`/`exists()`'s own
  path) — zero domain evidence beyond the single-level idiom this feature builds; named, deferred,
  no candidate feature id assigned (ADR-066's own explicit Resolution 2 scope boundary).
- **A read-budget cap matching real Firestore's own 10/20-document ceiling** — zero domain
  evidence this feature's own single-level structural bound is insufficient in practice; named,
  deferred (ADR-066 Resolution 4).
- **Listen's own per-event re-check of a cross-document-read-gated rule** — explicitly out of this
  feature's own locked scope (DISCUSS Resolution 5); both `listen_handler.rs` call sites carry a
  mechanical empty map today.
