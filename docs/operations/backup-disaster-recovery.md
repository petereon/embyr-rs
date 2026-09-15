# Backup and Disaster Recovery

## Overview

embyr-rs is a Firestore-compatible gRPC protocol translation layer that depends on two categories of Postgres databases: the **System Database** (housing accounts, sessions, billing, and metrics) and **Customer Databases** (holding customer documents and application state). This document clarifies backup ownership, recovery procedures, and the operational preconditions required to restore service after a database failure.

**Key principle:** embyr-rs is a protocol translation layer only — it does not provision, monitor, or verify backups for any database. Backup responsibility is determined by database ownership, which is determined by deployment topology. See [ADR-001 (Process Topology)](../product/architecture/adr-001-process-topology.md) for deployment topology.

---

## Backup Ownership Boundary

| Database | Who Runs It | Who Backs It Up | embyr's Role |
|---|---|---|---|
| **System DB** (accounts, sessions, admin, billing, metrics, signing keys) | The operator running embyr-rs (embyr for hosted SaaS; self-hoster for self-hosted deployments) | **Operator** (embyr for SaaS; self-hoster otherwise) | Documents the requirement and recommended mechanism; does not implement backup tooling itself. See § System Database Backup & Recovery. |
| **Customer DB, `backend_mode=direct_pg`** | Customer (embyr holds only an encrypted DSN, see [ADR-018](../product/architecture/adr-018-secrets-management.md)) | **Customer** | None — embyr never sees the instance beyond a connection string. See § Customer Database Backup (All Modes). |
| **Customer DB, `backend_mode=agent`** | Customer (credentials never leave the customer's VPC — `embyr-agent` holds them) | **Customer** | None — architecturally cannot back up what it cannot reach. See § Customer Database Backup (All Modes). |
| **Customer DB, `backend_mode=aws_secret`** | Customer (DSN fetched from the customer's own cloud secret manager at connect time) | **Customer** | None — same boundary as `direct_pg`, different credential-sourcing mechanism. See § Customer Database Backup (All Modes). |
| **Customer DB, `backend_mode=gcp_secret`** | Customer (DSN fetched from the customer's own cloud secret manager at connect time) | **Customer** | None — same boundary as `direct_pg`, different credential-sourcing mechanism. See § Customer Database Backup (All Modes). |

**Critical fact:** Customer-database backup is the **customer's responsibility in all four backend modes, without exception.** The mode names the mechanism by which embyr *connects* to the customer's database, not the ownership of the database itself.

---

## System Database Backup & Recovery

### Recommended Mechanism

System Database backup should use one of the following approaches, depending on your hosting platform:

- **Managed Postgres services (AWS RDS, Google Cloud SQL, Azure Database for PostgreSQL):** Enable automated backups and point-in-time recovery (PITR). These services provide continuous WAL archiving and typically offer a multi-day to multi-week retention window.
- **Self-managed Postgres:** Use a continuous WAL-archiving strategy with a tool like `pgbackrest`, `WAL-E`, or `barman`. This enables point-in-time recovery for any time within your retention window.

Both approaches provide continuous or near-continuous recovery capability without requiring downtime for backup operations.

### Recovery Objective Targets

**This is a target, pending hosting and infrastructure decisions (see [ADR-017 Production Startup](../product/architecture/adr-017-production-startup.md)):**

- **RPO (Recovery Point Objective) target:** ≤ 5 minutes of data loss (pending hosting decision to lock specific backup mechanism)
- **RTO (Recovery Time Objective) target:** ≤ 30 minutes from detection of System Database failure to embyr-rs serving requests again (pending hosting decision to lock specific recovery mechanism and LB/DNS configuration)

These targets are **aspirational pending a locked hosting platform decision.** Actual RTO/RPO will depend on:
- Which Postgres service or self-managed backup tooling is configured
- The backup/PITR retention window configured
- Network and storage I/O characteristics of your hosting environment
- How quickly the operator can re-provision infrastructure and re-point `DATABASE_URL` to the recovered instance

### Recovery Procedure

1. **Identify the failure:** System Database is unreachable; `embyr-server` process cannot connect to `DATABASE_URL` and logs show connection-pool exhaustion or repeated connection failures.

2. **Determine recovery scope:** Did the Postgres instance itself fail (hardware, disk, process crash), or is the problem network connectivity? This determines whether you restore from backup or reconnect to an existing instance.

3. **Provision a new Postgres instance (or recover the failed one):** Using your hosting platform's PITR capability, restore from the most recent backup to a point-in-time before the failure was detected. Name it (`<deployment>-systemdb-recovered`, for example).

4. **Verify encryption-key material is available (CRITICAL PRECONDITION):** Before proceeding, confirm that both `EMBYR_ENCRYPTION_KEY` and `EMBYR_ENCRYPTION_KEY_PREVIOUS` (if a rotation was in flight) are recoverable and available to the embyr-server process. See § Encryption-Key Recovery Precondition below.

5. **Point `DATABASE_URL` to the recovered instance:** Update the environment variable or configuration management system supplying `DATABASE_URL` to the new Postgres instance DNS/hostname.

6. **Restart embyr-server:** Start (or restart if already running) the embyr-server process. It will:
   - Connect to the recovered System Database
   - Run pending schema migrations (if any; see `embyr-db-prep`)
   - Populate the connection pool
   - Bind to its configured ports and resume accepting requests

7. **Verify system is healthy:** Check `GET /healthz` (readiness — confirms System Database connectivity via `SystemDb::probe()`) and `GET /livez` (liveness — process-alive only, never touches Postgres; see [ADR-078](../product/architecture/adr-078-liveness-readiness-split.md)). Monitor logs for any schema-version mismatches or connection errors.

8. **Reconnect agents (if applicable):** If your deployment uses `backend_mode=agent`, the embyr-agent binary continues to hold the customer-database credentials independently. No agent restart is required unless the agent's own local state was also lost.

---

## Encryption-Key Recovery Precondition

**This must be verified BEFORE declaring a System Database restore complete.**

Three columns in the System Database contain encryption-sensitive data protected by `EMBYR_ENCRYPTION_KEY` (see [ADR-018 Secrets Management](../product/architecture/adr-018-secrets-management.md)):

- `users.totp_secret_enc` — TOTP secrets used for admin account authentication
- `oidc_providers.client_secret_enc` — OAuth client secrets for OpenID Connect providers
- `projects.backend_pg_dsn_enc` — Encrypted customer-database connection strings (for `backend_mode=direct_pg`)

**The Postgres backup itself contains only the encrypted bytes.** The encryption key is stored externally (as environment variables or in AWS/GCP Secrets Manager) and is **not derivable from the Postgres backup.**

If `EMBYR_ENCRYPTION_KEY` (and `EMBYR_ENCRYPTION_KEY_PREVIOUS` during a rotation window) is lost or unavailable at restore time:

- Admin accounts with TOTP protection become locked out (TOTP verification will fail, decryption of `totp_secret_enc` fails)
- OpenID Connect providers stop working (client secrets cannot be decrypted)
- Direct-mode customer projects lose their stored connection strings (DSNs cannot be decrypted; operators must re-provision those projects' connection strings)
- **The raw Postgres data is intact and recoverable; only the encrypted columns become unreadable.**

### Before Restoring System Database

1. Verify that `EMBYR_ENCRYPTION_KEY` is available from its configured source:
   - If sourced from an environment variable: confirm the variable is set correctly
   - If sourced from AWS Secrets Manager: verify the ARN is correct and your IAM credentials have `secretsmanager:GetSecretValue` permission
   - If sourced from GCP Secrets Manager: verify the secret name and resource path are correct, and your GCP credentials have the appropriate permissions

2. If a key rotation is in flight, also verify `EMBYR_ENCRYPTION_KEY_PREVIOUS` is available. The presence of this key determines whether rows encrypted under the previous key can be decrypted during the restore.

3. Document where these keys are stored, and ensure they are part of your disaster-recovery plan. **These keys are not stored in Postgres; they must be separately managed and verified.**

---

## Customer Database Backup (All Modes)

embyr-rs **does not** provision, monitor, or manage backups of any customer database, regardless of backend mode. Backup of customer data is entirely your customers' responsibility.

### What embyr-rs Does When Customer Database Is Unreachable

If a customer's Postgres database becomes unreachable after a System Database restore (or at any other time), embyr-rs behaves as documented in [ADR-001 (Process Topology, Consequences)](../product/architecture/adr-001-process-topology.md):

- The connection pool times out attempting to reach the customer's database
- Requests fail with `Unavailable` (gRPC code 14, HTTP 503)
- The SDK retries according to its retry policy
- No data in embyr's System Database is modified; the outage is transparent to embyr's own infrastructure

### Per-Mode Guidance

#### direct_pg Mode

The customer provides embyr with a connection string (encrypted and stored in `projects.backend_pg_dsn_enc`). embyr connects to their Postgres instance using that string. The customer is responsible for:

- Provisioning and running their own Postgres instance
- Configuring backups and PITR on their instance
- Recovering their instance in case of failure
- Testing restore procedures

embyr's only involvement is reading the encrypted DSN and using it to connect. embyr does not touch the customer's instance beyond executing queries over that connection.

#### agent Mode

The customer runs `embyr-agent` in their own VPC; the agent holds the database credentials and embyr never sees them. The customer is responsible for:

- Running `embyr-agent` in their infrastructure
- Provisioning and running their own Postgres instance
- Backing up their Postgres instance
- Recovering their instance in case of failure
- Keeping the agent alive and network-reachable to embyr

embyr connects to the agent (not the customer's database directly) using mTLS. The agent is responsible for connecting to the customer's database and proxying queries. If the customer's database is unreachable, the agent returns an error to embyr, and embyr returns `Unavailable` to the client.

#### aws_secret, gcp_secret Modes

The customer stores their Postgres connection string in AWS Secrets Manager or GCP Secret Manager under their own cloud account. embyr fetches the secret at query time (or caches it, depending on configuration) and connects to the customer's Postgres instance using that connection string. The customer is responsible for:

- Storing their connection string in their own AWS/GCP secret manager
- Provisioning and running their own Postgres instance
- Configuring backups and PITR on their instance
- Recovering their instance in case of failure
- Managing IAM/secret-manager access control so embyr can retrieve the secret

embyr does not store the connection string persistently; it is fetched at connection time from the customer's cloud secret manager.

---

## Verification and Drill Cadence

Backup and recovery procedures are only useful if they work when needed. **Recommended cadence:**

- **Quarterly:** Run a full System Database restore drill (restore from backup to a test instance, verify health endpoints pass, verify a sample request succeeds, verify encryption key is available)
- **Annually:** Run a full recovery incident simulation, including DNS/load-balancer re-pointing, to verify all infrastructure and operational steps

**Note:** These drills are not currently automated by embyr-rs. There is no built-in verification script, no restore-drill automation, and no admin API endpoint to check backup freshness. Drills are a manual operator procedure, documented in your runbook.

---

## Non-Goals

The following are explicitly **not** embyr-rs's responsibility:

- **Provisioning or managing any database** — embyr does not create, scale, or patch System or Customer databases
- **Monitoring backup freshness** — no automated alert tells you whether a backup completed or is stale
- **Verifying restore functionality** — no automated test runs restore drills on your behalf
- **Encrypting or decrypting customer documents** — customer documents are stored in the customer's database and encrypted (or not) according to the customer's own schema
- **Implementing backup compliance or retention policies** — your organization's compliance requirements (GDPR, HIPAA, SOC2) for data retention and purge schedules are outside embyr-rs's scope

Backup and disaster recovery are operational responsibilities of the entity running embyr-rs (embyr's SRE team, your infrastructure team, or your managed-services provider).

---

## Cross-References

- **[ADR-001: Process Topology](../product/architecture/adr-001-process-topology.md)** — deployment topology, System DB ownership, customer DB modes (`direct_pg`, `agent`, `aws_secret`, `gcp_secret`)
- **[ADR-017: Production Startup](../product/architecture/adr-017-production-startup.md)** — startup configuration conventions, `DATABASE_URL` and environment-variable handling
- **[ADR-018: Secrets Management and Rotation](../product/architecture/adr-018-secrets-management.md)** — `EMBYR_ENCRYPTION_KEY` custody, dual-key rotation window, encrypted columns
- **[ADR-078: Liveness/Readiness Split](../product/architecture/adr-078-liveness-readiness-split.md)** — `/healthz` (readiness) vs `/livez` (liveness) semantics
- **[Architecture Brief: System Database](../product/architecture/brief.md)** — System Database schema and failure modes (referenced for "Customer DB unreachable → `Unavailable` returned to client")
