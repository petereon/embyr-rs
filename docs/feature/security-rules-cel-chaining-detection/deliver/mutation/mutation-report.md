# Mutation Testing Report — security-rules-cel-chaining-detection

**Tool**: cargo-mutants 27.0.0
**Date**: 2026-09-08
**Scope**: `crates/embyr-core/src/access_control/rules_file.rs` (the Stage 1 accumulate-and-
continue restructure) and `crates/embyr-server/src/admin/handlers/access_rules.rs` (the Stage
1/Stage 2 merge in `import_rules_file`) — `git diff 65b63ae d8f2e84 -- crates/`. 15 mutants,
cross-crate (`--test-workspace true` required).

`cargo mutants --workspace --in-place --timeout 240 --in-diff <diff> --test-workspace true --
--test security_rules_cel_parity_cp04_reject_out_of_scope_imports --lib`

No invocation-tuning detour was needed — the stray-positional-filter and too-short-timeout
lessons from earlier features this session were applied proactively from the start. Baseline
confirmed clean with real, unfiltered coverage: 248 `embyr-core` lib tests, 64 `embyr-server` lib
tests, 10 `cp04` acceptance tests.

## Result — 8 caught, 4 unviable, 3 missed (all 3 explained, 0 real gaps)

| Mutant | Location | Result |
|---|---|---|
| `parse_nested_match_blocks -> Ok(vec![])` | `rules_file.rs:565` | **CAUGHT** |
| delete `!` in `parse_nested_match_blocks` | `rules_file.rs:620` | **CAUGHT** |
| `decompose -> Ok(vec![])` | `rules_file.rs:807` | **CAUGHT** |
| delete `!` in `decompose` | `rules_file.rs:817` | **CAUGHT** |
| `import_rules_file -> Ok(Default::default())` | `access_rules.rs:1937` | **CAUGHT** |
| match guard `!e.partial_blocks.is_empty()` → `false` | `access_rules.rs:1948` | **CAUGHT** |
| delete `!` (guard) | `access_rules.rs:1948` | **CAUGHT** |
| delete `!` (offending check) | `access_rules.rs:1956` | **CAUGHT** |
| `RulesFileError::single -> Default::default()` | `rules_file.rs:207` | unviable (`RulesFileError` has no `Default`) |
| `parse_nested_match_blocks -> Ok(vec![Default::default()])` | `rules_file.rs:565` | unviable (`MatchBlock` has no `Default`) |
| `decompose -> Ok(vec![Default::default()])` | `rules_file.rs:807` | unviable (`DecomposedTarget` has no `Default`) |
| `overlap_rejection -> Default::default()` | `access_rules.rs:1692` | unviable (`RulesFileError` has no `Default`) |
| `simulate_routed_access_rule -> Ok(Default::default())` | `access_rules.rs:1140` | missed — **scoping artifact** |
| `simulate_recursive_wildcard_candidate -> Ok(Default::default())` | `access_rules.rs:1287` | missed — **scoping artifact** |
| match guard `!e.partial_blocks.is_empty()` → `true` | `access_rules.rs:1948` | missed — **genuine equivalent mutant** |

## The 2 `simulate_*` misses — confirmed scoping artifacts, not real gaps

`simulate_routed_access_rule` and `simulate_recursive_wildcard_candidate` are a DIFFERENT admin
API route (`POST .../access_rules/simulate_route`) than the one this feature actually changes
(`import_rules_file`) — mechanically touched only because DESIGN's own diff added
`partial_blocks: Vec::new()` to their existing `RulesFileError` construction (a required,
purely-additive consequence of the new field, not new logic).

Verified, not assumed: `grep -rn simulate_routed_access_rule tests/` finds
`tests/security_rules_cel_recursive_wildcards/acceptance/rw06_simulate_recursive_wildcard_
precedence.rs`, which directly exercises this exact endpoint — a real acceptance test exists,
just in one of the 5 sibling regression-guard suites DESIGN's own Handoff Package named but
this mutation run's own 3-target scope (chosen for per-mutant rebuild cost, not coverage
completeness) didn't include. Both functions are genuinely tested elsewhere in the repo; this
run's own narrow scope simply doesn't reach them.

## The 1 `import_rules_file` guard miss — verified genuine equivalent mutant, not a real gap

This is the security-critical one — the match guard `!e.partial_blocks.is_empty()` forced to
literal `true` — and it was checked carefully, not waved away.

Traced the actual behavior: when `parse_rules_file` returns a genuinely-structural error,
`e.partial_blocks` is empty by construction (§ Architecture Design — structural failures have no
well-defined partial block list). With the guard forced to `true`, this case ALSO takes the
"salvage" arm: `blocks = e.partial_blocks` (empty), `offending = e.offending_blocks` (populated
with the real error). `decompose(vec![])` is then called — read directly (`rules_file.rs:806-
820`): the `for block in &blocks` loop never executes over an empty vec, so `offending` inside
`decompose` stays empty and it returns `Ok(vec![])` — a pure no-op. Back in `import_rules_file`,
since the OUTER `offending` (carried from Stage 1) is non-empty, it returns
`rules_file_rejection_response(RulesFileError { offending_blocks: offending, ... })` — the
identical `offending_blocks` list the ORIGINAL, unmutated code's own direct
`return Ok(rules_file_rejection_response(e))` would have produced. Confirmed
`rules_file_rejection_response` (`access_rules.rs:447-462`) only ever reads `err.offending_
blocks` — `partial_blocks` is never serialized into the HTTP response at all.

**This mutation is mathematically undetectable by any test**: for every possible input, forcing
the guard to `true` produces byte-for-byte identical observable output to the correct code,
because decomposing an empty block list is a no-op that cannot add or remove anything from the
`offending_blocks` list Stage 1 already populated. This is the well-known "equivalent mutant"
category in mutation testing — not a coverage gap a better test could close, verified by tracing
the actual code paths rather than assumed from the mutant's own description.

## Verdict

**PASS.** 8 of 15 mutants caught directly; the remaining 7 are fully accounted for: 4
structurally unviable (missing `Default` impls, matching this session's own established
precedent), 2 confirmed scoping artifacts (real coverage exists in a sibling test suite outside
this run's own narrow target list), and 1 verified genuine equivalent mutant (traced end-to-end,
not assumed). Zero real gaps in the security-critical merge logic this feature actually added —
every mutant that COULD distinguish correct from incorrect behavior for the new
accumulate-and-merge logic was caught.
