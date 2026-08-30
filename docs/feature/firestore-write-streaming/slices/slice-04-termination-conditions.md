# Slice 04: Correct Termination Under Every Documented Close Condition

## Goal
Ensure a `Write` session terminates correctly and distinguishably under each of `docs/SPEC.md`'s own three documented conditions — client `io.EOF`, client cancellation (`codes.Canceled`), and any other server-side error — with no partial writes and no leaked spawned tasks, matching real Firestore's own reconnect-friendly contract.

## IN Scope
- Distinguish `Streaming::next()` returning `None` (clean EOF) from returning `Some(Err(status))` with `Status::code() == Cancelled` from any other error code, inside the spawned task's own loop.
- Clean-close (EOF, Cancel): task returns without error; no partial write from any in-flight request; no leaked resources.
- Genuine server-side apply error: propagate the error to the client via the `mpsc::Sender`, then terminate the task — distinguishable, on the wire, from the two clean-close cases.
- Verify no leaked spawned task or inconsistent active-stream bookkeeping under load (repeated open/close cycles).

## OUT Scope
- `stream_token` mismatch handling — Slice 03 (a distinct rejection path, not a termination condition).
- Any new reconnect/resume mechanism on the client or server side — this slice ensures clean termination only; resuming a dropped session is always a fresh handshake (Slice 01), not a stateful reconnect.
- Agent-mode — deferred, `feature-delta.md` § Out of Scope.

## Learning Hypothesis
**Disproves if it fails**: the three documented termination conditions cannot each be distinguished and handled correctly using tonic's own `Streaming<T>` error/`None` semantics without new, bespoke per-condition detection machinery beyond a straightforward match on the loop's own `next().await` result.
**Confirms if it succeeds**: tonic's existing `Streaming<T>` surface already carries everything needed to implement `docs/SPEC.md`'s own termination contract correctly — no new detection layer required, consistent with how cleanly `handle_listen`'s own (simpler) drain loop already handles stream closure today.

## Acceptance Criteria
- [ ] AC-04-01: A client-initiated `io.EOF` close terminates the server side cleanly, with no error.
- [ ] AC-04-02: A client cancellation (`codes.Canceled`) terminates the server side cleanly, with no error, and never leaves a partially-applied write.
- [ ] AC-04-03: A genuine server-side apply error terminates the stream with the error propagated to the client, distinguishable from the two clean-close conditions.
- [ ] AC-04-04: No condition above leaks the spawned per-session task or leaves the active-stream count inconsistent.

## Dependencies
- Slice 01 (this feature) — the receive-and-reply loop must exist first.
- Slice 02 (multi-write batches) — recommended landed first so the in-flight-request case (AC-04-02) is exercised against a realistic batch, not only a single write.

## Effort Estimate
1 day. Reference class: `handle_listen`'s own spawned-task lifecycle (existing, simpler drain-only case) as the starting point for correct `Streaming<T>` result handling.

## Pre-Slice SPIKE
Not needed — tonic's `Streaming<T>` error/`None` semantics are well-established library behavior, not a novel unknown for this codebase.
