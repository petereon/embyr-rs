# Secrets Manager Setup (AWS / GCP)

Closes High finding #46 from `docs/product/production-readiness-audit-2026-09-08.md`.

## Scope

This document covers **only** the four fixed operator secrets sourced via `ServerConfig`
(ADR-018): `EMBYR_ADMIN_KEY`, `EMBYR_ADMIN_KEY_PREVIOUS`, `EMBYR_ENCRYPTION_KEY`,
`EMBYR_ENCRYPTION_KEY_PREVIOUS` — each optionally resolved from an
`*_AWS_SECRET_ARN`/`*_GCP_SECRET_NAME` variable instead of a plain env var. ADR-018 documents
the *code's* fetch behavior; this is the missing operator-facing half: the actual IAM policy
JSON, secret naming, and CLI commands to create the secrets.

**Not in scope:** the per-project customer-database credential sourcing
(`backend_mode=aws_secret`/`gcp_secret` on a `Project`). That's a different secret (a customer
Postgres DSN, JSON-shaped `{"dsn": "..."}`), owned and IAM-managed by the *customer*, not the
operator — see `docs/operations/backup-disaster-recovery.md` § Customer Database Backup (All
Modes) for that boundary. Both paths happen to reuse the same `AwsSecretFetcher`/
`GcpSecretFetcher` adapter classes in code (`crates/embyr-server/src/adapters/`), but the
customer-DSN path calls `get_dsn()` (JSON-wrapped) while the operator-secret path this
document covers calls `get_raw_secret()` (plain string, no JSON) — **do not JSON-wrap the
secret value for `EMBYR_ADMIN_KEY`/`EMBYR_ENCRYPTION_KEY`.** Store the raw token/hex string
directly as the secret's value.

No prior `docs/operations/secrets-management*` doc exists in this repo — this is a new file,
not a duplicate.

## AWS Secrets Manager

### 1. Create the secrets

The value is the raw string `embyr-server` expects on the equivalent plain env var —
`EMBYR_ADMIN_KEY` is any non-empty bearer token; `EMBYR_ENCRYPTION_KEY`/
`EMBYR_ENCRYPTION_KEY_PREVIOUS` must be exactly 64 hex characters (32 bytes).

```bash
aws secretsmanager create-secret \
  --name embyr/prod/admin-key \
  --secret-string "$(openssl rand -hex 32)"

aws secretsmanager create-secret \
  --name embyr/prod/encryption-key \
  --secret-string "$(openssl rand -hex 32)"
```

Note the returned ARN — Secrets Manager appends a random 6-character suffix
(e.g. `arn:aws:secretsmanager:us-east-1:123456789012:secret:embyr/prod/admin-key-AbCdEf`).
Use the full ARN (with suffix) or the ARN prefix with a `??????` wildcard (see IAM policy
below) — both work with `GetSecretValue`.

### 2. IAM policy

`AwsSecretFetcher::get_raw_secret`/`get_dsn` call
`self.client.get_secret_value().secret_id(arn).send()` — the only action needed is
`secretsmanager:GetSecretValue`, scoped to the specific secret ARNs (never `"*"`):

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Sid": "EmbyrServerReadFixedSecrets",
      "Effect": "Allow",
      "Action": "secretsmanager:GetSecretValue",
      "Resource": [
        "arn:aws:secretsmanager:us-east-1:123456789012:secret:embyr/prod/admin-key-??????",
        "arn:aws:secretsmanager:us-east-1:123456789012:secret:embyr/prod/encryption-key-??????"
      ]
    }
  ]
}
```

Attach this to whatever identity `embyr-server` runs as (an EC2 instance profile, an ECS task
role, or an IAM user with static `AWS_ACCESS_KEY_ID`/`AWS_SECRET_ACCESS_KEY` env vars) — the
code uses the standard AWS SDK default credential chain (`aws_config::load_from_env()`), no
embyr-specific credential env var exists. If the attached identity is missing this
permission, startup fails loudly with `ConfigError::SecretFetchFailed` (exit 1, no port
bound) rather than silently — this is the "Sam's IAM role is missing
`secretsmanager:GetSecretValue`" scenario named in ADR-018's own design examples (US-SM-01).

### 3. Point `embyr-server` at the secret

In `/etc/embyr/embyr-server.env` (see
[`single-host-deployment.md`](./single-host-deployment.md)), replace the plain value with the
ARN variant:

```
EMBYR_ADMIN_KEY_AWS_SECRET_ARN=arn:aws:secretsmanager:us-east-1:123456789012:secret:embyr/prod/admin-key-AbCdEf
EMBYR_ENCRYPTION_KEY_AWS_SECRET_ARN=arn:aws:secretsmanager:us-east-1:123456789012:secret:embyr/prod/encryption-key-GhIjKl
```

Setting both the plain var and its `_AWS_SECRET_ARN`/`_GCP_SECRET_NAME` counterpart for the
same logical secret is a startup error (`ConfigError::AmbiguousSecretSource`) — set exactly
one source per secret. The `_PREVIOUS` variants (`EMBYR_ADMIN_KEY_PREVIOUS_AWS_SECRET_ARN`,
`EMBYR_ENCRYPTION_KEY_PREVIOUS_AWS_SECRET_ARN`) follow the identical pattern during a key
rotation window — see ADR-018 §5/§6 and the rotation note below.

## GCP Secret Manager

### 1. Create the secrets

```bash
openssl rand -hex 32 | gcloud secrets create embyr-prod-admin-key --data-file=-
openssl rand -hex 32 | gcloud secrets create embyr-prod-encryption-key --data-file=-
```

### 2. IAM binding

`GcpSecretFetcher` calls `GET {base_url}/v1/{resource_name}/versions/latest:access` with a
Bearer token — the equivalent scoped permission is the `roles/secretsmanager.secretAccessor`
predefined role, bound at the **secret** level (not project-wide):

```bash
gcloud secrets add-iam-policy-binding embyr-prod-admin-key \
  --member="serviceAccount:embyr-server@my-project.iam.gserviceaccount.com" \
  --role="roles/secretsmanager.secretAccessor"

gcloud secrets add-iam-policy-binding embyr-prod-encryption-key \
  --member="serviceAccount:embyr-server@my-project.iam.gserviceaccount.com" \
  --role="roles/secretsmanager.secretAccessor"
```

### 3. Point `embyr-server` at the secret

`resource_name` must be the GCP Secret Manager resource path (`projects/*/secrets/*`, no
`/versions/latest` suffix — the code appends that itself):

```
EMBYR_ADMIN_KEY_GCP_SECRET_NAME=projects/my-project/secrets/embyr-prod-admin-key
EMBYR_ENCRYPTION_KEY_GCP_SECRET_NAME=projects/my-project/secrets/embyr-prod-encryption-key
EMBYR_GCP_ACCESS_TOKEN=ya29.c.b0Aa...
```

### 4. `EMBYR_GCP_ACCESS_TOKEN` — read this before relying on it

Unlike the AWS path (which uses the SDK's own credential chain with automatic refresh), the
GCP path is a **static bearer token with no in-process refresh**, an explicit, documented
interim design (ADR-018 Alternatives A6 — workload-identity/metadata-server token acquisition
was scoped out of this feature, tracked as open question OQ-SM-4 for a future feature). GCP
OAuth2 access tokens typically expire in about an hour. This is safe *only* because the token
is read once at process startup and then discarded — there is no window where a stale
in-memory token is silently reused (D-SM-4: startup-only sourcing).

**Practical consequence:** generate a fresh token immediately before each `embyr-server`
(re)start, not once and left in a long-lived env file:

```bash
gcloud auth activate-service-account embyr-server@my-project.iam.gserviceaccount.com \
  --key-file=/etc/embyr/gcp-sa-key.json
gcloud auth print-access-token
```

Wire this into the `single-host-deployment.md` systemd unit as an `ExecStartPre` that writes
a fresh token into the env file before each start, if you use the GCP path:

```ini
ExecStartPre=/usr/local/bin/refresh-embyr-gcp-token.sh
```

(That script is not provided here — it is a small operator-written wrapper around
`gcloud auth print-access-token`, matching the "no automation exists, document the manual
step" convention used elsewhere in `docs/operations/`.) If a restart happens with an expired
token, startup fails loudly (`ConfigError::SecretFetchFailed`, exit 1) rather than serving
with a broken secret — no silent degradation.

## Key rotation

Both AWS and GCP paths support the same `_PREVIOUS` rotation window as the plain-env-var path
(ADR-018 §3/§5/§6): set `EMBYR_ADMIN_KEY_PREVIOUS_AWS_SECRET_ARN`/`_GCP_SECRET_NAME` (or
`EMBYR_ENCRYPTION_KEY_PREVIOUS_...`) alongside the primary, restart, and `embyr-server` accepts
either the old or new admin token / decrypts with either key until you remove the `_PREVIOUS`
variable and restart again. `embyr-server` logs a startup warning
(`rotation_window_open=true`) for as long as a `_PREVIOUS` value resolves to `Some` — there is
no automatic expiry timer (D-SM-7); closing the window is a manual step.

## Cross-References

- [ADR-018: Secrets Management and Rotation](../product/architecture/adr-018-secrets-management.md)
  — the code's actual fetch/resolve/rotation behavior this document operationalizes.
- [`single-host-deployment.md`](./single-host-deployment.md) — where the env file these
  variables live in comes from, and the systemd `ExecStartPre` hook point for GCP token
  refresh.
- [`backup-disaster-recovery.md`](./backup-disaster-recovery.md) — the encryption-key recovery
  precondition (verifying `EMBYR_ENCRYPTION_KEY`/`_PREVIOUS` are recoverable from *this*
  document's sourcing mechanism before restoring the System DB).
- [`runbook.md`](./runbook.md) — the API-key-compromise incident procedure (finding #33),
  which assumes you can locate and rotate these same secrets quickly.
