# Evolution: security-rules-cel-expression-grammar

**Date:** 2026-09-04
**Feature:** The remaining Firestore security-rules CEL expression surface — numeric literals
(`Integer`/`Double`), relational comparison (`<`/`<=`/`>`/`>=`), `in`/list-literal whitelist
matching, `request.time`, `duration.value(...)`, and a narrowly-scoped `+`/`-` arithmetic operand
pairing (timestamp+duration or matching-numeric-type only, non-nested).
**Job:** JOB-17 (`document-access-control`) — 11th realization, "Epic 4c" (the 2nd of the 3
remaining CEL-parity epics named by `security-rules-cel-parity`'s original DISCUSS split).
**ADRs:** ADR-065 (`docs/product/architecture/adr-065-expression-grammar-numeric-in-list-timestamp-duration.md`)

## Business Context

Closes the single largest remaining category of real Firestore `.rules`-file rejections after
`security-rules-cel-path-matching`/`security-rules-cel-recursive-wildcards` shipped path/wildcard
support: any clause involving a number, a whitelist, or a time window. Before this feature, EVERY
one of these idioms was rejected — numeric literals failed as an undifferentiated `SyntaxError` at
the first digit (no tokenizer branch existed for them at all), relational operators didn't exist
in the grammar, `in`/list-literal syntax didn't exist, and there was no `request.time`/`duration`
operand family.

The central scoping finding (DISCUSS Resolution 1): the 5 sub-surfaces the epic's own inherited
naming bundled together (arithmetic, relational comparison, `in`, list/map literals, numeric
literals, timestamp/duration) are NOT independently shippable in most cases — relational
comparison is useless without numeric literals to compare against, and timestamp/duration freshness
-window checks need arithmetic. Splitting each into its own feature (mirroring how 4d/4e were
isolated) would have produced features with no independently-testable value. The locked resolution:
one feature, 3 dependency-ordered releases (numeric+relational → `in`/list → timestamp+duration),
not 3 separate features.

A second key finding, confirmed by direct code read before any implementation began: `FieldValue`
already had `Integer`/`Double`/`Timestamp`/`Array` — every domain VALUE shape this feature needed
already existed at the storage/wire level. The entire gap was the condition GRAMMAR (tokenizer,
`Operand`, `Condition`, parser, `evaluate()`'s comparison logic) only — zero new storage schema,
zero new admin route, zero new bounded context.

## Key Decisions

| Decision | Verdict |
|---|---|
| One feature, 3 dependency-ordered releases, not 3 separate features (Resolution 1) — relational comparison needs numeric literals; timestamp/duration needs arithmetic, a genuine dependency chain | ADR-065 |
| List literals + `in` locked in scope; map literals and nested map-field traversal locked OUT (Resolution 2) — zero domain evidence for either, and nested traversal is a structurally different, separately-flagged gap (`OQ-CEG-01`) | ADR-065 |
| `request.time` reuses the existing `FieldValue::Timestamp`, threaded as a caller-supplied value via `evaluate()`'s new 7th parameter — mirrors `path_variable_value`'s/`ancestor_path_variable_values`'s own identical zero-new-I/O rollout precedent | ADR-065 |
| Arithmetic locked to `+`/`-` only, non-nested, `Integer+Integer`/`Double+Double`/`Timestamp+Duration` pairs only (Resolution 4) — a strict, evidenced subset designed to widen additively later, never rebuilt | ADR-065 |
| `*`/`/`/`%` and nested arithmetic (`a+b+c`) are NAMED `UNSUPPORTED_CONSTRUCT` rejections, never a bare `SyntaxError` (AC-CEG-19) — the tokenizer rejects the operators directly; `parse_operand` structurally rejects a second consecutive `+`/`-` | ADR-065 |
| `duration.value(...)` is legal ONLY as an arithmetic RHS, checked syntactically (parse-time position), never semantically (runtime type) — consistent with how every other operand's runtime type is never parse-time checked in this grammar | ADR-065 |
| One shared `UnsupportedExpressionGrammar` variant for 5 distinct named-rejection sites (map literals, nested traversal, `in`-against-non-list, unsupported/nested arithmetic, unrecognized duration unit), distinguished by `detail` text | ADR-065 |

## Steps Completed

1. **Slice 01** (US-01, Walking Skeleton) — numeric literals (`IntLiteral`/`DoubleLiteral`) +
   relational comparison operators, real `GetDocument` enforcement.
2. **Slice 02** (US-02) — write-path parity (`CreateDocument`), confirmatory: zero production code
   change needed.
3. **Slice 03** (US-03) — simulation parity, confirmatory: zero production code change needed.
4. **Slice 04** (US-04) — `in` + list literals, real write-path enforcement.
5. **Slice 05** (US-05) — read+simulation parity for `in`, confirmatory: zero production code
   change needed.
6. **Slice 06** (US-06) — `request.time` + `duration.value(...)` + scoped `+`/`-` arithmetic, real
   write-path enforcement. Bug found and fixed during acceptance testing: `compare_relational`'s
   `numeric_value()` helper only handled `Integer`/`Double`, so any `Timestamp` relational
   comparison silently fell closed to `Deny` regardless of actual ordering — added a direct
   `(seconds, nanos)` tuple comparison for the `Timestamp`/`Timestamp` pairing.
7. **Slice 07** (US-07, LAST slice) — read+simulation parity for timestamp/duration.
   `simulate_access_rule` gains a new `request_time: Option<i64>` synthetic-input field;
   `json_value_to_field_value` gains one narrow special case reusing the identical
   `{"t": "TS", "s", "n"}` wire shape `embyr-pg-storage` already uses for real documents.

**QUALITY_GATE** — feature-scoped `cargo-mutants` pass against the entirely-new pure functions
this feature added (`parse_operand`, `parse_base_operand`, `parse_arithmetic_rhs`,
`parse_duration_value`, `parse_list_literal`, `compare_relational`, `numeric_value`,
`DurationUnit::as_seconds`): first run found 45/53 missed — a real, feature-wide gap (all 7 DELIVER
slices were built exclusively against `embyr-server`'s own acceptance suite; zero
`embyr-core`-level unit tests existed for any of this feature's own new logic). Added 25 unit
tests mirroring this file's own established per-function-family pattern; final run: 47/47 viable
mutants caught, 100% kill rate. Full details:
`docs/feature/security-rules-cel-expression-grammar/deliver/mutation/mutation-report.md`.

**Full regression**: 64 `security_rules_*` targets, 511 tests, 0 failures (re-run clean after every
slice and again after the mutation-testing fix).

## Lessons Learned

1. **A feature built exclusively against integration/acceptance tests, with zero unit tests for
   its own new pure logic, can pass every acceptance test while still having near-zero mutation
   coverage at the unit level.** All 34 of this feature's own acceptance tests passed cleanly
   before mutation testing ran — the LOGIC was correct — but the mutation pass (module-filtered to
   `embyr-core`'s own unit tests only, deliberately excluding the crate's slow KDF tests) could
   only ever be caught by unit-level tests, and 45/53 initially had none at all. Acceptance tests
   proving end-to-end correctness and unit tests proving mutation-resistance are not
   substitutes for each other — a feature needs both, even when the underlying logic is provably
   correct by the former alone.
2. **A relational-comparison helper's own "which branch handles which `FieldValue` variant" logic
   needs a test for every variant it's SUPPOSED to handle, not just the variants its acceptance
   tests happen to exercise.** `compare_relational`'s `numeric_value()` fallback silently denied
   every `Timestamp` comparison for one full implementation pass (Slice 06's first acceptance-test
   run) before the bug was caught by the acceptance test itself, not by any unit test — because no
   unit test existed yet at that point. A `Timestamp`-specific branch was added and confirmed by
   both the acceptance test AND (later) direct unit coverage.
3. **A unit test asserting only `Lt`/`Le` for a 4-operator family (`Lt`/`Le`/`Gt`/`Ge`) leaves the
   other 2 operators' own strict-vs-inclusive boundary completely unexercised.** The mutation
   pass's 2nd run caught this precisely: `compare_relational`'s `Timestamp` branch's `Gt`/`Ge` arms
   survived because the first-pass unit test never included an EQUAL-timestamp-pair case — the
   ONLY input shape where `>` and `>=` (or `<` and `<=`) actually diverge. A future test addition
   for any comparison-operator family should include the equal-values case by default, not as an
   afterthought.
4. **Reusing an existing wire-format convention (rather than inventing a new one) for a
   previously-unrepresentable value shape closes a real gap cheaply.** `json_value_to_field_value`
   had no way to represent a `Timestamp` at all before Slice 07 — extending it to recognize the
   SAME `{"t": "TS", "s", "n"}` shape `embyr-pg-storage`'s own storage-layer decoder already uses
   (rather than inventing a second, divergent JSON convention for simulate's own inputs) kept the
   fix to one narrow, well-precedented case.

## Key Files

- `crates/embyr-core/src/access_control/mod.rs` — `Operand::{IntLiteral,DoubleLiteral,
  ListLiteral,RequestTime,Arithmetic,DurationLiteral}`, `CompareOp::{Lt,Le,Gt,Ge}`,
  `Condition::In`, `ArithmeticOp`, `DurationUnit`, `Parser::{parse_operand,parse_base_operand,
  parse_arithmetic_rhs,parse_duration_value,parse_list_literal}`, `compare_relational`,
  `numeric_value`, `evaluate()`'s 7th parameter (`request_time`)
- `crates/embyr-server/src/grpc/handler.rs` — `Some(now)` wired at `CreateDocument`/
  `UpdateDocument`/`GetDocument` (both exact-match and routed-pattern branches) and the Commit
  write-path helper; mechanical `None` at every other call site (out of this feature's own locked
  scope: `DeleteDocument`, `ListDocuments`, `BatchGetDocuments`, Listen's per-event recheck)
- `crates/embyr-server/src/admin/handlers/access_rules.rs` — `SimulateAccessRuleBody.
  request_time`, `json_value_to_field_value`'s `{"t": "TS", ...}` special case
- `docs/product/architecture/adr-065-expression-grammar-numeric-in-list-timestamp-duration.md`
- `tests/security_rules_cel_expression_grammar/acceptance/` — ceg01 through ceg07, 7 acceptance
  targets, 34 tests, shared `common/mod.rs` (re-exports `security_rules_cel_parity`'s own harness)
- `docs/feature/security-rules-cel-expression-grammar/feature-delta.md` — full DISCUSS/DESIGN
  narrative (retained in place, not migrated — this project's established SSOT convention)
- `docs/feature/security-rules-cel-expression-grammar/slices/` — 7 elephant-carpaccio slice briefs
- `docs/feature/security-rules-cel-expression-grammar/deliver/mutation/mutation-report.md`

## Follow-Up Work

- **Epic 4d — `security-rules-cel-cross-document-reads`** — `get()`/`exists()` cross-document
  reads. I/O in the hot request path, needs its own DESIGN pass revisiting BC-4's read-only
  dependency shape on BC-2. Named, deferred, not started. **Next in sequence.**
- **Epic 4e — `security-rules-cel-functions`** — custom `function` definitions and invocation.
  Composes over this feature's own grammar surface once built.
- **`OQ-CEG-01`** — nested map-field traversal (`resource.data.address.city` reaching INTO a
  map-valued field's own key) — a real, structurally different gap this feature does not fix.
  Needs its own DISCUSS on whether `.` in a resource-field position should mean nested-map
  -traversal (matching real Firestore) given the existing grammar's current flat-field treatment.
- **Map literals** (`{"role": "admin"}` as a condition-side operand) — zero domain evidence
  (Resolution 2). Named, deferred, no candidate feature id assigned.
- **`*`, `/`, `%`, and any nested arithmetic expression** — zero domain evidence beyond the one
  evidenced idiom (timestamp+duration). Named, deferred.
- **Real Firestore's `path`-type segment-indexing** (`path[0]`) — carried forward from
  `security-rules-cel-recursive-wildcards`' own scope note, still not built here.
