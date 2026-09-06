# Slice 01: GetDocument Read Registration + Commit-Time Validation (Walking Skeleton)

**Goal**: A `GetDocument` read performed with a real transaction ID registers `(path, version)` in
that transaction's own read set; `Commit` re-validates every registered read at commit time,
aborting (`ABORTED`) if any read document changed — closing the lost-update race for the single
-document read case.

## IN scope
- `embyr-core::storage::backend_adapter::BackendAdapter::get_document` gains a
  `transaction_id: Option<&TransactionId>` parameter (mirrors `run_query`'s existing shape).
- New migration `0003_transaction_reads.sql`: `ALTER TABLE transactions ADD COLUMN reads JSONB
  NOT NULL DEFAULT '{}'`.
- `embyr-pg-storage`'s `get_document` impl: when given `Some(transaction_id)`, after the read,
  UPSERT `(path, version-or-null)` into that transaction's `reads` column via bound parameters.
- New `verify_reads` function in `crates/embyr-pg-storage/src/transactions/occ.rs`, sibling to
  `verify_versions`.
- `commit_transaction`: call `verify_reads` immediately after the existing `verify_versions` call;
  extend the initial transaction-row SELECT to also fetch `reads`.
- `handle_get_document`: parse `consistency_selector.transaction`, thread it through.
- All OTHER `get_document` call sites (non-transactional reads, internal access-rule checks) pass
  `None` — explicitly confirmed unaffected.
- `AgentBackendAdapter::get_document` and `crates/embyr-agent/src/server.rs`'s own internal calls
  gain the parameter but ignore/pass `None` (agent-mode deferred, § Out of Scope in
  feature-delta.md).

## OUT scope
- `RunQuery`/`BatchGetDocuments` (Slices 02/03).
- `backend_mode=agent` real transactional-read behavior.
- `ReadTime`, `BatchGetDocuments`' `new_transaction` auto-begin variant.

## Learning hypothesis
**Disproves** (if it fails): the read-set-validation mechanism can reuse `verify_versions`'s own
FOR-UPDATE-lock-and-compare shape without inventing new OCC infrastructure. If `verify_reads` ends
up needing a fundamentally different locking/comparison strategy than `verify_versions`, that
would mean the "reuse existing machinery" premise this whole feature is built on was wrong.
**Confirms** (if it succeeds): transactional-read OCC is a small, mechanical addition to
already-proven infrastructure, not a new subsystem.

## Acceptance criteria
AC-TRC-01 through AC-TRC-05 (feature-delta.md § US-01).

## Dependencies
None — first slice.

## Effort estimate
≤1 day.

## Reference class
`crates/embyr-pg-storage/src/transactions/occ.rs::verify_versions` (already shipped, proven).

## Production-data acceptance criterion
The Slice's own acceptance test drives a REAL two-actor lost-update race against a real running
server + real Postgres backend (not mocked): actor A begins a transaction, reads a document, actor
B (a separate, non-transactional call) updates that same document, actor A commits — asserting
`ABORTED`. This is real end-to-end proof of the race being closed, not plumbing-only.
