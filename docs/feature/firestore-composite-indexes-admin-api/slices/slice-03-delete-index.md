# Slice 03: Alex Deletes a Composite Index He No Longer Needs (LAST slice)

**Story**: US-03 | **Release**: 1 | **Estimate**: 0.5 day

## Goal
A real `DELETE /admin/v1/projects/:project_id/indexes/:index_id` call removes the row; no
dependency-safety check (Resolution 4).

## IN Scope
- `delete_composite_index` handler: Owner/Admin gate, `verify_project_ownership`, `DELETE FROM
  composite_indexes WHERE id = $1 AND project_id = $2`, 0 rows affected → 404.
- Router entry: `DELETE /admin/v1/projects/:project_id/indexes/:index_id`.
- Proof: delete removes it from a subsequent `ListIndexes`; a query newly requiring the deleted
  index (with no other ready index for that collection) fails `FAILED_PRECONDITION` again.

## OUT Scope
- Any dependency-safety check (explicitly locked OUT, Resolution 4).

## Learning Hypothesis
Disproves: deleting an index needs a dependency-safety check beyond a straightforward
`DELETE ... WHERE id = $1 AND project_id = $2` — confirmatory (Resolution 4 already locked
"no check").

## Acceptance Criteria
AC-CIX-07 through AC-CIX-09 (see `feature-delta.md` § User Stories, US-03).

## Dependencies
Slices 01–02.

## Effort Estimate
0.5 day.

## Reference Class
Mirrors `delete_service_account`'s own identical shape, adapted to project-scoping.

## Pre-Slice SPIKE
Not required.
