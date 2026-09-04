# Slice 02: Alex Lists His Project's Own Composite Indexes

**Story**: US-02 | **Release**: 1 | **Estimate**: 0.5 day

## Goal
A real `GET /admin/v1/projects/:project_id/indexes` call returns every composite index defined
for that project.

## IN Scope
- `list_composite_indexes` handler: any role, read-only, `verify_project_ownership`, filtered
  `SELECT ... ORDER BY created_at ASC`.
- Router entry chained onto the same path as Slice 01's `POST` (`get(list_composite_indexes)
  .post(create_composite_index)`).
- Empty-project-returns-empty-list proof.

## OUT Scope
- `DeleteIndex` (Slice 03).

## Learning Hypothesis
Disproves: listing a project's own indexes needs anything beyond a straightforward filtered
`SELECT` — confirmatory, mirrors `list_service_accounts`'s own identical shape.

## Acceptance Criteria
AC-CIX-05, AC-CIX-06 (see `feature-delta.md` § User Stories, US-02).

## Dependencies
Slice 01.

## Effort Estimate
0.5 day.

## Reference Class
Mirrors `list_service_accounts`'s own identical shape, adapted to project-scoping.

## Pre-Slice SPIKE
Not required.
