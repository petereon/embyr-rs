# Slice 02: Path-Captured Variable Gates Reads (Walking Skeleton)

**Story**: US-02 | **Release**: 1 | **Walking Skeleton**: Yes | **Estimate**: 1.5 days

## Goal
A `match` block's leaf-level wildcard (e.g. `{userId}`) is resolved, for a specific document, to that document's own ID, and made available to the block's own condition by the name the file gave it — correctly evaluated on `GetDocument`.

## IN Scope
- One new evaluable capability: a condition may reference a bare identifier matching its own enclosing block's captured path-variable name.
- Resolution against the document's own already-known ID at the `GetDocument` call site — zero new I/O.
- Correct evaluation for: document owner (allow), a different signed-in user (deny), anonymous session (deny, reusing existing `request.auth == null` semantics), and existence non-leakage (reusing AC-17-10 unchanged).
- Per-document resolution — the same collection's sibling documents each resolve their own variable independently.

## OUT Scope
- Write-path wiring (Slice 03).
- `RunQuery`/Listen wiring — confirmed structurally already-correct (auto-reject via the existing decidable-shape catch-all) with zero new code; not a build item, but DESIGN must verify this claim (Handoff Package flag 5).
- Multiple wildcards per block, recursive wildcards, nested paths (Epic 4b).

## Learning Hypothesis
Disproves: "A path-captured document-ID variable cannot be resolved and evaluated on a real `GetDocument` call without either new I/O or a second, drift-prone resolution mechanism separate from the existing `evaluate()`."

## Acceptance Criteria
AC-17-179, AC-17-180, AC-17-181, AC-17-182, AC-17-183 (see feature-delta.md § User Stories, US-02).

## Dependencies
- Slice 01 (a rule using a path variable must exist to evaluate against).
- `security-rules`'s existing `evaluate()`/`AuthContext`/existence-non-leakage mechanism (DONE, shipped).

## Effort Estimate
1.5 days. Reference class: `custom-claims`'s own `AuthTokenClaim` operand addition (one new operand, `resolve_field_value` extension, zero new `compare_operands` arm).

## Pre-Slice SPIKE
Not required.
