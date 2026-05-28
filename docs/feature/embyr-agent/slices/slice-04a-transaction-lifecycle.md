# Slice 04A — Transaction Lifecycle: Begin, GetForTransaction, Commit (OCC), Rollback, Sweep

**Goal**: Full Firestore transaction lifecycle via the agent StorageAgent protocol, with OCC semantics identical to direct-mode.

**Feature**: embyr-agent
**Estimated effort**: ≤1 day
**Sequence**: 4 of 6 (after S02A, which proves write path; OCC latency risk validates early)

---

## IN Scope

- `StorageAgent::begin_transaction`
  - Inserts transaction record in Postgres with `expires_at = now + transactions_ttl`
  - Returns 128-bit hex transaction ID encoded as bytes
  - `read_only` option supported
- `StorageAgent::get_document` with `transaction` bytes (read within transaction)
  - Returns document
  - Records `(path, document.version)` in the transaction's read set
  - Returns `NotFound` if document absent (still records in read set as missing)
- `StorageAgent::commit` with `transaction` bytes
  - OCC re-validation: for each path in read set, re-reads `version` in SQL; mismatch → `Aborted` with `"version mismatch for <path>"`
  - If document in read set was deleted → `Aborted` with `"<path> was deleted"`
  - Applies write ops inside SQL transaction
  - Deletes transaction record
  - Emits `DocChange` for each modified document
- `StorageAgent::rollback`
  - Deletes transaction record without applying writes
  - No OCC checks
  - Returns `InvalidArgument` if transaction bytes are empty
- Background sweep: `SweepExpiredTransactions` — deletes records with `expires_at < now`

## OUT Scope

- `BeginTransaction` in the Firestore gRPC service sense (that's `embyr-server`'s responsibility)
- Non-transaction `Commit` path (covered by S02A — write operations use `WithTransaction`)

---

## Learning Hypothesis

**Disproves**: "OCC read-set re-validation via the agent adds unacceptable latency (>50ms overhead vs direct Postgres mode) because it requires an extra network round-trip to re-read versions."

**Confirms if successful**: Agent transaction commit round-trip is within 10ms of direct Postgres adapter commit (network between SaaS and agent is LAN-speed in typical VPC deployments).

---

## Acceptance Criteria

- [ ] `BeginTransaction` inserts a transaction record with correct `expires_at = started_at + transactions_ttl` (SPEC.md §Transactions §Begin)
- [ ] `GetDocument` with `transaction` bytes records the document version in the read set (SPEC.md §Transactions §Read-Within-Transaction)
- [ ] `CommitTransaction` returns `Aborted` with `"version mismatch for <path>"` when a read document has been updated concurrently (SPEC.md §Transactions §Commit step 3)
- [ ] `CommitTransaction` returns `Aborted` with `"<path> was deleted"` when a read document has been deleted (SPEC.md §Transactions §Commit step 3)
- [ ] `CommitTransaction` applies all write ops atomically when OCC passes (SPEC.md §Transactions §Commit step 4)
- [ ] `CommitTransaction` deletes the transaction record on success (SPEC.md §Transactions §Commit step 5)
- [ ] `CommitTransaction` emits `DocChange` for each modified document (SPEC.md §Transactions §Commit step 7)
- [ ] `RollbackTransaction` deletes the transaction record without applying writes (SPEC.md §Transactions §Rollback)
- [ ] `RollbackTransaction` returns `InvalidArgument` for empty transaction bytes (SPEC.md §Rollback Errors)
- [ ] `SweepExpiredTransactions` deletes records with `expires_at < now` (SPEC.md §Transactions §Sweep)
- [ ] `BatchGetDocuments` with `new_transaction` option: first response message carries transaction bytes (SPEC.md §BatchGetDocuments)

---

## Spec Traceability

| AC | SPEC.md Reference |
|----|------------------|
| Transaction record + expires_at | §Transactions §Begin; §Data Model §Transaction |
| Read set recording | §Transactions §Read-Within-Transaction |
| OCC re-validation logic (version mismatch / deleted) | §Transactions §Commit steps 1–3 |
| Atomic write application | §Transactions §Commit step 4 |
| Transaction record deletion on commit | §Transactions §Commit step 5 |
| DocChange emission on commit | §Transactions §Commit step 7 |
| Rollback behavior | §Transactions §Rollback |
| InvalidArgument for empty tx | §Rollback Errors |
| Sweep timing | §Transactions §Sweep |
| BatchGet new_transaction first message | §gRPC Service §BatchGetDocuments |

---

## Dependencies

- S01A: `required` (Postgres pool and agent wiring)
- S02A: `required` (writes must work for OCC conflict test scenario — need to write a document, start tx, read it with tx, update it externally, then commit tx and observe Aborted)
