# Slice 07 — Listen: Live Changes (Postgres NOTIFY)

**Goal**: Active listeners receive change events within 2 seconds of a write.

## IN scope
- Postgres `LISTEN` on `firestore_changes` channel
- `NOTIFY` trigger on `documents` table (INSERT, UPDATE, DELETE)
- Fan-out: broadcast change to all active `Listen` streams for the affected collection
- In-memory filter: apply query predicates against the changed document before delivering
- `DocumentChange(ADDED | MODIFIED | REMOVED)` delivery
- Updated resume token issued with each `TargetChange(NO_CHANGE)`

## OUT scope
- Resume token reconnect delta delivery (slice 08)
- BrowserChannel transport (slice 12)

## Learning Hypothesis
Disproves: "Postgres NOTIFY fan-out latency exceeds 2s for typical write rates (100 writes/sec)."
Confirms if: a listener receives a `DocumentChange` within 2 seconds of a Postgres COMMIT, measured end-to-end from SDK write to SDK `onSnapshot` callback fire.

## Acceptance Criteria
- `onSnapshot` fires within 2 seconds of `setDoc` from a second client
- Filter applied: listener on `where("age", ">=", 18)` does not fire for documents where age < 18
- `REMOVED` event delivered when document is deleted while listener is active
- At 100 concurrent listeners + 10 writes/sec, no change event is missed

## Dependencies
S06 (Listen initial snapshot)

## Effort estimate
≤1 day

## Pre-slice SPIKE
Verify Postgres `LISTEN`/`NOTIFY` payload size limit (8000 bytes); define truncation strategy if document exceeds limit (fetch full document on listener side).
