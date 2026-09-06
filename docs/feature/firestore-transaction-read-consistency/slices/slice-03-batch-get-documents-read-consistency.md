# Slice 03: BatchGetDocuments Read Registration + Commit-Time Validation

**Goal**: A `BatchGetDocuments` call performed with a real transaction ID registers `(path,
version)` for every requested document; `Commit` re-validates all of them, aborting if any changed.

## IN scope
- `handle_batch_get_documents`: parse `consistency_selector.transaction`, thread it through to
  each of its own per-document `get_document` calls — since `get_document` is already
  transaction-aware after Slice 01, this is purely a handler-side wiring change, reusing the
  identical per-document mechanism `batch-get-documents`' own original feature already established
  as its own precedent (N calls to the same adapter method, unchanged).

## OUT scope
- `GetDocument`/`RunQuery` (Slices 01/02, already done).
- `BatchGetDocuments`' own `new_transaction` auto-begin variant (feature-delta.md § Out of Scope).
- Any new pg-storage-level code — Slice 01's own `get_document` registration covers this
  automatically once the transaction ID is threaded through.

## Learning hypothesis
**Disproves** (if it fails): batch-get read-registration is the same N-times application already
used, by this codebase's own established precedent, for `batch-get-documents`' access-control
sequence. If threading the transaction ID through requires anything beyond passing it to N
already-transaction-aware `get_document` calls, that precedent doesn't generalize as cleanly as
assumed.
**Confirms** (if it succeeds): Slice 01's design choice (put the real logic in `get_document`
itself, not in a batch-specific code path) was the right layering — batch behavior falls out for
free.

## Acceptance criteria
AC-TRC-09, AC-TRC-10 (feature-delta.md § US-03).

## Dependencies
Slice 01 (reuses `get_document`'s own transaction-awareness directly, added zero new logic here).

## Effort estimate
≤0.5 day.

## Reference class
`batch-get-documents`' own "N times, once per requested document, unchanged" precedent (JOB-01
NOTE, 2026-08-30).

## Production-data acceptance criterion
A real transaction runs a real `BatchGetDocuments` for 2+ documents; a non-transactional actor
updates ONE of them; the original transaction's commit is asserted `ABORTED`.
