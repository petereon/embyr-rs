# Slice 02: `get()`'s Own `.data.<field>` Access Fails Closed on a Nonexistent Document

**Story**: US-02 | **Release**: 1 | **Estimate**: 1 day

## Goal
Extend Slice 01's mechanism to `get()`'s own `.data.<field>` access, reusing the existing
`FieldMissing` fail-closed mechanism for a nonexistent referenced document.

## IN Scope
- New `Operand::CrossDocumentGet(PathTemplate, field_name: String)` — parses
  `get(<path template>).data.<field>`.
- `resolve_field_value`'s new `CrossDocumentGet` arm: looks up the pre-fetched map; `None` (doc
  doesn't exist) or the doc exists but lacks `field_name` both resolve to `Err(FieldMissing)` —
  the SAME mechanism AC-17-09 already uses for any other missing field, zero new control-flow
  shape.
- Real `GetDocument` enforcement proof: matching field value succeeds, missing document denies,
  mismatched field value denies.

## OUT Scope
- Deduplication proof (Slice 03).
- Write-path, simulation (Slices 04–05).

## Learning Hypothesis
Disproves: `get()`'s own `.data.<field>` access cannot reuse the EXISTING `FieldMissing`
fail-closed mechanism for a nonexistent-document case without a new control-flow shape.

## Acceptance Criteria
AC-CDR-05 through AC-CDR-08 (see `feature-delta.md` § User Stories, US-02).

## Dependencies
Slice 01 (the two-phase mechanism, path-discovery/fetch/evaluate).

## Effort Estimate
1 day.

## Reference Class
Mirrors AC-17-09's own fail-closed precedent, reused unchanged.

## Pre-Slice SPIKE
Not required.
