# Feature Delta: security-rules-cel-chaining-detection

## Wave: DISCUSS / [REF] Prior Wave Consultation — Reading Confirmation

✓ `docs/product/known-gaps.md`, gap #8 — read in full. Exact current wording: "CEL rule-import
'chaining' construct (a `get()` whose path is built from another `get()`'s own result) isn't
detected as an offending import — only 1 of 3 expected offending blocks named"; location
`tests/security_rules_cel_parity/acceptance/cp04_reject_out_of_scope_imports.rs:264`, underlying
detection logic in `crates/embyr-server`'s rule-import validation; real-client reachable: yes (a
real rule author could write a chaining `get()` and not be warned it's unsupported); severity:
degrades feature, not data-corrupting; status before this DISCUSS: "Not started — found as a
byproduct of `firestore-transaction-read-consistency`'s own regression testing, bisection-confirmed
pre-existing." This is the LAST unclaimed gap in the table — gap #8 of 8, all others CLOSED.
✓ `docs/product/jobs.yaml`, JOB-17 (`document-access-control`) — read in full, including every NOTE
appended since origin (2026-08-17) through `security-rules-cel-cross-document-reads` (2026-09-04),
the most recent NOTE actually present in the file. Persona P1 Alex, unchanged across all 13 prior
realizations. Most relevant prior NOTEs for this feature:
  - `security-rules-cel-cross-document-reads` (2026-09-04, 12th realization): locks Resolution 2
    ("single-level only — every `get()`/`exists()` path is built ONLY from bindings already known
    before evaluation begins — never from another `get()`'s own result") as a deliberate,
    evidenced non-goal, not a v1-only deferral. Chaining is real-Firestore-accurate but zero domain
    evidence needs it, and the iterative discover-fetch-discover mechanism chaining would require is
    a materially larger, riskier build than the single-pass mechanism this feature's own predecessor
    shipped.
  - `security-rules-cel-expression-grammar` (2026-09-04, 11th realization): confirms
    `UNSUPPORTED_EXPRESSION_GRAMMAR` already exists as a named `OffendingBlock` construct tag in this
    codebase — this feature does not invent a new construct name, it makes an EXISTING one apply
    reliably to a construct shape it was always meant to cover.
  - Note: `docs/feature/security-rules-cel-functions/feature-delta.md` § SSOT Updates promises a
    13th-realization NOTE for JOB-17 that is NOT present in `jobs.yaml` today (the same
    "promised-but-never-applied" pattern `docs/product/jobs.yaml`'s own
    `firestore-composite-indexes-admin-api` NOTE flagged and back-filled once already). Not this
    feature's own concern to backfill — flagged here only so the "14th realization" ordinal used in
    this feature's own NOTE below is understood as a narrative sequence position, not a mechanically
    verified count of NOTEs literally present in the file.
✓ `tests/security_rules_cel_parity/acceptance/cp04_reject_out_of_scope_imports.rs` — read in FULL
(all 6 tests), not only the failing one. Confirms the existing, already-correct pattern this feature
extends:
  - `a_wildcard_at_a_collection_name_position_is_rejected_naming_that_block` — `NESTED_PATH`
    detection works correctly today, single-block.
  - `a_recursive_wildcard_path_is_now_accepted_superseding_the_original_rejection` — recursive
    wildcards are a SUPPORTED construct as of `security-rules-cel-recursive-wildcards`; not relevant
    to this feature except as an example of this file's own "superseded, not silently deleted"
    convention.
  - `multiple_offending_blocks_are_all_named_in_a_single_rejection_response` (lines 209–282, the
    failing test named by the gap) — imports 3 deliberately-offending blocks: a
    wildcard-at-collection-position block (`NESTED_PATH`, already correct), a custom-function-call
    block (`isEditor()`, `CUSTOM_FUNCTION`, already correct), and a chaining `get()` block
    (`get(/databases/$(database)/documents/orgs/$(get(/databases/$(database)/documents/users/
    $(request.auth.uid)).data.orgId))`, expected `UNSUPPORTED_EXPRESSION_GRAMMAR`, line 264's own
    `assert_eq!(offending.len(), 3, ...)` currently fails). The test's own comment block (lines
    215–235) confirms this scenario was DELIBERATELY repurposed during
    `security-rules-cel-cross-document-reads` Slice 01 to use chaining specifically because it "IS
    still genuinely out of scope," and cites that feature's own CDR01 unit test
    (`a_substitution_beyond_auth_uid_or_path_variable_is_a_named_rejection`) as proof chaining is
    "unsupported directly" — i.e., there is already SOME code path in this codebase that names
    chaining `UNSUPPORTED_EXPRESSION_GRAMMAR` correctly, at least in an isolated/unit context. This
    is the load-bearing clue behind this feature's own working hypothesis (§ Job Discovery Framing
    Resolution below) — not confirmed by reading implementation, per this DISCUSS's own scope
    boundary.
  - `a_rejected_import_leaves_every_existing_and_would_be_rule_completely_unchanged` — confirms the
    existing atomicity guarantee (AC-17-192: a rejected import stores NOTHING, not even for its own
    in-scope blocks) this feature's own AC-CCD-04 must not regress.
  - `a_condition_referencing_an_undefined_path_variable_is_rejected_at_import_time` and
    `differing_conditions_for_the_same_verb_bucket_are_rejected_as_conflicting` — confirm
    `SYNTAX_ERROR` and `CONFLICTING_VERB_CONDITIONS` are two further already-correct, independently
    named constructs this feature must not disturb.
✓ `docs/product/architecture/adr-066-cross-document-reads-two-phase-evaluation.md` — confirms
Resolution 2 (single-level only, no chaining) is the architecturally locked reason chaining itself is
a permanent non-goal, not merely undocumented. This feature does NOT re-litigate that decision — it
is entirely about DETECTING and CLEANLY REJECTING the construct ADR-066 already decided not to
support, not making it work.
✓ `docs/feature/security-rules-cel-functions/feature-delta.md` — read for this project's own
established DISCUSS-wave feature-delta.md section convention and format (mirrored below).
✓ Explicitly NOT read, per this DISCUSS's own scope boundary: any `crates/embyr-server` or
`crates/embyr-core` implementation file for the rule-import validation pipeline. Root-cause
diagnosis of WHY the chaining block is dropped from the multi-block response is DESIGN's own task,
not DISCUSS's.

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

- Feature type: **Backend** (Decision 1) — extends the existing rule-import validation pipeline's
  own offending-block detection/naming; zero new admin route, zero new RPC.
- JTBD: **reuse JOB-17** (Decision 4 = "Yes", real value story, NOT infrastructure-only) — the 14th
  realization of the same job. A dated NOTE is appended to `jobs.yaml`'s JOB-17 entry
  (§ SSOT Updates).
- Walking Skeleton: **Yes** (Decision 2) — a single chaining `get()` block, alone, correctly named
  `UNSUPPORTED_EXPRESSION_GRAMMAR` in a real import rejection.
- UX Research Depth: **Lightweight** (Decision 3) — a backend import-time detection-reliability fix,
  one already-fully-profiled persona (Alex), zero new emotional arc, zero new mental model (Alex
  already knows chaining is rejected in some contexts today — this feature makes that rejection
  reliable everywhere it should apply).

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: P1 Alex (SDK Developer), unchanged across all prior JOB-17 realizations.

**Job**: JOB-17 `document-access-control`, unchanged job_story — reused, not a new job. This
feature's own realization: Alex writes a real Firestore `.rules` file containing a chaining `get()`
call — a `get()` whose own path expression is itself built from ANOTHER `get()`'s own result (e.g.
looking up an org membership document whose path depends on a user document's own field, fetched via
a nested `get()`). Real Firestore's own grammar permits this; embyr's own `security-rules-cel-
cross-document-reads` feature deliberately does not (ADR-066 Resolution 2 — single-level `get()`/
`exists()` only). Today, that rejection is unreliable: it is proven correct in at least one isolated
context (the CDR01 unit test cited in cp04's own comments) but proven UNRELIABLE in the exact
production-shaped scenario cp04's own multi-block test exercises — a chaining block sitting alongside
other offending blocks in the same file.

### Job Discovery Framing Resolution

This is a "make it real"/close-the-remaining-gap realization, not a "same persona, different goal"
new-job pattern (mirrors `firestore-query-filter-operator-support` → JOB-01 and every other
"close-the-gap" NOTE in this session, not the JOB-16→JOB-17 "different goal" pattern).

**Working hypothesis on WHERE the gap lives** (explicitly a hypothesis for DESIGN to confirm or
refute by reading the actual implementation — not confirmed here): cp04's own test-file comments
(§ Reading Confirmation above) cite an existing CDR01 unit test that already proves a chaining
substitution is named `UNSUPPORTED_EXPRESSION_GRAMMAR` correctly in isolation. That, combined with
gap #8's own wording ("only 1 of 3 expected offending blocks named" — not "0 of 3", and not "the
import is silently accepted") suggests the single-condition detection logic itself likely already
recognizes the chaining shape correctly, but something in the per-block iteration/aggregation that
collects `offending_blocks` across MULTIPLE `match` blocks in one file either short-circuits, drops,
or misclassifies the third block specifically in this multi-block shape — an aggregation/orchestration
gap, not necessarily a total absence of detection logic. An equally plausible alternative the same
evidence does not rule out: the isolated CDR01 unit test may exercise a narrower or structurally
different code path (e.g. a lower-level condition-grammar check) than whatever produces
`offending_blocks` for the admin-facing import response, and the two have simply never been proven
consistent with each other. **DESIGN must confirm the actual root cause by reading
`crates/embyr-core/src/access_control/rules_file.rs`/`mod.rs` and `crates/embyr-server`'s own
rule-import validation call site before designing a fix** — this DISCUSS deliberately stops at
requirements and does not prescribe a mechanism.

## Wave: DISCUSS / [REF] Business Context

This closes the LAST remaining row in `docs/product/known-gaps.md` (gap #8 of 8; gaps 1–7 are all
CLOSED). Unlike gap #7 (`secrets-management` test flakiness, infrastructure-only, no real-client
impact), gap #8 is a genuine product gap reachable by a real Alex: if he writes a chaining `get()` in
a real `.rules` file, whether he is warned depends on what ELSE is in that same file — an
inconsistency that undermines trust in the whole rejection mechanism, not just this one construct.
The failure mode this feature closes is exactly the class of risk JOB-17's own "anxiety" four-force
names: "What if I get the condition backwards and either leak every user's data or lock every user
out of their own?" — an unreliably-detected unsupported construct that silently imports could reach
runtime and behave unpredictably (undefined evaluation of a construct the CEL evaluator was never
built to handle), instead of being cleanly rejected at import time with the SAME clear signal cp04's
own sibling constructs (`NESTED_PATH`, `CUSTOM_FUNCTION`) already reliably give.

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

**Signals checked**: >10 user stories? No (1). >3 bounded contexts/modules? No — the existing
rule-import validation pipeline only (same module family as every prior CEL-parity epic); zero new
crate, zero new bounded context, zero new dependency edge. Walking skeleton >5 integration points?
No (1: import → real rejection response). Estimated effort >2 weeks? No — a detection-reliability
fix scoped to already-existing constructs and an already-existing test, well under a day. Multiple
independent user outcomes? No — "chaining is named reliably, everywhere it should apply" is one
coherent outcome, not several independently-shippable ones.

**Scope Assessment: PASS** (0 signals fired) — right-sized as a single, small story; the smallest
CEL-parity-family feature in this initiative (smaller than `security-rules-cel-functions`, which
still touched a new scanning mechanism — this feature touches only detection/naming reliability for
an ALREADY-EXISTING construct and an ALREADY-EXISTING construct tag).

## Wave: DISCUSS / [REF] System Constraints

- Whatever fix DESIGN designs must not touch the already-correct detection/naming for the 4
  already-shipped constructs this same file's own tests prove work today: `NESTED_PATH`,
  `CUSTOM_FUNCTION`, `SYNTAX_ERROR`, `CONFLICTING_VERB_CONDITIONS` (all 4 have their own passing
  test in `cp04_reject_out_of_scope_imports.rs`).
- Must not touch the already-correct SUPPORTED-construct paths: recursive wildcards
  (`security-rules-cel-recursive-wildcards`), fixed-depth multi-segment patterns
  (`security-rules-cel-path-matching`), the single-level `get()`/`exists()` idiom
  (`security-rules-cel-cross-document-reads`), numeric/whitelist/duration grammar
  (`security-rules-cel-expression-grammar`), and named helper functions
  (`security-rules-cel-functions`) — a naive "reject any `get()` containing another `get()` anywhere
  in the file" fix would risk over-rejecting a file that legitimately uses the single-level idiom
  TWICE (once per collection) rather than a genuine chain; this is named explicitly as AC-CCD-03
  below, not left implicit.
- Whatever code changes, `embyr-core`'s zero-IO boundary (`deny.toml`-enforced) must hold —
  detection/naming of an unsupported syntax shape is pure computation over already-parsed condition
  text, not a new I/O-adjacent capability like `security-rules-cel-cross-document-reads`' own
  Resolution 1.

## Wave: DISCUSS / [REF] Walking Skeleton Strategy

**Strategy A** (real, minimal, end-to-end) — the walking skeleton IS the smallest reproducible case
of the gap itself: a single-block import containing ONLY a chaining `get()`, against a real project,
through the real `POST /admin/v1/projects/:project_id/access_rules/import` endpoint, asserting a
real 400 response naming `UNSUPPORTED_EXPRESSION_GRAMMAR`. The multi-block scenario (the actual
currently-failing test) and the regression guard are both proven on top of the same walking skeleton
mechanism, not independent slices — this feature is too small to warrant a multi-release split.

## Wave: DISCUSS / [REF] User Story

### US-01: A Chaining `get()` Is Reliably Named, Alone Or Alongside Other Offending Blocks

**job_id**: JOB-17 | **Release**: 1 (Walking Skeleton) | **Persona**: P1 Alex

#### Elevator Pitch
Before: Alex writes or imports a `.rules` file containing a chaining `get()` — a `get()` whose own
path expression is built from ANOTHER `get()`'s own result (e.g.
`get(/databases/$(database)/documents/orgs/$(get(/databases/$(database)/documents/users/
$(request.auth.uid)).data.orgId))`). Whether Alex is warned this construct is unsupported depends,
unpredictably, on what else is in the same file: cp04's own multi-block test proves that when the
chaining block sits alongside other offending blocks (a wildcard-at-collection-position block, a
custom-function-call block) in one import, only 2 of the 3 offending blocks are named in the
rejection response — the chaining block goes unmentioned.
After: Alex imports that same file via `POST /admin/v1/projects/:project_id/access_rules/import`
and receives a 400 `IMPORT_REJECTED` response whose `offending_blocks` array reliably names the
chaining block with `construct: "UNSUPPORTED_EXPRESSION_GRAMMAR"` — both when it is the ONLY
offending construct in the file, and when it appears alongside any other offending construct in the
same import.
Decision enabled: Alex knows, definitively and before ever issuing a real `GetDocument`/
`CreateDocument`/`UpdateDocument` call in production, that this specific rule block needs rewriting
to the single-level `get()`/`exists()` idiom embyr already supports — instead of discovering an
unpredictable runtime failure later, or wrongly believing the block was accepted because the
response happened not to mention it.

#### Domain Examples
1. **Sole offending construct** — Alex imports a `trailmark-prod` `.rules` file whose ONLY `match`
   block condition is `exists(/databases/$(database)/documents/orgs/$(get(/databases/$(database)/
   documents/users/$(request.auth.uid)).data.orgId))` on collection `journal_entries`. The import is
   rejected 400, `offending_blocks` has exactly 1 entry, `construct: "UNSUPPORTED_EXPRESSION_GRAMMAR"`.
2. **Multi-block, the exact cp04 scenario** — Alex imports a file with 3 blocks: a
   wildcard-at-collection-position block (`/{expeditionId}/journal_entries`), a
   `allow write: if isEditor();` block on `trail_guides`, and the same chaining `get()` block on
   `journal_entries` from example 1. The rejection response's `offending_blocks` names all 3, with
   constructs `NESTED_PATH`, `CUSTOM_FUNCTION`, and `UNSUPPORTED_EXPRESSION_GRAMMAR` respectively.
3. **Regression guard — the already-supported single-level idiom** — Alex imports a file whose only
   `match` block condition is CDR01's own proven idiom,
   `exists(/databases/$(database)/documents/organizations/$(request.auth.uid))` on collection
   `memberships`. The import succeeds 200, the rule is stored, and a real `GetDocument` call from an
   authenticated member of that organization succeeds while one from a non-member is denied — this
   construct is NOT chaining (its own path is built only from `request.auth.uid`, an already-known
   binding) and must never be caught by whatever fix closes this feature's own gap.
4. **Atomicity with an in-scope sibling block** — Alex imports a file naming `profiles` (a plain,
   in-scope, ownership-equality block: `allow read, write: if request.auth.uid == userId;`) together
   with the same chaining `get()` block from example 1 on `journal_entries`. The whole import is
   rejected 400; `profiles` receives no rule either, mirroring the existing zero-partial-application
   guarantee (AC-17-192) already proven for `NESTED_PATH`.

#### UAT Scenarios (BDD)

##### Scenario: A chaining `get()` is the only construct in the file and is named on rejection
```gherkin
Given project "trailmark-prod" exists with no rules imported yet
When Alex imports a rules file whose only match block condition is a chaining get() call
  (a get() whose own path is built from another get()'s own result)
Then the import is rejected with HTTP 400 and reason "IMPORT_REJECTED"
And the offending_blocks array names exactly 1 block
And that block's construct is "UNSUPPORTED_EXPRESSION_GRAMMAR"
And no rule is stored for that block's collection
```

##### Scenario: A chaining `get()` is named alongside other offending blocks in one rejection
```gherkin
Given project "trailmark-prod" exists
When Alex imports a rules file containing a wildcard-at-collection-position block,
  a custom-function-call block, and a chaining get() block, together in one file
Then the import is rejected with HTTP 400
And the offending_blocks array names all 3 blocks, not just 2
And the constructs named include "NESTED_PATH", "CUSTOM_FUNCTION", and "UNSUPPORTED_EXPRESSION_GRAMMAR"
```

##### Scenario: The already-supported single-level get()/exists() idiom keeps importing successfully
```gherkin
Given project "trailmark-prod" exists
When Alex imports a rules file using the already-supported single-level exists() role-lookup idiom
  (a path built only from request.auth.uid, never from another get()'s own result)
Then the import succeeds with HTTP 200
And the rule is stored
And a real GetDocument call from a matching authenticated caller succeeds
And a real GetDocument call from a non-matching authenticated caller is denied
```

##### Scenario: A rejected import naming a chaining block leaves an in-scope sibling block unstored
```gherkin
Given project "trailmark-prod" exists and collection "profiles" has no rule yet
When Alex imports a rules file naming profiles (an in-scope, ownership-equality block)
  together with a chaining get() block on a different collection
Then the whole import is rejected with HTTP 400
And profiles receives no rule either, matching the existing zero-partial-application guarantee
```

##### Scenario: No other CEL-parity construct detection regresses
```gherkin
@property
Given the full existing acceptance-test baseline for security_rules_cel_parity,
  security_rules_cel_path_matching, security_rules_cel_expression_grammar,
  security_rules_cel_cross_document_reads, security_rules_cel_recursive_wildcards,
  and security_rules_cel_functions
When this feature's own fix is applied
Then every currently-passing test in all 6 suites remains green
And no previously-accepted construct becomes wrongly rejected
And no previously-rejected construct becomes wrongly accepted
```

#### Acceptance Criteria
- [ ] AC-CCD-01: a rules file whose only offending construct is a chaining `get()` is rejected 400
      `IMPORT_REJECTED`, naming exactly 1 offending block with `construct: "UNSUPPORTED_EXPRESSION_GRAMMAR"`.
- [ ] AC-CCD-02: `multiple_offending_blocks_are_all_named_in_a_single_rejection_response`
      (`tests/security_rules_cel_parity/acceptance/cp04_reject_out_of_scope_imports.rs`) passes —
      `offending_blocks.len() == 3`, including `NESTED_PATH`, `CUSTOM_FUNCTION`, AND
      `UNSUPPORTED_EXPRESSION_GRAMMAR`.
- [ ] AC-CCD-03: a rules file using the already-supported single-level `get()`/`exists()` idiom
      (path built only from `request.auth.uid`/a path variable, never from another `get()`'s own
      result) continues to import successfully and enforce correctly — a regression guard against a
      naive "reject any `get()` nested inside another `get()`-bearing file" over-rejection.
- [ ] AC-CCD-04: an in-scope block named in the same file as a rejected chaining block receives no
      rule (zero partial application), mirroring the existing AC-17-192 guarantee.
- [ ] AC-CCD-05 (`@property`): the full existing acceptance-test suites for
      `security_rules_cel_parity`, `security_rules_cel_path_matching`,
      `security_rules_cel_expression_grammar`, `security_rules_cel_cross_document_reads`,
      `security_rules_cel_recursive_wildcards`, and `security_rules_cel_functions` remain 100% green
      after this feature's fix — zero regression to any already-correct construct detection
      (`NESTED_PATH`, `CUSTOM_FUNCTION`, `SYNTAX_ERROR`, `CONFLICTING_VERB_CONDITIONS`, recursive
      wildcards, fixed-depth patterns, the single-level `get()`/`exists()` idiom, numeric/whitelist/
      duration grammar, named helper functions).

## Wave: DISCUSS / [REF] Outcome KPIs

### Feature: security-rules-cel-chaining-detection

### Objective
Close the last remaining known gap in `docs/product/known-gaps.md` — make chaining-`get()` rejection
reliable in every import shape, not just isolated ones — completing this session's full 8-gap
production-readiness/parity sweep.

### Outcome KPIs
| # | Who | Does What | By How Much | Baseline | Measured By | Type |
|---|---|---|---|---|---|---|
| 1 | Alex (P1, SDK Developer / rules author) | Receives a reliably-named `UNSUPPORTED_EXPRESSION_GRAMMAR` rejection for a chaining `get()` at import time | 100% of chaining-construct imports named as offending, both isolated and multi-block | Multi-block case fails today (2 of 3 offending blocks named, per cp04's own failing assertion) | `cp04_reject_out_of_scope_imports.rs::multiple_offending_blocks_are_all_named_in_a_single_rejection_response` + a new single-block chaining test, both green | Leading |

### Metric Hierarchy
- **North Star**: 0 silently-unnamed offending constructs in any rejected import response, across
  every import shape (single-block, multi-block).
- **Leading Indicators**: AC-CCD-01/02 (the two direct detection-reliability scenarios).
- **Guardrail Metrics**: AC-CCD-03 (no over-rejection of the already-supported idiom), AC-CCD-05
  (zero regression across the 6 named CEL-parity suites).

### Hypothesis
We believe that making `UNSUPPORTED_EXPRESSION_GRAMMAR` detection consistent across single- and
multi-block imports for Alex (SDK Developer authoring real `.rules` files) will close the last
production-readiness gap this session's scan found.
We will know this is true when Alex's own multi-block import test names all 3 offending blocks,
every time, with zero regression to any of the 8 other already-correct construct detections this
codebase ships today.

## Wave: DISCUSS / [REF] Definition of Done

- All 5 UAT scenarios pass (green), including the previously-failing
  `multiple_offending_blocks_are_all_named_in_a_single_rejection_response`.
- Full baseline re-run of all 6 named CEL-parity suites (`security_rules_cel_parity`,
  `security_rules_cel_path_matching`, `security_rules_cel_expression_grammar`,
  `security_rules_cel_cross_document_reads`, `security_rules_cel_recursive_wildcards`,
  `security_rules_cel_functions`) is clean.
- Unit tests for any new/changed pure detection logic written during DELIVER; mutation-testing pass
  per this project's `per-feature` strategy.
- Code reviewed and merged to master.
- `docs/product/known-gaps.md` row 8 updated to CLOSED at FINALIZE, with a link to this feature's own
  evolution doc.
- Story demoable in a single session: one `curl`/HTTP call showing the multi-block rejection now
  names all 3 constructs.

## Wave: DISCUSS / [REF] Out of Scope

- **Making chaining `get()` actually WORK** — a deliberate, evidenced non-goal locked by
  `security-rules-cel-cross-document-reads`' own DISCUSS Resolution 2 and ADR-066. This feature is
  ONLY about detecting and cleanly rejecting the construct, never evaluating it.
- **N-level or general chaining semantics** (real Firestore's own 10/20-read-budget-bounded chaining
  capability) — unchanged, permanently deferred per ADR-066, no candidate feature id assigned.
- **Any other currently-passing construct's own detection logic** — `NESTED_PATH`, `CUSTOM_FUNCTION`,
  `SYNTAX_ERROR`, `CONFLICTING_VERB_CONDITIONS`, and every already-supported construct
  (recursive wildcards, fixed-depth patterns, single-level `get()`/`exists()`, numeric/whitelist/
  duration grammar, named helper functions) — out of bounds, guarded explicitly by AC-CCD-03/05.
- **Root-cause diagnosis of the underlying implementation defect** — explicitly DESIGN's own task
  (§ Job Discovery Framing Resolution's own working hypothesis is a hypothesis, not a finding).
- **Re-opening any part of the 13 prior JOB-17 realizations' own already-shipped scope** — done, out
  of bounds.

## Wave: DISCUSS / [REF] Driving Ports

Admin HTTP `:9090` — the existing `POST /admin/v1/projects/:project_id/access_rules/import` route,
reused unchanged from every prior CEL-parity epic's own driving port (cp01's own fixture,
`SecurityRulesAdminContext`). No new admin route, no new RPC, no new port/adapter trait method.

## Wave: DISCUSS / [REF] Pre-requisites

- `security-rules-cel-cross-document-reads` (closes ADR-066) — establishes the
  `UNSUPPORTED_EXPRESSION_GRAMMAR` construct tag and locks Resolution 2 (single-level only, no
  chaining) as the reason a chaining `get()` must be rejected at all.
- `security-rules-cel-parity` / `security-rules-cel-path-matching` — establish the `decompose`/
  offending-block collection mechanism across multiple `match` blocks in one file, the exact
  mechanism this feature's own multi-block scenario (AC-CCD-02) depends on behaving consistently.
- `security-rules-cel-functions` — most recent sibling feature; establishes current
  `rules_file.rs` parsing/detection call-chain shape DESIGN will need to re-read for this feature's
  own root-cause diagnosis, even though its own JOB-17 NOTE was never applied to `jobs.yaml` (flagged,
  not fixed, in § Reading Confirmation above).
- No new external dependency, no new bounded context, no new dependency edge.

## Wave: DISCUSS / [REF] Definition of Ready Validation

### DoR Checklist (9-item hard gate)
| # | DoR Item | Status | Evidence |
|---|---|---|---|
| 1 | Every story traces to a `job_id` | PASS | US-01 → JOB-17, reused, real value story (not `infrastructure-only`) |
| 2 | Every non-`@infrastructure` story has a complete Elevator Pitch | PASS | US-01 § Elevator Pitch (Before/After/Decision enabled) |
| 3 | Problem statement clear, domain language | PASS | § Business Context — no technical jargon, framed as Alex's own trust-in-the-rejection-mechanism problem |
| 4 | User/persona identified with specific characteristics | PASS | P1 Alex, SDK Developer, fully profiled across 13 prior JOB-17 realizations |
| 5 | 3+ domain examples with real data | PASS | 4 domain examples, `trailmark-prod`, real collection/field names, real condition text |
| 6 | UAT in Given/When/Then (3–7 scenarios) | PASS | 5 scenarios (§ UAT Scenarios) |
| 7 | AC derived from UAT, testable | PASS | AC-CCD-01 through 05, each maps 1:1 to a scenario above |
| 8 | Right-sized (1–3 days, 3–7 scenarios) | PASS | Scope Assessment PASS, 5 scenarios, single walking-skeleton story |
| 9 | Dependencies resolved or tracked | PASS | § Pre-requisites — all 3 named prior features already FINALIZED |

### DoR Status: **PASSED**

## Wave: DISCUSS / [REF] Open Questions

None carried forward unresolved as a requirements question. One item is explicitly escalated to
DESIGN as a hypothesis, not a decision (§ Job Discovery Framing Resolution): the actual root cause of
why the chaining block is dropped from the multi-block response, which this DISCUSS deliberately does
not diagnose.

## Wave: DISCUSS / [REF] SSOT Updates

`docs/product/jobs.yaml`, JOB-17 entry: append a new dated NOTE (mirroring this job's own established
NOTE-append convention) recording this feature's own scope — reliable, consistent detection/naming of
a chaining `get()` construct across single- and multi-block imports, closing gap #8 of
`docs/product/known-gaps.md`, the last gap in this session's full 8-gap sweep. Applied in this same
DISCUSS pass (see the NOTE itself in `jobs.yaml`).

`docs/product/known-gaps.md`, row 8: Status column updated to "IN PROGRESS —
`security-rules-cel-chaining-detection`" (not closed — CLOSED happens at this feature's own FINALIZE).
Applied in this same DISCUSS pass.

## Wave: DISCUSS / [REF] Wave Decisions Summary

### Key Decisions
- [D1] Reuse JOB-17, 14th realization, real value story (not infrastructure-only) — Alex gets a
  reliable, trustworthy rejection signal, a genuine user-facing outcome.
- [D2] One story, one release, one walking skeleton — the smallest CEL-parity-family feature in this
  initiative; Scope Assessment PASS with 0 signals fired.
- [D3] Chaining itself remains a deliberate non-goal (ADR-066 Resolution 2, not re-litigated) — this
  feature is scoped strictly to detection/naming reliability, never evaluation.
- [D4] Root-cause diagnosis is explicitly deferred to DESIGN — DISCUSS states a working hypothesis
  (an aggregation/orchestration gap across multiple `match` blocks, not necessarily a total absence
  of single-condition detection) without confirming it by reading implementation.

### Requirements Summary
- Primary need: the last known gap in this session's production-readiness/parity sweep — a chaining
  `get()` is not reliably named as an offending construct when it shares a file with other offending
  blocks.
- Walking skeleton scope: one chaining `get()` block, alone, correctly rejected and named.
- Feature type: Backend.

### Constraints Established
- Must not regress any of the 4 already-correct rejection constructs or 5 already-supported
  constructs this same test suite family already proves (§ System Constraints).
- `embyr-core`'s zero-IO boundary (`deny.toml`) must hold — this is pure computation over
  already-parsed condition text.

### Upstream Changes
- None — this feature is a reliability fix for an already-decided, already-partially-implemented
  construct (`UNSUPPORTED_EXPRESSION_GRAMMAR`), not a scope expansion.

## Wave: DISCUSS / [REF] Next Wave

**Handoff To**: nw-solution-architect (DESIGN wave)
**Deliverables**: this feature-delta.md, the working hypothesis in § Job Discovery Framing
Resolution (explicitly unconfirmed — DESIGN must verify against real implementation before designing
a fix), 1-story/5-scenario walking-skeleton plan.
