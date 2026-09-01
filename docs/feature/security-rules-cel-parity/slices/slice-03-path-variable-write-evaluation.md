# Slice 03: Path-Captured Variable Gates Writes (Walking Skeleton)

**Story**: US-03 | **Release**: 1 | **Walking Skeleton**: Yes | **Estimate**: 1 day

## Goal
The identical path-variable resolution Slice 02 built for `GetDocument` extends to `CreateDocument`/`UpdateDocument`/`DeleteDocument`, so a combined `allow read, write: if request.auth.uid == userId` block behaves consistently across both verbs — never silently denying legitimate writes.

## IN Scope
- Wire the new operand into all 3 write handlers, resolved from the request's own target document path (available before any of these handlers touch storage — zero new I/O).
- Correct evaluation for create (no pre-existing document — variable still resolves from the target path, not fetched content), update, and delete.
- Denial before any write reaches the storage adapter (zero partial-write side effect).

## OUT Scope
- Any new grammar construct beyond Slice 02's own operand.
- `RunQuery`/Listen (unaffected by this slice, same as Slice 02).

## Learning Hypothesis
Disproves: "The identical captured-variable mechanism cannot extend to `CreateDocument`/`UpdateDocument`/`DeleteDocument` without a second write-specific resolution path" — i.e., that Resolution 3's own "mechanical, uniform propagation" claim (mirroring `custom-claims`'s ADR-034 precedent) does not actually hold once real code is touched.

## Acceptance Criteria
AC-17-184, AC-17-185, AC-17-186, AC-17-187 (see feature-delta.md § User Stories, US-03).

## Dependencies
- Slice 02 (the operand and its resolution mechanism must exist).
- `security-rules-write-path`'s existing write-handler composition (DONE, shipped).

## Effort Estimate
1 day. Reference class: `custom-claims`'s own confirmed "US-03 requires zero production code beyond US-02's own change" finding (ADR-034 § Decision — Write-Path Falsifiability) — this slice re-verifies that same class of claim for a new operand, not a new mechanism.

## Pre-Slice SPIKE
Not required.
