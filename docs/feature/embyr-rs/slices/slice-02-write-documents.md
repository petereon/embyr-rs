# Slice 02 — CreateDocument + UpdateDocument

**Goal**: SDK can write and overwrite documents; server-assigned timestamps are correct.

## IN scope
- `CreateDocument` RPC (auto-generated ID + explicit ID)
- `UpdateDocument` RPC (field mask + full replace)
- `commit_time` assigned by server clock (microsecond precision)
- `update_time` precondition check (optimistic lock via `version` column)
- `WriteResult` with `update_time` returned to SDK

## OUT scope
- `DeleteDocument` (slice 03)
- Write stream (`Write` RPC)
- Transforms (server-side: `ServerTimestamp`, `Increment`, etc.) — out of scope this slice
- Batch writes

## Learning Hypothesis
Disproves: "OCC via `version` column causes phantom conflicts under normal SDK write patterns."
Confirms if: concurrent `setDoc` + `updateDoc` either both succeed (last-writer-wins) or the second is rejected with `ABORTED` and the SDK retries correctly.

## Acceptance Criteria
- `setDoc` creates a document and `getDoc` returns it
- `setDoc` on existing path overwrites (UpdateDocument semantics)
- Concurrent write to the same document: exactly one succeeds per version check
- `WriteResult.update_time` matches server commit time

## Dependencies
S01 (gRPC server + auth + DB adapter)

## Effort estimate
≤1 day
