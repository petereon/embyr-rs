# Slice 02: The Same Numeric-Comparison Grammar Gates Writes

**Story**: US-02 | **Release**: 1 | **Estimate**: 1 day

## Goal
Extend Slice 01's numeric-comparison grammar to real write-path enforcement
(`CreateDocument`/`UpdateDocument`), reusing the existing `RequestResourceField` operand family.

## IN Scope
- `request.resource.data.<field> </<=/>/>= <numeric literal or another field>` evaluates correctly
  against `request_resource_fields` at the write handlers.
- Real `CreateDocument`/`UpdateDocument` enforcement proof.

## OUT Scope
- Simulation (US-03).
- Any new operand family — this slice is pure mechanical propagation of Slice 01's own operators
  into the already-existing write-path call sites.

## Learning Hypothesis
Disproves: write-path parity for numeric comparisons cannot reuse `evaluate()`'s existing
`request_resource_fields` parameter unchanged.

## Acceptance Criteria
AC-CEG-06, AC-CEG-07 (see `feature-delta.md` § User Stories, US-02).

## Dependencies
Slice 01 (numeric literals + relational operators).

## Effort Estimate
1 day.

## Reference Class
Mirrors every prior epic's own write-parity slice (e.g. security-rules-write-path's own extension
of 4a's read-path grammar).

## Pre-Slice SPIKE
Not required.
