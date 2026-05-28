# Slice 06 — Listen: Initial Snapshot

**Goal**: `onSnapshot` listener receives all current documents on connection.

## IN scope
- `Listen` bidirectional streaming RPC
- `AddTarget` request: collection target
- `DocumentChange` delivery for all existing documents
- `TargetChange(CURRENT)` sentinel: all initial documents delivered
- `TargetChange(ADD)` acknowledgement
- Resume token issued with `TargetChange(CURRENT)`: base64.RawURLEncoding(RFC3339Nano)

## OUT scope
- Live change delivery (slice 07)
- Resume token reconnect (slice 08)
- Document target (single document listen)

## Learning Hypothesis
Disproves: "Bidirectional gRPC streaming in Rust with Tonic requires a separate async runtime thread per listener."
Confirms if: 100 concurrent listeners each receive their initial snapshot within 500ms using a single Tokio task pool.

## Acceptance Criteria
- `onSnapshot` receives all collection documents on first call
- `TargetChange(CURRENT)` is sent after last document
- Resume token embedded in `TargetChange(CURRENT)` response
- 100 concurrent listeners all receive initial snapshots within 1 second

## Dependencies
S01 (gRPC server + auth)

## Effort estimate
≤1 day
