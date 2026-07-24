# Slice 07 — Backend API Wiring (V2)

**Feature:** user-admin-ui  
**Slice:** 07 of 07  
**Estimate:** 5 days  
**Stories:** all 11  
**Depends on:** Slices 01–06 complete (all UI components, all Msg variants)

---

## Goal

Replace mock data layer (`data.rs`) with real axum routes in `embyr-admin`. Every `Resource` and `Action` body calls a real server function. Zero component changes — validates the TEA migration contract.

## Learning Hypothesis

Disproves: "The Resource/Action abstraction is leaky — components need changes when moving from mock to real data."  
Confirms if succeeds: Mock-to-real migration is a pure `data.rs` swap; all UI components, Msg variants, and update() logic are unchanged.

## IN Scope

### embyr-admin new axum routes

- `POST /admin/v1/auth/signin` — Argon2id verify + TOTP validate + session creation
- `POST /admin/v1/auth/signout` — session invalidation
- `GET /admin/v1/projects` — list databases for account
- `POST /admin/v1/projects` — create database
- `PATCH /admin/v1/projects/{id}` — patch db (status, config, logging)
- `DELETE /admin/v1/projects/{id}` — soft-delete
- `GET /admin/v1/projects/{id}/metrics` — KPI + chart data from `daily_project_metrics`
- `GET /admin/v1/projects/{id}/query_logs` — filterable log query
- `GET /admin/v1/projects/{id}/sdk_keys` / `POST` / `DELETE /{key_id}` — SDK key CRUD
- `GET /admin/v1/members` / `POST invite` / `PATCH /{id}/role` / `DELETE /{id}`
- `GET /admin/v1/service_accounts` / `POST` / `DELETE /{id}`
- `GET /admin/v1/admin_keys` / `POST` / `DELETE /{key_id}`
- `GET /admin/v1/oidc_providers` / `POST` / `PATCH /{id}` / `DELETE /{id}`
- `GET /admin/v1/billing?range=last_30d` — aggregated from `daily_project_metrics`

### embyr-admin-ui changes

- Replace `data.rs` mock bodies with `#[server]` fn calls (one fn per Resource/Action)
- Add `leptos_axum` integration — `LeptosOptions` wired at startup
- Update Cargo.toml: add `[features] ssr/hydrate` gates

### New tables (from spec Data Model)

Migrations: `accounts`, `account_members`, `users`, `oidc_providers`, `service_accounts`, `admin_api_keys`, `sdk_api_keys`, `sessions`, `invitations`, `query_logs` (partitioned)

## OUT Scope

- Hydration / SSR (stays pure CSR; `#[server]` fns are HTTP endpoints, not SSR)
- Email delivery in production (SMTP adapter wired but acceptance-tested with `NoopEmailSender`)
- Live connection counts (still "—" until V2 shared store)

## Acceptance Criteria

All ACs from US-001–011 that were labeled "V1 mock" upgrade to real behavior. Specifically:
- AC-001-02: Real Argon2id + TOTP validation
- AC-001-05: Real recovery code single-use invalidation
- AC-001-06: Real OIDC callback with `id_token` validation
- AC-003-02/04/05: Real DB create/suspend/delete persist to Postgres
- AC-005-03: Real PATCH persists backend config; next connection picks it up
- AC-006-02: Real `embyr_sdk_<uuid>` stored as BLAKE3 hash
- AC-007-06: Real query log rows from Postgres partitioned table
- AC-008-04: Real metrics from `daily_project_metrics`

## Dependencies

- All DB migrations merged
- `EMBYR_ENCRYPTION_KEY` (32-byte) set in environment
- `EMBYR_SMTP_*` env vars for invitation email (or `NoopEmailSender` in test)
- `embyr-core` driven port `IEmailSender` defined
