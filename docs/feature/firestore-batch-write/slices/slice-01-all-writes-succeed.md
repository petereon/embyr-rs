# Slice 01: All Writes Succeed Independently (Walking Skeleton)

**Feature**: firestore-batch-write | **Story**: US-01 | **Release**: 1 | **Estimate**: 1 day

## Goal

Prove the `BatchWrite` RPC works end-to-end for a batch of well-formed writes: a new unary handler, a new per-write translate-and-catch variant, and the per-write `begin_transaction`+`commit_transaction` loop (called once per write, not once for the whole batch) all compose correctly, returning positionally-aligned `write_results`/`status` arrays.

## IN Scope

- New `BatchWriteRequest`/`BatchWriteResponse` proto messages (`write.proto`), reusing the already-vendored `Write`/`WriteResult` message types unchanged, plus a `repeated google.rpc.Status status` field on the response.
- New `rpc BatchWrite(BatchWriteRequest) returns (BatchWriteResponse);` declaration on the `Firestore` service.
- New `handle_batch_write` unary handler: auth/rate-limit/suspension sequence mirrors `handle_commit`'s own (once per call, not per write).
- New per-write translate-and-catch variant of `translate_writes_for_commit` — translates and access-rule-evaluates ONE write, returning `Result<DomainWrite, Status>` for that write alone, never aborting a caller's loop over the rest of the batch.
- Per-write apply loop: for each write, `adapter.begin_transaction(...)` then `adapter.commit_transaction(...)` with a single-element `writes` vec.
- Backend modes: `direct_pg`, `aws_secret`, `gcp_secret` only (agent-mode explicitly out — see feature-delta.md Resolution 2).
- Empty-batch handling (0 writes → immediate empty-arrays response, no error).
- Suspended-project rejection at call time (mirrors every other RPC).

## OUT Scope

- Partial failure / per-write isolation under a real precondition violation (Slice 02).
- `backend_mode=agent` (escalated to DESIGN, feature-delta.md § Handoff Package).
- OCC `version` wiring, `DocumentTransform.field_transforms` translation (pre-existing `handle_commit` gaps, inherited unchanged).
- REST/gRPC-Web routing detail beyond confirming the existing generic `tonic-web` wrap applies (unary RPC, no new transport-level work expected).

## Learning Hypothesis

**Disproves if it fails**: the per-write `begin_transaction`+`commit_transaction` loop — confirmed feasible by direct reading of `crates/embyr-pg-storage/src/backend_adapter.rs` (§ Reading Confirmation, feature-delta.md) — cannot actually be wired end-to-end through a new unary handler and a new per-write translate variant without requiring a new `BackendAdapter` trait method after all.

**Confirms if it succeeds**: `BatchWrite`'s own walking skeleton is a straightforward composition of already-existing, already-proven primitives (`handle_commit`'s translation shape, `begin_transaction`/`commit_transaction`) — no new mechanism class, matching Decision 2's own call.

## Acceptance Criteria

- [ ] AC-01-01: A `BatchWriteRequest` containing N well-formed writes returns a `BatchWriteResponse` whose `write_results` and `status` arrays each contain exactly N entries, positionally aligned to the request's own `writes` array.
- [ ] AC-01-02: Every well-formed write in the batch is applied and readable via `GetDocument` immediately after the `BatchWriteResponse` is returned.
- [ ] AC-01-03: A successful write's `status[i]` entry is null.
- [ ] AC-01-04: An empty `BatchWriteRequest.writes` returns an immediate response with empty `write_results`/`status` arrays and no error.
- [ ] AC-01-05: A suspended project's `BatchWriteRequest` is rejected with `permission_denied` before any write is attempted.

## Production-Data Taste Test

Real Postgres, a real batch of 3 well-formed writes across `trip_entries` (Trailmark/Maria Santos domain data), a real `BatchWrite` call, asserting all 3 `write_results` are populated, all 3 `status` entries are null, and all 3 documents are readable via `GetDocument` immediately after — no mocked adapter, no synthetic single-write shortcut.

## Dependencies

- `handle_commit`'s own translation shape (shipped) — reuse TARGET for the new per-write-catching variant, not reused unchanged.
- `BackendAdapter::begin_transaction`/`commit_transaction` (shipped) — reused unchanged, at new per-write granularity.
- `security-rules-write-path` (shipped) — per-write access-rule evaluation.
- `core_error_to_status` (shipped) — per-write `google.rpc.Status` construction (used from Slice 02 onward for real failures; Slice 01's own happy path exercises the success path of this mapping only).

## Reference Class

`handle_commit` (unary handler shape, translation loop) + `firestore-write-streaming`'s own ADR-046 finding (begin_transaction-before-commit_transaction synthesis pattern, here applied at finer per-write granularity).

## Pre-Slice SPIKE

Not needed — the central architectural question (can `commit_transaction` be called once per write) was already resolved with direct evidence during DISCUSS (feature-delta.md § Reading Confirmation), corroborated independently by ADR-046.
