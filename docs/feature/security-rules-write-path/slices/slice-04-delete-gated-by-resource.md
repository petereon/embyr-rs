# Slice 04: A Delete Is Gated by the Write Rule Using Only the Existing Document

**Story**: US-04 | **Release**: 1 | **Walking Skeleton**: Yes | **Estimate**: 1 day

## Goal
Make `DeleteDocument` fetch the document's pre-write state and evaluate the write rule against `resource.data.<field>` only, with `request.resource.data.<field>` naturally failing closed since no proposed new data exists on a delete.

## IN Scope
- Reuse Slice 03's fetch-before-decide pattern inside `handle_delete_document`.
- Evaluate the collection's write rule with `resource_fields` populated (pre-write) and `request_resource_fields` empty.
- Gate the delete: `Deny` → `PermissionDenied`, identical response regardless of document existence. `Allow` → proceed to the existing `adapter.delete_document` call, unchanged.

## OUT Scope
- Create/update gating (Slices 02/03, already shipped).
- Simulation extension (Slice 07).

## Learning Hypothesis
Disproves: a condition referencing only `resource` (delete) cannot reuse the exact same fail-closed mechanism that already handles `request.resource`'s absence on delete, without asymmetric special-casing between the two operand families.
Confirms (if it passes): `resource.data.<field>` and `request.resource.data.<field>` are symmetric in how they fail closed when absent — no operand-specific branching logic needed in `evaluate()` beyond what Slice 02 already added.

## Acceptance Criteria
- AC-17-35, AC-17-36, AC-17-37, AC-17-38.

## Dependencies
- Slice 03 (fetch-before-decide pattern and `RequestResourceField` evaluator support).

## Production-Data Taste Test
Real Maria/Dana delete calls against real owned/non-owned `journal_entries` documents, including one missing the `owner_id` field entirely.

## Reference Class
Simplest of the three operation-shape slices — mirrors `security-rules`' own US-02/US-03 sizing discipline (smaller slice once the riskier mechanism is proven).
