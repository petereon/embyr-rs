# Slice 01: Alex's Named Helper Function Parses and Enforces on Reads (Walking Skeleton)

**Story**: US-01 | **Release**: 1 | **Estimate**: 1 day

## Goal
Build the import-time text-substitution mechanism end-to-end for the simplest case: one
zero-parameter function, one call site, gating a real `GetDocument` call.

## IN Scope
- `parse_function_blocks(service_body) -> (BTreeMap<String, String>, &str)` — scans the leading
  portion of `service_body` for `function <name>() { return <expr>; }` blocks, reusing
  `find_matching_close`. Validates each body via unmodified `parse_condition`.
- `expand_function_calls(condition_text, functions, path_pattern) -> Result<String, RulesFileError>`
  — quote-aware linear scan splicing `(<body>)` in place of each `<name>()` call site.
- Threading: `parse_rules_file` → `parse_match_blocks` → `parse_nested_match_blocks` →
  `parse_block_body` → `parse_allow_clauses`, mirroring `full_path_pattern`'s own existing thread.
- `UNDEFINED_FUNCTION` named rejection (a call to a name matching no defined function).
- Wired into the real import → `GetDocument` path only this slice.

## OUT Scope
- Write-path, simulation parity proofs (Slices 02–03) — should already work per ADR-067's own
  structural argument, but the proof is those slices' own concern.
- Multiple functions, multiple call sites, parameters/nesting/duplicate-name rejections (Slice 04).

## Learning Hypothesis
Disproves: a pure import-time text-substitution mechanism cannot correctly splice a function's own
body into a call site's condition text without either corrupting a string literal or breaking
operator precedence.

## Acceptance Criteria
AC-CF-01 through AC-CF-04 (see `feature-delta.md` § User Stories, US-01).

## Dependencies
None — first slice.

## Effort Estimate
1 day.

## Reference Class
New mechanism (text-level AST-free substitution), nearest reference class: `rewrite_path_variable`'s
own existing text-rewrite precedent in the same file, widened from a single-token rewrite to a
whole-expression splice.

## Pre-Slice SPIKE
Not required — the exact splice point (`parse_allow_clauses`'s own `condition_text`) and the exact
brace-matching helper (`find_matching_close`) were both confirmed by direct code read during DESIGN
(ADR-067 § Reading Confirmation).
