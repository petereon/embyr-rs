# Slice 09 — BeginTransaction + Commit + Rollback

**Goal**: SDK `runTransaction` works correctly under concurrent writes; OCC guarantees atomicity.

## IN scope
- `BeginTransaction` RPC → returns `transaction_id`
- `GetDocument` with `transaction` field (read-in-transaction)
- `Commit` RPC with mutations (write set) and `transaction` field
- `Rollback` RPC
- OCC conflict detection: if any read document's `version` changed between `BeginTransaction` and `Commit` → `ABORTED`
- Transaction TTL: 60 seconds (uncommitted transactions auto-expire)
- `transactions` table: id, project_id, started_at, expires_at

## OUT scope
- Read-only transactions (`READ_ONLY` option) — treated as regular read in this slice
- Pessimistic locking

## Learning Hypothesis
Disproves: "OCC transaction abort rate is too high to be usable by the Firebase SDK auto-retry logic."
Confirms if: `runTransaction` with counter increment succeeds under 10 concurrent clients without deadlock, and the SDK's retry logic converges within 5 attempts.

## Acceptance Criteria
- `runTransaction` read-increment-write succeeds; counter is exactly 1 after one client runs it
- 10 concurrent `runTransaction` incrementing the same counter: final value is exactly 10
- Transaction expired (>60s): `Commit` returns `NOT_FOUND`
- `Rollback` of an active transaction: subsequent `Commit` with same ID returns `NOT_FOUND`

## Dependencies
S02 (writes), S01 (auth + DB adapter)

## Effort estimate
≤1 day
