# Slice 02A — Write Operations: Create, Update, Delete via Agent

**Goal**: All document mutation RPCs work through the agent with identical OCC semantics to the direct Postgres adapter.

**Feature**: embyr-agent
**Estimated effort**: ≤1 day
**Sequence**: 3 of 6 (after S01A and S06A; unblocks S03A, S04A, S05A)

---

## IN Scope

- `StorageAgent::create_document`
  - Inserts new document with version=1
  - Generates 20-char random document ID when `document_id` is empty (SPEC.md §Document ID Generation)
  - Returns `AlreadyExists` if path taken
  - Emits `DocChange{Upsert}` after successful insert
- `StorageAgent::update_document`
  - Upsert (no precondition): create or overwrite
  - Update mode (`exists=true` precondition): `NotFound` if absent
  - InsertOnly mode (`exists=false` precondition): `AlreadyExists` if present
  - update_time precondition: `FailedPrecondition` if `updated_at` mismatch at microsecond precision
  - `update_mask`: read current doc, merge masked fields, write result
  - Field transforms: `setToServerValue: REQUEST_TIME`, `increment` (integer + double), `appendMissingElements`, `removeAllFromArray`
  - Increments `version` by 1 on every successful write
  - Emits `DocChange{Upsert}` after commit
- `StorageAgent::delete_document`
  - Idempotent by default (no error if absent)
  - `mustExist=true` (from `exists=true` or `update_time` precondition): `NotFound` if absent
  - Inserts tombstone record after successful delete
  - Emits `DocChange{Delete}`
- `BatchWrite` equivalent: each write in its own SQL transaction; failures isolated (one failed write does not affect siblings)

## OUT Scope

- Subscribe stream delivery of DocChange events to embyr SaaS (S05A)
- Transaction RPCs (S04A)
- Query RPCs (S03A)

---

## Learning Hypothesis

**Disproves**: "Field transforms (increment, array append/remove) require 2+ days to implement correctly in the agent because they must read-then-write atomically."

**Confirms if successful**: Transforms can be implemented as a SQL read-then-write within a single Postgres transaction, with no new abstraction beyond what GetDocument already provides.

---

## Acceptance Criteria

- [ ] `CreateDocument` with no document_id generates a 20-char `[a-zA-Z0-9]` ID (SPEC.md §Document ID Generation)
- [ ] `CreateDocument` returns `AlreadyExists` if path already exists (SPEC.md §CreateDocument Errors)
- [ ] `UpdateDocument` upsert mode creates document if absent (SPEC.md §UpdateDocument precondition table)
- [ ] `UpdateDocument` with `exists=true` returns `NotFound` if document is absent (SPEC.md §UpdateDocument)
- [ ] `UpdateDocument` with `update_time` precondition returns `FailedPrecondition` on mismatch at microsecond precision (SPEC.md §Write Semantics)
- [ ] `UpdateDocument` with `update_mask` preserves fields not in the mask (SPEC.md §Write Semantics §Mask application)
- [ ] `increment` transform on integer field: result is integer (SPEC.md §Field Transforms)
- [ ] `increment` transform on missing field: treats as 0 (SPEC.md §Field Transforms)
- [ ] `appendMissingElements`: merges new values, skips existing values (proto structural equality) (SPEC.md §Field Transforms)
- [ ] `removeAllFromArray` on missing field: returns empty array as transform result, no error (SPEC.md §Field Transforms)
- [ ] `DeleteDocument` without precondition: idempotent, no error if absent (SPEC.md §DeleteDocument)
- [ ] `DeleteDocument` with `exists=true`: returns `NotFound` if absent (SPEC.md §DeleteDocument)
- [ ] `DeleteDocument` successful: tombstone row inserted in `deleted_documents` (SPEC.md §Tombstone)
- [ ] `version` increments by 1 on every successful write; never decrements (SPEC.md Invariant 3)
- [ ] `DocChange{Upsert}` emitted after CreateDocument and UpdateDocument (SPEC.md §CreateDocument side effects)
- [ ] `DocChange{Delete}` emitted after DeleteDocument (SPEC.md §DeleteDocument side effects)

---

## Spec Traceability

| AC | SPEC.md Reference |
|----|------------------|
| 20-char ID generation | §Document Paths §Document ID Generation |
| AlreadyExists on create | §gRPC Service §CreateDocument Errors |
| update_time microsecond precision | §Write Semantics §Update Write (update_time precondition) |
| update_mask sibling preservation | §Write Semantics §Mask application |
| Field transforms behavior table | §Field Transforms (full table) |
| Tombstone insertion | §Data Model §Tombstone; §DeleteDocument side effects |
| version monotonicity | §Invariants Invariant 3 |
| DocChange emission | §gRPC Service §CreateDocument / §DeleteDocument side effects |

---

## Dependencies

- S01A (agent wiring + Postgres pool): `required`

---

## Reference Class

Analogous to embyr-rs Slice S02 + S03 combined. Those were two separate slices in the SaaS-side implementation. In the agent, they share the same SQL substrate already validated in S01A, so they can merge into one slice.
