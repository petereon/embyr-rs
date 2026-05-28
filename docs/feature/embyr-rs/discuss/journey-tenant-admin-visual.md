# Journey: Tenant Admin — Cloud Secret Manager Integration

> Persona: P3 — Morgan (Tenant Admin / DevOps Lead)
> Job: JOB-05 (Cloud-Secret)
> Research depth: Comprehensive

---

## Emotional Arc

```
Sceptical  Cautious   Confident  Efficient  Reassured  Trusting
    │          │          │          │          │          │
    ▼          ▼          ▼          ▼          ▼          ▼
[Evaluate] [IAM]     [Register] [Verify]  [Rotate]  [Operate]
```

---

## Journey Steps

### Step 1 — Evaluate the Cloud-Secret Option
**Action**: Morgan reads embyr docs. Confirms: (a) embyr fetches DSN from AWS/GCP at project registration and caches for TTL=5 min, (b) secret rotation is transparent (embyr re-fetches on cache miss), (c) no long-lived credential copy lives in embyr system DB.
**Expected output**: Morgan's evaluation notes: "embyr uses our existing secret store — no second credential store to operate."
**Emotion**: Sceptical → Cautious → "This fits our existing controls"
**Shared artifacts consumed**: none

### Step 2 — Grant embyr Cloud IAM Access
**Action (AWS path)**: Morgan creates IAM role with `secretsmanager:GetSecretValue` on the specific secret ARN. Configures IRSA (IAM Roles for Service Accounts) so embyr's Kubernetes pod can assume the role.
**Action (GCP path)**: Morgan grants embyr's GCP service account `roles/secretmanager.secretAccessor` on the specific secret resource.
**Expected output**: `aws secretsmanager get-secret-value --secret-id ${secret_arn}` succeeds from embyr's execution context.
**Emotion**: Cautious → Confident → "Least-privilege, exactly one secret"
**Shared artifacts produced**: `${secret_arn}` (AWS) OR `${secret_gcp_name}` (GCP)
**Error path**: Permission denied → Morgan verifies trust policy / workload identity binding; checks embyr service account ARN matches IRSA annotation.

### Step 3 — Register Project with Cloud-Secret Backend
**Action (AWS)**: `POST /admin/v1/projects` with `backend_mode=aws_secret`, `backend_secret_arn=${secret_arn}`.
**Action (GCP)**: `POST /admin/v1/projects` with `backend_mode=gcp_secret`, `backend_secret_gcp=${secret_gcp_name}`.
**Expected output**: `201 Created`. embyr fetches DSN at registration time and runs migrations against customer DB. Auth key returned one-time.
**Emotion**: Efficient → "One API call, no credential copy"
**Shared artifacts consumed**: `${secret_arn}` OR `${secret_gcp_name}`
**Shared artifacts produced**: `${project_id}`, `${auth_key}`
**Error path**: embyr cannot read secret → `400 Bad Request: backend_secret_fetch_failed` → Morgan checks IAM; verifies secret format is `{"dsn": "postgres://..."}`.

### Step 4 — Verify Connectivity
**Action**: `GET /admin/v1/projects/${project_id}` → confirm `status=active`. Run a smoke-test SDK write and read.
**Expected output**: Project shows `status=active`. SDK round-trip succeeds.
**Emotion**: Reassured → "Credential fetch is invisible to the app"
**Error path**: `status=migration_failed` → embyr fetched secret but migrations failed → Morgan checks DSN points to correct DB with migration permissions.

### Step 5 — Rotate Database Password
**Action**: Morgan rotates DB password in AWS/GCP Secret Manager (standard cloud-native rotation). No embyr interaction needed; embyr picks up new value on next cache expiry (≤5 min TTL).
**Expected output**: After ≤5 min, all new SDK requests succeed with the rotated password. Zero downtime.
**Emotion**: Trusting → "Rotation is completely transparent"
**Shared artifacts produced**: none (secret value updated in cloud provider — embyr unaware)
**Error path**: Rotation window > TTL causes transient `Unavailable` → SDK retries; embyr re-fetches on next auth. Max gap = 5 min.

### Step 6 — Operate and Audit
**Action**: Morgan audits embyr system DB — confirms no plaintext DSN stored, only `backend_secret_arn` or `backend_secret_gcp` resource pointer. Cloud-native secret audit log shows embyr's `GetSecretValue` calls.
**Expected output**: Audit finding: "embyr stores only a secret reference, not the secret value. Access auditable via AWS CloudTrail / GCP Cloud Audit Logs."
**Emotion**: Trusting → "We have full audit trail in our existing tooling"

---

## Shared Artifacts Registry

| Artifact | Produced in | Consumed in | Source of Truth |
|---|---|---|---|
| `${secret_arn}` | Step 2 (AWS IAM setup) | Step 3 | AWS Secrets Manager |
| `${secret_gcp_name}` | Step 2 (GCP IAM setup) | Step 3 | GCP Secret Manager |
| `${project_id}` | Step 3 | Steps 4–6 | Admin API project record |
| `${auth_key}` | Step 3 | App deployment | One-time; never stored in plaintext |
