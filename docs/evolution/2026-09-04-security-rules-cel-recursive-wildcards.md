# Evolution: security-rules-cel-recursive-wildcards

**Date:** 2026-09-04
**Feature:** Recursive-wildcard (`{path=**}`) support for the security-rules access-control
grammar — importing terminal, even-prefix recursive-wildcard patterns (including the empty-prefix,
project-wide catch-all shape), routing a concrete document path to the single most-specific
applicable pattern with a new precedence composition (exact-match > 4b fixed-depth > deepest
recursive-wildcard prefix), read+write+Listen-per-event parity, import-time tie/overlap rejection,
non-regression proof, and precedence-aware simulation.
**Job:** JOB-17 (`document-access-control`) — "Epic 4b′", a further split of
`security-rules-cel-path-matching`'s own originally-bundled recursive-wildcard scope into its own
feature.
**ADRs:** ADR-064 (`docs/product/architecture/adr-064-recursive-wildcard-prefix-matching-precedence-and-storage.md`)

## Business Context

`security-rules-cel-path-matching` (Epic 4b, prior feature) shipped multi-segment/nested `match`
block routing but deliberately deferred recursive-wildcard (`{path=**}`) patterns — the single
largest remaining piece of real-world `.rules`-file coverage, and the idiomatic way real Firebase
customers write a specific-override-plus-general-catch-all safety net (e.g. a per-collection
owner-only rule, backed by a project-wide `match /{document=**} { allow read, write: if false; }`
default-deny). Without this feature, importing any real customer `.rules` file containing a
recursive wildcard was rejected outright, naming it unsupported.

The central architectural finding (DISCUSS Resolution 1): `bind_ancestor`'s hard equal-length
precondition — the primitive ADR-063's whole fixed-depth routing mechanism is built on — returns
`None` for every recursive-wildcard case by construction, before any compatibility logic even
runs, because a recursive wildcard's own match boundary can fall at many different total path
lengths simultaneously. This is not a query-shape fix to the existing mechanism; it requires a
genuinely new primitive family (`bind_recursive_prefix`, matching a full concrete document path
against a pattern's own fixed prefix, of the correct parity) and a new composition step — but
reuses `PathSegment`, `positions_compatible`, and every one of 4b's own already-shipped
rows/tables/call sites unchanged.

The second major finding (Resolution 2): 4b's own "reject on any structural overlap" precedence
rule would make BOTH of this feature's own evidenced domain examples (a specific override plus a
catch-all) permanently unimportable — the catch-all's whole purpose is to overlap with everything
it isn't more specifically overridden by. Real Firestore's true semantics (OR-composition across
every matching pattern) was rejected as reopening the single-condition-per-request model every
JOB-17 epic assumes, and as a documented "gotcha" even in Firebase's own canonical docs. The locked
resolution is most-specific-wins precedence: exact-match always beats 4b fixed-depth, which always
beats any recursive wildcard, and among recursive wildcards the deepest/longest fixed prefix wins
— a deliberate, evidenced, **partial**-fidelity choice, not real Firestore's actual OR-composition
behavior, named as such rather than silently treated as equivalent.

A live web-verified correction during DISCUSS resolved an open question about zero-length matches:
real Firestore's behavior here is **version-dependent** (`rules_version` 1 vs. 2), and embyr's own
parser honors no `rules_version` directive anywhere across any of the 3 features in this
initiative. The locked choice matches the modern, actively-promoted `rules_version = '2'`
semantics (zero-or-more, includes the boundary document itself) — the more permissive of the two,
consistent with this initiative's fail-closed-not-fail-open discipline applied to the *scope
choice* itself.

## Key Decisions

| Decision | Verdict |
|---|---|
| Recursive-wildcard routing is a NEW step 3 in `resolve_access_rule_pattern`'s composition, reached only on a step-1/step-2 miss (exact-match and 4b fixed-depth patterns are structurally consulted first — precedence falls out of composition ORDER, zero runtime containment check) | ADR-064 |
| New pure primitive `bind_recursive_prefix(prefix, concrete_full_path)` — matches a stored pattern's fixed prefix against a concrete document's FULL path (ancestor + document ID), not just its ancestor (unlike `bind_ancestor`'s hard equal-length precondition) | ADR-064 |
| Only terminal, even-prefix recursive wildcards are accepted (Resolution 3) — the segment must be the pattern's own final segment, at an even-indexed (collection-name) position. Odd-prefix shapes are rejected with a distinguishable `RECURSIVE_WILDCARD_ODD_PREFIX`, non-terminal ones with `RECURSIVE_WILDCARD_NOT_TERMINAL` — named, deferred, zero domain evidence for either | ADR-064, Resolution 3 |
| A recursive wildcard's own captured remainder is never bound to a condition-referenceable name (`PathSegment::RecursiveWildcard` is unit-like, carries no name data) — real Firestore's `path`-type segment-indexing (`path[0]`) is explicitly out of scope, noted as a scope addition for the future `security-rules-cel-expression-grammar` epic | Resolution 3 |
| Among recursive-wildcard patterns, the deepest/longest fixed prefix wins wherever both structurally reach the same concrete path; same-specificity ties (identical depth+skeleton) and unrelated-shape overlaps (neither prefix structurally contains the other) are rejected at IMPORT time, naming both colliding patterns — reusing 4b's own Option C discipline at this narrower tie-breaking level only | Resolution 2, US-04 |
| Read+write+Listen-per-event parity locked within this same feature (not deferred again) — the same concrete document path already resolves every candidate at each of the 6 already-locked call sites (`GetDocument`, 3 write handlers, `handle_add_target`'s 2 per-event arms), zero new I/O | Resolution 4 |
| `RunQuery`'s non-group arm and Listen's own subscribe-time (initial-snapshot) compliance gate remain explicitly out of scope — unchanged boundary from 4b's own `OQ-PM-07` | Resolution 4 |
| Slice 06's `simulate_routed_access_rule` extension weighs a candidate recursive-wildcard pattern against REAL, already-stored patterns via the identical two-step composition real enforcement uses — a stored winner is evaluated against ITS OWN real `read_condition`, never the candidate's, so a "deferred" simulation outcome reflects what real routing would actually do | Slice 06, US-06 |

## Steps Completed

1. **Slice 01** (US-01) — Alex imports an even-prefix recursive-wildcard pattern, including the
   empty-prefix project-wide catch-all shape. Widened `validate_segment_shape` +
   `decompose_block`'s new `RecursiveWildcardPattern` branch.
2. **Slice 02** (US-02, Walking Skeleton, ADR-064) — A concrete path routes to the single
   most-specific applicable pattern with precedence. `bind_recursive_prefix` +
   `resolve_access_rule_pattern`'s new step 3.
3. **Slice 03** (US-03) — Writes and Listen's per-event re-check gain precedence-aware routing
   (`evaluate_write_rule_for_commit`, `handle_add_target`'s 2 arms via
   `resolve_recursive_wildcard_condition`).
4. **Slice 04** (US-04) — Reject recursive-wildcard precedence ties and unrelated-shape overlaps
   at import time, naming both colliding patterns.
5. **Slice 05** (US-05) — Full external regression baseline (every `security_rules_*` target)
   proven unaffected; a co-existing recursive-wildcard catch-all has zero effect on 4a's/4b's own
   specific rules, an untouched collection, or an unrelated pattern's own storage row.
6. **Slice 06** (US-06, LAST slice) — Alex simulates a candidate recursive-wildcard pattern's
   precedence outcome before importing it, extending `simulate_routed_access_rule` with a
   `winning_pattern` attribution (`"candidate"` | `"stored"`).

**QUALITY_GATE** — feature-scoped `cargo-mutants` pass (see
`docs/feature/security-rules-cel-recursive-wildcards/deliver/mutation/mutation-report.md`): 57
mutants against the pure `embyr-core` primitives this feature added/extended — 21 caught, 3
missed, 3 unviable, 30 inconclusive timeouts (environment contention from a concurrent build, not
a coverage signal). One missed mutant was a real gap in this feature's own new code (the recursive
branch's own conflicting-verb-condition guard, `decompose_block`) — closed with a new unit test.
The `embyr-server` (Docker/testcontainers-backed) pass is a documented skip: a first attempt ran 4
hours unattended and piled up 68 simultaneous Postgres containers before being killed, for zero
classified mutants — that layer is instead covered by this feature's own 58-target, 468-test
acceptance-test baseline, purpose-written to exercise the same branches.

**Full regression**: 58 `security_rules_*` targets, 468 tests, 0 failures (re-run clean after every
slice and again after the mutation-testing fix).

## Lessons Learned

1. **A hard equal-length precondition in a shared primitive is a real extension boundary, not just
   an implementation detail.** `bind_ancestor`'s `pattern_ancestor.len() != concrete_ancestor.len()`
   check made the WHOLE existing routing mechanism structurally incapable of expressing recursive
   wildcards, confirmed by direct code read before any design work started — this is the kind of
   fact DISCUSS's own "confirm structurally, don't assume from the charter's framing" discipline
   exists to surface.
2. **A precedence rule can be locked at a much narrower scope than "full real-world semantics."**
   Most-specific-wins precedence (Resolution 2, Option C) delivers both evidenced domain examples
   without building real Firestore's actual OR-composition behavior — a deliberate, named,
   partial-fidelity choice is often the correctly-scoped answer, not a compromise to be embarrassed
   about, as long as the gap is documented rather than silently treated as equivalent.
3. **Live web verification during DISCUSS caught a wrong training-data recollection before it
   reached code.** The zero-length-match boundary condition (Resolution 1, Fact 2) was initially a
   MODERATE-HIGH-confidence recollection; an actual fetch of Firebase's own docs revealed the real
   answer is version-dependent — a correction applied cleanly because the original DISCUSS had
   already structured its own representation (prefix + possibly-empty remainder) to accommodate
   either answer without a redesign.
4. **cargo-mutants against a Docker/testcontainers-backed integration suite does not scale
   unattended without careful job/container-lifecycle control.** A naive `-j 4` run against 6
   acceptance-test binaries (each spinning its own Postgres container) piled up 68 simultaneous
   containers over 4 hours with zero mutants classified — a mutation-testing pass against IO-heavy
   integration tests needs either much smaller scope, serial execution, or a fundamentally
   different (e.g. mocked/in-process) test harness to be tractable; this was documented as a skip
   rather than forced through at disproportionate cost.
5. **`--in-place` mode plus test-command filtering (narrowing to the relevant module, e.g.
   `-- access_control::`) is far more tractable for pure, zero-IO crates than the default
   scratch-copy model** — the same crate's OWN deliberately-slow, unrelated tests (Argon2id KDF,
   ~50s of real hashing) were silently inflating every mutant's wall-clock time until the test
   filter was narrowed.

## Key Files

- `crates/embyr-core/src/access_control/path_routing.rs` — `bind_recursive_prefix`,
  `classify_prefix_relation`, `generalizes`, `fixed_depth_full_reach`
- `crates/embyr-core/src/access_control/rules_file.rs` — `validate_segment_shape` (widened, made
  `pub`), `decompose_block`'s new `RecursiveWildcardPattern` branch, `DecomposedRecursivePattern`
- `crates/embyr-server/src/adapters/system_db.rs` — `list_recursive_access_rule_patterns_up_to`,
  `list_all_access_rule_patterns`
- `crates/embyr-server/src/grpc/handler.rs` — `resolve_access_rule_pattern`'s new step 3,
  `evaluate_write_rule_for_commit`
- `crates/embyr-server/src/realtime/listen_handler.rs` — `resolve_recursive_wildcard_condition`,
  `handle_add_target`'s per-event arms
- `crates/embyr-server/src/admin/handlers/access_rules.rs` — `overlap_rejection`,
  `parse_recursive_prefix` (Slice 04 tie rejection), `simulate_recursive_wildcard_candidate`,
  `evaluate_stored_pattern_outcome` (Slice 06)
- `docs/product/architecture/adr-064-recursive-wildcard-prefix-matching-precedence-and-storage.md`
- `tests/security_rules_cel_recursive_wildcards/acceptance/` — rw01 through rw06, 6 acceptance
  targets, shared `common/mod.rs` (re-exports `security_rules_cel_path_matching`'s own harness)
- `docs/feature/security-rules-cel-recursive-wildcards/feature-delta.md` — full DISCUSS/DESIGN
  narrative (retained in place, not migrated — this project's established SSOT convention)
- `docs/feature/security-rules-cel-recursive-wildcards/slices/` — 6 elephant-carpaccio slice briefs
- `docs/feature/security-rules-cel-recursive-wildcards/deliver/mutation/mutation-report.md` —
  full mutation-testing findings and the Docker-contention skip justification

## Follow-Up Work

- **Epic 4c — `security-rules-cel-expression-grammar`** — the remaining full CEL expression
  surface: arithmetic operators, `in`, list/map literals, numeric literals, timestamp/duration
  types. Also now carries a scope note from this feature: real Firestore's `path`-type
  segment-indexing (`path[0]`) for referencing a recursive wildcard's own captured remainder,
  deliberately not built here. Named, deferred, zero-IO, lowest architectural risk of the 3
  remaining CEL-parity epics — **next in sequence**.
- **Epic 4d — `security-rules-cel-cross-document-reads`** — `get()`/`exists()` cross-document
  reads. Isolated deliberately (I/O in the hot request path, a materially different risk/
  consistency profile from every other epic in this initiative). Needs its own DESIGN pass
  revisiting BC-4's read-only dependency shape on BC-2.
- **Epic 4e — `security-rules-cel-functions`** — custom `function` definitions and invocation.
  Composes over 4c's own grammar surface once it exists; lowest-risk of the 3 remaining epics.
- **Odd-prefix recursive wildcards** (`expeditions/{path=**}`, no intervening document-ID
  position) — named, deferred, no candidate feature id assigned yet. Zero domain evidence today.
- **Real-Firestore full OR-composition precedence semantics** — this feature's most-specific-wins
  precedence is a deliberate partial-fidelity choice, not equivalent. Unbucketed, unscheduled.
- **`RunQuery`/Listen subscribe-time compliance under recursive-wildcard patterns** — unchanged
  boundary from 4b's own `OQ-PM-07`, carried forward again.
- A repeat, better-scoped `embyr-server` mutation-testing pass (serial, single-binary-scoped, once
  the machine is not under concurrent build/container contention) — not attempted again in this
  pass to avoid repeating the same 4-hour failure mode.
