# Slice 01: Alex Creates a Composite Index and His Query Finally Succeeds (Walking Skeleton)

**Story**: US-01 | **Release**: 1 | **Estimate**: 0.5 day

## Goal
A real `POST /admin/v1/projects/:project_id/indexes` call inserts a `composite_indexes` row that
the EXISTING, unmodified `IndexManager::is_index_ready` gate reads — unblocking a real,
previously-`FAILED_PRECONDITION` `RunQuery`.

## IN Scope
- `IndexFieldSpec`/`IndexFieldOrder`/`CreateCompositeIndexBody`/`CompositeIndexResponse` types.
- `create_composite_index` handler: Owner/Admin gate, `verify_project_ownership`, `INSERT ...
  ON CONFLICT (project_id, collection_path, fields) DO UPDATE ... RETURNING`.
- Router entry: `POST /admin/v1/projects/:project_id/indexes`.
- Real end-to-end proof: a `RunQuery` that fails `FAILED_PRECONDITION`, then a real `CreateIndex`
  call, then the identical `RunQuery` succeeds.
- Idempotent-duplicate-create proof (AC-CIX-03).

## OUT Scope
- `ListIndexes`/`DeleteIndex` (Slices 02–03).
- Any change to `crates/embyr-server/src/grpc/handler.rs`.

## Learning Hypothesis
Disproves: a new admin `CreateIndex` handler cannot correctly satisfy the EXISTING, unmodified
`is_index_ready` gate without also touching `handler.rs`'s own `RunQuery` path.

## Acceptance Criteria
AC-CIX-01 through AC-CIX-04 (see `feature-delta.md` § User Stories, US-01).

## Dependencies
None — first slice.

## Effort Estimate
0.5 day.

## Reference Class
Mirrors `create_service_account`'s own "insert a row, existing gate reads it" precedent
(`admin/router.rs`), adapted from account-scoping to project-scoping.

## Pre-Slice SPIKE
Not required — the exact auth/ownership shape and SQL CRUD shape were both confirmed by direct
code read during DESIGN (ADR-068 § Reading Confirmation).
