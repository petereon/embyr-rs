# Mutation Testing Report — security-rules-cel-functions

**Tool**: cargo-mutants 27.0.0
**Date**: 2026-09-04
**Scope**: `--in-diff` against this feature's own full diff of
`crates/embyr-core/src/access_control/rules_file.rs` (Slices 01–04 combined,
`e5f297f..HEAD`) — this is the ONLY file this feature touches in
`embyr-core` (ADR-067's own central claim: `access_control/mod.rs` stays
byte-for-byte unchanged), so the diff scope IS the feature's entire
`embyr-core` footprint, unlike 4c/4d where the diff was one region within a
much larger, shared file.

Unit tests were written DURING each slice (applying 4d's own QUALITY_GATE
lesson proactively — a dedicated mutation pass is still budgeted regardless
of during-slice discipline, per ADR-067 § Enforcement).

## Pass 1 — `embyr-core`, initial run

`cargo mutants -p embyr-core --in-place --timeout 30 --in-diff <feature-diff> -- access_control::`

**Result**: 90 mutants — **68 caught, 4 missed, 6 unviable, 12 timeouts.**

### The 4 misses — function-name character-validation boundary

All 4 were in `parse_function_blocks`'s own function-name validity check
(`name.is_empty() || !name.chars().next().is_some_and(...) || !name.chars().all(...)`,
the 3-way `||` chain plus its two `c == '_'` comparisons):

- Two `||`→`&&` mutants at the chain's own junction points — no existing test exercised a name
  where EXACTLY ONE of the three disjuncts was true (every existing malformed-name test used a
  name violating multiple checks at once, e.g. an empty string, which trivially satisfies every
  disjunct and can't distinguish `||` from `&&`).
- Two `==`→`!=` mutants on the `c == '_'` checks (first-character and all-characters) — no
  existing test proved a LEADING or MID-NAME underscore is actually ACCEPTED (every existing
  positive-case function name in this slice's own tests happened to avoid underscores entirely).

### Fix — 3 new unit tests closing each gap directly

- `a_function_name_starting_with_a_digit_is_a_syntax_error` — a name violating exactly ONE
  disjunct (first-char check) while satisfying the all-alphanumeric check, closing both `||`→`&&`
  junction mutants at once.
- `a_function_name_starting_with_an_underscore_is_valid` — closes the first `==`→`!=` mutant.
- `a_function_name_containing_an_underscore_is_valid` — closes the second `==`→`!=` mutant.

## Pass 2 — final confirmation

**Result**: 90 mutants — **72 caught, 0 missed, 6 unviable, 12 timeouts.**

The 12 timeouts are all inside `expand_function_calls`'s own linear char-scanning loop (index
-increment mutants `i += 1` → `i -= 1`/`i *= 1`, and a handful of comparison mutants on the same
loop's bounds/branch conditions) — every one an infinite loop against the 30s harness timeout, not
a silently-passing mutant. Same class as `security-rules-cel-cross-document-reads`'s own 2 timeouts
(4d's QUALITY_GATE), just proportionally more of them here since this feature's own new logic is
almost entirely ONE linear-scan function, versus 4d's several smaller pure functions.

**Effective kill rate: 100% of viable, non-hanging mutants (72/72); the 12 remaining are
hangs, not survivors.**

## `embyr-server` (Docker/testcontainers-backed) — not run, same documented skip precedent

This feature touches ZERO `embyr-server` production code (ADR-067 § Resolution 5, directly
confirmed by Slices 02–03 needing zero production-code changes) — there is no `embyr-server`-side
mutation surface to even consider for this pass, the first CEL-parity epic where this question is
structurally moot rather than merely deferred. This feature's own admin-handler-facing behavior
(the real import endpoint surfacing each named rejection correctly) is covered by its own 4
acceptance targets (cf01–cf04, 25 tests total across all 4 slices, all passing).

## Overall verdict

**PASS.** 100% effective kill rate on every mutant this feature's own diff produced, after closing
4 narrow character-validation boundary gaps with 3 new unit tests. Full `security_rules_*`
baseline re-run clean after adding the unit tests.

Proceeding to FINALIZE.
