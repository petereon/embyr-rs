# Journey: Service Operator — Provision and Manage Tenants

> Persona: P2 — Sam (Service Operator)
> Jobs: JOB-02 (Tenant-Provision), JOB-06 (Tenant-Control)
> Research depth: Comprehensive

---

## Emotional Arc

```
Organised  Efficient  Reassured  Watchful   In-Control  Confident
    │          │          │          │           │          │
    ▼          ▼          ▼          ▼           ▼          ▼
[Provision] [Auth]    [Verify]  [Monitor]  [Suspend]  [Clean-up]
```

---

## Journey Steps

### Step 1 — Provision Project
**Action**: `POST /admin/v1/projects` with `{project_id, auth_mode, auth_key, backend_mode, backend_pg_dsn}` using `Authorization: Bearer <admin_key>`.
**Expected output**: `201 Created` with project record (no raw key or DSN in response). Customer DB migrated.
**Emotion**: Organised → "I can script this"
**Shared artifacts produced**: `${project_id}`, `${auth_key}` (one-time), `${admin_endpoint}`
**Error path**: `Unavailable` — customer DB unreachable → Sam checks DSN, retries.

### Step 2 — Configure Auth and Hand Off Credentials
**Action**: Record the project `project_id` and `auth_key` and deliver them to the customer (via their own credential delivery mechanism). Optionally run `PATCH /admin/v1/projects/{id}` to update auth config.
**Expected output**: Customer can connect their SDK.
**Emotion**: Efficient → "This is a 3-minute onboarding"
**Shared artifacts consumed**: `${project_id}`, `${auth_key}`
**Error path**: Customer reports auth failures → `GET /admin/v1/projects/{id}` to verify auth_mode; check key hash matches.

### Step 3 — Verify Project Health
**Action**: `GET /admin/v1/projects/{id}` to confirm `status=active`.
**Expected output**: Project record shows `status=active`, `backend_mode`, `auth_mode` correct.
**Emotion**: Reassured → "The project is live"

### Step 4 — Monitor Usage
**Action**: ETL job queries `daily_project_metrics` for the project. Produces billing report.
**Expected output**: `ingress_bytes`, `egress_bytes`, `cpu_ms` rows for each day.
**Emotion**: Watchful → "I can see what they're consuming"

### Step 5 — Suspend Non-Paying Project
**Action**: `POST /admin/v1/projects/{id}/suspend`.
**Expected output**: `200 OK`. All subsequent Firestore requests to the project return `PermissionDenied: "project suspended"`.
**Emotion**: In-Control → "I stopped the bleed immediately"
**Shared artifacts consumed**: `${project_id}`
**Error path**: Customer contacts support claiming everything broke → expected; Sam explains suspension.

### Step 6 — Soft-Delete and Clean Up
**Action**: `DELETE /admin/v1/projects/{id}`. Background sweeper purges data after `admin.deletion_retention`.
**Expected output**: `200 OK`. Project returns `NotFound` immediately. Data purged asynchronously.
**Emotion**: Confident → "The data will be gone, no orphans"

---

## Shared Artifacts Registry

| Artifact | Produced in | Consumed in | Source of Truth |
|---|---|---|---|
| `${admin_endpoint}` | Infra config | All admin calls | service config `admin.port` |
| `${project_id}` | Step 1 | Steps 2–6 | Admin API project record |
| `${auth_key}` | Step 1 | Step 2 (delivery) | One-time; never stored in plaintext |
