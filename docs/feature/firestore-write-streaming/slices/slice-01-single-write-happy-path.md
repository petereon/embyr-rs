# Slice 01: A Working Write Stream, Single Write, Happy Path (Walking Skeleton)

## Goal
Declare and implement the `Write` bidirectional-streaming RPC end-to-end for the simplest realistic case: open a stream, handshake, send one write, receive one `WriteResponse`, close cleanly — proving both halves this feature depends on (the bidi-streaming scaffold, the write-application primitive) compose correctly into a genuine receive-and-reply loop, for `direct_pg`/`aws_secret`/`gcp_secret` backend modes.

## IN Scope
- Author `WriteRequest`/`WriteResponse` messages (`proto/google/firestore/v1/write.proto`, alongside `CommitRequest`) and `rpc Write(stream WriteRequest) returns (stream WriteResponse);` (`proto/google/firestore/v1/firestore.proto`), matching the wire contract `docs/SPEC.md` §Write Stream already documents.
- New `handle_write` handler: peek the handshake `WriteRequest` via `request.get_mut().next()` (mirrors `handle_listen`'s own mechanism, `handler.rs:1924-1929`), run the standard `rate_limiter.check` → `authenticate` (+suspension) → `attach_client_identity_if_present` sequence exactly once, then `request.into_inner()` + `tokio::spawn` a task owning the stream + an `mpsc::Sender<Result<WriteResponse, Status>>`, wrapped as `ReceiverStream` for the response — mirrors `handle_listen`'s own scaffold (`handler.rs:1913-2042`) structurally.
- The spawned task's own loop (the genuinely new mechanism): `while let Some(msg) = in_stream.next().await { ... }`, translating each `WriteRequest.writes` entry via the identical logic `handle_commit` already uses (`handler.rs:1319-1351`) and calling the existing atomic-apply primitive (`adapter.commit_transaction`-shaped), then constructing and sending a `WriteResponse`.
- `stream_id`/`stream_token` generation at handshake and rotation on each subsequent response (format informative only from `docs/SPEC.md`: 16-hex-encoded-unix-nanoseconds / RFC3339Nano timestamp).
- Reject a non-empty handshake message (`writes` or `stream_id` populated) before any write is attempted.
- Reject a handshake for a suspended project, mirroring every other RPC's existing suspension check.

## OUT Scope
- Multi-write batches (>1 write per `WriteRequest`) — Slice 02.
- `stream_token` mismatch/rejection handling — Slice 03 (this slice's own single-round-trip happy path never exercises a mismatch).
- Client-cancellation / server-error termination handling beyond the trivial clean-close case — Slice 04.
- Agent-mode (`backend_mode=agent`) — deferred, `feature-delta.md` § Out of Scope, Resolution 4.
- OCC `version`/`DocumentTransform.field_transforms` wiring — inherited `handle_commit` gap, not this slice's own scope.
- Per-`WriteRequest` write-path access-control evaluation beyond what already applies via the reused translation/apply call — full per-message rule evaluation semantics are exercised meaningfully once Slice 02 introduces batches; Slice 01's own single-write case reuses whatever `handle_commit`'s existing composition already does for one write.

## Learning Hypothesis
**Disproves if it fails**: `handle_listen`'s own peek-then-spawn-then-mpsc-then-`ReceiverStream` scaffold cannot be adapted into a genuine receive-and-reply loop (as opposed to merely draining leftover messages) — OR `handle_commit`'s own proto-Write→DomainWrite translation and atomic-apply call cannot be reused unchanged for a write arriving via this new transport.
**Confirms if it succeeds**: this codebase's two already-proven mechanisms (bidi-streaming scaffold, write-application primitive) compose cleanly with zero new domain logic — direct evidence that Slices 02-04 (which all extend this same loop) are safe to build without re-deriving the loop itself.

## Acceptance Criteria
- [ ] AC-01-01: Opening a `Write` stream with a valid, empty handshake `WriteRequest` returns a `WriteResponse` carrying `stream_id`, `stream_token`, `commit_time`, with no `write_results`.
- [ ] AC-01-02: A subsequent `WriteRequest` containing exactly one write, presenting the issued `stream_id`/`stream_token`, is applied atomically and the server replies with a `WriteResponse` containing exactly one `write_result` and a rotated `stream_token`.
- [ ] AC-01-03: A stream that receives no `WriteRequest` beyond the handshake, then closes, terminates cleanly with zero writes applied.
- [ ] AC-01-04: A non-empty handshake `WriteRequest` is rejected as invalid before any write is attempted.
- [ ] AC-01-05: Rate-limiting, authentication, suspension-check, and client-identity resolution each run exactly once per stream, at handshake time.
- [ ] AC-01-06: A suspended project's `Write` stream is rejected with `permission_denied` at handshake, before any write is accepted.

## Dependencies
- `handle_listen` / `realtime::listen_handler` (bidi-streaming scaffold) — shipped.
- `handle_commit`'s own translation/atomic-apply primitive — shipped.
- `security-rules-write-path` (write-rule evaluation, if exercised) — shipped.
- No dependency on `aggregation-queries` or `batch-get-documents`.

## Effort Estimate
1.5 days. Reference class: `handle_listen` (existing, ~130 lines) as the scaffold precedent, `handle_commit`'s own translation loop (existing, ~35 lines) reused unchanged — the genuinely new work is the receive-and-reply loop shape itself plus proto authoring, not new domain logic.

## Pre-Slice SPIKE
Not needed — `docs/SPEC.md` §Write Stream already documents the wire contract in full; both reused mechanisms (`handle_listen`, `handle_commit`) are already read in full and confirmed directly reusable (see `feature-delta.md` § Reading Confirmation).
