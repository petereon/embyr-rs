# Slice B-01 — Auth + DB Migrations (Walking Skeleton)

**Feature:** admin-api-v2
**Slice:** B-01 of B-06
**Estimate:** 1.5 days
**Stories:** US-B01
**Depends on:** user-admin-ui slices 01–06 complete

---

## Goal

Session auth endpoints + all 10 DB migrations. Dashboard signs in with a real session cookie and gets a real session back.

## Learning Hypothesis

Disproves: "Session cookie auth collides with the existing Bearer admin_key middleware on the same port."  
Confirms if succeeds: Two auth stacks coexist on :9090 with zero interference — operator routes reject session cookies, user-facing routes reject Bearer tokens.

## IN Scope

- DB migrations (run in order on the system DB):
  1. `accounts` table
  2. `users` table
  3. `account_members` table
  4. `sessions` table
  5. `invitations` table
  6. `oidc_providers` table
  7. `service_accounts` table
  8. `admin_api_keys` table
  9. `sdk_api_keys` table
  10. `query_logs` table (daily partitioned; parent table + trigger or range partition strategy)
  11. `ALTER TABLE projects ADD COLUMN account_id UUID REFERENCES accounts(id)`, `ADD COLUMN logging_enabled BOOLEAN NOT NULL DEFAULT false`, `ADD COLUMN log_retention_days INT`
- `POST /admin/v1/auth/signin` — Argon2id verify + TOTP check + session creation
- `POST /admin/v1/auth/signout` — cookie clear + session expiry
- Session auth middleware (Axum extractor: `SessionUser { id, account_id, role }`)
- Router annotation: existing operator routes tagged to require Bearer; new routes require session or admin key

## OUT Scope

- TOTP enrollment (users must have TOTP pre-set in DB for tests)
- OIDC callback handler (B-06)
- Account or user creation UI (seed data only in tests)
- Recovery codes UI

## Acceptance Criteria

- AC-B01-01 through AC-B01-11 (see feature-delta.md US-B01)

## Dependencies

- `EMBYR_ENCRYPTION_KEY` env var (32 bytes)
- System DB running (Postgres)
- Test seed: one `accounts` row + one `users` row with TOTP secret + one `account_members` row

## Effort Estimate

1.5 days. Reference class: auth middleware in embyr-server is ~150 LOC; DB migrations are mechanical; TOTP validation is well-supported by `totp-rs` crate.
