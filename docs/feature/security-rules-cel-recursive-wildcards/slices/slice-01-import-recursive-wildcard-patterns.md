# Slice 01: Alex Imports a Real File Containing an Even-Prefix Recursive-Wildcard Pattern

**Story**: US-01 | **Release**: 1 | **Walking Skeleton**: Yes | **Estimate**: 2 days

## Goal
Widen 4a's/4b's own `rules_file` outer-syntax scanner and decomposition to accept a terminal, even-prefix recursive-wildcard segment (`{name=**}`), decomposing it into a new fixed-prefix-plus-open-remainder shape, distinct from 4b's own fixed-depth `DecomposedPatternRule`.

## IN Scope
- Widen `validate_segment_shape` to accept `PathSegment::RecursiveWildcard` ONLY as the final segment, and only when the preceding segments have even total length.
- Reject a recursive wildcard at an odd-prefix position (`RECURSIVE_WILDCARD_ODD_PREFIX` or DESIGN's own equivalent naming).
- Reject a recursive wildcard that is not the final segment (`RECURSIVE_WILDCARD_NOT_TERMINAL` or DESIGN's own equivalent naming).
- Produce a new decomposition-target shape: fixed-prefix segments (possibly empty, for a project-wide `{document=**}`) + `read_condition`/`write_condition`, additive to 4b's own `DecomposedTarget` enum.
- Every captured wildcard segment WITHIN the fixed prefix (e.g. `{expeditionId}`) retained by its own name, reusing 4a's/4b's own leaf/ancestor capture mechanism unchanged.
- Idempotent re-import (mirrors 4a's/4b's own precedent).
- Import composes cleanly with 4a's/4b's own shapes in the same file.

## OUT Scope
- Odd-prefix recursive wildcards (deferred, unevidenced, no candidate feature id yet assigned).
- Condition-grammar referencing of the captured remainder (deferred to 4c's own scope note).
- Runtime routing/precedence (Slice 02).
- Precedence-tie/overlap detection against other patterns (Slice 04).
- The storage target itself (DESIGN's call).

## Learning Hypothesis
Disproves: an even-prefix, terminal recursive-wildcard pattern cannot be parsed and decomposed into a coherent, storable prefix-plus-remainder shape without either a scanner rewrite or losing the ability to reuse 4b's own `PathSegment`/`positions_compatible` foundation.
Confirms (if it succeeds): 4b's own scanner already recognizes `RecursiveWildcard` as a distinct segment kind (confirmed by direct read — `parse_one_segment` already classifies it); only `validate_segment_shape`'s own unconditional rejection needs to change.

## Acceptance Criteria
AC-17-232 through AC-17-237 (see `feature-delta.md` § User Stories, US-01).

## Dependencies
`security-rules-cel-path-matching` (4b) — `crates/embyr-core/src/access_control/rules_file.rs`, `path_routing.rs`, shipped and confirmed by direct read.

## Effort Estimate
2 days.

## Reference Class
Mirrors 4b's own Slice 01 (widening `decompose_block`'s own shape allow-list one function at a time) — this slice extends the SAME parser file, not a new module.

## Pre-Slice SPIKE
Not required — direct code read (Reading Confirmation) already confirmed `PathSegment::RecursiveWildcard` exists and is already scanned; only `validate_segment_shape`'s own match arm needs widening. Uncertainty is low for the odd/even-prefix parity rule (derived directly from `DocumentPath`'s own always-even-total-length invariant, § Job Discovery Framing Resolution, Resolution 1).
