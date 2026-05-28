# Slice 03 — DeleteDocument + Tombstones

**Goal**: Documents can be deleted; tombstones are written for delta delivery.

## IN scope
- `DeleteDocument` RPC (with optional `current_document.exists` precondition)
- Tombstone written to `deleted_documents` (path, delete_time)
- Tombstone retention: 24 hours
- `getDoc` on deleted path returns `NOT_FOUND`

## OUT scope
- Tombstone consumption by Listen stream (slice 08)
- Sweeper cleanup of old tombstones (slice 15)

## Learning Hypothesis
Disproves: "Tombstone-based delta delivery is leaky — deletes are not propagated to reconnecting listeners."
Confirms if: a document deleted during a listener disconnect appears as a `REMOVED` change when the listener reconnects with a resume token.

## Acceptance Criteria
- `deleteDoc` returns success; `getDoc` returns `NOT_FOUND`
- Tombstone row exists in `deleted_documents` immediately after delete
- Delete with `exists: false` precondition on non-existent doc returns `NOT_FOUND`
- Delete with `exists: true` precondition on non-existent doc returns `FAILED_PRECONDITION`

## Dependencies
S02 (writes working)

## Effort estimate
≤1 day
