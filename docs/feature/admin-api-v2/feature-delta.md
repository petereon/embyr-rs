# Feature Delta — admin-api-v2

**Feature ID:** `admin-api-v2`
**Wave:** DISCUSS
**Date:** 2026-07-29
**Status:** Complete — ready for DESIGN handoff
**Upstream:** `docs/feature/user-admin-ui/slices/slice-07-backend-api.md` (primary input)
**Spec ref:** `docs/feature/user-admin-ui/spec.md`

---

## Wave: DISCUSS / [REF] Persona

**Primary — P5: Chris (Account Admin / Platform Engineer)**  
`job_id: JOB-10` — account-admin

Customer-side account owner. Uses the web console (user-admin-ui) built in the prior feature. This backend feature is what makes the console's mock data layer real. Chris has no visibility into this work — they only see it as "the console actually shows my databases now."

**Secondary — P2: Sam (Service Operator)**  
`job_id: JOB-02, JOB-06` — tenant-provision + tenant-control

Embyr SaaS operator. Uses the existing 5 static-key routes. This feature must not break their workflow. Sam's routes (Bearer admin_key) coexist with Chris's routes (session cookie) on the same port.

---

## Wave: DISCUSS / [REF] JTBD

### Jobs consumed (existing)

| Job | Coverage in this feature |
|-----|--------------------------|
| JOB-10 (account-admin) | All new user-facing endpoints: projects list, SDK keys, members, billing, OIDC, query logs |
| JOB-02 (tenant-provision) | `GET /admin/v1/projects` list consumed by the UI; `POST /admin/v1/projects` already done |
| JOB-06 (tenant-control) | `GET /admin/v1/projects/:id/metrics` + `PATCH /admin/v1/projects/:id` logging toggle |
| JOB-04 (credential-isolation) | `PATCH /admin/v1/projects/:id` backend_mode switch + agent_endpoint update |
| JOB-05 (cloud-secret) | Same PATCH — secret ARN / GCP resource name field editable via UI |

No new jobs required — all endpoints trace to existing JTBD entries.

---

## Wave: DISCUSS / [REF] Scope Assessment

### Oversized signals

| Signal | Threshold | Observed |
|--------|-----------|----------|
| Endpoint count | >10 | 22 new routes |
| New DB tables | >5 | 10 new tables + 2 schema alterations |
| Bounded contexts touched | >3 | BC-1 (Tenant Management), BC-3 (Auth session), BC-2 (query_logs) |
| Effort estimate | >2 weeks | 5–8 days |
| Independent user outcomes | >1 | 6 (auth, project list, SDK keys, members, billing, OIDC+logs) |

**Result: OVERSIZED (5/5 signals). Feature split into 6 Elephant Carpaccio slices.**

### Scope Assessment: PASS (after slicing)

Walking Skeleton strategy: **B — Thin End-to-End Slice**. DB migrations + session auth (`POST /auth/signin` + `POST /auth/signout`) + `GET /admin/v1/projects` list. Proves session auth integrates with existing static-key routes on the same port, and that the dashboard shows real data.

---

## Wave: DISCUSS / [REF] Journey

Backend APIs have no end-user journey of their own — they exist to fulfil the user-admin journey documented in `docs/product/journeys/user-admin.yaml`. The relevant journey steps gated on this feature:

| Journey Step | Gating Endpoint |
|---|---|
| Step 2 (sign in) | `POST /admin/v1/auth/signin` |
| Step 3 (scan database cards) | `GET /admin/v1/projects` |
| Step 4/5 (create database) | `POST /admin/v1/projects` (exists) |
| Step 6 (connections tab) | `GET /admin/v1/projects/:id` (exists, expand response shape) |
| Step 7 (SDK key) | `GET + POST + DELETE /admin/v1/projects/:id/sdk_keys` |
| Step 8 (invite member) | `POST /admin/v1/members/invite` |
| Step 9 (billing) | `GET /admin/v1/billing` |
| Step 10 (sign out) | `POST /admin/v1/auth/signout` |

---

## Wave: DISCUSS / [REF] Story Map

### Backbone

```
Authenticate → List Databases → Manage Keys → Control Access → Observe Usage → Configure Account
```

### Slice map

| Slice | Stories | Routes | Est. |
|-------|---------|--------|------|
| B-01 — Auth + Migrations | US-B01 | `POST /auth/signin`, `POST /auth/signout` + 10 DB migrations | 1.5 d |
| B-02 — Project List + Detail | US-B02 | `GET /projects`, expand `GET /projects/:id` response | 0.5 d |
| B-03 — SDK Keys | US-B03 | `GET/POST/DELETE /projects/:id/sdk_keys` | 0.5 d |
| B-04 — Project Patch + Metrics | US-B04 | `PATCH /projects/:id`, `GET /projects/:id/metrics`, `GET /projects/:id/query_logs` | 1 d |
| B-05 — Members + Service Accounts + Admin Keys | US-B05 | 9 routes across members, service_accounts, admin_keys | 1.5 d |
| B-06 — OIDC + Billing | US-B06 | `GET/POST/PATCH/DELETE /oidc_providers`, `GET /billing` | 1 d |

### Prioritization (learning leverage order)

1. **B-01 (WS)** — highest uncertainty: does session cookie auth interoperate with existing static-key routes on the same port without middleware collision?
2. **B-02** — core dashboard; disproves whether the account-scoping query works correctly
3. **B-03** — SDK key CRUD is the most-used self-service action; unblocks developer onboarding
4. **B-04** — patch + metrics; validates that runtime config changes don't require restart
5. **B-05** — access control; gating for real multi-user usage; role enforcement must be correct before any other route goes live
6. **B-06** — OIDC and billing; lower daily traffic; billing is read-only from existing `daily_project_metrics`

---

## Wave: DISCUSS / [REF] User Stories

### US-B01: Session Authentication

**As** a user-admin (P5),  
**I want** to sign in with email + password + TOTP and receive a session cookie,  
**so that** I can authenticate to the admin UI without exposing a long-lived static key.

`job_id: JOB-10`  
`slice: B-01`

#### Elevator Pitch
Before: The only auth method is a static `EMBYR_ADMIN_KEY` Bearer token — a server secret unsuitable for browser use.  
After: `POST /admin/v1/auth/signin` with `{ email, password, totp_code }` → HTTP-only session cookie set → dashboard loads real data.  
Decision enabled: user-admin decides to proceed with creating a database without needing a raw API token.

#### Acceptance Criteria

- AC-B01-01: `POST /admin/v1/auth/signin` accepts JSON `{ email, password, totp_code }`. Valid credentials → `Set-Cookie: embyr_session=<token>; HttpOnly; Secure; SameSite=Strict; Path=/admin`. Body: `{ account_id, display_name, role }`. HTTP 200.
- AC-B01-02: Wrong password or unknown email → HTTP 401 `{ "message": "Invalid credentials" }`. Response does not reveal whether email exists.
- AC-B01-03: Wrong TOTP → HTTP 401 `{ "message": "Invalid or expired code" }`. No session set.
- AC-B01-04: Three consecutive TOTP failures within 15 min → account locked for 15 min; subsequent attempts → HTTP 429 `{ "message": "Too many failed attempts — account locked for 15 minutes" }`.
- AC-B01-05: Valid unused recovery code (from `mfa_recovery_codes`) → HTTP 200 + session. Code invalidated in DB immediately.
- AC-B01-06: `POST /admin/v1/auth/signout` with valid session cookie → cookie cleared (`Max-Age=0`), session row `expires_at` set to now. HTTP 204.
- AC-B01-07: All user-facing routes require session auth by default. Routes tagged `#[operator_only]` continue to use static Bearer `EMBYR_ADMIN_KEY`. Routes tagged `#[session_auth]` use the session cookie. The two middleware stacks never share a route.
- AC-B01-08: DB migrations execute cleanly in a fresh Postgres cluster: `accounts`, `account_members`, `users`, `oidc_providers`, `service_accounts`, `admin_api_keys`, `sdk_api_keys`, `sessions`, `invitations`, `query_logs` (partitioned by day). `projects` table gains `account_id UUID NOT NULL REFERENCES accounts(id)` and `logging_enabled BOOLEAN NOT NULL DEFAULT false` and `log_retention_days INT` columns.
- AC-B01-09: Session token stored as `BLAKE3(cookie_value)` in `sessions.token_hash`. Plaintext token never stored.
- AC-B01-10: Session expires after 24h of inactivity (`last_active_at` checked on each request; 24h idle = expired).
- AC-B01-11: `projects.account_id` FK is the sole account-scoping mechanism. All session-auth routes derive account scope from the session, then filter queries with `WHERE account_id = $account`.

---

### US-B02: Project List and Expanded Detail

**As** a user-admin,  
**I want** to list all databases in my account and retrieve full detail for one,  
**so that** the dashboard and database detail views can show real data instead of mocks.

`job_id: JOB-10`  
`slice: B-02`

#### Elevator Pitch
Before: `GET /admin/v1/projects/:id` returns only `{ status, backend_mode }`. No list endpoint exists.  
After: `GET /admin/v1/projects` → `[ { id, name, status, backend_mode, logging_enabled, created_at } ]`. Dashboard populates real cards.  
Decision enabled: user-admin sees their real databases and decides which one to open first.

#### Acceptance Criteria

- AC-B02-01: `GET /admin/v1/projects` (session auth) → 200 JSON array. Each item: `{ id, name, status, backend_mode, logging_enabled, created_at }`. Excludes `deleted` status projects. Filtered by `account_id` from session.
- AC-B02-02: Empty account → `[]` (not 404).
- AC-B02-03: `GET /admin/v1/projects/:id` (session auth) response expanded: `{ id, name, status, backend_mode, auth_mode, logging_enabled, log_retention_days, created_at }`. Returns 403 if project belongs to different account.
- AC-B02-04: Existing operator route `GET /admin/v1/projects/:id` (Bearer admin_key) continues to work unchanged for Sam. The two routes share the handler but use different auth middleware.
- AC-B02-05: Response time: `GET /admin/v1/projects` p99 ≤ 100ms for accounts with ≤ 50 databases (indexed by `account_id`).

---

### US-B03: SDK Key CRUD

**As** a user-admin with Owner or Admin role,  
**I want** to create and revoke SDK API keys scoped to a specific database,  
**so that** SDK developers can authenticate to that database and I can revoke access when needed.

`job_id: JOB-10`  
`slice: B-03`

#### Elevator Pitch
Before: SDK key creation requires a raw admin API call. No key list endpoint exists.  
After: `POST /admin/v1/projects/:id/sdk_keys` with `{ name }` → `{ id, key: "embyr_sdk_<32chars>", prefix }`. Key shown once. UI displays it in the copy-once modal.  
Decision enabled: user-admin copies the key, hands it to the SDK developer team, and sets a rotation reminder.

#### Acceptance Criteria

- AC-B03-01: `GET /admin/v1/projects/:id/sdk_keys` (session auth, any role) → `[ { id, name, prefix, created_at, last_used_at, revoked_at } ]`. `revoked_at` non-null entries included with their revoked state (for audit). Active-only filter: `?active=true`.
- AC-B03-02: `POST /admin/v1/projects/:id/sdk_keys` (Owner or Admin) body: `{ name }` → 201 `{ id, name, key, prefix, created_at }`. `key` = `embyr_sdk_<32 random chars>`. Stored as `BLAKE3(key)` in `sdk_api_keys.key_hash`. `prefix` = first 8 chars of key. Plaintext key never stored. `key` field absent from all subsequent GET responses.
- AC-B03-03: Generated SDK key is registered in `projects.auth_key_hash` via the `RotateAuthKey` command (BC-1 aggregate). The existing ECIES scheme links the API key to the ECIES private key — verify this integration works before merge (acceptance test must call a real Firestore RPC with the generated key against a test project).
- AC-B03-04: `DELETE /admin/v1/projects/:id/sdk_keys/:key_id` (Owner or Admin) → 204. Sets `revoked_at = now()`. The project's `auth_key_hash` is cleared if this was the active key; the project effectively becomes unauthenticated until a new key is created or the old one reverted (the auth layer checks `revoked_at`).
- AC-B03-05: Viewer role attempting POST or DELETE → 403.
- AC-B03-06: Deleting a project (`DELETE /admin/v1/projects/:id`) cascades: all `sdk_api_keys` for that project set `revoked_at = now()` in the same transaction.
- AC-B03-07: Key name max 64 chars. Empty name → 422.

---

### US-B04: Project Patch, Metrics, and Query Logs

**As** a user-admin,  
**I want** to update my database's backend configuration and logging settings, and read its metrics and query logs,  
**so that** I can manage the connection backend and troubleshoot slow queries without direct Postgres access.

`job_id: JOB-10, JOB-04, JOB-05, JOB-06`  
`slice: B-04`

#### Elevator Pitch
Before: `PATCH /admin/v1/projects/:id` doesn't exist. Logging state cannot be toggled via UI.  
After: `PATCH /admin/v1/projects/:id` with `{ logging_enabled: true, log_retention_days: 7 }` → 200. Logs tab shows entries immediately on next operation.  
Decision enabled: user-admin enables logging after seeing elevated P95 in the metrics chart.

#### Acceptance Criteria

- AC-B04-01: `PATCH /admin/v1/projects/:id` (session auth, Owner or Admin). Accepts any subset of: `{ name, backend_mode, backend_pg_dsn, backend_agent_endpoint, backend_secret_name, logging_enabled, log_retention_days, status }`. Partial update — omitted fields unchanged. 200 returns updated project object.
- AC-B04-02: `backend_pg_dsn` update: value ECIES-encrypted at the application layer before writing to `projects.backend_pg_creds_enc`. Plaintext DSN never stored. Cache eviction: `credential_cache.evict_project(id)` called after successful write.
- AC-B04-03: `status` field in PATCH can only be `suspended` or `active`. `deleted` is not patchable via this route (use DELETE). Attempting `status: "deleted"` → 422.
- AC-B04-04: `GET /admin/v1/projects/:id/metrics` (session auth, any role). Returns: `{ p95_read_ms, p95_write_ms, reads_today, writes_today, deletes_today, sparkline: [{ hour, reads, writes }] }`. V1: values from `daily_project_metrics` (non-hourly granularity; sparkline uses 24 equal buckets with last recorded value). 200. 404 if project not in account.
- AC-B04-05: `GET /admin/v1/projects/:id/query_logs` (session auth, any role). Query params: `op` (operation type filter), `prefix` (collection path prefix), `range` (`1h` / `6h` / `24h` / `7d`), `status` (`ok` / `error`), `sort` (`ts_desc` default). Returns `{ total, entries: [...] }`. Max 500 rows per page; cursor pagination via `?after=<id>`. 200. When `logging_enabled = false`: 200 `{ total: 0, entries: [] }`.
- AC-B04-06: Query logs written only when `projects.logging_enabled = true`. The storage write path (BC-2) checks this flag before inserting into `query_logs`. Disabling logging does not delete existing entries (they expire per `log_retention_days`).

---

### US-B05: Members, Service Accounts, and Admin Keys

**As** a user-admin with Owner or Admin role,  
**I want** to invite members, create service accounts, and issue admin API keys,  
**so that** my team and CI/CD pipelines have appropriately scoped, auditable, revocable access.

`job_id: JOB-10`  
`slice: B-05`

#### Elevator Pitch
Before: Team management requires raw admin API calls; no list endpoints for members or keys.  
After: `POST /admin/v1/members/invite` with `{ email, role }` → 202. Invitee receives email. `GET /admin/v1/members` → table of members with roles and last login.  
Decision enabled: user-admin grants a new team member Viewer role and upgrades to Admin once vetted.

#### Acceptance Criteria

**Members:**
- AC-B05-01: `GET /admin/v1/members` (session auth, any role) → `[ { id, email, display_name, role, auth_method, mfa_enabled, last_login_at, joined_at } ]`. Includes pending invites (`joined_at = null`).
- AC-B05-02: `POST /admin/v1/members/invite` (Owner or Admin). Body: `{ email, role }`. Creates `invitations` row. Sends invitation email via `IEmailSender`. 202 `{ invitation_id, email, role, expires_at }`. Invitations expire after 7 days.
- AC-B05-03: `PATCH /admin/v1/members/:id/role` (Owner or Admin). Body: `{ role }`. Sole Owner cannot demote self. Admin cannot change another Owner's role. 200. 403 on invariant violation with message.
- AC-B05-04: `DELETE /admin/v1/members/:id` (Owner or Admin). Last Owner → 409 `{ "message": "Transfer ownership before removing the last Owner." }`. Otherwise → 204. Cascade: invalidate all sessions for that member; revoke all admin_api_keys for that member.

**Service Accounts:**
- AC-B05-05: `GET /admin/v1/service_accounts` (any role) → `[ { id, name, description, role, created_at } ]`.
- AC-B05-06: `POST /admin/v1/service_accounts` (Owner or Admin). Body: `{ name, description?, role }`. 201 `{ id, name, role, created_at }`.
- AC-B05-07: `DELETE /admin/v1/service_accounts/:id` (Owner or Admin). 204. Cascade: revoke all admin_api_keys linked to this service account.

**Admin Keys:**
- AC-B05-08: `GET /admin/v1/admin_keys` (any role) → `[ { id, name, role, member_id?, service_account_id?, prefix, created_at, last_used_at, revoked_at } ]`.
- AC-B05-09: `POST /admin/v1/admin_keys` (Owner or Admin). Body: `{ name, member_id? | service_account_id?, role }`. Role must be ≤ creating user's role (Owners can create any; Admins cannot create Owner keys). 201 `{ id, key: "embyr_adm_<32chars>", prefix, role }`. Stored as `BLAKE3(key)`. Key shown once.
- AC-B05-10: `DELETE /admin/v1/admin_keys/:key_id` (Owner or Admin). 204. Sets `revoked_at`. Immediate — next request with that key → 401.
- AC-B05-11: Admin key auth middleware: checks `admin_api_keys.key_hash = BLAKE3(Bearer token)` AND `revoked_at IS NULL`. Updates `last_used_at` on each successful auth. This is an alternative to session cookie auth for the same user-facing routes.

---

### US-B06: OIDC Providers and Billing

**As** an account Owner,  
**I want** to configure OIDC providers for SSO and view per-database billing data,  
**so that** my team can sign in via GitHub/Google and I can understand usage costs.

`job_id: JOB-10`  
`slice: B-06`

#### Elevator Pitch
Before: OIDC provider configuration requires raw admin API calls. Billing data requires direct Postgres queries against `daily_project_metrics`.  
After: `GET /admin/v1/oidc_providers` → list of configured providers. `GET /admin/v1/billing?range=last_30d` → per-database breakdown table.  
Decision enabled: Owner enables GitHub OIDC SSO for the team and identifies the highest-traffic database.

#### Acceptance Criteria

**OIDC Providers:**
- AC-B06-01: `GET /admin/v1/oidc_providers` (Owner) → `[ { id, issuer, client_id, enabled, created_at } ]`. `client_secret_enc` never returned.
- AC-B06-02: `POST /admin/v1/oidc_providers` (Owner). Body: `{ issuer, client_id, client_secret }`. `client_secret` encrypted with AES-256-GCM using `EMBYR_ENCRYPTION_KEY` before storage. 201 `{ id, issuer, client_id, enabled: true }`.
- AC-B06-03: `PATCH /admin/v1/oidc_providers/:id` (Owner). Body: any subset of `{ issuer, client_id, client_secret, enabled }`. 200. Disabling a provider (`enabled: false`) does not sign out existing sessions authenticated via it.
- AC-B06-04: `DELETE /admin/v1/oidc_providers/:id` (Owner). 204. Does not sign out existing sessions.
- AC-B06-05: OIDC callback route: `GET /admin/v1/auth/oidc/callback?code=...&state=...`. Validates `id_token` issuer matches a configured, enabled `oidc_providers` row for the account. Sets session cookie on success. 302 redirect to `/admin/`. On validation failure → 401 redirect to `/admin/` with query `?error=oidc_failed`.

**Billing:**
- AC-B06-06: `GET /admin/v1/billing?range=<7d|30d|month|last_month>` (session auth, any role). Returns: `{ range, databases: [ { id, name, reads, writes, deletes, peak_connections, log_storage_bytes } ], totals: {...} }`. Sourced from `daily_project_metrics` WHERE `account_id = $account` GROUP BY project. 200.
- AC-B06-07: Peak connections column returns `null` (UI shows "—") — deferred per OQ-4 in spec.
- AC-B06-08: Projects with zero metrics rows still appear in the response with all counters as `0`.

---

## Wave: DISCUSS / [REF] Outcome KPIs

| KPI | Target | Measurement |
|-----|--------|-------------|
| Dashboard real-data load time | p95 ≤ 500ms from session cookie to dashboard cards | Server-side timing header on `GET /admin/v1/projects` |
| SDK key creation success rate | >99% of `POST .../sdk_keys` succeed on first try | Error rate on that route |
| Session auth regression rate | 0 incidents where operator Bearer-key routes broken by session middleware | Integration test suite: operator routes must pass with Bearer, fail with session cookie |
| Account invariant violations | 0 cases of last-Owner removal succeeding | Integration test for AC-B05-04 invariant |
| Query log query p99 | ≤ 200ms for accounts with ≤ 10,000 log rows per day | `EXPLAIN ANALYZE` on partitioned `query_logs` table |

---

## Wave: DISCUSS / [REF] Definition of Done

- [ ] All 6 story sets (B-01 through B-06) implemented and passing their ACs
- [ ] DB migrations run cleanly on a fresh Postgres cluster (automated test)
- [ ] Session auth and Bearer admin_key auth coexist on the same port without collision (integration test)
- [ ] Sole-Owner invariant enforced (cannot remove or demote last Owner)
- [ ] SDK key ECIES integration: generated key authenticates a real Firestore RPC call in integration test
- [ ] BLAKE3 hash storage: no plaintext keys in any DB column
- [ ] `EMBYR_ENCRYPTION_KEY` used for OIDC `client_secret_enc` and `totp_secret_enc`
- [ ] Query log partition: `query_logs` partitioned by day, old partitions pruned per `log_retention_days`
- [ ] Mutation testing kill rate ≥80% (per `per-feature` strategy in CLAUDE.md)
- [ ] Cargo workspace compiles without warnings (`cargo check --all`)

---

## Wave: DISCUSS / [REF] Out of Scope

- SMTP email delivery with real relay (stub `IEmailSender`; `NoopEmailSender` in tests)
- TOTP enrollment flow (users need TOTP pre-configured; enrollment UI is a future slice)
- Admin UI password reset / forgot-password flow
- Active connection counts (still "—"; V2 shared store)
- Pricing rates / invoice generation
- Live query log streaming (REST poll only; SSE is V2)
- Rate limiting on user-facing routes (inherits operator-route rate limiting in V2)
- `embyr-agent` binary changes (no agent-side work in this feature)

---

## Wave: DISCUSS / [REF] Locked Decisions

| # | Decision | Verdict | Rationale |
|---|----------|---------|-----------|
| D1 | Auth model: session cookie + BLAKE3 token hash | Accepted | HTTP-only cookie prevents JS token theft. BLAKE3 is fast for lookup; Argon2id not needed for session tokens (tokens are cryptographically random, not derived from passwords). |
| D2 | Same port (:9090), two middleware stacks | Accepted | Adding a new port would require infrastructure changes. Routes tagged `#[operator_only]` vs `#[session_auth]` are cleanly separated at the router level. No middleware collision possible — Axum route matching is exhaustive. |
| D3 | `projects.account_id` FK as sole account-scoping mechanism | Accepted | All session-auth queries must include `WHERE account_id = $session_account`. This is enforced in the handler, not in a shared middleware, to keep the join simple. Downstream risk: a missing WHERE clause leaks cross-account data — mitigated by integration tests that verify cross-account 403. |
| D4 | SDK key ECIES integration via `RotateAuthKey` command | Accepted | The existing ECIES scheme in `embyr-core` links an API key to an ECIES private key derived from `HKDF(api_key, project_id)`. SDK keys created via the UI must go through the same `RotateAuthKey` aggregate command to register the hash correctly. UI key creation cannot bypass BC-1. |
| D5 | `query_logs` partitioned by day | Accepted | Per spec OQ-2. High insert rate (every SDK operation when logging enabled). Daily partitions allow efficient pruning by simply dropping old partition tables rather than expensive DELETE sweeps. Partitioning key: `project_id + timestamp::date`. |
| D6 | Admin key auth as alternative to session cookie | Accepted | Service accounts and CI/CD need non-browser auth. Admin keys (`embyr_adm_<32>` stored as BLAKE3 hash) use the same session-auth middleware path but with a different token extraction step (Bearer header vs cookie). Role enforcement is identical. |
| D7 | `GET /admin/v1/projects/:id` expanded in-place (no version bump) | Accepted | Adding fields to a GET response is non-breaking. Existing operator clients (Sam) ignore unknown fields. No version bump needed. |

---

## Wave: DISCUSS / [REF] Wave Decisions

```markdown
# DISCUSS Decisions — admin-api-v2

## Key Decisions
- [D1] Session cookie auth: HTTP-only, BLAKE3 token hash, 24h idle expiry
- [D2] Same port (:9090), route-level auth tagging (not middleware-level mux split)
- [D3] account_id FK on projects table: sole account-scoping mechanism; enforced in handlers
- [D4] SDK keys go through RotateAuthKey aggregate command (BC-1); no shortcut
- [D5] query_logs partitioned by day; old partitions dropped by background sweeper

## Requirements Summary
- Primary job: JOB-10 — replace all mock data in user-admin-ui Slice 07
- Secondary jobs: JOB-04, JOB-05 (Connections patch), JOB-06 (metrics)
- WS scope: DB migrations + session auth + GET /admin/v1/projects (dashboard loads real data)
- Feature type: backend (22 new routes, 10 new DB tables)

## Constraints Established
- BLAKE3 hash storage: no plaintext credentials anywhere in DB
- EMBYR_ENCRYPTION_KEY required at startup (32-byte; refuse to start if absent when accounts exist)
- Operator routes (Bearer admin_key) must not be broken by any change in this feature
- SDK key generation must use existing ECIES scheme (RotateAuthKey command) — no bypass

## Upstream Changes
- `projects` table gains 3 new columns: account_id, logging_enabled, log_retention_days
- `projects.auth_key_hash` management moves from manual admin API to SDK key CRUD handlers
- No existing API response shapes changed (only extended with new fields — non-breaking)
```

---

## Wave: DISCUSS / [REF] Pre-requisites

- `docs/feature/user-admin-ui` slices 01–06 complete (all UI components exist; mock layer ready for replacement)
- `embyr-core` `RotateAuthKey` command defined on the `Project` aggregate (or confirmed equivalent path)
- `EMBYR_ENCRYPTION_KEY` env var provisioned in deployment environment
- `EMBYR_SMTP_*` env vars provisioned (or `NoopEmailSender` for tests)

---

## Wave: DESIGN / [REF] DDD List

> Mode: Propose | Date: 2026-07-29 | Author: Morgan (nw-solution-architect)

The feature extends **BC-1 (Tenant Management)** with an account-and-identity sub-domain, and extends **BC-2 (Document Storage)** with a diagnostic log record. No new bounded context is warranted — the vocabulary of accounts, users, sessions, and keys is clearly inside the "tenant management" language.

### New Aggregates and Domain Records (BC-1 — Tenant Management)

| # | Name | Type | Lives In | Key Invariants |
|---|------|------|----------|---------------|
| 1 | `Account` | Aggregate Root | System DB `accounts` | Identified by `AccountId (UUID)`. Has at least one `Owner` member at all times (last-Owner invariant). Accounts are not deleted in V1 (no self-service deactivation). |
| 2 | `User` | Aggregate Root | System DB `users` | Stores `email (unique)`, `Argon2id(password)`, `totp_secret_enc (AES-GCM)`, `failed_totp_attempts`, `locked_until`. Invariant: `failed_totp_attempts >= 3 within 15 min → locked_until = now() + 15 min`. |
| 3 | `Session` | Aggregate Root | System DB `sessions` | `token_hash = BLAKE3(cookie_value)`. Invariant: expires after 24h idle (`last_active_at`). Expired sessions are never re-activated — create a new session. |
| 4 | `AccountMember` | Entity (within Account) | System DB `account_members` | Junction between `User` and `Account` with `role: Owner|Admin|Viewer`. Invariant: last Owner cannot be removed or demoted. |
| 5 | `Invitation` | Supporting Record | System DB `invitations` | Pending invite: `email, role, expires_at (7 days), accepted_at`. Not an aggregate (no lifecycle commands beyond accept/expire). |
| 6 | `ServiceAccount` | Aggregate Root | System DB `service_accounts` | Non-human principal within an account. Has `name`, `description`, `role`. Can hold `AdminApiKey`s. |
| 7 | `AdminApiKey` | Aggregate Root | System DB `admin_api_keys` | `key_hash = BLAKE3(raw_key)`. `prefix = first_8_chars`. May be linked to a `User` or `ServiceAccount`. Invariant: `revoked_at IS NOT NULL` → key permanently invalid; cannot be un-revoked. |
| 8 | `SdkApiKey` | Supporting Record (within Project) | System DB `sdk_api_keys` | `key_hash = BLAKE3(raw_key)`. `prefix = first_8_chars`. Linked to exactly one `Project`. Invariant: creating a new SDK key triggers `RotateAuthKey` on the owning project (`projects.api_key_hash_current` update). |
| 9 | `OidcProvider` | Aggregate Root | System DB `oidc_providers` | `client_secret_enc = AES-GCM(EMBYR_ENCRYPTION_KEY, client_secret)`. Invariant: `client_secret_enc` never returned in any GET response. |

### New Domain Records (BC-2 — Document Storage)

| # | Name | Type | Lives In | Notes |
|---|------|------|----------|-------|
| 10 | `QueryLog` | Append-Only Record | System DB `query_logs` (partitioned by day) | Written fire-and-forget when `projects.logging_enabled = true`. `(project_id, timestamp::date)` partition key. Not queryable as an aggregate — read-only for diagnostic display. Pruned by `QueryLogSweeper`. |

### New Value Objects (BC-1 additions)

| Name | Definition |
|------|-----------|
| `SessionToken` | Cryptographically random 32-byte value formatted as base64url. Only the BLAKE3 hash is stored. |
| `AdminApiKeyValue` | `embyr_adm_<base64url(32 random bytes)>`. Only the BLAKE3 hash is stored. |
| `SdkApiKeyValue` | `embyr_sdk_<base64url(32 random bytes)>`. BLAKE3 hash stored in `sdk_api_keys`; Argon2id hash stored in `projects.api_key_hash_current`. |
| `TotpSecret` | TOTP shared secret. AES-GCM encrypted under `EMBYR_ENCRYPTION_KEY`; never stored plaintext. |
| `OidcClientSecret` | OIDC OAuth2 client secret. AES-GCM encrypted under `EMBYR_ENCRYPTION_KEY`. |
| `Role` | `Owner (3) > Admin (2) > Viewer (1)`. Comparable. Used in both `account_members` and `admin_api_keys`. |
| `SdkKeyMaterial` | Output of `new_sdk_key_material(raw_key)`: `{ argon2id_hash, ecies_pubkey, blake3_hash }`. Pure domain type. |

### Schema Changes (Projects Table — BC-1 extension)

The `projects` table gains three new columns (`account_id`, `logging_enabled`, `log_retention_days`) and a new column for server-side DSN encryption (`backend_pg_dsn_enc`). `account_id` is the FK linking projects to accounts — the sole account-scoping mechanism (D3).

---

## Wave: DESIGN / [REF] Component Decomposition

### New Source Modules in `embyr-server/src/admin/`

| Module Path | Responsibility | Slice |
|-------------|---------------|-------|
| `admin/state.rs` | Defines `OperatorState` (rename of `AdminState`) and `UserAdminState`. Both exported for sub-router construction. | B-01 |
| `admin/middleware/operator_auth.rs` | Tower `from_fn_with_state` middleware. Validates `Authorization: Bearer <EMBYR_ADMIN_KEY>`. Returns 401 if missing or mismatched. | B-01 |
| `admin/middleware/session_auth.rs` | Tower `from_fn_with_state` middleware. Tries session cookie BLAKE3 lookup, then admin_api_key BLAKE3 lookup. Sets `SessionContext` request extension on success. Returns 401 on failure. | B-01 |
| `admin/middleware/dual_auth.rs` | Tower middleware for `GET /admin/v1/projects/:id`. Tries session auth first, then operator Bearer. Sets `AuthPrincipal` extension. Returns 401 if neither validates. | B-02 |
| `admin/extractors/session_context.rs` | `FromRequestParts` impl for `SessionContext`. Reads cookie or admin_api_key Bearer, queries DB, returns `SessionContext { user_id, account_id, role }`. | B-01 |
| `admin/extractors/dual_auth_principal.rs` | `AuthPrincipal { User(SessionContext), Operator }` enum. Extractor used by `get_project` handler. | B-02 |
| `admin/handlers/auth.rs` | `signin` (POST), `signout` (POST), `oidc_callback` (GET). `signin`: validates email+password+TOTP, creates session row, sets HTTP-only cookie. `oidc_callback`: validates OIDC id_token, creates session, redirects. | B-01 |
| `admin/handlers/projects.rs` | `list_projects` (GET, session auth), `patch_project` (PATCH, session auth, Admin+). | B-02, B-04 |
| `admin/handlers/sdk_keys.rs` | `list_sdk_keys`, `create_sdk_key`, `revoke_sdk_key`. `create_sdk_key` calls `new_sdk_key_material` on blocking thread, writes atomically, evicts cache. | B-03 |
| `admin/handlers/metrics.rs` | `get_project_metrics` (GET, session auth, any role). Queries `daily_project_metrics`. | B-04 |
| `admin/handlers/query_logs.rs` | `list_query_logs` (GET, session auth, any role). Paginated cursor query on `query_logs` partition. | B-04 |
| `admin/handlers/members.rs` | `list_members`, `invite_member`, `change_member_role`, `remove_member`. `invite_member` calls `state.email_sender.send(...)`. | B-05 |
| `admin/handlers/service_accounts.rs` | `list_service_accounts`, `create_service_account`, `delete_service_account`. | B-05 |
| `admin/handlers/admin_keys.rs` | `list_admin_keys`, `create_admin_key`, `revoke_admin_key`. BLAKE3 hash storage. Role cap check via `check_rbac`. | B-05 |
| `admin/handlers/oidc_providers.rs` | `list_oidc_providers`, `create_oidc_provider`, `patch_oidc_provider`, `delete_oidc_provider`. `client_secret` AES-GCM encrypted with `EMBYR_ENCRYPTION_KEY`. | B-06 |
| `admin/handlers/billing.rs` | `get_billing` (GET, session auth, any role). Aggregates `daily_project_metrics` by account and time range. | B-06 |
| `admin/router.rs` (extend) | `build_admin_router(...)` merges four sub-routers. Replaces existing `build(...)` / `build_with_aws(...)` functions. | B-01 |

### New Modules in `embyr-core/src/admin/`

| Module Path | Responsibility |
|-------------|---------------|
| `embyr-core/src/admin/mod.rs` | Re-exports all admin domain types and port traits |
| `embyr-core/src/admin/account.rs` | `AccountId`, `User`, `UserId`, `Role`, `AccountMember` |
| `embyr-core/src/admin/session.rs` | `SessionContext`, `SessionToken` value types |
| `embyr-core/src/admin/rbac.rs` | `check_rbac(...)`, `RbacAction`, `RbacError` |
| `embyr-core/src/admin/email.rs` | `IEmailSender` trait, `EmailMessage`, `EmailError` |
| `embyr-core/src/admin/query_log.rs` | `IQueryLogWriter` trait, `QueryLogEntry`, `OperationType`, `OpStatus` |

### Extension to `embyr-core/src/domain/project.rs`

| Addition | Purpose |
|----------|---------|
| `SdkKeyMaterial { argon2id_hash, ecies_pubkey, blake3_hash }` | Output of new pure function |
| `fn new_sdk_key_material(raw_key: &[u8]) -> Result<SdkKeyMaterial, CoreError>` | Combines argon2id + ecies::derive_public_key + blake3; called on blocking thread |

### New Adapters in `embyr-server/src/adapters/`

| Module Path | Responsibility |
|-------------|---------------|
| `adapters/email.rs` | `NoopEmailSender` (V1): logs and returns Ok. `SmtpEmailSender` (V2): lettre SMTP client. Both implement `IEmailSender`. |
| `adapters/query_log.rs` | `PostgresQueryLogAdapter`: implements `IQueryLogWriter`. `record()` spawns Tokio task with bounded channel (1000). Inserts into `query_logs` partition. |

### New Background Task

| Component | Crate | Responsibility |
|-----------|-------|---------------|
| `QueryLogSweeper` | `embyr-server::sweepers` | Daily tokio task. Queries `projects` for `logging_enabled = true` and `log_retention_days`. Drops `query_logs_<project_id>_<date>` partition tables older than `log_retention_days`. DDL executed via `system_db` pool. Failure logged and retried next cycle. |

---

## Wave: DESIGN / [REF] Driving Ports (Inbound)

The admin-api-v2 extends the existing `AdminHttpPort` with a second principal type.

| Port | Location | Adapter | Auth Layer | What it does |
|------|----------|---------|-----------|--------------|
| `AdminHttpPort` (operator subport) | `embyr-server::admin::middleware::operator_auth` | `OperatorAuthMiddleware` (Tower) | Bearer EMBYR_ADMIN_KEY | Passes through only if static admin key matches. Routes: 4 existing operator routes. |
| `AdminHttpPort` (session subport) | `embyr-server::admin::middleware::session_auth` | `SessionAuthMiddleware` (Tower) | Session cookie OR admin_api_key Bearer | Validates against `sessions` or `admin_api_keys`. Sets `SessionContext` extension. Routes: 22 new user routes. |
| `AdminHttpPort` (public subport) | `embyr-server::admin::handlers::auth` | None (no middleware) | No auth | `POST /auth/signin`, `GET /auth/oidc/callback`. |
| `AdminHttpPort` (dual-auth subport) | `embyr-server::admin::middleware::dual_auth` | `DualAuthMiddleware` (Tower) | Either principal | `GET /admin/v1/projects/:id` only. Sets `AuthPrincipal` extension. |

---

## Wave: DESIGN / [REF] Driven Ports and Adapters

All driven port traits are defined in `embyr-core` (no IO imports). Concrete adapters in `embyr-server`.

| Port Trait | Defined In | Adapter(s) | Probe Required |
|-----------|-----------|------------|---------------|
| `IEmailSender` | `embyr-core::admin::email` | `NoopEmailSender` (V1), `SmtpEmailSender` (V2) | V1: no external substrate; probe trivially Ok. V2: probe EHLO handshake to SMTP server — hard failure. |
| `IQueryLogWriter` | `embyr-core::admin::query_log` | `PostgresQueryLogAdapter` | None — best-effort append-only; failure never fatal. Underlying `SystemDb` already probed at startup. |
| `SystemDb` (existing) | `embyr-server::adapters::system_db` | `SystemDb` (existing) | Existing startup probe (SELECT 1). No change. |
| `CredentialCache` (existing) | `embyr-server::adapters::credential_cache` | `CredentialCache` (existing) | No probe — in-process LRU; no external substrate. |

### New Startup Probe (B-01 Walking Skeleton gate)

A new startup probe is added to the composition root: **`EMBYR_ENCRYPTION_KEY` presence check**.

If the `accounts` table has any rows AND `EMBYR_ENCRYPTION_KEY` is absent or not exactly 32 bytes:
- Refuse to start: `health.startup.refused: encryption_key_missing`

If the `accounts` table is empty (fresh deployment), startup proceeds without the key but emits `health.startup.warn: encryption_key_not_configured`.

This is consistent with the existing "admin key present" startup probe pattern.

### Earned Trust — Session Store

The `sessions` table lookup in `SessionContextExtractor` is the authentication boundary for all user-facing routes. It is NOT a separate port trait — it uses the existing `SystemDb` connection pool. However, the extractor has two behavioral requirements that must be validated in CI:

1. **Expired session rejected:** A session with `expires_at < now()` must return 401. CI test: insert an expired session row, attempt request, assert 401.
2. **Revoked admin key rejected:** A row with `revoked_at IS NOT NULL` must return 401. CI test: create admin key, revoke it, attempt request with the revoked key, assert 401.

These are behavioral probe tests for the extractor, not adapter probes. They belong in the acceptance test suite (DISTILL wave).

---

## Wave: DESIGN / [REF] Technology Choices

All choices reuse existing workspace dependencies. No new dependencies required for V1.

| Concern | Technology | Version | License | Notes |
|---------|-----------|---------|---------|-------|
| Session token hashing | `blake3` | 1.x | CC0/Apache 2.0 | Already in workspace. `BLAKE3(cookie_value)` for `sessions.token_hash`. |
| Admin API key hashing | `blake3` | 1.x | CC0/Apache 2.0 | `BLAKE3(raw_key)` for `admin_api_keys.key_hash` and `sdk_api_keys.key_hash`. |
| OIDC client secret + TOTP encryption | `aes-gcm` (RustCrypto) | 0.10.x | MIT/Apache 2.0 | Already in workspace (via ECIES). AES-256-GCM under `EMBYR_ENCRYPTION_KEY`. |
| TOTP validation | `totp-rs` | 5.x | MIT | New dependency. RFC 6238 TOTP. Pure Rust, no C. See OQ-B03. |
| HTTP cookie parsing | `axum-extra` with `cookie` feature | 0.9.x | MIT | Axum companion crate for typed cookie extraction. |
| Password hashing (login) | `argon2` (RustCrypto) | 0.5.x | MIT/Apache 2.0 | Already in workspace. Same parameters (memory=65536, iter=3, par=4). |
| Email delivery (V1) | none — `NoopEmailSender` | — | — | No SMTP dependency in V1. |
| Email delivery (V2) | `lettre` | 0.11.x | MIT/Apache 2.0 | Async SMTP client. Add in V2 slice only. |

**New workspace dependencies (V1 only):**
- `totp-rs = { version = "5", default-features = false, features = ["gen_secret", "otpauth"] }` — in `embyr-server/Cargo.toml`
- `axum-extra = { version = "0.9", features = ["cookie", "typed-header"] }` — in `embyr-server/Cargo.toml`

Both are narrowly-scoped, widely-used crates. Bundle impact: none (server-side only; not compiled into the WASM bundle).

---

## Wave: DESIGN / [REF] Decisions Table

| ID | Decision | Verdict | ADR | Rationale |
|----|----------|---------|-----|-----------|
| AA-01 | Auth separation: sub-router merge with per-router Tower layers | Accepted | ADR-009 | Structural isolation; operator routes cannot accidentally run session middleware. Option B (extractors) and Option C (route_layer sequence) both have structural enforcement gaps. |
| AA-02 | `SessionContext` as `FromRequestParts` extractor | Accepted | ADR-010 | Compiler enforces presence; handler cannot forget to scope by `account_id` because `account_id` is only available via `SessionContext`. |
| AA-03 | `IEmailSender` in `embyr-core::admin::email` | Accepted | ADR-011 | Port traits belong in the inner hexagon. V1 `NoopEmailSender` has trivially-passing probe. V2 `SmtpEmailSender` adds real SMTP probe. |
| AA-04 | RBAC via pure domain function `check_rbac` | Accepted | ADR-012 | Non-trivial invariants (self-demotion, last-owner, key role cap) require domain logic, not just role comparison. Pure function: testable without Axum. |
| AA-05 | Query log writes: fire-and-forget spawn from gRPC handler | Accepted | ADR-013 | Zero client latency impact. Consistent with MetricsPort pattern. Bounded channel prevents unbounded task accumulation. |
| AA-06 | SDK key → `new_sdk_key_material` domain function; `backend_pg_dsn_enc` column | Accepted | ADR-014 | No ECIES duplication. D4 compliance: RotateAuthKey path is the canonical key registration route. `backend_pg_dsn_enc` resolves the old-key-unavailable problem for DSN re-encryption. |
| AA-07 | `embyr-core::admin` new sub-module | Accepted | — | Admin domain types and port traits belong in core alongside existing BC-1/BC-2/BC-3 modules. No new crate needed — existing `embyr-core` is the right home. |
| AA-08 | `UserAdminState` separate from `OperatorState` | Accepted | — | Different dependency graphs: `UserAdminState` needs `email_sender`, `encryption_key`; `OperatorState` needs `aws_secret_fetcher`, `gcp_secret_fetcher`. Merging them would require every route handler to carry the full dependency set. |
| AA-09 | `QueryLogSweeper` Tokio task for partition pruning | Accepted | — | `DELETE` sweeps on large partitioned tables are expensive and lock-prone. `DROP TABLE` on old partitions is instant and lock-free. Consistent with existing sweeper pattern (`TombstoneSweeper`, `DeletedProjectSweeper`). |

---

## Wave: DESIGN / [REF] Reuse Analysis

| Existing Component | File | Overlap | Decision | Justification |
|-------------------|------|---------|----------|---------------|
| `AdminState` struct | `embyr-server/src/admin/handlers/provision.rs` | Direct dependency for all 5 existing operator handlers | RENAME to `OperatorState` | `AdminState` becomes ambiguous when a second state type (`UserAdminState`) is introduced. Rename is a compile-time-safe refactor; all 5 handlers updated. |
| `extract_bearer` function | `embyr-server/src/admin/handlers/provision.rs` | Used by all 5 existing operator handlers | MOVE to `admin/middleware/operator_auth.rs` | Bearer extraction now centralized in the operator auth middleware. Handlers no longer perform manual auth. |
| `argon2::hash_api_key` | `embyr-core/src/auth/argon2.rs` | Identical Argon2id parameters needed for SDK keys | REUSE (called by `new_sdk_key_material`) | No duplication. Single call site in domain function. |
| `ecies::derive_public_key` | `embyr-core/src/auth/ecies.rs` | Identical ECIES derivation needed for SDK key → DSN re-encryption | REUSE (called by `new_sdk_key_material`) | No duplication. Same HKDF scheme. |
| `blake3::derive_cache_key` | `embyr-core/src/auth/blake3.rs` | Same hash function needed for session token storage and key storage | REUSE (renamed use: cache key → also storage hash) | The function `BLAKE3(input)` is the same operation regardless of what it's used for. Rename is a comment-level clarification only. |
| `CredentialCache::evict_project` | `embyr-server/src/adapters/credential_cache.rs` | Must be called after SDK key creation (new auth key invalidates cached DSN) | REUSE | Same call as in lifecycle.rs `suspend_project` / `delete_project`. |
| `SystemDb` Postgres pool | `embyr-server/src/adapters/system_db.rs` | All new admin routes query System DB | REUSE | `UserAdminState.system_db: Arc<SystemDb>` — same pool, shared. |
| `daily_project_metrics` queries | `embyr-server/src/adapters/metrics.rs` | Metrics endpoint reads from the same table the MetricsAdapter writes | REUSE (new query path only) | New `get_project_metrics` handler queries `daily_project_metrics` with a SELECT; existing MetricsAdapter does UPSERT. Same table, different operations. |
| `TombstoneSweeper` pattern | `embyr-server/src/sweepers/` | Same background task loop pattern | DERIVE | `QueryLogSweeper` follows identical structure: tokio sleep loop, failure logged and retried, no crash on error. |

**New Components (no existing equivalent):**

| Component | Justification |
|-----------|---------------|
| `embyr-core::admin` sub-module | No existing admin domain types. Creating alongside this feature. |
| `SessionContextExtractor` | No existing session mechanism in the codebase. |
| `operator_auth_middleware` | The existing inline `extract_bearer` pattern is promoted to a proper Tower middleware. |
| `session_auth_middleware` | No existing session-based auth anywhere in the codebase. |
| `dual_auth_middleware` | Specific to the one dual-access route; no existing equivalent. |
| All 10 new handler modules | No existing handlers for user-facing routes. |
| `PostgresQueryLogAdapter` | No existing query logging in the codebase. |
| `NoopEmailSender` | No existing email infrastructure. |
| DB migrations (10 new tables + 2 alterations) | All new schema. |

---

## Wave: DESIGN / [REF] Open Questions

| ID | Question | Blocking | Resolution Owner | Resolution Timing |
|----|----------|---------|-----------------|-------------------|
| OQ-B01 | DSN re-encryption for projects provisioned before admin-api-v2: `backend_pg_dsn_enc` cannot be backfilled. Do pre-existing projects permanently lose SDK key rotation DSN re-encryption? | No — fallback: skip DSN re-encryption when `backend_pg_dsn_enc IS NULL` (pre-existing projects keep existing `ecies_encrypted_dsn`). Document as a known limitation. | Crafter to confirm fallback behaviour in B-03 implementation. | Before B-03 merge. |
| OQ-B02 | `sessions.token_hash` column type: `BYTEA` (32 bytes) or `TEXT` (hex string)? | No — minor DDL decision. | Crafter. | B-01 migration DDL. Recommend `BYTEA` for storage efficiency. |
| OQ-B03 | TOTP library selection: `totp-rs` assumed. Is this the correct crate? Acceptance-designer needs TOTP behaviour confirmed to write B-01 ACs. | Yes — blocks B-01 implementation. | Platform-architect confirms crate; acceptance-designer verifies TOTP ACs. | Before B-01 crafter handoff. |
| OQ-B04 | Do admin API keys have idle-expiry (24h like sessions) or are they long-lived? AC-B05-11 says "immediate revocation only" — implying no idle expiry. | No — `admin_api_keys` table has no `last_active_at` + expiry logic per current design. | Acceptance-designer clarify in B-05 ACs. | Before B-05 crafter handoff. |
| OQ-B05 | `GET /admin/v1/projects/:id` via dual-auth for operator (Sam): the response shape includes `logging_enabled` and `log_retention_days` (expanded per D7). Does Sam's tooling ignore unknown fields? AC-B02-04 says "existing operator clients ignore unknown fields" — confirm. | No — AC-B02-04 asserts this; integration test confirms. | Crafter integration test in B-02. | Before B-02 merge. |

---

## Wave: DISTILL / [REF] Acceptance Test Suite

> Author: Quinn (nw-acceptance-designer) | Date: 2026-07-29

### Walking Skeleton

**Entry:** `tests/admin_api_v2/acceptance/walking_skeleton.rs`
**Journey:** POST /auth/signin → GET /admin/v1/projects → POST /auth/signout
**Litmus test:** "Can a user admin sign in and see their databases?"
**Status:** RED (panics at `AdminTestContext::new()` — correct pre-implementation state)

### Scenario List

| File | Count | Error/Edge | Error Ratio | First Unskip Target |
|------|-------|-----------|-------------|---------------------|
| `walking_skeleton.rs` | 1 | 0 | — | `admin_user_signs_in_views_databases_and_signs_out` |
| `b01_auth_migrations.rs` | 12 | 9 | 75% | `sign_in_with_valid_credentials_returns_session_cookie` |
| `b02_project_list.rs` | 8 | 4 | 50% | `authenticated_user_sees_only_their_account_databases` |
| `b03_sdk_keys.rs` | 9 | 5 | 56% | `owner_creates_sdk_key_and_key_shown_once_then_absent` |
| `b04_project_patch.rs` | 13 | 6 | 46% | `owner_enables_logging_and_new_entries_appear_in_logs` |
| `b05_members.rs` | 21 | 10 | 48% | `member_list_includes_pending_invitations` |
| `b06_oidc_billing.rs` | 13 | 6 | 46% | `oidc_provider_list_excludes_client_secret` |
| **Total** | **77** | **40** | **52%** | |

Target error ratio ≥40%: **MET (52%)**.

### Test Infrastructure

**Common module:** `tests/admin_api_v2/common/mod.rs`
- `AdminTestContext`: ephemeral Postgres (testcontainers-rs) + Axum admin server + reqwest::Client
- `FakeEmailSender`: captures emails for `IEmailSender` assertions (B-05)
- `assert_state_delta` re-exported from `tests/common/state_delta.rs`
- `universe` constants: port-exposed observable names for state-delta assertions

**Driving port mechanism:** `reqwest::Client` with `cookies` feature enabled

### Production Scaffold Files Created

**embyr-core additions:**
- `crates/embyr-core/src/admin/mod.rs`
- `crates/embyr-core/src/admin/account.rs` — AccountId, UserId, Role, AccountMember
- `crates/embyr-core/src/admin/session.rs` — SessionToken, SessionContext
- `crates/embyr-core/src/admin/rbac.rs` — check_rbac, RbacAction, RbacError
- `crates/embyr-core/src/admin/email.rs` — IEmailSender, NoopEmailSender
- `crates/embyr-core/src/admin/query_log.rs` — IQueryLogWriter, QueryLogEntry

**embyr-server additions:**
- `crates/embyr-server/src/admin/state.rs` — OperatorState, UserAdminState
- `crates/embyr-server/src/admin/middleware/operator_auth.rs`
- `crates/embyr-server/src/admin/middleware/session_auth.rs`
- `crates/embyr-server/src/admin/middleware/dual_auth.rs`
- `crates/embyr-server/src/admin/extractors/session_context.rs`
- `crates/embyr-server/src/admin/extractors/dual_auth_principal.rs`
- `crates/embyr-server/src/admin/handlers/auth.rs` — signin, signout, oidc_callback
- `crates/embyr-server/src/admin/handlers/projects.rs` — list_projects, patch_project
- `crates/embyr-server/src/admin/handlers/sdk_keys.rs`
- `crates/embyr-server/src/admin/handlers/metrics.rs`
- `crates/embyr-server/src/admin/handlers/query_logs.rs`
- `crates/embyr-server/src/admin/handlers/members.rs`
- `crates/embyr-server/src/admin/handlers/service_accounts.rs`
- `crates/embyr-server/src/admin/handlers/admin_keys.rs`
- `crates/embyr-server/src/admin/handlers/oidc_providers.rs`
- `crates/embyr-server/src/admin/handlers/billing.rs`
- `crates/embyr-server/src/adapters/email.rs`
- `crates/embyr-server/src/adapters/query_log.rs`

### Driven Adapter Coverage

Every driven adapter has at least one `@real-io @adapter-integration` scenario:

| Adapter | Scenario | File |
|---------|----------|------|
| Postgres migrations (testcontainers-rs) | `db_migrations_run_cleanly_on_fresh_postgres` | b01 |
| BLAKE3 session token storage | `session_token_stored_as_blake3_hash_not_plaintext` | b01 |
| BLAKE3 SDK key storage | `sdk_key_stored_as_blake3_hash_not_plaintext` | b03 |
| ECIES + RotateAuthKey | `sdk_key_authenticates_to_firestore_grpc_rpc` | b03 |
| AES-256-GCM DSN encryption | `backend_pg_dsn_never_stored_plaintext_after_patch` | b04 |
| BLAKE3 admin key storage | `admin_key_stored_as_blake3_not_plaintext` | b05 |
| AES-256-GCM OIDC secret encryption | `owner_creates_oidc_provider_and_secret_encrypted` | b06 |
| OIDC JWKS validation (mock JWKS server) | `oidc_callback_with_valid_id_token_grants_session` | b06 |
| FakeEmailSender (IEmailSender port) | `owner_invites_member_and_invitation_email_is_sent` | b05 |
| tokio paused clock | `session_expires_after_24h_of_inactivity` | b01 |

### Mandate Compliance Evidence

- **CM-A (driving port):** All scenarios invoke through `reqwest::Client` → Axum admin router (`:9090` port). Zero direct domain calls in Tier A tests except the sole-owner proptest.
- **CM-B (business language):** Zero technical terms in function names (functions use domain vocabulary: `sign_in_as_owner`, `owner_creates_sdk_key`, `removing_last_owner_returns_409`).
- **CM-C (walking skeleton + focused):** 1 walking skeleton, 76 focused scenarios across B-01–B-06.
- **Mandate 11 compliance:** Layer-3 sad paths are named example tests (`wrong_password_returns_401`, `cross_account_project_detail_returns_403`). No PBT machinery at layer 3.
- **Pillar 1:** Scenario function names use domain verbs — no HTTP status codes in function names, no technical acronyms (except domain-defined ones like TOTP, OIDC, BLAKE3, ECIES).
- **Pillar 2:** Chained narrative — B-02's `Given` reuses B-01's `sign_in_and_get_cookie` helper. B-03 extends with `sign_in_as_owner`. B-04–B-06 chain through the same auth helpers.
- **Pillar 3:** `AdminTestContext::new()` wires the production Axum admin router. No business logic in step methods.

### Pre-DISTILL OQ Resolutions

| OQ | Resolution | Evidence in Tests |
|----|-----------|-------------------|
| OQ-B01 | pre-existing null-DSN projects skip re-encryption | `pre_existing_project_with_null_dsn_enc_skips_re_encryption_silently` (b04) |
| OQ-B03 | totp-rs 5.x confirmed | `ctx.totp_code_now()` in all auth helpers |
| OQ-B04 | admin keys long-lived, no idle expiry | `@OQ-B04` tag on `admin_key_updates_last_used_at_on_successful_auth` (b05) |

### Prerequisites for DELIVER

1. `embyr_server::admin::build_admin_router(state: UserAdminState)` — must exist for `AdminTestContext::new()`.
2. B-01 migrations (10 new tables) — must run via `sqlx::migrate!`.
3. `totp-rs 5.x` in embyr-server Cargo.toml (V1 dep listed in DESIGN).
4. `axum-extra` with `cookie` feature in embyr-server Cargo.toml.
5. `proptest 1.x` now added to workspace and embyr-server dev-dependencies.
