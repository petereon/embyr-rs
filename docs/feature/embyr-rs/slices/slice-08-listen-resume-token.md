# Slice 08 — Listen: Resume Token + Delta Delivery

**Goal**: SDK reconnects after a network drop and receives only the delta — not a full re-snapshot.

## IN scope
- `AddTarget.resume_token` path in `Listen` handler
- Delta query: `SELECT` from `documents` WHERE `update_time > token_time` UNION tombstones from `deleted_documents` WHERE `delete_time > token_time`
- Token expiry check: token older than 24h → fallback to full snapshot (no error)
- Resume token updated with each `TargetChange(NO_CHANGE)` response
- Tombstone consumption: `REMOVED` events from `deleted_documents`

## OUT scope
- Client-side reconnection backoff (SDK responsibility)

## Learning Hypothesis
Disproves: "Resume tokens after reconnect lead to duplicate or missed documents when writes occur during the disconnect window."
Confirms if: exactly the documents created/modified/deleted between disconnect and reconnect appear in the delta (no extras, no missing).

## Acceptance Criteria
- 3 documents written during 30s disconnect; reconnect delivers exactly those 3 as `ADDED` / `MODIFIED`
- 1 document deleted during disconnect appears as `REMOVED` via tombstone
- Token > 24h old: full snapshot sent, no `RESOURCE_EXHAUSTED` or error
- Resume token in reconnect response is newer than the token used in request

## Dependencies
S07 (live changes), S03 (tombstones)

## Effort estimate
≤1 day
