# Slice 02: RunQuery Read Registration + Commit-Time Validation

**Goal**: A `RunQuery` performed with a real transaction ID registers `(path, version)` for every
document the query returns; `Commit` re-validates all of them, aborting if any changed.

## IN scope
- `embyr-pg-storage`'s `run_query` impl: when given `Some(transaction_id)` (the trait parameter
  already exists — Slice 01 does not touch it), register every returned document's `(path,
  version)` via the SAME upsert mechanism Slice 01 built for `get_document`.
- `handle_run_query` (both call sites, lines ~2323 and ~3180): parse
  `consistency_selector.transaction`, thread it through instead of the current hardcoded `None`.

## OUT scope
- `GetDocument` (Slice 01, already done), `BatchGetDocuments` (Slice 03).
- Any change to `verify_reads`/`commit_transaction` — Slice 01's own validation logic is reused
  unchanged; a query-registered read looks identical to a `GetDocument`-registered read once it's
  in the `reads` map.

## Learning hypothesis
**Disproves** (if it fails): query-result read-registration is a trivial N-times application of
Slice 01's own single-document mechanism. If this requires new locking/comparison logic beyond
"call the same registration helper once per result row," Slice 01's own mechanism was less
reusable than assumed.
**Confirms** (if it succeeds): the trait-level `transaction_id` plumbing `run_query` already had
(added by a prior feature, previously always `None`) was correctly anticipatory — this slice
proves it out.

## Acceptance criteria
AC-TRC-06 through AC-TRC-08 (feature-delta.md § US-02).

## Dependencies
Slice 01 (reuses its own read-registration helper and `verify_reads`/`reads` column unchanged).

## Effort estimate
≤0.5 day.

## Reference class
Slice 01 itself — this is a direct, mechanical extension, not a new design.

## Production-data acceptance criterion
A real transaction runs a real `RunQuery` against a real multi-document collection; a
non-transactional actor updates ONE of the returned documents; the original transaction's commit
is asserted `ABORTED`. A second test confirms a non-transactional `RunQuery` is completely
unaffected (AC-TRC-08).
