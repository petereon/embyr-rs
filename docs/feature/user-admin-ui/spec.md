# User-Admin Interface — Specification

> **Scope**: Web-based administrative interface for user-admins (customers who own or manage embyr-rs accounts).
> Excludes: embyr-admin (operator) interface; SDK client libraries; CLI tooling.
> **Version**: 2026-06-05
> **Audience**: Implementer building the admin UI and its supporting API endpoints.

---

## Overview

User-admins manage their account's databases, observe system health, control access, and review usage costs. The interface is a browser-based web app served by the existing `embyr-server` admin port (:9090) via axum. It exposes no new backend concepts — all data already exists in the Postgres schema or is derivable from runtime state.

The top-level organizational model is: **Account → Databases (projects)**. All resources belong to an account; databases are the primary operational unit within an account.

---

## Concepts and Terminology

| Term | Definition |
|------|------------|
| Account | Top-level billing and identity container. One or more humans or service accounts share it. Maps to a tenant root (no explicit `accounts` table yet — see Open Questions). |
| Database | User-facing label for a `projects` row. The underlying concept is identical; the UI calls it "database" to match Firestore mental models. |
| Member | A human identity attached to an account with an assigned role. |
| Service Account | A non-human identity attached to an account. Can hold API keys; cannot log in interactively. |
| SDK API Key | A bearer token authorizing Firestore SDK clients to access a specific database. Per-database, unscoped internally (database boundary is the security perimeter). |
| Admin API Key | A bearer token authorizing programmatic management operations (create/list/delete databases, manage members). Per-account, carries a role. |
| Role | One of Owner, Admin, Viewer. Applies to both human members and admin API keys. |

---

## Information Architecture

### Top-Level Navigation

```
[embyr logo / account switcher]
├── Dashboard          ← multi-database KPI overview (landing page)
├── Databases          ← project list → drill into per-database sections
├── Billing            ← account-level usage metrics
├── Identities         ← account members + service accounts
├── API Keys           ← admin API keys (account-level)
└── [avatar menu]
    ├── Account Settings
    └── Sign Out
```

### Per-Database Navigation (context: one selected database)

```
[← Back to Databases]  "my-database"
├── Overview           ← KPIs: P95 latency tile, op counts, active connections
├── Connections        ← backend config + live client connection count
├── Query Logs         ← filterable log table (when logging enabled)
└── API Keys           ← SDK API keys for this database
```

---

## Authentication

### Methods

Two independent login methods. Users choose at account setup; both can be enabled simultaneously on one account.

#### Method 1: OIDC

- Account owner configures one or more trusted OIDC providers (e.g. Google, GitHub, Okta) via Account Settings.
- Login flow: redirect to OIDC provider → callback with `id_token` → validate issuer + audience → establish session.
- No password stored in embyr-rs.

#### Method 2: Email + Password + MFA

- Password stored as Argon2id hash (same parameters as credential encryption: memory=65536 KiB, iter=3, par=4).
- MFA is mandatory for this login method. Users choose their second factor during setup.
- **MFA options** (user's choice, one required):
  - **TOTP**: time-based one-time password via authenticator app (RFC 6238). User scans QR code at setup; stores secret in app. Login: password → TOTP code.
  - **Email OTP**: 6-digit code sent to the account's email address. 10-minute expiry. Login: password → request code → enter code.
- Recovery codes: 8 single-use recovery codes generated at MFA setup. User downloads/saves them. Each code is single-use and invalidated on consumption.

### Session

- Session token issued as an HTTP-only, Secure, SameSite=Strict cookie.
- Session lifetime: 24 hours with sliding expiry on activity.
- Implemented as axum middleware. All routes under `/admin/ui/` require a valid session cookie.
- Session store: Postgres (new table, see Data Model).

### Roles and Permissions

| Action | Owner | Admin | Viewer |
|--------|-------|-------|--------|
| View all sections | ✓ | ✓ | ✓ |
| Create / delete databases | ✓ | ✓ | — |
| Edit backend connection config | ✓ | ✓ | — |
| Manage SDK API keys | ✓ | ✓ | — |
| Manage account members | ✓ | ✓ | — |
| Manage service accounts | ✓ | ✓ | — |
| Manage admin API keys | ✓ | ✓ | — |
| Configure OIDC providers | ✓ | — | — |
| Delete account | ✓ | — | — |
| Transfer ownership | ✓ | — | — |

---

## Dashboard

Landing page after login. Shows all databases in the account as a card grid or table.

**Per-database KPI tile:**
- Database name
- Status badge: active / suspended / deleted
- P95 read latency (last hour) — single number in ms
- Op counts today: reads, writes, deletes
- Active client connections: "--" (V2; deferred — multi-pod counter not yet implemented)

Clicking a tile navigates to that database's Overview sub-section.

---

## Databases Section

### List View

Table of all databases in the account. Columns: Name, Status, Backend Mode, Created, Actions (open, suspend/activate, delete).

### Create Database

Form fields:
- Name (unique within account, immutable after creation)
- Backend mode: `direct_pg` | `agent_mode` (aws_secret and gcp_secret are backend config variants of agent_mode)
- Backend-specific fields (see Connections section)

### Database Detail — Overview

KPI tiles identical to Dashboard card, plus:
- Logging enabled/disabled toggle (see Query Logs)
- Quick links to sub-sections

---

## Connections

Per-database section. Two panels:

### Panel 1: Backend Configuration

Displays and allows editing of how this database connects to its Postgres backend.

| Field | `direct_pg` | `agent_mode` |
|-------|------------|-------------|
| Mode | `direct_pg` | `agent_mode` |
| Connection URL | Postgres DSN (masked — show only host:port/dbname, mask credentials) | — |
| Agent Endpoint | — | mTLS gRPC endpoint (host:port) |
| Secret Backend | — | `env` / `aws_secretsmanager` / `gcp_secretmanager` |
| Secret Name / ARN | — | When secret backend is AWS or GCP |

Editing connection config requires Owner or Admin role. Changes take effect on next connection attempt (no live migration).

### Panel 2: Active Client Connections

Live count of currently open Subscribe (gRPC streaming) and Listen connections for this database. Displayed as:
- Total active streams: "--" (deferred to V2 — requires shared counter across pods)

V1 shows "--" for active stream count. V2 will introduce a shared store (Redis or Postgres heartbeat table) to aggregate counts across `embyr-server` pods.

---

## Query Logs

Per-database section.

### Enable / Disable Logging

Logging is **off by default**. Toggle per database. Enabling logs all subsequent operations; disabling stops new log entries (existing entries remain until purged or retention expires).

**Retention options** (set when enabling): 1 day / 7 days / 30 days.

Storage consumed by query log entries is billed to the account as a storage line item (same rate as document storage).

### Log Table

Columns:
| Column | Description |
|--------|-------------|
| Timestamp | UTC timestamp of operation |
| Operation | `read` / `write` / `delete` / `query` / `listen` / `subscribe` |
| Collection Path | Target collection or document path |
| Duration (ms) | Execution time from request receipt to response sent |
| Status | `ok` / error code |
| Client | Truncated API key identifier (last 6 chars) |

**Filtering**: by operation type, collection path prefix, time range, status.

**Sorting**: by timestamp (default desc), by duration.

No pagination limit shown; table virtualizes rows. Max export: 10,000 rows as CSV.

---

## Billing

Account-level section. Shows usage metrics only — no invoicing, no payment method management.

### Usage Summary

Time range selector: Last 7 days / Last 30 days / This month / Last month.

**Per-database breakdown table:**

| Database | Read Ops | Write Ops | Delete Ops | Peak Connections | Log Storage |
|----------|----------|-----------|------------|-----------------|-------------|
| my-db    | 1.2M     | 340K      | 12K        | 48              | 2.1 GB      |
| other-db | ...      | ...       | ...        | ...             | ...         |
| **Total**| ...      | ...       | ...        | —               | ...         |

**Source**: `daily_project_metrics` table (existing schema: `read_ops`, `write_ops`, `delete_ops`, `listen_connections` per day per project). Log storage derived from query log entry count × average row size (or measured from a dedicated `query_log_storage_bytes` column — see Open Questions).

**Note**: Pricing rates are not displayed. This section shows raw usage quantities. Rate cards and invoice generation are handled externally.

---

## Identities

Account-level section. Two sub-tabs: **Members** and **Service Accounts**.

### Members

Table of human identities with account access.

Columns: Email, Name, Role, Auth Method (OIDC / Email+Password), MFA Enabled, Last Login, Actions (edit role, remove).

**Invite flow**: Owner or Admin enters an email address and selects a role. An invitation email is sent. Recipient clicks the link, sets up their credentials (OIDC or email+password+MFA), and joins the account. Invitations expire after 7 days.

**Role change**: Owner or Admin can change any member's role except their own (prevents self-demotion from Owner if sole Owner).

**Remove**: Immediately revokes session and access. Any admin API keys held by the removed member are invalidated.

### Service Accounts

Table of non-human identities.

Columns: Name, Description, Role, Created, Last Used (from API key last-used timestamp), Actions (edit, delete).

**Create**: Name + optional description + role. No credentials at creation — credentials are admin API keys created separately in the API Keys section and associated with this service account.

**Delete**: Invalidates all admin API keys associated with this service account.

---

## API Keys — Account Level (Admin Keys)

Account-level section. Lists admin API keys used for programmatic management.

Table columns: Name, Associated Identity (member name or service account name), Role, Created, Last Used, Prefix (first 8 chars), Actions (revoke).

**Create**: 
- Select associated identity (member or service account)
- Role inherited from the associated identity's account role
- Name (human label)
- On creation: full key shown once, never again. User copies and stores it.

**Revoke**: Immediate. No undo.

**Key format**: `embyr_adm_<random-32-chars>` (distinguishable from SDK keys by prefix).

---

## API Keys — Per-Database (SDK Keys)

Per-database sub-section.

Table columns: Name, Created, Last Used, Prefix (first 8 chars), Actions (revoke).

**Create**:
- Name (human label)
- Scoped to this database only (no collection-level restrictions)
- On creation: full key shown once, never again.

**Revoke**: Immediate. In-flight requests using the key are rejected on next token validation.

**Key format**: `embyr_sdk_<random-32-chars>` (distinguishable from admin keys by prefix).

---

## Data Model

### New tables required

**`accounts`**
- `id` — UUID, primary key
- `name` — text, display name
- `created_at` — timestamptz
- `deleted_at` — timestamptz nullable

**`account_members`**
- `account_id` — FK → accounts.id
- `user_id` — FK → users.id
- `role` — enum: `owner` / `admin` / `viewer`
- `invited_by` — FK → users.id nullable
- `joined_at` — timestamptz nullable (null = pending invite)

**`users`**
- `id` — UUID, primary key
- `email` — text, unique
- `display_name` — text nullable
- `password_hash` — text nullable (Argon2id; null if OIDC-only)
- `totp_secret_enc` — bytea nullable (encrypted at rest)
- `email_otp_enabled` — boolean
- `mfa_recovery_codes` — text[] nullable (hashed)
- `created_at` — timestamptz

**`oidc_providers`** (per-account)
- `id` — UUID
- `account_id` — FK → accounts.id
- `issuer` — text (e.g. `https://accounts.google.com`)
- `client_id` — text
- `client_secret_enc` — bytea (encrypted at rest)
- `enabled` — boolean

**`service_accounts`**
- `id` — UUID
- `account_id` — FK → accounts.id
- `name` — text
- `description` — text nullable
- `role` — enum: `owner` / `admin` / `viewer`
- `created_at` — timestamptz
- `deleted_at` — timestamptz nullable

**`admin_api_keys`**
- `id` — UUID
- `account_id` — FK → accounts.id
- `name` — text
- `key_hash` — text (BLAKE3 of full key)
- `key_prefix` — text (first 8 chars, for display)
- `role` — enum: `owner` / `admin` / `viewer`
- `member_id` — FK → users.id nullable
- `service_account_id` — FK → service_accounts.id nullable
- `created_at` — timestamptz
- `last_used_at` — timestamptz nullable
- `revoked_at` — timestamptz nullable

**`sdk_api_keys`**
- `id` — UUID
- `project_id` — FK → projects.id
- `name` — text
- `key_hash` — text (BLAKE3 of full key)
- `key_prefix` — text (first 8 chars, for display)
- `created_at` — timestamptz
- `last_used_at` — timestamptz nullable
- `revoked_at` — timestamptz nullable

**`sessions`**
- `id` — UUID (session token = opaque random, not the UUID itself)
- `token_hash` — text (BLAKE3 of session cookie value)
- `user_id` — FK → users.id
- `account_id` — FK → accounts.id
- `created_at` — timestamptz
- `expires_at` — timestamptz
- `last_active_at` — timestamptz

**`invitations`**
- `id` — UUID
- `account_id` — FK → accounts.id
- `email` — text
- `role` — enum
- `invited_by` — FK → users.id
- `token_hash` — text (one-time token)
- `created_at` — timestamptz
- `expires_at` — timestamptz
- `accepted_at` — timestamptz nullable

**`query_logs`** (partitioned by day, per project)
- `id` — UUID
- `project_id` — FK → projects.id
- `timestamp` — timestamptz
- `operation` — enum: `read` / `write` / `delete` / `query` / `listen` / `subscribe`
- `collection_path` — text
- `duration_ms` — integer
- `status` — text (`ok` or error code)
- `sdk_key_prefix` — text (last 6 chars of SDK key used)

### Existing tables used (read-only from UI)

- `projects` — database list, status, backend config
- `daily_project_metrics` — billing usage data

---

## Invariants

- An account must always have at least one Owner member. The last Owner cannot be removed or demoted.
- An SDK API key is always scoped to exactly one database. Revoking a database cascades to revoke all its SDK keys.
- A session is always bound to exactly one user and one account. Switching accounts requires a new session or re-authentication.
- MFA is mandatory for the email+password login method. A user cannot complete email+password login without a configured second factor.
- Recovery codes are single-use. A consumed code is invalidated immediately regardless of whether the login succeeded.
- Query logs are only written when logging is explicitly enabled for the database. Logging state is stored on the `projects` row.

---

## Error Model

- Authentication failures return HTTP 401 with a generic message (no information about whether the email exists).
- Authorization failures (insufficient role) return HTTP 403.
- Validation errors return HTTP 422 with a structured body: `{ "field": "name", "message": "..." }`.
- Server errors return HTTP 500 with a request ID for log correlation.
- All interactive errors are displayed inline in the form that triggered them (no page-level error screens for validation failures).

---

## Resolved Decisions

| # | Question | Decision |
|---|----------|----------|
| OQ-1 | Account-project foreign key | `projects.account_id UUID NOT NULL REFERENCES accounts(id)` baked into initial schema (`0001_initial_schema.sql`). `accounts` table created before `projects`. No migration strategy needed — not deployed. |
| OQ-2 | Query log storage billing | Add `log_storage_bytes BIGINT DEFAULT 0` to `daily_project_metrics` (modify `0002_metrics.sql` directly). Updated by a daily background job on retention expiry/flush. |
| OQ-3 | Active connection count | **Deferred to V2.** Multi-pod deployment makes in-process counters unworkable. UI shows "--" for active connections. V2 will use a shared store (Redis or Postgres heartbeat). |
| OQ-4 | Listen-hours in billing | **Deferred to V2.** Same multi-pod constraint. `listen_connections` column kept as daily snapshot count; UI label changed to "Peak Connections" not "Hours". |
| OQ-5 | OIDC provider config ownership | Per-account. One set of trusted OIDC providers shared across all databases in the account. |
| OQ-6 | Key encryption at rest | `EMBYR_ENCRYPTION_KEY` env var (32-byte random, set at deploy time). All `totp_secret_enc` and `client_secret_enc` encrypted with AES-256-GCM using this key at the application layer. |
| OQ-7 | Email delivery | `IEmailSender` driven port. Production adapter: `SmtpEmailSender` (configured via `EMBYR_SMTP_HOST/PORT/USER/PASS`, works with any SMTP relay). Test adapter: `NoopEmailSender` (captures sent messages for assertions). |
| OQ-8 | Frontend tech stack | **Leptos** — Rust/WASM frontend with `leptos_axum` integration on :9090. New `embyr-admin-ui` crate in the workspace. Shared types between server and UI crates. No TypeScript/JavaScript context switch. |
