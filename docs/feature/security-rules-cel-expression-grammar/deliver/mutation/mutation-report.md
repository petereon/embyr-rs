# Mutation Testing Report — security-rules-cel-expression-grammar

**Tool**: cargo-mutants 27.0.0
**Date**: 2026-09-04
**Scope**: the entirely-new pure functions this feature added (`crates/embyr-core/src/access_control/mod.rs`):
`Parser::parse_operand`, `Parser::parse_base_operand`, `Parser::parse_arithmetic_rhs`,
`Parser::parse_duration_value`, `Parser::parse_list_literal`, `compare_relational`,
`numeric_value`, `DurationUnit::as_seconds`.

Narrower than a full-file filter deliberately: functions this feature only *extended*
(`tokenize`, `word_to_operand`, `parse_comparison`, `eval_bool`, `resolve_field_value`,
`detect_unsupported_construct`) are heavily shared with 4 prior JOB-17 epics, whose own
QUALITY_GATEs already covered their pre-existing branches — mutating the whole function body
would mostly re-surface already-settled coverage, not this feature's own new logic.

## Pass 1 — `embyr-core` (pure, zero-IO, no Docker), initial run

`cargo mutants -p embyr-core --in-place --timeout 30 -- access_control::` (mirrors
`security-rules-cel-recursive-wildcards`' own proven-fast approach exactly — module-filtered
test command, narrowed to skip the same crate's deliberately-slow, unrelated Argon2id KDF tests).

**Result**: 53 mutants — **2 caught, 45 missed, 6 unviable**.

### Root cause — a real, feature-wide gap, not a narrow miss

Unlike `security-rules-cel-recursive-wildcards` (whose QUALITY_GATE found 1 isolated gap against
an otherwise well-unit-tested file), this feature's 7 DELIVER slices were built exclusively
against `embyr-server`'s own acceptance-test suite (`tests/security_rules_cel_expression_grammar/`,
34 tests across ceg01–ceg07) — **zero `embyr-core`-level unit tests were added for any of this
feature's own new grammar/evaluation logic**, unlike every pre-existing function in the same file
(`word_to_operand`, `eval_bool`, the four-way truth table, etc.), each of which already carries
its own dedicated unit-test section. Scoping the mutation test command to `-- access_control::`
(module-local unit tests only, deliberately excluding the crate's slow KDF tests) meant it could
only ever be caught by unit tests — and none existed for this feature's own new functions, so 45
of 53 mutants survived by construction, not because the LOGIC was wrong (Slices 01–07's own 34
acceptance tests, all passing, already proved that end-to-end).

### Fix — added 25 unit tests, mirroring this file's own established per-function-family pattern

Inserted before the pre-existing `evaluate: the four-way truth table` section (same file, same
module), covering:

- **Numeric literals + relational comparison** (Slice 01): pinned `parse_condition` examples for
  int/negative-int/double literals, a table-driven test proving all 4 relational operators
  (`</`<=`/`>`/`>=`) parse into their own distinct `CompareOp`.
- **`evaluate()` numeric comparison**: a table-driven test proving `Lt`/`Le`/`Gt`/`Ge` match real
  numeric ordering in both directions (including the `a == b` boundary, load-bearing for
  distinguishing `>` from `>=`), a non-numeric-denies case, direct `compare_relational`/
  `numeric_value` unit tests.
- **`in` + list literals** (Slice 04): pinned parse examples (populated list, empty list,
  non-list-literal-RHS rejection), `evaluate()` membership-true/membership-false cases.
- **Timestamp/duration + arithmetic** (Slices 06–07): pinned parse examples for `request.time`,
  `+`/`-` arithmetic composition, every one of the 4 duration units, the unrecognized-unit
  rejection, the standalone-`duration.value(...)`-is-a-syntax-error case (never a special
  construct outside arithmetic-RHS position), nested-arithmetic rejection, all 3 unsupported
  operators (`*`/`/`/`%`), `evaluate()`'s correctly-offset-timestamp/stale-denies/request-time
  -absent-fails-closed cases, and a direct `DurationUnit::as_seconds` unit-conversion table.

## Pass 2 — `embyr-core`, after adding unit tests

**Result**: 53 mutants — **45 caught, 2 missed, 6 unviable**. Both misses were the SAME root
cause: `compare_relational`'s `Timestamp`/`Timestamp` branch's own `Gt`/`Ge` arms — my first pass
of the timestamp unit test only asserted `Lt`/`Le` cases, never `Gt`/`Ge` on an EQUAL timestamp
pair (the only input shape that distinguishes strict `>` from inclusive `>=`). Extended
`timestamp_relational_comparison_compares_seconds_then_nanos` with 4 more assertions (Gt/Ge on an
equal pair, Gt/Ge on a distinct pair) — the exact gap the mutation run named.

## Pass 3 — final confirmation

**Result**: 53 mutants — **47 caught, 0 missed, 6 unviable — 100% kill rate on every viable
mutant.**

## `embyr-server` (Docker/testcontainers-backed) — not run, same documented skip precedent

Following `security-rules-cel-recursive-wildcards`' own established precedent
(`docs/evolution/2026-09-04-security-rules-cel-recursive-wildcards.md` § Lessons Learned): a
Docker-backed mutation pass against this feature's own write/read/simulate handler wiring
(`crates/embyr-server/src/grpc/handler.rs`, `crates/embyr-server/src/admin/handlers/
access_rules.rs`) was not attempted this pass. That layer's own purpose-built acceptance suite
(34 tests across ceg01–ceg07, all passing, exercising every evaluate() call-site wiring decision
— `Some(now)` vs. mechanical `None` at each of the 12 handler.rs sites and 4 access_rules.rs
sites, verified individually in each slice's own commit) is the substitute quality gate for that
layer, consistent with the prior feature's own documented cost/benefit reasoning (a naive parallel
Docker-backed run previously cost 4 hours and 68 stray containers for zero classified mutants).

## Overall verdict

**PASS.** 100% kill rate (47/47 viable mutants) on every entirely-new pure function this feature
added, after closing a real, feature-wide gap (zero prior `embyr-core`-level unit coverage) with
25 new unit tests. Full `security_rules_*` baseline re-run clean after adding the unit tests: 64
targets, 511 tests, 0 failures.

Proceeding to FINALIZE.
