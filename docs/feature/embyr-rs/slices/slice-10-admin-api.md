# Slice 10 — Admin API (CRUD Project, direct_pg Backend)

**Goal**: Operator can provision, inspect, and delete a project via the admin API; DB migrations run on creation.

## IN scope
- `POST /admin/v1/projects` (create): validates project_id format (`^[a-z][a-z0-9-]{0,62}$`), hashes auth_key with Argon2id, runs DB migrations, returns project record
- `GET /admin/v1/projects/{id}` (read): returns project record (no raw key, no DSN)
- `PATCH /admin/v1/projects/{id}` (update): auth_mode, auth_key rotation (dual-hash window), backend config
- `DELETE /admin/v1/projects/{id}` (soft-delete): marks deleted_at, returns 200; data purge async
- Admin API on separate port (config: `admin.port`, default 9090)
- `Authorization: Bearer <admin_key>` on all admin endpoints
- Duplicate project_id → 409
- `direct_pg` backend only this slice

## OUT scope
- `aws_secret`, `gcp_secret`, `agent` backends (slices 13, 14)
- Suspend endpoint (slice 11)
- Index management endpoints (slice 05)
- Project listing

## Learning Hypothesis
Disproves: "Admin project provisioning with DB migrations is a multi-day integration due to schema coordination complexity."
Confirms if: `POST /admin/v1/projects` with a real Postgres DSN applies all migrations and returns 201 in a single request.

## Acceptance Criteria
- `POST` with valid body: 201, project record returned, migrations applied, no DSN in response
- `POST` with invalid project_id: 400 with message
- `POST` duplicate: 409
- `GET` existing project: 200 with correct fields
- `GET` non-existent: 404
- `DELETE`: 200; subsequent `GET` returns 404
- Admin endpoint rejects request with wrong admin_key: 401
- Admin endpoint unreachable on data port (only on `admin.port`)

## Dependencies
S01 (DB adapter)

## Effort estimate
≤1 day
