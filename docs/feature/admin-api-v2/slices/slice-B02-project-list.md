# Slice B-02 — Project List + Expanded Detail

**Feature:** admin-api-v2
**Slice:** B-02 of B-06
**Estimate:** 0.5 days
**Stories:** US-B02
**Depends on:** B-01 complete (session auth middleware available)

---

## Goal

`GET /admin/v1/projects` with account scoping. Dashboard renders real database cards.

## Learning Hypothesis

Disproves: "Account-scoping via `account_id` FK breaks for projects created before the migration (no `account_id` set)."  
Confirms if succeeds: Account-scoped project list returns only the caller's databases, with zero cross-account leakage.

## IN Scope

- `GET /admin/v1/projects` (session auth): account-scoped list, excludes deleted
- Expand `GET /admin/v1/projects/:id` (session auth): add `name`, `logging_enabled`, `log_retention_days`, `created_at` to response
- Cross-account isolation integration test: session from account A cannot see account B's projects (403)
- Index: `CREATE INDEX ON projects(account_id) WHERE status != 'deleted'`

## OUT Scope

- Metrics or chart data (B-04)
- Creating projects via session auth (operator route stays; no change)

## Acceptance Criteria

- AC-B02-01 through AC-B02-05 (see feature-delta.md US-B02)

## Dependencies

- B-01 complete
- `projects.account_id` column in place (from B-01 migration)

## Effort Estimate

0.5 days. Two small handlers; one index; one integration test for cross-account isolation.
