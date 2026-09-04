# Slice 04: The Same Cross-Document Grammar Gates Writes

**Story**: US-04 | **Release**: 2 | **Estimate**: 1 day

## Goal
Extend Release 1's two-phase mechanism to real write-path enforcement
(`CreateDocument`/`UpdateDocument`).

## IN Scope
- The same path-discovery → fetch → evaluate mechanism wired into `handle_create_document`'s and
  `handle_update_document`'s own exact-match write-rule branches.
- Real `CreateDocument`/`UpdateDocument` enforcement proof.

## OUT Scope
- Routed-pattern (multi-segment/recursive-wildcard) write branches — no domain example combines
  cross-document reads with pattern-routed rules; if evidenced later, a follow-up slice.
- Simulation (Slice 05).

## Learning Hypothesis
Disproves: write-path parity needs anything beyond the identical mechanical extension every prior
epic's own write-parity slice already proved.

## Acceptance Criteria
AC-CDR-11, AC-CDR-12 (see `feature-delta.md` § User Stories, US-04).

## Dependencies
Slices 01-03 (the full read-path mechanism, proven).

## Effort Estimate
1 day.

## Reference Class
Mirrors 4c's own Slices 02/06 (write-path parity, confirmatory extension of a proven mechanism).

## Pre-Slice SPIKE
Not required.
