# Slice 01: Alex Imports a Real File Containing Multi-Segment and Nested-Match-Block Patterns

**Story**: US-01 | **Release**: 1 | **Walking Skeleton**: Yes | **Estimate**: 2.5 days

## Goal
Widen 4a's own `rules_file` outer-syntax scanner and decomposition to accept fixed-depth, multi-segment `match` block path patterns (any number of literal-collection / wildcard-or-literal-document-ID segment pairs) and nested `match { match { ... } } }` outer syntax, decomposing both to the identical internal pattern representation.

## IN Scope
- Widen `decompose_block`'s path-shape allow-list beyond `[Literal]` / `[Literal, Wildcard]` to arbitrary-length, alternating literal-collection / wildcard-or-literal-document-ID segment sequences.
- Flatten nested `match { match { ... } } }` outer syntax to the same pattern representation flat multi-segment syntax produces.
- Reject any segment sequence containing a `RecursiveWildcard` (`{name=**}` or bare `**`), taxonomy unchanged: `RECURSIVE_WILDCARD`.
- Reject any segment sequence that is not valid literal-collection/document-ID alternation (odd total length, a wildcard/literal at a collection-name position), taxonomy: `NESTED_PATH` (reused) or a new, distinguishable reason DESIGN names.
- Every captured wildcard segment retained by its own name, distinguishable within the same pattern.
- Idempotent re-import (mirrors 4a's own AC-17-176).
- Import composes cleanly with 4a's own single-collection/single-wildcard shapes in the same file.

## OUT Scope
- Recursive wildcards (deferred, `security-rules-cel-recursive-wildcards`).
- Runtime routing (Slice 02).
- Structural-overlap detection against other patterns (Slice 04).
- The storage target itself (DESIGN's call — this slice produces `DecomposedRule`-equivalent output; where it's written is Slice 01's own DESIGN-time decision, not locked here).

## Learning Hypothesis
Disproves: a fixed-depth multi-segment/nested-match-block pattern cannot be parsed, decomposed, and represented internally without either widening the existing scanner beyond a one-function shape change, or inventing a new outer-grammar layer.
Confirms (if it succeeds): 4a's own outer-syntax scanner (`parse_rules_file`/`parse_match_blocks`/`parse_path_segments`) is already fully general — only `decompose_block`'s own shape-check needs to change.

## Acceptance Criteria
AC-17-202 through AC-17-206 (see `feature-delta.md` § User Stories, US-01).

## Dependencies
`security-rules-cel-parity` (4a) — `crates/embyr-core/src/access_control/rules_file.rs`, shipped and confirmed by direct read.

## Effort Estimate
2.5 days.

## Reference Class
Mirrors 4a's own Slice 01 (`upsert_access_rule`/`upsert_write_access_rule` decomposition target reuse) — this slice extends the SAME parser file, not a new module.

## Pre-Slice SPIKE
Not required — direct code read (Reading Confirmation) already confirmed `parse_path_segments` is shape-agnostic; only `decompose_block`'s match arm needs widening. Uncertainty is low for parsing, moderate for the collection/document-ID alternation validation rule (new semantic check not present in 4a).
