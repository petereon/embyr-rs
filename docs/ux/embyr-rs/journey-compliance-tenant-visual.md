# Journey: Compliance-first Tenant — Agent Mode Setup

> Persona: P4 — Riley (Compliance-first Tenant / CISO)
> Job: JOB-04 (Credential-Isolation)
> Research depth: Comprehensive

---

## Emotional Arc

```
Sceptical  Cautious   Controlled  Secure    Validated  Confident
    │          │          │          │          │          │
    ▼          ▼          ▼          ▼          ▼          ▼
[Evaluate] [Deploy]  [Register]  [Test]   [Audit]   [Operate]
```

---

## Journey Steps

### Step 1 — Evaluate the Agent Architecture
**Action**: Riley reads the embyr agent documentation. Confirms: (a) DSN lives only in `EMBYR_AGENT_DB_DSN` env var, (b) embyr SaaS connects to agent over mTLS, (c) no credentials traverse the internet.
**Expected output**: Riley's risk register entry is updated: "embyr agent — zero credential egress, mTLS verified."
**Emotion**: Sceptical → Cautious → "This could work"

### Step 2 — Deploy Agent in VPC
**Action**: Deploy embyr agent Kubernetes pod: set `EMBYR_AGENT_DB_DSN`, mount TLS cert/key, configure `EMBYR_AGENT_CA` with embyr's CA cert. Agent starts on `:9191`.
**Expected output**: Agent logs show `"listening on :9191"` and `"connected to Postgres"`.
**Emotion**: Controlled → "I own this binary"
**Shared artifacts produced**: `${agent_endpoint}` (internal K8s service), `${agent_ca_pem}`

### Step 3 — Register Project with embyr SaaS
**Action**: Sam (service operator) calls `POST /admin/v1/projects` with `backend_mode=agent`, `backend_agent_endpoint=${agent_endpoint}`, `backend_agent_ca=${agent_ca_pem}`.
**Expected output**: Project created. embyr SaaS connects to agent and runs migrations.
**Emotion**: Secure → "embyr never saw our password"
**Shared artifacts consumed**: `${agent_endpoint}`, `${agent_ca_pem}`
**Error path**: Agent unreachable from SaaS → Riley checks VPC peering / network policy; verifies agent port is accessible from embyr's egress IPs.

### Step 4 — Test SDK Connectivity
**Action**: Riley's dev team runs the test suite against embyr endpoint using the project credentials. All CRUD, query, and onSnapshot tests pass.
**Expected output**: Test suite green. Agent logs show incoming gRPC calls and DB queries.
**Emotion**: Validated → "The app works and our DB never left the VPC"

### Step 5 — Security Audit Walkthrough
**Action**: Auditor reviews: embyr SaaS config (no DSN stored), agent config (`EMBYR_AGENT_DB_DSN` in K8s Secret), network traffic (only mTLS agent gRPC visible on egress), agent binary (statically linked Rust, no dynamic deps).
**Expected output**: Auditor confirms zero credential egress. Control passes.
**Emotion**: Confident → "We passed"

### Step 6 — Operate and Monitor
**Action**: Periodic cert rotation: update `EMBYR_AGENT_CERT`/`EMBYR_AGENT_KEY`, rolling restart agent, update `backend_agent_ca` in embyr project record via admin PATCH.
**Expected output**: Zero downtime during cert rotation (agent stays up during rolling restart).
**Emotion**: Confident → "Operations are routine"

---

## Shared Artifacts Registry

| Artifact | Produced in | Consumed in | Source of Truth |
|---|---|---|---|
| `${agent_endpoint}` | Step 2 | Step 3 | K8s service DNS |
| `${agent_ca_pem}` | Step 2 (cert issuance) | Step 3 | Customer PKI |
| `${embyr_ca_pem}` | embyr infra | Step 2 (`EMBYR_AGENT_CA`) | embyr operator config |
