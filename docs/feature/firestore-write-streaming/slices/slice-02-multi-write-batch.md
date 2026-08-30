# Slice 02: Multi-Write Batches in One WriteRequest

## Goal
Extend Slice 01's own receive-and-reply loop so a single `WriteRequest` can carry and atomically apply N writes (N > 1), spanning one or more collections — matching the SDK's own realistic offline-queue-flush pattern (several pending edits flushed together, not one write per round trip).

## IN Scope
- Reuse `handle_commit`'s own existing per-`Write`-message translation loop (already proven for N writes in one `CommitRequest.writes`) unchanged, inside the stream's own per-`WriteRequest` write loop (Slice 01).
- Atomicity: all N writes in one `WriteRequest` succeed together or none are applied — mirrors `Commit`'s own all-or-nothing guarantee.
- `WriteResponse.write_results` length exactly equals the batch's own `writes` length, in request order — mirrors the identical invariant `docs/SPEC.md` already documents for `CommitResponse`.
- A precondition-violating write anywhere in the batch causes zero writes from that batch to apply.
- `stream_token` rotates on every successful round trip, batch size notwithstanding.

## OUT Scope
- `stream_token` mismatch rejection — Slice 03.
- Termination-condition handling beyond the trivial success path — Slice 04.
- Agent-mode — deferred, `feature-delta.md` § Out of Scope.
- Any new translation logic — this slice is pure reuse/composition of `handle_commit`'s own existing loop, not new write-semantics code.

## Learning Hypothesis
**Disproves if it fails**: `handle_commit`'s own existing multi-`Write`-per-call translation loop (already proven for N writes in one `CommitRequest`) cannot be reused unchanged for N writes arriving in one `WriteRequest.writes` batch inside the stream's own write loop — would indicate the stream's own per-message context (e.g., ordering, atomicity boundary) differs from `Commit`'s own call-level context in some load-bearing way.
**Confirms if it succeeds**: the write-application primitive is fully transport-agnostic — reusable identically whether the batch arrives via one-shot `Commit` or via a `Write` stream's own per-message loop.

## Acceptance Criteria
- [ ] AC-02-01: A `WriteRequest` containing N writes (N > 1), spanning one or more collections, is applied atomically — all N succeed together or none are applied.
- [ ] AC-02-02: A successful batch's `WriteResponse.write_results` length exactly equals the batch's own `writes` length, in request order.
- [ ] AC-02-03: A batch containing any single precondition-violating write applies zero writes from that batch.
- [ ] AC-02-04: Each successive `WriteRequest`/`WriteResponse` round trip on the same session issues a newly rotated `stream_token`, distinct from the previous one.

## Dependencies
- Slice 01 (this feature) — the receive-and-reply loop must exist first.
- `handle_commit`'s own multi-write translation loop — shipped.

## Effort Estimate
1 day. Reference class: `handle_commit`'s own existing loop (`handler.rs:1319-1351`), reused verbatim inside Slice 01's own per-message context — compositional work, not new design.

## Pre-Slice SPIKE
Not needed — direct reuse of already-proven, already-read logic (see `feature-delta.md` § Reading Confirmation).
