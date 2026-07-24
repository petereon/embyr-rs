# Feature Delta — user-admin-ui

**Feature ID:** `user-admin-ui`
**Wave:** DISCUSS
**Date:** 2026-06-05
**Status:** Complete — ready for DESIGN handoff
**Spec ref:** `docs/feature/user-admin-ui/spec.md`, `docs/superpowers/specs/2026-06-05-embyr-admin-ui-design.md`

---

## Wave: DISCUSS / [REF] Persona

**Primary — P5: Chris (Account Admin / Platform Engineer)**

Customer-side account owner. Manages one embyr account with 2–20 databases, a small team (2–10 members), and programmatic access via admin API keys. Pays the bill. Not the embyr SaaS operator (that is P2/Sam). Not exclusively an SDK developer (that is P1/Alex), though the same person often holds both roles at small companies.

**Secondary — P3: Morgan (Tenant Admin / DevOps Lead)**

Configures cloud-managed secrets (JOB-05). Uses the Connections panel to wire agent_mode databases to AWS/GCP secret backends without storing DSNs in embyr SaaS.

**Secondary — P1: Alex (SDK Developer)**

Observes database health and copies SDK keys. Uses Dashboard and the Keys sub-tab. Does not manage members or billing.

---

## Wave: DISCUSS / [REF] JTBD

### Existing jobs consumed

| Job | Coverage in this feature |
|-----|--------------------------|
| JOB-04 (credential-isolation) | Connections panel → agent_mode backend config; DSN never leaves VPC |
| JOB-05 (cloud-secret) | Connections panel → `aws_secretsmanager` / `gcp_secretmanager` secret name config |

### Gap: JOB-10 (account-admin) — new job

No existing job covers the customer's self-service account management workflow. JOB-02 (tenant-provision) is structurally similar but models the SaaS operator provisioning customer accounts, not the customer managing databases within their own account.

**Job story (JOB-10):**
> When I have an embyr account with multiple databases and a team of developers, I want a web UI to create and monitor databases, manage team access, view usage costs, and rotate SDK/admin keys, so I can operate self-sufficiently without contacting the embyr SaaS operator or writing admin API scripts.

**Dimensions:**
- Functional: Create/list/detail databases; view P95 latency + ops charts; manage Members + Service Accounts; create/revoke SDK + Admin keys; view billing breakdown; configure OIDC + account settings
- Emotional: Feel in control of the account; feel like a self-sufficient operator rather than a dependent customer; feel confident keys and access are under my governance
- Social: Be seen by the engineering team as running a mature, access-controlled data platform; demonstrate clean secrets hygiene to auditors

**Four forces:**
- Push: No UI exists; all management requires raw admin API calls or asking the embyr operator; error-prone and time-consuming
- Pull: Dashboard + CRUD + key management via browser = complete self-service in minutes; no operator dependency
- Anxiety: "What if I accidentally revoke a key that's in production?" — mitigated by confirm-before-revoke modal and key prefix display so admins can identify the right key
- Habit: Admins expect a web console (AWS Console, Vercel dashboard, Supabase studio mental model); CLI-only access creates friction and slows onboarding

**Opportunity score:** 16 (importance: 9/10 — every customer needs this; satisfaction: 0/10 — no UI exists)
**Priority:** critical

---

## Wave: DISCUSS / [REF] Scope Assessment

### Oversized signals

| Signal | Threshold | Observed |
|--------|-----------|----------|
| User story count | >10 | 11 |
| Bounded contexts | >3 | 4 (Auth, Database Mgmt, Identity/Access, Billing) |
| Effort estimate | >2 weeks | 5–8 weeks total |
| Independent user outcomes | >1 | 5 (auth+dashboard / db CRUD / identity / billing / settings) |

**Result: OVERSIZED (4/5 signals). Feature split into 7 Elephant Carpaccio slices.**

### Scope Assessment: PASS (after slicing)

Walking Skeleton strategy: **B — Thin End-to-End Slice**. Leptos crate scaffolded, served from embyr-admin, auth gate + dashboard render with mock data. No new domain model, no real backend calls. Proves tech stack and deployment pipeline in one slice.

---

## Wave: DISCUSS / [REF] Journey

### Happy path — Account Admin (P5/Chris)

| Step | Action | Expected output | Emotional state |
|------|--------|----------------|-----------------|
| 1 | Navigate to `https://<host>/admin/` | Login form renders | Neutral — familiar pattern |
| 2 | Enter email + password + TOTP code | Redirect to Dashboard | Relief — authentication worked |
| 3 | Scan database cards | See all databases with status + P95 latency | Oriented — aware of health at a glance |
| 4 | Click "New Database" | Create form with name + backend mode | Confident — self-service workflow |
| 5 | Submit create form | New database card appears | Satisfied — no operator involvement |
| 6 | Open database → Connections tab | See backend config, edit DSN or agent endpoint | In control — connection details visible |
| 7 | Open Keys tab | Create SDK key, copy it once | Purposeful — key ready to share with dev team |
| 8 | Navigate to Identities | Invite team member via email + role | Trusted — team member onboarded |
| 9 | Navigate to Billing | View 30-day usage breakdown per database | Informed — usage vs budget clear |
| 10 | Sign out | Session invalidated | Complete |

**Emotional arc:** Neutral → Relief → Oriented → Confident → Satisfied → In control → Purposeful → Trusted → Informed → Complete (progressive confidence build ✓)

### Shared artifacts registry

| Artifact | Source step | Consumed by |
|----------|-------------|-------------|
| `session_cookie` | Step 2 (login response) | All subsequent requests |
| `account_id` | Step 2 (session) | All sections |
| `db_id` | Step 4/5 (create or select database) | Database detail, Connections, Keys, Logs |
| `sdk_key` (plaintext, shown once) | Step 7 (key creation modal) | Shared with SDK developers |
| `invite_token` | Step 8 (invitation email) | Recipient's sign-up flow |

### Critical error paths

| Step | Failure | Recovery |
|------|---------|----------|
| Step 2 | Wrong password | Generic 401 message (no email-existence hint); retry |
| Step 2 | Expired TOTP | "Code expired — enter the current 6-digit code from your app" |
| Step 2 | Lost MFA → use recovery code | Recovery code input shown via "Use recovery code" link |
| Step 5 | Database name conflict | Inline form error: "Name already in use in this account" |
| Step 6 | Invalid DSN format | Inline validation before save |
| Step 7 | Key revoked in production | Confirm modal: "Revoking a key is immediate and cannot be undone. In-flight requests will fail." |

---

## Wave: DISCUSS / [REF] Story Map

### Backbone (user activities)

```
Sign In → Monitor Health → Manage Databases → Control Access → Observe Costs → Configure Account
```

### Walking Skeleton (Slice 01)

**Sign In → Monitor Health** (mock data): login form + TOTP flow + dashboard with database cards.

Proves: Leptos 0.8 WASM SPA served from embyr-admin axum binary on :9090 within 5MB bundle constraint.

### Slice map

| Slice | Stories | Activities | Est. |
|-------|---------|-----------|------|
| 01 — WS | US-001, US-002 | Sign In + Monitor Health (mock) | 2 d |
| 02 — DB Management | US-003, US-004 | Manage Databases CRUD + Overview (mock) | 2 d |
| 03 — Connections + Keys | US-005, US-006 | Configure Connections + SDK Keys (mock) | 1 d |
| 04 — Identity + Admin Keys | US-009, US-010 | Control Access — Members + Service Accounts + Admin Keys (mock) | 2 d |
| 05 — Billing + Logs | US-007, US-008 | Observe Costs + Query Logs (mock) | 1 d |
| 06 — Settings | US-011 | Configure Account + OIDC (mock) | 1 d |
| 07 — Backend Wiring (V2) | all | Replace mock layer with real axum routes | 5 d |

### Prioritization (learning leverage order)

1. **Slice 01 (WS)** — highest uncertainty: does Leptos WASM fit the 5MB budget inside embyr-admin ServeDir?
2. **Slice 02 (DB Management)** — core value; disproves whether self-service DB creation is viable
3. **Slice 03 (Connections + Keys)** — serves JOB-04 / JOB-05; unblocks teams waiting on agent_mode config UI
4. **Slice 04 (Identity)** — access control; dependency for real multi-user usage
5. **Slice 05 (Billing + Logs)** — lower urgency; mock data low-risk
6. **Slice 06 (Settings)** — owner-only; low daily traffic
7. **Slice 07 (Backend Wiring)** — validates TEA Resource/Action migration path; no component changes expected

---

## Wave: DISCUSS / [REF] User Stories

### US-001: Authentication Gate

**As** a user-admin,  
**I want** to sign in to my embyr account via email+password+MFA or OIDC,  
**so that** I can access the admin UI securely and start managing my account.

`job_id: JOB-10`  
`slice: 01`

#### Elevator Pitch
Before: No browser UI exists; access requires raw admin API calls with Bearer token.  
After: navigate to `/admin/` → sees login form → enters email + password → enters 6-digit TOTP → redirected to Dashboard showing database cards.  
Decision enabled: user-admin decides to proceed to create a database or review existing ones without contacting the embyr operator.

#### Acceptance Criteria

- AC-001-01: GET `/admin/` returns the Leptos SPA HTML (200, Content-Type: text/html). WASM bundle loads within 3s on a 10 Mbps connection.
- AC-001-02: Submitting valid email + valid password + valid 6-digit TOTP → HTTP-only session cookie set, redirect to Dashboard.
- AC-001-03: Submitting valid email + valid password + expired/wrong TOTP → stays on login form, error message: "Invalid or expired code — enter the current 6-digit code from your authenticator app." No session cookie set.
- AC-001-04: Three consecutive TOTP failures → account temporarily locked for 15 minutes; error message states lockout duration.
- AC-001-05: "Use recovery code" link visible on MFA step. Entering a valid unused recovery code → session granted; that code invalidated (cannot be reused).
- AC-001-06: OIDC flow (when enabled): "Sign in with [Provider]" button → redirect to OIDC provider → callback validates `id_token` issuer and audience → session cookie set, redirect to Dashboard.
- AC-001-07: Authentication failure response never reveals whether the email exists (generic "Invalid credentials" for both wrong-email and wrong-password cases).
- AC-001-08: V1 mock: any non-empty password + any 6 digits → granted. Real validation is V2 (#[server] fn).
- AC-001-09: Session cookie is HTTP-only, Secure, SameSite=Strict. Session expires after 24h of inactivity.
- AC-001-10: POST `/admin/signout` invalidates session cookie and redirects to login form.

---

### US-002: Dashboard — Multi-Database Health Overview

**As** a user-admin,  
**I want** to see all my account's databases on a single dashboard with health KPIs,  
**so that** I can identify databases that need attention before drilling in.

`job_id: JOB-10`  
`slice: 01`

#### Elevator Pitch
Before: Health check requires a separate API call per database; no single-pane view.  
After: post-login → Dashboard renders a card grid showing each database's name, status badge, P95 read latency (last hour), and today's op counts.  
Decision enabled: admin decides which database to open first based on the latency or status shown.

#### Acceptance Criteria

- AC-002-01: Dashboard renders a card per database in the account. Zero databases → empty state with "Create your first database" prompt.
- AC-002-02: Each card displays: database name, status badge (`active` / `suspended` / `deleted`), P95 read latency label ("—" in V1 mock), today's read/write/delete op counts ("—" in V1 mock).
- AC-002-03: Clicking a card navigates to that database's Overview tab (DB Detail view).
- AC-002-04: "New Database" button in topbar navigates to the create database form.
- AC-002-05: Suspended databases render a muted/dimmed card with a `suspended` badge; deleted databases are not shown.
- AC-002-06: V1 mock: KPI values populated from `data.rs` mock data. V2: Resource fetches from real API.

---

### US-003: Database Management — Create, List, Suspend, Delete

**As** a user-admin with Owner or Admin role,  
**I want** to create, suspend/activate, and delete databases from the Databases section,  
**so that** I can provision and lifecycle-manage databases without operator involvement.

`job_id: JOB-10`  
`slice: 02`

#### Elevator Pitch
Before: Creating a database requires a POST to the admin API with correct JSON payload; no UI.  
After: Databases → "New Database" → fill name + backend mode → click Create → new database appears in the list immediately.  
Decision enabled: admin decides whether to configure the backend connection now or share mock SDK key with the team first.

#### Acceptance Criteria

- AC-003-01: Databases section shows a table with columns: Name, Status, Backend Mode, Created, Actions.
- AC-003-02: "New Database" form: name (required, unique within account, immutable after creation), backend mode (`direct_pg` | `agent_mode`). Submit → new row appears in table.
- AC-003-03: Name uniqueness validated inline before submit; error shown if duplicate: "Name already in use in this account."
- AC-003-04: Suspend action (Owner/Admin only): sets status to `suspended`; confirm modal shown before action. Reactivate returns status to `active`.
- AC-003-05: Delete action (Owner/Admin only): sets `deleted_at`; confirm modal with database name re-entry. Cascades to revoke all SDK keys for that database. Deleted databases no longer appear in list.
- AC-003-06: Viewer role: table visible, Actions column hidden.
- AC-003-07: V1 mock: mutations update the in-memory AppModel only; V2: Action calls server fn.

---

### US-004: Database Detail — Overview (KPIs, Charts, Logging Toggle)

**As** a user-admin,  
**I want** to see per-database KPIs, a latency chart, an ops bar chart, and a logging toggle on the Overview tab,  
**so that** I can monitor health and decide whether to enable query logging for troubleshooting.

`job_id: JOB-10`  
`slice: 02`

#### Elevator Pitch
Before: P95 latency and op counts require separate metrics API calls; no visual trend.  
After: click a database card → Overview tab → sees P95 latency tile + 24h latency sparkline + ops bar chart + logging enabled/disabled toggle.  
Decision enabled: admin decides to enable query logging after seeing elevated latency in the chart.

#### Acceptance Criteria

- AC-004-01: Overview tab renders: P95 latency tile (ms), latency sparkline (24 data points, last 24h), ops bar chart (reads/writes/deletes per hour, last 24h), active connections label ("—" V1).
- AC-004-02: All chart data is "—" or flat mock in V1. V2 fetches from `/admin/v1/projects/{id}/metrics`.
- AC-004-03: Logging toggle: default OFF. Toggling ON prompts for retention period (1d / 7d / 30d) before enabling. Toggle OFF shows confirmation modal ("Existing log entries are retained until expiry").
- AC-004-04: Logging state persisted in AppModel immediately (V1 mock). V2: PATCH to server fn.
- AC-004-05: Back breadcrumb ("← Databases") returns to the database list.

---

### US-005: Database Connections — Backend Configuration

**As** a user-admin with Owner or Admin role,  
**I want** to view and edit the backend connection configuration for each database,  
**so that** I can confirm my DSN or agent endpoint is correct and update it if the backend changes.

`job_id: JOB-04, JOB-05`  
`slice: 03`

#### Elevator Pitch
Before: Backend config is visible only via admin API GET; no UI to edit or verify.  
After: Connections tab → shows masked DSN (host:port/dbname only, credentials hidden) for direct_pg, or agent endpoint + secret backend + secret name for agent_mode → edit fields → Save.  
Decision enabled: admin confirms correct backend config before sharing SDK credentials with developers.

#### Acceptance Criteria

- AC-005-01: Connections tab Panel 1 shows: mode label, plus mode-specific fields:
  - `direct_pg`: Postgres DSN masked to `host:port/dbname` (credentials never shown in UI)
  - `agent_mode`: Agent endpoint (host:port), Secret Backend (`env` / `aws_secretsmanager` / `gcp_secretmanager`), Secret Name/ARN (when AWS or GCP)
- AC-005-02: Edit (Owner/Admin only): fields become editable, Save button appears. Cancel discards changes.
- AC-005-03: Save emits `Msg::PatchDb` updating AppModel. V2: PATCH to server fn. Change takes effect on next connection attempt (UI shows info banner: "Config saved — takes effect on next connection").
- AC-005-04: Panel 2 (Active Connections): shows "—" with "V2" badge explaining multi-pod deferral.
- AC-005-05: Viewer role: edit controls hidden; read-only view only.

---

### US-006: Database API Keys — Create and Revoke SDK Keys

**As** a user-admin with Owner or Admin role,  
**I want** to create and revoke SDK API keys scoped to a specific database,  
**so that** I can give SDK developers access to that database and revoke access when credentials are compromised.

`job_id: JOB-10`  
`slice: 03`

#### Elevator Pitch
Before: SDK key creation requires an admin API call; no UI visibility into existing keys.  
After: Keys tab (per-database) → "Create Key" → enter name → modal shows full key ONCE with copy button → key appears in table with prefix and last-used date.  
Decision enabled: admin decides to share the key with the SDK team and sets a rotation reminder.

#### Acceptance Criteria

- AC-006-01: Keys table (per-database) shows: Name, Created, Last Used ("—" V1), Prefix (first 8 chars), Revoke action.
- AC-006-02: Create flow: name field → submit → modal shows `embyr_sdk_<32 chars>` once. Modal text: "This key will not be shown again. Copy it now." Copy-to-clipboard button present.
- AC-006-03: Closing the modal without copying: "I've copied the key" confirmation checkbox required before dismiss.
- AC-006-04: Revoke: confirm modal ("Revoking this key is immediate and cannot be undone. In-flight SDK requests will fail.") → row removed from table.
- AC-006-05: Revoking a database (US-003) cascades: all SDK keys for that database immediately revoked.
- AC-006-06: Viewer role: table visible, Create + Revoke controls hidden.

---

### US-007: Query Logs — Enable, Filter, and Browse

**As** a user-admin,  
**I want** to enable query logging for a database and browse the filtered log table,  
**so that** I can diagnose slow queries and audit operation patterns without direct Postgres access.

`job_id: JOB-10`  
`slice: 05`

#### Elevator Pitch
Before: Per-operation visibility requires direct Postgres queries against `query_logs`; no self-service access.  
After: enable logging toggle (from Overview) → navigate to Logs tab → filter by operation type + time range → see table of operations with duration and status.  
Decision enabled: admin identifies the collection path causing elevated P95 latency and routes it to the engineering team.

#### Acceptance Criteria

- AC-007-01: Logs tab visible for all roles; empty state when logging is disabled: "Logging is off — enable it on the Overview tab."
- AC-007-02: Log table columns: Timestamp (UTC), Operation, Collection Path, Duration (ms), Status, Client (last 6 chars of SDK key).
- AC-007-03: Filter controls: Operation type (multi-select), Collection path prefix (text input), Time range (1h / 6h / 24h / 7d / custom), Status (ok / error).
- AC-007-04: Default sort: Timestamp descending. Clicking column header toggles sort direction.
- AC-007-05: "Export CSV" button: downloads up to 10,000 rows of the current filtered view as CSV.
- AC-007-06: V1 mock: 50 synthetic log rows from `data.rs`. V2: fetches from `/admin/v1/projects/{id}/query_logs`.

---

### US-008: Billing — Usage Summary and Per-Database Breakdown

**As** a user-admin,  
**I want** to view usage metrics for my account broken down by database and time range,  
**so that** I can identify cost-driving databases and understand my usage trajectory.

`job_id: JOB-10`  
`slice: 05`

#### Elevator Pitch
Before: Usage data requires direct Postgres queries against `daily_project_metrics`; no self-service view.  
After: Billing → select "Last 30 days" → table shows each database's read/write/delete ops + peak connections + log storage.  
Decision enabled: admin identifies the highest-traffic database and decides whether to impose soft quotas.

#### Acceptance Criteria

- AC-008-01: Billing section renders a time range selector (Last 7d / Last 30d / This month / Last month) and a per-database breakdown table.
- AC-008-02: Table columns: Database name, Read Ops, Write Ops, Delete Ops, Peak Connections, Log Storage (GB). Total row at bottom.
- AC-008-03: "—" shown for Peak Connections (V1 deferred, per OQ-4 in spec).
- AC-008-04: Pricing rates not displayed. Numbers only (raw ops + storage GB).
- AC-008-05: V1 mock: values from `data.rs`. V2: fetches from `daily_project_metrics` grouped by selected range.

---

### US-009: Members — Invite, Manage Roles, Remove

**As** a user-admin with Owner or Admin role,  
**I want** to invite team members by email, assign roles, and remove members from the account,  
**so that** I can control who has management access without operator involvement.

`job_id: JOB-10`  
`slice: 04`

#### Elevator Pitch
Before: Adding team members requires admin API calls; no UI visibility into existing members.  
After: Identities → Members → "Invite" → enter email + role → invitee receives email → joins account → appears in Members table.  
Decision enabled: admin grants a new team member the minimum-privilege Viewer role and upgrades to Admin once vetted.

#### Acceptance Criteria

- AC-009-01: Members table columns: Email, Display Name, Role, Auth Method (OIDC / Email+Password), MFA Enabled, Last Login, Actions.
- AC-009-02: Invite flow (Owner/Admin): email + role selector → submit → invitation email sent; pending invitees appear in table with "Pending" last-login value.
- AC-009-03: Invitations expire after 7 days; expired invitations removed from table.
- AC-009-04: Role change (Owner/Admin): any member's role changeable except: the sole Owner cannot demote themselves; Owner cannot be demoted by an Admin.
- AC-009-05: Remove (Owner/Admin): confirm modal → member removed immediately, session invalidated, all admin API keys held by that member revoked.
- AC-009-06: Account invariant: last Owner cannot be removed. UI disables Remove action for the sole Owner with tooltip: "Transfer ownership first."
- AC-009-07: Viewer role: Members table visible, Invite + Role Change + Remove actions hidden.

---

### US-010: Service Accounts and Admin API Keys

**As** a user-admin with Owner or Admin role,  
**I want** to create service accounts and associate admin API keys with them,  
**so that** I can grant CI/CD pipelines and automation tools programmatic admin access with auditable, revocable credentials.

`job_id: JOB-10`  
`slice: 04`

#### Elevator Pitch
Before: Programmatic admin access requires sharing a user's credentials or calling the admin API to issue keys manually.  
After: Identities → Service Accounts → "New Service Account" → name + role → then API Keys → "Create Key" → link to service account → copy key once → CI/CD pipeline uses it.  
Decision enabled: admin decides to scope the CI key to Viewer role (read-only) and create a separate Admin key for deployment pipelines.

#### Acceptance Criteria

- AC-010-01: Service Accounts table: Name, Description, Role, Created, Last Used (from linked key), Actions (edit role, delete).
- AC-010-02: Create service account: name + optional description + role. No credentials at creation.
- AC-010-03: Delete service account (Owner/Admin): confirm modal → immediately invalidates all admin API keys associated with this service account.
- AC-010-04: Admin API Keys section (account-level): table columns: Name, Associated Identity (member or service account), Role, Created, Last Used, Prefix (first 8 chars), Revoke.
- AC-010-05: Create admin key: select associated identity (member or service account) → role inherited from identity → name → submit → modal shows `embyr_adm_<32 chars>` once. Same copy-once UX as SDK keys (AC-006-02, AC-006-03).
- AC-010-06: Revoke: immediate. Confirm modal states irreversibility.
- AC-010-07: Viewer role: tables visible, Create + Revoke actions hidden.

---

### US-011: Settings — Account Config, OIDC Providers, Danger Zone

**As** an account Owner,  
**I want** to configure account settings including trusted OIDC providers and access the danger zone (delete account, transfer ownership),  
**so that** I can manage SSO authentication and perform high-stakes account lifecycle actions safely.

`job_id: JOB-10`  
`slice: 06`

#### Elevator Pitch
Before: OIDC provider configuration requires admin API calls; no UI for account-level settings.  
After: Settings → OIDC Providers → "Add Provider" → enter issuer URL + client ID + client secret → Save → members can now sign in via that OIDC provider.  
Decision enabled: Owner enables GitHub OIDC SSO for the team, eliminating password-based logins.

#### Acceptance Criteria

- AC-011-01: Settings sections: Account (display name edit), Auth Methods (toggle Email+Password / OIDC per-provider), Security (session lifetime setting — future), Danger Zone.
- AC-011-02: OIDC Providers list: Issuer, Client ID, Enabled toggle, Delete action.
- AC-011-03: Add OIDC provider (Owner only): issuer URL + client ID + client secret → Save. Client secret encrypted at rest (AES-256-GCM via `EMBYR_ENCRYPTION_KEY`). Secret never returned in UI after save (edit shows masked placeholder).
- AC-011-04: Toggling OIDC provider off: members using that provider are not immediately signed out but cannot re-authenticate via that provider.
- AC-011-05: Danger Zone — Delete Account (Owner only): requires typing account name to confirm. Deletes `accounts` row (soft-delete via `deleted_at`). Cascade: revokes all sessions, suspends all databases. Non-reversible from UI (operator intervention required to restore).
- AC-011-06: Danger Zone — Transfer Ownership (Owner only): enter new owner's email → new owner receives confirmation email → accepts → role swapped. Original owner becomes Admin.
- AC-011-07: All Settings actions restricted to Owner role. Admin and Viewer see Settings section as read-only (OIDC list visible, no edit controls).

---

## Wave: DISCUSS / [REF] Outcome KPIs

| KPI | Target | Measurement |
|-----|--------|-------------|
| Self-service database creation rate | >90% of new databases created via UI (not raw admin API) within 60 days of launch | Admin API call log: ratio of UI-originated vs. direct API requests |
| Operator escalation tickets | <5 tickets/month for "help me create/manage my database" after launch | Support ticket tagging |
| Mean time to SDK key (MTTK) | <3 minutes from login to first SDK key copied | Frontend telemetry: `login_complete` → `sdk_key_copied` event duration |
| WASM bundle size | <5 MB (hard constraint from CLAUDE.md) | `trunk build --release` output, measured in CI |
| Auth completion rate | >95% of login attempts reach Dashboard (excluding intentional sign-outs) | Frontend telemetry: `auth_attempt` → `dashboard_rendered` funnel |

---

## Wave: DISCUSS / [REF] Definition of Done

- [ ] All 11 user stories implemented and passing their ACs
- [ ] WASM bundle size verified <5 MB (`trunk build --release` in CI)
- [ ] `embyr-admin` serves `/admin/` path from `ServeDir` (integration test)
- [ ] TEA model: all Msg variants handled in `update()` with no panics
- [ ] All primitives render without visual regression (screenshots in CI, V2)
- [ ] Auth gate: mock flow completes end-to-end in browser
- [ ] No `unwrap()` in production code paths; all error states surface as `Msg::PushToast`
- [ ] Mutation testing kill rate ≥80% (per `per-feature` strategy in CLAUDE.md)
- [ ] SSOT updated: `jobs.yaml` JOB-10 added, `journeys/user-admin.yaml` created

---

## Wave: DISCUSS / [REF] Out of Scope (V1)

- SSR / hydration (pure WASM SPA)
- Real server function bodies (all data from mock; V2 work)
- Live active connection counts (shown as "—"; V2 — per OQ-3 in spec)
- `leptos_sse` / `leptos-server-signal` (V2, once Subscribe stream counts implemented)
- URL-based deep linking / browser back-button navigation (nav state is in-memory; V2 with `leptos_router`)
- Tweaks panel (design-time tool, excluded from production build)
- Email delivery in V1 mock (SMTP adapter is V2; invitations shown in table but no email sent)
- Payment method management / invoice generation (out of scope forever per spec)

---

## Wave: DISCUSS / [REF] Locked Decisions

| # | Decision | Verdict | Rationale |
|---|----------|---------|-----------|
| D1 | Frontend stack | Leptos 0.8 CSR WASM | OQ-8 in spec; avoids TypeScript/JS context switch; shared types with embyr-admin crate |
| D2 | V1 data layer | Mock only (`data.rs`) | Decouples UI completion from backend API work; Resource/Action pattern enables zero-component-change V2 migration |
| D3 | Build pipeline | `trunk build --release` → `ServeDir` in embyr-admin | Per design spec; no `cargo build` coupling |
| D4 | Active connections display | "--" with V2 badge | OQ-3 in spec; multi-pod counter not implementable in V1 without shared store |
| D5 | WS strategy | B — thin end-to-end slice | Leptos+WASM bundle size is the highest-uncertainty assumption; WS slice validates it cheaply |
| D6 | JTBD gap | JOB-10 added | No existing job modeled customer self-service account management; JOB-02 is SaaS operator perspective |

---

## Wave: DISCUSS / [REF] Wave Decisions

```markdown
# DISCUSS Decisions — user-admin-ui

## Key Decisions
- [D1] Leptos 0.8 CSR WASM: OQ-8 in spec locked this before DISCUSS; no re-evaluation (see: docs/superpowers/specs/2026-06-05-embyr-admin-ui-design.md)
- [D2] Mock-only V1: decouples UI sprint from backend API sprint; Resource/Action migration requires zero component changes
- [D5] Walking Skeleton = thin slice: Leptos WASM budget is the highest-risk assumption; validate first
- [D6] New job JOB-10 added: customer self-service management was absent from jobs.yaml

## Requirements Summary
- Primary job: JOB-10 (account-admin) — self-service database, access, and billing management via browser
- Secondary jobs: JOB-04 (credential-isolation), JOB-05 (cloud-secret) — served by Connections panel
- Walking skeleton scope: auth gate + dashboard with mock data; proves Leptos→embyr-admin pipeline
- Feature type: user-facing (browser SPA)

## Constraints Established
- WASM bundle <5 MB (hard, from CLAUDE.md + design spec)
- No deep linking in V1 (in-memory nav state)
- Active connections = "--" in V1 (multi-pod constraint)
- Real backend wiring deferred to V2 (Slice 07)

## Upstream Changes
- No DISCOVER wave ran; spec (`docs/feature/user-admin-ui/spec.md`) treated as primary evidence
- JOB-10 added to jobs.yaml — assumption: customer admins exist as a distinct persona (P5) from SaaS operator (P2)
```

---

## Changed Assumptions

**From spec:** The spec (`docs/feature/user-admin-ui/spec.md`) was written without reference to `jobs.yaml`. It defines the "user-admin" persona implicitly without assigning a job ID.

**DISCUSS resolution:** This DISCUSS wave introduces JOB-10 (`account-admin`) to fill the gap and assigns P5 (Chris, Account Admin) as the primary persona. All 11 user stories trace to JOB-10 (or JOB-04/05 for Connections). The spec's functional requirements are unchanged — only the JTBD traceability layer is added.

---

## Wave: DISCUSS / [HOW] Gherkin Scenarios

### Feature: Authentication Gate (US-001)

```gherkin
Feature: Authentication Gate
  Background:
    Given the embyr-admin UI is served at "/admin/"
    And account "acme" exists with one Owner member "chris@acme.com"
    And "chris@acme.com" has password "correct-password" and TOTP secret "BASE32SECRET"

  # Happy path
  Scenario: Successful email+password+TOTP login
    Given I navigate to "/admin/"
    When I enter email "chris@acme.com" and password "correct-password"
    And I enter the current valid TOTP code for "BASE32SECRET"
    And I click "Sign In"
    Then I am redirected to the Dashboard
    And a session cookie is set (HTTP-only, Secure, SameSite=Strict)
    And the Dashboard shows database cards for account "acme"

  # Wrong password
  Scenario: Login fails with wrong password
    Given I navigate to "/admin/"
    When I enter email "chris@acme.com" and password "wrong-password"
    And I click "Sign In"
    Then I remain on the login form
    And I see the message "Invalid credentials"
    And no session cookie is set
    And the message does not indicate whether the email exists

  # Wrong TOTP
  Scenario: Login fails with expired TOTP code
    Given I have entered valid email and password
    When I enter an expired 6-digit TOTP code
    And I click "Verify"
    Then I remain on the MFA step
    And I see the message "Invalid or expired code — enter the current 6-digit code from your authenticator app"
    And no session cookie is set

  # Lockout after 3 TOTP failures
  Scenario: Account locked after three consecutive TOTP failures
    Given I have entered valid email and password
    When I enter an incorrect TOTP code 3 times in a row
    Then I see the message "Too many failed attempts — account locked for 15 minutes"
    And subsequent login attempts within 15 minutes are rejected immediately
    And no session cookie is set

  # Recovery code
  Scenario: Login succeeds via recovery code
    Given I have entered valid email and password
    And I am on the MFA step
    When I click "Use recovery code"
    And I enter a valid unused recovery code "ABCD-EFGH-IJKL"
    And I click "Verify"
    Then I am redirected to the Dashboard
    And recovery code "ABCD-EFGH-IJKL" is invalidated (cannot be reused)
    And a session cookie is set

  # Recovery code reuse rejected
  Scenario: Consumed recovery code cannot be reused
    Given recovery code "ABCD-EFGH-IJKL" has already been consumed
    When I attempt to log in with recovery code "ABCD-EFGH-IJKL"
    Then I see the message "Invalid or expired recovery code"
    And no session cookie is set

  # Sign out
  Scenario: Sign out invalidates session
    Given I am signed in as "chris@acme.com"
    When I click "Sign Out" in the avatar menu
    Then my session cookie is cleared
    And I am redirected to the login form
    And a subsequent request to "/admin/" with the old cookie returns 401
```

### Feature: SDK API Key Management (US-006)

```gherkin
Feature: SDK API Key Management
  Background:
    Given I am signed in as Owner "chris@acme.com"
    And database "my-db" exists in my account

  # Create key — shown once
  Scenario: Create SDK key shows full key exactly once
    Given I navigate to the Keys tab for "my-db"
    When I click "Create Key"
    And I enter name "prod-key"
    And I click "Create"
    Then a modal appears showing a key matching "embyr_sdk_[a-zA-Z0-9]{32}"
    And the modal contains a copy-to-clipboard button
    And the modal contains a checkbox "I've copied the key" (unchecked)
    And the "Done" button is disabled until the checkbox is checked
    When I check "I've copied the key" and click "Done"
    Then the modal closes
    And "prod-key" appears in the Keys table with prefix (first 8 chars) and Created date
    And Last Used shows "—"
    And the full key is no longer accessible in the UI

  # Key prefix visible for identification
  Scenario: Key prefix helps identify the correct key to revoke
    Given keys "prod-key" and "staging-key" exist for "my-db"
    When I view the Keys table
    Then each key shows its first 8 characters as a prefix
    And I can identify "prod-key" by its prefix before deciding to revoke

  # Revoke requires confirmation
  Scenario: Revoke key shows irreversibility warning
    Given key "old-key" exists for "my-db"
    When I click "Revoke" next to "old-key"
    Then a confirm modal appears with the text "Revoking this key is immediate and cannot be undone. In-flight SDK requests will fail."
    When I click "Confirm Revoke"
    Then "old-key" is removed from the Keys table immediately

  # Viewer cannot create or revoke
  Scenario: Viewer role sees keys but cannot create or revoke
    Given I am signed in as Viewer "alice@acme.com"
    When I navigate to the Keys tab for "my-db"
    Then I see the Keys table with existing keys
    And the "Create Key" button is not present
    And the "Revoke" action is not present in any row
```

### Feature: Member Management (US-009)

```gherkin
Feature: Member Management
  Background:
    Given I am signed in as Owner "chris@acme.com"
    And account "acme" has members:
      | email            | role   |
      | chris@acme.com   | Owner  |
      | alice@acme.com   | Admin  |
      | bob@acme.com     | Viewer |

  # Invite flow
  Scenario: Invite new member sends invitation
    When I navigate to Identities → Members
    And I click "Invite"
    And I enter email "dan@acme.com" and select role "Viewer"
    And I click "Send Invite"
    Then "dan@acme.com" appears in the Members table with Last Login "Pending"
    And an invitation email is queued (V2; V1: pending entry only)

  # Expired invitation removed
  Scenario: Expired invitation is removed from the table
    Given "dan@acme.com" was invited 8 days ago and has not accepted
    When I view the Members table
    Then "dan@acme.com" is not shown in the table

  # Role change
  Scenario: Admin promotes Viewer to Admin
    When I click the role selector for "bob@acme.com"
    And I select "Admin"
    Then "bob@acme.com" role updates to "Admin" in the table

  # Sole Owner cannot be demoted
  Scenario: Sole Owner cannot demote themselves
    Given "chris@acme.com" is the only Owner in the account
    When I view the role selector for "chris@acme.com"
    Then the "Admin" and "Viewer" options are disabled
    And a tooltip reads "Transfer ownership first"

  # Remove member
  Scenario: Remove member with confirmation
    When I click "Remove" next to "alice@acme.com"
    Then a confirm modal appears
    When I click "Confirm Remove"
    Then "alice@acme.com" is removed from the Members table
    And any Admin API keys held by "alice@acme.com" are revoked

  # Last Owner cannot be removed
  Scenario: Last Owner cannot be removed
    Given "chris@acme.com" is the only Owner
    When I view the Members table
    Then the "Remove" button for "chris@acme.com" is disabled
    And a tooltip reads "Transfer ownership first"
```

---

## Wave: DISCUSS / [WHY] Alternatives Considered

### D1: Frontend stack — Leptos 0.8 CSR WASM

**Decision:** Leptos 0.8 CSR WASM (`crates/embyr-admin-ui`, served via trunk + ServeDir).

**Alternatives weighed:**

| Option | Verdict | Rejection reason |
|--------|---------|-----------------|
| React/TypeScript SPA | Rejected | Introduces JS toolchain (npm, webpack/vite) into a pure-Rust workspace; requires context-switch for contributors; shared type definitions between Rust server and TypeScript UI require manual duplication or codegen |
| Elm | Rejected | TEA pattern is exactly what Leptos provides; Elm adds a foreign-language runtime, no shared types with embyr-admin |
| Server-rendered Axum HTML (Askama/Minijinja templates) | Rejected | No reactive interactivity without adding HTMX or Alpine; complex forms (logging toggle + modal + chart) become painful; not aligned with the existing design prototype which assumes component-based rendering |
| HTMX + Askama (hypermedia) | Considered seriously | Fits the Rust-only constraint; would work for CRUD-heavy sections. Rejected because: (1) pure SVG charts with mousemove crosshair require JS event handling anyway, (2) modal + clipboard API need JS regardless, (3) Leptos CSR uses identical pattern for both simple CRUD and interactive charts — no mixed approach needed |
| Leptos 0.8 SSR + hydration | Deferred to V2 | Migration path is explicitly designed in spec: add `ssr/hydrate` features, add `#[server]` — no component changes. V1 CSR with mock data is faster to validate the tech stack and design |

**Why Leptos CSR won:** Single-language workspace, shared type definitions between `embyr-admin` and `embyr-admin-ui` without codegen, TEA pattern built-in via `RwSignal` + `Callback`, pure SVG charts in Rust, zero JS toolchain. Bundle size risk is real but measurable at WS slice.

---

### D2: Mock-first V1 vs implement real API first

**Decision:** All V1 slices use mock data from `data.rs`; real axum routes deferred to Slice 07.

**Alternatives weighed:**

| Option | Verdict | Rejection reason |
|--------|---------|-----------------|
| Build backend API and UI in parallel | Rejected | Creates a blocking dependency between two workstreams; if the API schema changes, UI components must change too — mock-first breaks this coupling |
| Build backend API first, then UI | Rejected | Delays any UI feedback; the highest-uncertainty assumption (Leptos bundle size) isn't validated until late; design prototype already exists so UI-first iteration is lower risk |
| Mock-first V1 → real V2 (chosen) | Chosen | Resource/Action abstraction means component code is unchanged when swapping mock for `#[server]` fn; WS slice validates tech stack in days, not weeks; UI can ship to staging with mock data while backend migration work proceeds in parallel |

**Migration guarantee:** The `Resource::new(|| (), move |_| async move { mock::databases() })` body is the ONLY thing that changes in V2 — `dispatch(Msg::SetDatabases(dbs))` is identical. This is testable: a V2 acceptance test confirms zero component diffs.

---

### Cross-cutting: Single SPA vs micro-frontends for 4 bounded contexts

**Decision:** Single Leptos SPA serving Auth + DB Management + Identity/Access + Billing.

**Alternatives weighed:**

| Option | Verdict | Rejection reason |
|--------|---------|-----------------|
| Separate SPA per bounded context (4 mini-apps) | Rejected | 4× build pipelines, 4× bundle size monitoring, shared nav/session state requires cross-SPA messaging — massive operational overhead for a V1 admin console with a small team |
| Monolith SPA with module boundaries enforced in code | Chosen | Single trunk build, single ServeDir mount, shared session context via `use_context`. Module boundaries maintained by directory structure (`views/auth.rs`, `views/databases.rs` etc.) — equivalent to micro-frontend isolation but without the deployment overhead |
| Separate routes served by separate binaries | Rejected | Session cookie would need to be shared across subdomains or a reverse proxy added; far exceeds the scope of an admin console |

**Why single SPA:** The 4 bounded contexts share `account_id`, `session_cookie`, and nav state. In a micro-frontend architecture these become cross-boundary shared artifacts requiring a message bus. A single SPA with directory-level module boundaries delivers the same isolation at 1/4 the operational cost.

---

### Walking Skeleton strategy — B vs A/C/D

**Decision:** Strategy B — thin end-to-end slice (auth + dashboard, mock data).

**Alternatives (per WS strategy taxonomy):**

| Strategy | Description | Verdict |
|----------|-------------|---------|
| A — Hardcoded | Return hardcoded HTML, no real components | Rejected — doesn't validate Leptos WASM build pipeline; too thin to be useful |
| B — Thin E2E (chosen) | Real component tree, real routing, real mock data, deployed to embyr-admin | Chosen — validates the full pipeline (trunk → dist → ServeDir → browser) while keeping scope to 2 days |
| C — Feature complete, no polish | Implement all sections, rough styling | Rejected — too large for a WS; defeats the purpose of validating the highest-risk assumption (bundle size) cheaply |
| D — Configurable (env-switch) | WS behind a feature flag, real data behind another | Rejected — adds indirection; no env-switching needed since V1 is entirely mock and V2 is a code change, not a flag change |

**Risk being validated by WS:** The Leptos 0.8 WASM binary + all required web-sys features may push the bundle past 5 MB. Discovering this at WS (2 days in) costs far less than discovering it at Slice 06 (9 days in).

---

## Wave: DESIGN / [REF] Component Decomposition

New workspace crate: `crates/embyr-admin-ui/`

**Core TEA layer:**

| Component | File | Responsibility | Slice |
|-----------|------|---------------|-------|
| WASM entry | `src/main.rs` | `mount_to_body(App)`, `console_error_panic_hook` | 01 |
| Domain model | `src/model.rs` | `AppModel` struct + all domain types (`Database`, `Member`, `NavState`, `Section`, `DbTab`, etc.) | 01 |
| Message enum | `src/msg.rs` | `Msg` — 30+ variants, `Clone`, exhaustive | 01 |
| Pure update | `src/update.rs` | `fn update(&mut AppModel, Msg)` — no IO, no async, deterministic | 01 |
| Mock data | `src/data.rs` | `mock::databases()`, `mock::members()`, `mock::sdk_keys()`, `mock::billing_usage()`, `mock::query_logs()` | 01 |
| App root | `src/app.rs` | Creates `RwSignal<AppModel>` + `Callback<Msg>`; provides both via Leptos context | 01 |

**Primitives (`src/components/primitives/`):** Button, Badge, Card, Modal, Input, Toggle, Tabs, Menu — each typed to CSS contract in `styles.css`.

**Charts (`src/components/charts/`):** Sparkline, LatencyChart, BarChart, Donut — pure SVG, no JS library.

**Views:**

| View | File | Slice |
|------|------|-------|
| Auth gate | `src/views/auth.rs` | 01 |
| Dashboard | `src/views/dashboard.rs` | 01 |
| Database list | `src/views/databases.rs` | 02 |
| DB Overview | `src/views/db_detail/overview.rs` | 02 |
| DB Connections | `src/views/db_detail/connections.rs` | 03 |
| DB Logs | `src/views/db_detail/logs.rs` | 05 |
| DB Keys | `src/views/db_detail/keys.rs` | 03 |
| Billing | `src/views/billing.rs` | 05 |
| Identities | `src/views/identities.rs` | 04 |
| Admin API Keys | `src/views/api_keys.rs` | 04 |
| Settings | `src/views/settings.rs` | 06 |

---

## Wave: DESIGN / [REF] Technology Stack

| Layer | Choice | Version | License | Rationale |
|-------|--------|---------|---------|-----------|
| UI framework | `leptos` (CSR) | 0.8 | MIT | TEA-compatible, zero JS toolchain, shared Rust types. See ADR-005. |
| WASM runtime bridge | `wasm-bindgen` | 0.2 | MIT / Apache 2.0 | Required by Leptos for Rust→WASM compilation. Standard crate. |
| Browser API bindings | `web-sys` | 0.3 | MIT / Apache 2.0 | DOM access, MouseEvent (chart crosshair), clipboard API. Features: `Window`, `Document`, `Element`, `MouseEvent`, `KeyboardEvent`, `Navigator`, `Clipboard`, `ClipboardItem`. |
| JS interop utilities | `js-sys` | 0.3 | MIT / Apache 2.0 | `Math.random()` for UUID in WASM target. |
| Serialization | `serde` + `serde_json` | 1.x | MIT / Apache 2.0 | Domain type `Serialize`/`Deserialize` for V2 server function payloads. |
| UUIDs | `uuid` | 1.x | Apache 2.0 | `v4` + `js` feature for WASM-compatible random UUID generation. |
| Date/time | `chrono` | 0.4 | MIT / Apache 2.0 | `wasmbind` feature for WASM-compatible date formatting. |
| WASM bundler | `trunk` | 0.21.x | MIT / Apache 2.0 | Compiles Leptos WASM, processes `index.html`, copies assets. OSS, no npm. |
| Admin file server | `tower-http` | 0.5 | MIT | `ServeDir` serves `admin-ui/dist/` at `/admin/` in `embyr-admin`. Already in Tokio ecosystem. |
| Dependency enforcement | `cargo-deny` | 0.14.x | MIT / Apache 2.0 | `deny.toml` for `embyr-admin-ui` blocks server-side IO crates. |
| Mutation testing | `cargo-mutants` | current | MIT | Tests pure `update()` function without browser infrastructure. |

**What is deliberately excluded:**

- No `leptos_router` (V1: in-memory nav state; V2 addition)
- No `leptos_sse` / `leptos-server-signal` (V2: real-time data)
- No JavaScript chart library (pure SVG in Rust)
- No CSS framework (verbatim port of design prototype CSS)
- No npm/webpack/Vite/TypeScript

---

## Wave: DESIGN / [REF] Reuse Analysis

#### JSX Prototype → Leptos (cross-language translation)

| File | Classification | Notes |
|------|---------------|-------|
| `store.jsx` | PORT | `AppProvider` useState + mutations → `model.rs` + `msg.rs` + `update.rs`. 7 state slices → 7 AppModel fields. 12 mutations → 12 Msg variants with exhaustive match. |
| `app.jsx` | PORT | Sidebar, Topbar, Root → `sidebar.rs`, `topbar.rs`, `app.rs`. TweaksPanel excluded (see DROP below). |
| `views_auth.jsx` | PORT | AuthGate → `views/auth.rs`. TOTP CodeInput → typed Leptos component with focus management. |
| `views_dashboard.jsx` | PORT | DashView → `views/dashboard.rs`. |
| `views_databases.jsx` | PORT | DatabasesView + CreateDatabaseModal → `views/databases.rs`. |
| `views_db_detail.jsx` | PORT | DB Detail wrapper + Overview + Connections → `views/db_detail/`. |
| `views_db_logs.jsx` | PORT | Log table + filters → `views/db_detail/logs.rs`. |
| `views_billing.jsx` | PORT | BillingView → `views/billing.rs`. |
| `views_identities.jsx` | PORT | IdentitiesView → `views/identities.rs`. |
| `views_apikeys.jsx` | PORT | ApiKeysView → `views/api_keys.rs`. |
| `views_settings.jsx` | PORT | SettingsView → `views/settings.rs`. |
| `ui.jsx` | PORT | Button, Badge, Card, Modal, Input, Toggle, Tabs, Menu → `components/primitives/`. CSS class names preserved. |
| `charts.jsx` | PORT | Sparkline, LatencyChart, BarChart, Donut → `components/charts/`. SVG path math translated to Rust pure functions. |
| `icons.jsx` | PORT | SVG icon paths → `pub enum Icon` + `impl IntoView` in `components/icons.rs`. |
| `styles.css` | REUSE_ASSET | Copy verbatim to `public/styles.css`. No modifications. All CSS class names, custom properties, and `data-look` attribute selectors preserved exactly. |
| `tweaks-panel.jsx` | DROP | Design-time tool. Not included in production Leptos build. Default theme `ember`/`comfortable` hardcoded in `index.html`. |
| `data.js` | DERIVE | Mock object shapes → Rust structs in `src/data.rs`. Type-system guarantees added: `Vec<Database>` (not array of `any`), `chrono::NaiveDate` (not string), `Option<String>` (not `null`). |

#### Existing Rust Code

| Component | File | Classification | Action |
|-----------|------|---------------|--------|
| `embyr-admin` binary | `crates/embyr-admin/src/main.rs` | EXTEND | Add `ServeDir` route at `/admin/`. Stub becomes a real Axum server. |
| `embyr-admin` Cargo.toml | `crates/embyr-admin/Cargo.toml` | EXTEND | Add `tower-http` with `fs` feature. |

---

## Wave: DESIGN / [REF] Driving Ports

The admin UI SPA is a browser-side application. Its inbound event sources (driving ports) are:

| Driving Port | Leptos / Web Mechanism | Notes |
|-------------|----------------------|-------|
| Initial page load | HTTP GET `/admin/` → `ServeDir` returns `index.html` | `trunk` entry point; loads WASM and CSS |
| DOM user interaction | Leptos event handlers (`on:click`, `on:input`, `on:submit`) | Every button, input, form |
| ESC key (modal close) | `window_event_listener("keydown", ...)` | Applied by Modal primitive globally |
| Mouse movement (chart crosshair) | `web-sys::MouseEvent` via `mousemove` listener | Applied to LatencyChart SVG element |
| Clipboard write (key copy) | `web_sys::Navigator::clipboard().write_text(...)` | SDK key and admin key copy-once flow |
| Resource completion | `Effect::new(move |_| dispatch(Msg::SetX(data)))` | Async data load result dispatched to TEA |

---

## Wave: DESIGN / [REF] Driven Ports and Adapters

| Port (what component needs) | V1 Adapter | V2 Adapter | Migration |
|-----------------------------|-----------|-----------|---------|
| Database list | `mock::databases()` in `Resource` async block | `fetch_databases().await` (`#[server]`) | Replace one line inside async block |
| Create database | `mock::make_database(&input)` in `Action` async block | `create_database_server_fn(input).await` | Replace one line inside async block |
| Member list | `mock::members()` | `fetch_members().await` | Same |
| Invite member | In-memory `Msg::MemberInvited(m)` (mock construct) | `invite_member_server_fn(email, role).await` | Same |
| SDK key list | `mock::sdk_keys(db_id)` | `fetch_sdk_keys(db_id).await` | Same |
| Admin key list | `mock::admin_keys()` | `fetch_admin_keys().await` | Same |
| Billing usage | `mock::billing_usage(range)` | `fetch_billing(range).await` | Same |
| Query logs | `mock::query_logs(db_id)` | `fetch_query_logs(db_id, filters).await` | Same |
| OIDC providers | `mock::oidc_providers()` | `fetch_oidc_providers().await` | Same |

**V2 migration invariant (ADR-007):** The `dispatch(Msg::SetX(data))` call inside every `Resource`/`Action` async block is identical in V1 and V2. No component file changes between V1 and V2. Verified by git diff check in Slice 07 acceptance criteria.

**Pure UI state (never needs a driven port):** Navigation (`NavState`), modal visibility (`bool` in parent component signal), form field values (local `RwSignal`), toast queue (`AppModel.toasts`) — all go directly to `dispatch(Msg::...)` without any async IO.

---

## Wave: DESIGN / [REF] Open Questions

| ID | Question | Blocking | Resolution |
|----|----------|---------|-----------|
| OQ-UI-01 | **WASM bundle size**: estimated 810 KB–1.4 MB compressed; unvalidated until `trunk build --release` on the full component tree. May exceed 5 MB if `web-sys` features are over-requested. | Yes — Slice 01 gate | CI job in Slice 01 WS; fail if `dist/` > 5 MB |
| OQ-UI-02 | **Email delivery in V1**: member invitations show a pending table entry but no email is sent. Is this sufficient for V1 or does P5 expect a "resend invite" button? | No — confirmed deferred | DISCUSS Out of Scope; V2 SMTP adapter in embyr-admin |
| OQ-UI-03 | **Active connections placeholder**: "—" with V2 badge. Single-pod count is technically feasible but deferred. Does P5 need any connection count in V1? | No — confirmed deferred | DISCUSS locked decision D4 |
| OQ-UI-04 | **SSR migration timing**: when does faster initial paint justify `leptos_axum` build coupling? | No — V3 consideration | Design spec § Migration documents the path |
| OQ-UI-05 | **Browser back button** (`leptos_router`): deferred to V2. Any P5 feedback on in-memory navigation after V1 deploy? | No — confirmed deferred | Collect feedback post-Slice 01 deploy |

---

## Wave: DESIGN / [REF] Locked Decisions (ADR refs)

| # | Decision | ADR | Summary |
|---|----------|-----|---------|
| D-UI-01 | Frontend paradigm: Leptos 0.8 CSR WASM | ADR-005 | Single Rust toolchain, shared types, TEA built-in, pure SVG charts, zero npm |
| D-UI-02 | TEA state management: `RwSignal<AppModel>` + `Callback<Msg>` via context | ADR-006 | Pure testable `update()`, no prop drilling, fine-grained reactivity |
| D-UI-03 | Mock-first V1: `data.rs` mock, `#[server]` in V2 | ADR-007 | Decouples UI from backend; zero-component-change V2 migration |
| D-UI-04 | Crate structure: separate `embyr-admin-ui` workspace member | ADR-008 | Build isolation (trunk vs cargo), dependency isolation, future shared types path |
| D-UI-05 | Pure SVG charts (no JS chart library) | UI-AD-05 in brief.md | Zero bundle impact, pure Rust math functions |
| D-UI-06 | Verbatim CSS (no CSS-in-Rust framework) | UI-AD-06 in brief.md | Design-reviewed stylesheet, no build complexity |
| D-UI-07 | Drop TweaksPanel from production build | Reuse Analysis | Design-time tool; default theme hardcoded in index.html |
| D-UI-08 | No `leptos_router` in V1 | UI-AD-09 in brief.md | In-memory nav state sufficient; V2 addition with zero model changes |

---

## Wave: DISTILL

### [REF] Inherited commitments

| Origin | Commitment | DDD | Impact |
|--------|------------|-----|--------|
| DISCUSS#D1 | Leptos 0.8 CSR WASM is the frontend stack | n/a | Tests must not require WASM runtime; host-target `cargo test` only |
| DISCUSS#D2 | V1 data is mock-only (`data.rs`) | n/a | No driven-external ports in scope; no Testcontainers needed for UI tests |
| DISCUSS#D5 | Walking Skeleton = thin E2E slice (auth + dashboard) | n/a | WS tests embyr-admin HTTP + bundle size; not browser automation |
| DESIGN#D-UI-02 | `fn update(&mut AppModel, Msg)` is pure, cargo-testable | ADR-006 | Primary acceptance test surface is direct `update()` call; no process spawn |
| DESIGN#D-UI-03 | Zero-component-change V2 migration | ADR-007 | Test vocabulary is stable across V1 and V2; no test rewrites on V2 |
| DESIGN#D-UI-04 | Separate workspace crate `embyr-admin-ui` | ADR-008 | `[[test]]` entries registered in `crates/embyr-admin-ui/Cargo.toml`; test path `tests/user_admin_ui/` |

---

## Wave: DISTILL / [REF] Scenario List

| Scenario | File | Tags | US | Classification |
|----------|------|------|----|----------------|
| admin_spa_http_probe | walking_skeleton.rs | @walking_skeleton @driving_adapter @real-io @US-001 | US-001 | GREEN_PENDING_SLICE_01 |
| wasm_bundle_size_gate | walking_skeleton.rs | @bundle_size @ci_gate | US-001 | GREEN_PENDING_TRUNK_BUILD |
| sign_in_sets_authed | tea_state_scenarios.rs | @US-001 @in-memory @property | US-001 | RED |
| sign_out_clears_authed | tea_state_scenarios.rs | @US-001 @in-memory @property | US-001 | RED |
| three_totp_failures_lock_account | tea_state_scenarios.rs | @US-001 @in-memory @error | US-001 | RED |
| totp_success_resets_failure_counter | tea_state_scenarios.rs | @US-001 @in-memory @error | US-001 | RED |
| set_databases_populates_model | tea_state_scenarios.rs | @US-002 @in-memory @property | US-002 | RED |
| set_databases_excludes_deleted | tea_state_scenarios.rs | @US-002 @in-memory @error | US-002 | RED |
| database_created_appends_db | tea_state_scenarios.rs | @US-003 @in-memory @property | US-003 | RED |
| delete_database_removes_db_and_sdk_keys | tea_state_scenarios.rs | @US-003 @in-memory @property | US-003 | RED |
| set_db_status_updates_database | tea_state_scenarios.rs | @US-003 @in-memory @property | US-003 | RED |
| set_db_status_missing_id_is_noop | tea_state_scenarios.rs | @US-003 @in-memory @error | US-003 | RED |
| set_db_logging_enables_logging | tea_state_scenarios.rs | @US-004 @in-memory @property | US-004 | RED |
| set_db_logging_disables_logging | tea_state_scenarios.rs | @US-004 @in-memory @property | US-004 | RED |
| set_db_logging_wrong_id_is_noop | tea_state_scenarios.rs | @US-004 @in-memory @error | US-004 | RED |
| patch_db_updates_backend_config | tea_state_scenarios.rs | @US-005 @in-memory @property | US-005 | RED |
| patch_db_missing_id_is_noop | tea_state_scenarios.rs | @US-005 @in-memory @error | US-005 | RED |
| sdk_key_created_appends_key | tea_state_scenarios.rs | @US-006 @in-memory @property | US-006 | RED |
| revoke_sdk_key_removes_key | tea_state_scenarios.rs | @US-006 @in-memory @property | US-006 | RED |
| revoke_sdk_key_missing_key_is_noop | tea_state_scenarios.rs | @US-006 @in-memory @error | US-006 | RED |
| sdk_key_created_unknown_db_is_noop | tea_state_scenarios.rs | @US-006 @in-memory @error | US-006 | RED |
| member_invited_appends_member | tea_state_scenarios.rs | @US-009 @in-memory @property | US-009 | RED |
| set_member_role_updates_role | tea_state_scenarios.rs | @US-009 @in-memory @property | US-009 | RED |
| remove_member_removes_non_owner | tea_state_scenarios.rs | @US-009 @in-memory @property | US-009 | RED |
| sole_owner_invariant_holds_after_member_removal | tea_state_scenarios.rs | @US-009 @in-memory @property @invariant | US-009 | RED |
| sole_owner_invariant_holds_after_role_change | tea_state_scenarios.rs | @US-009 @in-memory @property @invariant | US-009 | RED |
| remove_member_missing_uid_is_noop | tea_state_scenarios.rs | @US-009 @in-memory @error | US-009 | RED |
| service_account_created_appends | tea_state_scenarios.rs | @US-010 @in-memory @property | US-010 | RED |
| delete_service_account_removes_it | tea_state_scenarios.rs | @US-010 @in-memory @property | US-010 | RED |
| admin_key_created_appends | tea_state_scenarios.rs | @US-010 @in-memory @property | US-010 | RED |
| revoke_admin_key_removes_key | tea_state_scenarios.rs | @US-010 @in-memory @property | US-010 | RED |
| revoke_admin_key_missing_is_noop | tea_state_scenarios.rs | @US-010 @in-memory @error | US-010 | RED |
| toggle_oidc_flips_enabled | tea_state_scenarios.rs | @US-011 @in-memory @property | US-011 | RED |
| toggle_oidc_double_toggle_restores | tea_state_scenarios.rs | @US-011 @in-memory @property | US-011 | RED |
| push_toast_appends | tea_state_scenarios.rs | @US-011 @in-memory @property | US-011 | RED |
| dismiss_toast_removes_toast | tea_state_scenarios.rs | @US-011 @in-memory @property | US-011 | RED |
| dismiss_toast_missing_id_is_noop | tea_state_scenarios.rs | @US-011 @in-memory @error | US-011 | RED |
| push_many_toasts_does_not_panic | tea_state_scenarios.rs | @US-011 @in-memory @error @boundary | US-011 | RED |
| (+ 30 per-slice scenarios across slice_01–06) | slice_0N_*.rs | @in-memory @error / @happy | US-001..011 | RED |

**Error/edge scenario count**: 17 of ~40 total = **~43% error/edge** (meets ≥40% mandate).

---

## Wave: DISTILL / [REF] WS Strategy

**Strategy: HTTP probe + bundle size gate** (per ATDD Infrastructure Policy — `Browser WASM SPA` row added 2026-06-14).

| Test | Mechanism | Port class |
|------|-----------|------------|
| `admin_spa_http_probe` | `reqwest::Client` → in-process Axum test server (ephemeral port) | Driving: HTTP GET `/admin/` |
| `wasm_bundle_size_gate` | `std::fs::metadata` on `admin-ui/dist/*.wasm` | CI artifact check |
| All TEA state scenarios | Direct `update(&mut AppModel, Msg)` call | Driving: in-process (pure fn) |

**Why no browser automation**: The TEA architecture makes `update()` the single seam. Browser automation tests nothing the pure-function tests don't, at 100× the infrastructure cost. V2 may add Playwright if visual regression testing is needed.

**WS litmus test**: a non-technical stakeholder can confirm "yes, when I navigate to `/admin/` my browser shows the SPA" (admin_spa_http_probe) and "the app loads in <3s on 10Mbps" (bundle_size_gate). Both are user-observable outcomes.

---

## Wave: DISTILL / [REF] Adapter Coverage Table

| Adapter | @real-io scenario | Covered by |
|---------|-------------------|------------|
| `embyr-admin` ServeDir (`/admin/` route) | YES | `admin_spa_http_probe` (walking skeleton) |
| WASM bundle (`trunk build` artifact) | YES | `wasm_bundle_size_gate` (CI gate) |
| `data.rs` mock functions | NO (pure fn, no real I/O) | All TEA state scenarios via `update()` — no adapter I/O in V1 |

**Note**: V1 has no driven-external ports (all data is in-process mock). The `data.rs` functions are not adapters — they are pure Rust functions returning in-memory structs. No Testcontainers needed. The ATDD policy's `Driven external / non-deterministic (fake)` section has zero entries for this feature.

---

## Wave: DISTILL / [REF] Scaffolds

| File | Type | Scaffold marker | Classification |
|------|------|-----------------|----------------|
| `crates/embyr-admin-ui/src/lib.rs` | Crate root | `pub const __SCAFFOLD__: bool = true;` | RED scaffold |
| `crates/embyr-admin-ui/src/model.rs` | Domain types | `// SCAFFOLD: true` | RED scaffold |
| `crates/embyr-admin-ui/src/msg.rs` | Message enum | `// SCAFFOLD: true` | RED scaffold |
| `crates/embyr-admin-ui/src/update.rs` | Pure update fn | `// SCAFFOLD: true` | RED — every arm `panic!()` |
| `crates/embyr-admin-ui/src/data.rs` | Mock data | `// SCAFFOLD: true` | RED — every fn `panic!()` |
| `crates/embyr-admin-ui/src/app.rs` | App root (CSR) | `// SCAFFOLD: true` | RED scaffold |
| `tests/user_admin_ui/user_admin_ui.rs` | Test entry point | `// SCAFFOLD: true` | Compile-time only |
| `tests/user_admin_ui/common/mod.rs` | Test helpers | `// SCAFFOLD: true` | Support only |
| `tests/user_admin_ui/common/arb.rs` | proptest strategies | `// SCAFFOLD: true` | Support only |

**Scaffold detection**: `grep -r "SCAFFOLD: true" crates/embyr-admin-ui/src/`

---

## Wave: DISTILL / [REF] Test Placement

```
tests/user_admin_ui/
  user_admin_ui.rs                        # Entry point for cargo test binary
  common/
    mod.rs                                # make_model_with_db(), assert_owner_invariant()
    arb.rs                                # proptest strategies for all domain types
  acceptance/
    walking_skeleton.rs                   # HTTP probe + bundle size gate (NOT #[ignore])
    tea_state_scenarios.rs                # All 11 US covered via proptest (#[ignore])
    slice_01_auth_dashboard.rs            # US-001 + US-002 focused (#[ignore])
    slice_02_database_management.rs       # US-003 + US-004 focused (#[ignore])
    slice_03_connections_sdk_keys.rs      # US-005 + US-006 focused (#[ignore])
    slice_04_identity_admin_keys.rs       # US-009 + US-010 focused (#[ignore])
    slice_05_billing_logs.rs              # US-007 + US-008 focused (#[ignore])
    slice_06_settings.rs                  # US-011 focused (#[ignore])
```

**Precedent**: mirrors `tests/acceptance/embyr_agent/` pattern — subdirectory modules registered via `#[path = "..."]` in a top-level entry point `.rs` file, which is declared as `[[test]]` in `Cargo.toml`.

**Run command**: `cargo test --package embyr-admin-ui --test user_admin_ui_tea_state -- --include-ignored`

---

## Wave: DISTILL / [REF] Pre-requisites

| Prerequisite | Source | Required for |
|-------------|--------|-------------|
| `embyr-admin-ui` crate compiles (host target) | `cargo check --package embyr-admin-ui` | All tests |
| `embyr-admin` serves `/admin/` via ServeDir | Slice 01 DELIVER | `admin_spa_http_probe` WS |
| `trunk build --release` runs successfully | Slice 01 CI | `wasm_bundle_size_gate` |
| `Cargo.toml` `[workspace]` lists `crates/*` (already present) | Workspace | Crate discovery |
| `proptest` 1.x in dev-dependencies | `crates/embyr-admin-ui/Cargo.toml` | All TEA proptest tests |
| `reqwest` + `axum` in dev-dependencies | `crates/embyr-admin-ui/Cargo.toml` | Walking skeleton test |
