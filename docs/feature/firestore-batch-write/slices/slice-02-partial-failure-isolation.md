# Slice 02: True Per-Write Isolation Under Partial Failure

**Feature**: firestore-batch-write | **Story**: US-02 | **Release**: 2 | **Estimate**: 1 day

## Goal

Prove the property that actually differentiates `BatchWrite` from `Commit`: a failing write (e.g., a precondition violation) never rolls back, blocks, or is blocked by any sibling write in the same batch — the whole reason SDK developers reach for `db.bulkWriter()` over a batched `Commit` call.

## IN Scope

- Exercising Slice 01's own per-write loop against a batch containing at least one deliberately-failing write (precondition violation) amid otherwise well-formed siblings.
- Verifying failure position within the batch (first, middle, last) does not change isolation behavior.
- Verifying the degenerate all-fail case still returns a normal `BatchWriteResponse` with no top-level RPC error.
- Verifying a failing write's `write_results[i]` entry is always present (empty placeholder), never absent — preserving the length/position invariant Slice 01 established.
- Verifying `status[i]` for a failing write carries a specific, actionable code/message via `core_error_to_status`'s existing mapping.

## OUT Scope

- Any new component — this slice exercises Slice 01's own loop against a new input shape; no new handler code, no new proto, no new trait method expected.
- `backend_mode=agent` (still escalated, unresolved by this slice).
- Malformed-input failures (invalid document path, undecodable field value) — covered implicitly by the per-write translate-and-catch variant's own `Result` type, but not the focus of this slice's own dedicated scenarios (precedent: US-01/US-02 both focus on precondition-violation as the representative failure mode, matching `docs/SPEC.md`'s own primary documented failure case for BatchWrite).

## Learning Hypothesis

**Disproves if it fails**: a failing write inside the per-write loop cannot be isolated from its siblings without either (a) accidentally rolling back a sibling that already committed in an earlier loop iteration, or (b) accidentally blocking a sibling scheduled for a later loop iteration.

**Confirms if it succeeds**: the per-write `begin_transaction`+`commit_transaction` loop genuinely delivers `BatchWrite`'s own documented isolation contract — each iteration's own Postgres transaction is fully independent of every other iteration's.

## Acceptance Criteria

- [ ] AC-02-01: A precondition-violating write's own failure never rolls back, blocks, or otherwise affects any sibling write in the same `BatchWriteRequest`.
- [ ] AC-02-02: A failing write's `status[i]` entry is non-null and carries a specific, actionable code/message (via `core_error_to_status`'s existing mapping).
- [ ] AC-02-03: A batch where every write fails still returns a normal `BatchWriteResponse` with no top-level RPC error.
- [ ] AC-02-04: Failure position within the batch (first, middle, last) does not change isolation behavior for sibling writes.
- [ ] AC-02-05: A failing write's `write_results[i]` entry is always present (an empty placeholder), never absent, preserving the length/position invariant from AC-01-01.

## Production-Data Taste Test

A real batch of 3 writes against real Postgres where the 2nd write deliberately violates `current_document.exists = false` against an already-existing `trip_entries` document (Trailmark/Maria Santos domain data) — asserting write 1 and write 3 both commit and are readable via `GetDocument`, write 2's `status[1]` is non-null with a specific reason, and `write_results[1]` is an empty placeholder (position preserved, no error thrown at the RPC level).

## Dependencies

- Slice 01 (this feature) — the per-write loop must exist first; this slice adds no new component, only new test input shapes.

## Reference Class

Slice 01's own loop, exercised against a failure case instead of an all-success case — same mechanism, new input.

## Pre-Slice SPIKE

Not needed.

## Regression-Risk Note (carried to DELIVER)

This slice's own UAT scenario ("A precondition-violating write fails without affecting its siblings") is the scenario most likely to catch a future refactor that accidentally collapses the per-write loop back into a single `commit_transaction` call over the whole batch (which would silently reintroduce `Commit`'s own all-or-nothing semantics). Recommend this scenario carry an explicit comment at DELIVER time naming that exact regression risk.
