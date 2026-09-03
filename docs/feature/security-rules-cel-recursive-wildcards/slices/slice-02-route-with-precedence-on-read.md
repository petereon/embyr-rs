# Slice 02: A Concrete Path Is Routed to the Single Most-Specific Applicable Pattern (Walking Skeleton)

**Story**: US-02 | **Release**: 1 | **Walking Skeleton**: Yes | **Estimate**: 3 days

## Goal
Build a new pure primitive (working name `bind_recursive_prefix`) that matches a recursive-wildcard pattern's own fixed prefix against a concrete document's own FULL path (not just its ancestor — ADR-063's `bind_ancestor` cannot represent the zero-remaining-segments case), and a precedence-composition step that resolves the single most-specific applicable pattern across exact-match (4a), fixed-depth pattern (4b), and recursive-wildcard candidates (this feature), wired into `GetDocument`.

## IN Scope
- `bind_recursive_prefix(prefix_segments, concrete_full_path_segments) -> Option<(bindings, remainder)>`, built on the SAME `positions_compatible` predicate 4b's own `bind_ancestor`/`structurally_overlap` already use.
- Precedence composition: exact-match (4a) always wins; 4b fixed-depth pattern always wins over any recursive wildcard whose prefix structurally contains it; among recursive-wildcard candidates, the longer/deeper prefix wins.
- Wire the 3-step composition (exact-match → 4b fixed-depth pattern → this feature's own recursive-wildcard scan) into `GetDocument`.
- A concrete path matching no pattern of any kind falls through to pre-existing unrestricted behavior, unaffected.
- Existence-non-leakage on denial, reusing AC-17-10 unchanged.

## OUT Scope
- Write handlers, Listen per-event re-check (Slice 03).
- Precedence-tie/overlap detection at import time (Slice 04).
- Simulation (Slice 06).
- The storage/indexing mechanism itself (DESIGN's call — two candidate directions named, not locked).

## Learning Hypothesis
Disproves: a concrete document path cannot be routed to the single most-specific applicable pattern — across exact-match, 4b fixed-depth, and this feature's own recursive-wildcard candidates — without either an unbounded per-request scan, a second, drift-prone precedence implementation, or an ambiguous/undefined outcome for a genuinely new collection with no specific rule.

## Acceptance Criteria
AC-17-238 through AC-17-244 (see `feature-delta.md` § User Stories, US-02).

## Dependencies
Slice 01 (this feature's own decomposition shape). `security-rules-cel-path-matching` (4b) — `path_routing::positions_compatible`, `resolve_access_rule_pattern` (or DESIGN's own equivalent), shipped and confirmed by direct read.

## Effort Estimate
3 days — the single largest slice in this feature, reflecting the single riskiest new mechanism (§ Journey, Shared Artifact table's own CRITICAL rating).

## Reference Class
No direct precedent in this codebase — confirmed by direct grep (`system_db.rs`) that no matching/routing function of any kind existed before 4b, and 4b's own equivalent primitives are built on a hard equal-length precondition this slice cannot reuse unmodified (§ Job Discovery Framing Resolution, Resolution 1).

## Pre-Slice SPIKE
Not required, but named fallback if DESIGN's own mechanism design surfaces further complexity once implementation begins: split into "pure catch-all, no co-existing more-specific pattern" first, "precedence against a co-existing pattern" second (§ Elephant Carpaccio Slices, feature-delta.md).
