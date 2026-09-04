# Mutation Testing Report — security-rules-cel-cross-document-reads

**Tool**: cargo-mutants 27.0.0
**Date**: 2026-09-04
**Scope**: `--in-diff` against this feature's own full diff of
`crates/embyr-core/src/access_control/mod.rs` (Slices 01–05 combined,
`ea51e44~1..HEAD`) — precise, not a hand-picked function list: every mutant
this feature's own new/changed lines produce, nothing from the 4 prior
JOB-17 epics' pre-existing logic in the same file.

Unlike `security-rules-cel-expression-grammar` (4c), unit tests were written
DURING each slice (Slice 01's own 25-test batch, applying 4c's own QUALITY_GATE
lesson proactively — see `feedback_mutation_testing_docker_contention.md`-
adjacent session learning), not deferred to a post-hoc pass.

## Pass 1 — `embyr-core`, initial run

`cargo mutants -p embyr-core --in-place --timeout 30 --in-diff <feature-diff> -- access_control::`

**Result**: 95 mutants — **78 caught, 8 missed, 7 unviable, 2 timeouts.**

### The 8 misses — all narrow boundary/exemption gaps, not logic errors

The per-slice unit tests already covered the HAPPY-path shape of every new function
(`discover_cross_document_paths`, `resolve_path_template`, `parse_path_template`, the
tokenizer's `exists(`/`get(` branches, `starts_with_at`, `find_matching_paren`). The misses were
all exact-boundary or narrow-exemption cases no existing test happened to exercise:

- `detect_unsupported_construct`: the `get`/`exists` exemption (Slice 01) had no test proving a
  GENERIC (non-`get`/`exists`/`duration.value`) function call is still rejected — the exemption
  could have silently widened into a blanket bypass without any test noticing.
- `starts_with_at`: no test pinned the exact-length boundary (needle ending PRECISELY at the end
  of `chars`, neither short nor overflowing).
- `find_matching_paren` / `tokenize`'s `get(...).data.<field>` scan: no test drove an unterminated
  `exists(...)` construct, or a field name reaching the absolute end of the condition string —
  both loop-bounds edge cases only visible at the very last character.
- `parse_path_template`: no test distinguished the exact-prefix-only case's OWN error message
  (`"...at least one collection/document segment"`) from the too-short-prefix case's message, and
  no test exercised a WELL-FORMED (cleanly parenthesized) `$(...)` substitution whose content is
  neither `request.auth.uid` nor `request.path.<var>` — the one existing negative test
  (`a_substitution_beyond_auth_uid_or_path_variable_is_a_named_rejection`) only exercised the
  MALFORMED-substitution fallback (a chained `get()` never cleanly strip_prefix/suffix's).
- `walk_operand_for_cross_document_paths`: the `Arithmetic`/`ListLiteral` recursion arms had no
  test proving a cross-document operand NESTED inside either shape is still discovered.

### Fix — 9 new unit tests closing each gap directly

Added alongside the existing Slice 01 unit-test batch, one test per bullet above (see
`crates/embyr-core/src/access_control/mod.rs`, `mod tests`).

## Pass 2 — final confirmation

**Result**: 95 mutants — **86 caught, 0 missed, 7 unviable, 2 timeouts.**

The 2 timeouts (`find_matching_paren`/`tokenize`'s own index-increment `i += 1` mutated to
`i *= 1`) are both an infinite loop against a 30s harness timeout, not a silently-passing mutant —
observably broken, just reported by the tool as TIMEOUT rather than CAUGHT. Treated as effectively
caught for this gate's own purposes (mirrors how a panic-producing mutant is treated as caught even
though it isn't a clean assertion failure).

**Effective kill rate: 100% of viable, non-hanging mutants (86/86); the 2 remaining are
hangs, not survivors.**

## `embyr-server` (Docker/testcontainers-backed) — not run, same documented skip precedent

Following `security-rules-cel-expression-grammar`'s own established precedent: this feature's
own write/simulate handler wiring (`crates/embyr-server/src/grpc/handler.rs`,
`crates/embyr-server/src/admin/handlers/access_rules.rs`) is covered by its own 12-test
acceptance suite (`tests/security_rules_cel_cross_document_reads/`, cdr01–cdr05, all passing,
exercising every `evaluate()`/`discover_cross_document_paths`/`fetch_cross_document_reads`
call-site wiring decision individually per-slice). A Docker-backed mutation pass was not attempted
this pass, consistent with the prior feature's documented cost/benefit reasoning.

## Overall verdict

**PASS.** 100% effective kill rate on every mutant this feature's own diff produced, after
closing 8 narrow boundary/exemption gaps with 9 new unit tests. Full `security_rules_*` baseline
re-run clean after adding the unit tests.

Proceeding to FINALIZE.
