# Slice 05: Simulate a Rule Before Publishing It

**Story**: US-05 | **Release**: 2 | **Estimate**: 1 day | **job_id**: JOB-17

## Goal
Alex can test a candidate rule against synthetic identity/document pairs and see the resolved allow/deny outcome, with zero effect on live/published traffic — before any real end user is affected by a bug.

## IN Scope
- Admin-API simulation action: accepts a candidate condition plus a synthetic identity (or none, for the anonymous case) and a synthetic document payload; returns allow/deny.
- Reuse of the exact evaluation routine from Slice 02/03 — not a separately-maintained copy.
- Guarantee that simulation never affects live/published rule state or real traffic.

## OUT Scope
- Persisting simulation history/results — not required for v1; each simulation call is stateless.
- Batch/bulk simulation (multiple pairs in one call) — not evidenced as needed, single-pair only for v1.

## Learning Hypothesis
**Disproves if it fails**: a rule-simulation action cannot share the exact same evaluation routine as real enforcement without duplicating (and risking drift in) the evaluation logic.
**Confirms if it succeeds**: Slice 02's evaluation function is cleanly reusable as a pure function callable from both the real `GetDocument` path and a standalone admin action — no drift risk.

## Acceptance Criteria
- AC-17-17: Simulation returns the same allow/deny outcome real evaluation would produce for the identical synthetic pair.
- AC-17-18: Simulation has zero effect on live/published traffic.
- AC-17-19: Simulation supports the anonymous (no synthetic identity) case, matching US-03's real semantics.

## Dependencies
- Slice 02 (the evaluation routine this slice wraps).
- Slice 03 (the anonymous-case semantics this slice must match).

## Effort Estimate
1 day. Reference class: `client-auth` US-04 (standalone debug-verify check, 1 day — near-identical shape: thin, read-only wrapper over an already-tested pure evaluation function).

## Pre-Slice SPIKE
Not required.
