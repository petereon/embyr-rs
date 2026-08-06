# Slice B-05 — Members, Service Accounts, Admin Keys

**Feature:** admin-api-v2
**Slice:** B-05 of B-06
**Estimate:** 1.5 days
**Stories:** US-B05
**Depends on:** B-04 complete

---

## Goal

Team management and programmatic access. Real RBAC enforcement. Required before real multi-user usage.

## Learning Hypothesis

Disproves: "The sole-Owner invariant can be bypassed by a concurrent DELETE + PATCH/role race condition."  
Confirms if succeeds: Sole-Owner removal returns 409 even under concurrent requests (Postgres UPDATE with row-level check).

## IN Scope

**Members (4 routes):**
- `GET /admin/v1/members`
- `POST /admin/v1/members/invite`
- `PATCH /admin/v1/members/:id/role`
- `DELETE /admin/v1/members/:id`

**Service Accounts (3 routes):**
- `GET /admin/v1/service_accounts`
- `POST /admin/v1/service_accounts`
- `DELETE /admin/v1/service_accounts/:id`

**Admin Keys (3 routes):**
- `GET /admin/v1/admin_keys`
- `POST /admin/v1/admin_keys`
- `DELETE /admin/v1/admin_keys/:key_id`

- Sole-Owner invariant: Postgres `SELECT COUNT(*) WHERE role='owner' AND revoked_at IS NULL` before remove/demote, under transaction
- Admin key auth middleware: alternative to session cookie, same route set
- `IEmailSender` port wired; `NoopEmailSender` in tests; `SmtpEmailSender` for production

## OUT Scope

- Invitation acceptance flow (recipient's sign-up page) — the invitation row is created; the UI shows "Pending"; click-through from email is V2
- Account ownership transfer (Danger Zone) — B-06

## Acceptance Criteria

- AC-B05-01 through AC-B05-11 (see feature-delta.md US-B05)

## Dependencies

- B-04 complete
- `IEmailSender` driven port defined in `embyr-core`
- `EMBYR_SMTP_*` env vars or `NoopEmailSender` configured

## Effort Estimate

1.5 days. 10 handlers. Invariant enforcement + concurrency test is the riskiest part. Admin key BLAKE3 auth path reuses pattern from B-01 session auth.
