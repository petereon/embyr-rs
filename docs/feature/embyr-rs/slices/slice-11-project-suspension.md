# Slice 11 — Project Suspension + Enforcement

**Goal**: Suspended project immediately rejects all SDK data requests with `PermissionDenied`.

## IN scope
- `POST /admin/v1/projects/{id}/suspend` → status: SUSPENDED
- `POST /admin/v1/projects/{id}/activate` → status: ACTIVE (un-suspend)
- Auth middleware checks project status on every data-plane request; SUSPENDED → `PERMISSION_DENIED: "project suspended"`
- Credential cache invalidation on status change (TTL=0 for suspended projects)
- `daily_project_metrics` table: per-project ingress_bytes, egress_bytes, cpu_ms written after each request
- Background sweeper: purge `deleted` projects' customer data after `admin.deletion_retention` (default 168h)

## OUT scope
- Billing aggregation / reporting (ETL consumer, not embyr's responsibility)
- Soft-delete (handled in slice 10 admin DELETE)

## Learning Hypothesis
Disproves: "SUSPENDED state check adds unacceptable per-request latency because it requires a DB read per request."
Confirms if: suspend takes effect within 1 second (cache TTL window) and p99 latency increase is < 1ms when cache is warm.

## Acceptance Criteria
- `POST .../suspend`: 200; within 1 second all SDK calls return `permission-denied`
- `POST .../activate`: 200; SDK calls succeed again
- Suspend on already-suspended project: 200 (idempotent)
- Activate on active project: 200 (idempotent)
- `daily_project_metrics` has a row for today after an SDK request

## Dependencies
S10 (admin API)

## Effort estimate
≤1 day
