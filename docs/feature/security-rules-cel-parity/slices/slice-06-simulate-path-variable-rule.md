# Slice 06: Simulate a Path-Variable-Bound Rule Before Importing

**Story**: US-06 | **Release**: 2 | **Estimate**: 0.75 day

## Goal
Extend the existing `simulate_access_rule` action so Alex can test a candidate path-variable-bound condition against a synthetic identity and a synthetic document ID — before importing it — catching a reversed-comparison or misnamed-variable bug pre-publish.

## IN Scope
- Additive, backward-compatible field on the existing simulation request body: a synthetic document ID (the path variable has no real document to derive it from in a simulation context).
- Calls the identical `evaluate()` real enforcement uses (no second, independently-maintained evaluation path).
- Anonymous-case support (no synthetic identity), matching real anonymous-evaluation semantics.
- Zero effect on live/imported traffic.

## OUT Scope
- Any new endpoint or handler — extends the existing `simulate_access_rule` in place (response contract unchanged), mirroring `security-rules-write-path`'s own precedent for `request_resource`, not `security-rules-collection-group-rules`'s new-sibling-handler precedent (the request AND response contracts here are not genuinely different, only additively extended).

## Learning Hypothesis
Disproves: "A simulation of a path-variable-bound candidate rule cannot share the exact same evaluation routine as real enforcement without either duplicating logic or omitting a way to supply the synthetic document's own ID."

## Acceptance Criteria
AC-17-198, AC-17-199, AC-17-200, AC-17-201 (see feature-delta.md § User Stories, US-06).

## Dependencies
- Slice 02 (the operand and resolution mechanism this slice simulates).
- `security-rules`'s existing `simulate_access_rule` (DONE, shipped).

## Effort Estimate
0.75 day. Reference class: `security-rules`'s own Slice 05 (1 day, first simulation build) — this slice is smaller because it's an additive field on an already-proven handler, not a new handler.

## Pre-Slice SPIKE
Not required.
