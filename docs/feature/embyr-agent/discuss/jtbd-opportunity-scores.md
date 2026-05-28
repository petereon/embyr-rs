# JTBD Opportunity Scores — embyr-agent

> Feature: embyr-agent full StorageAgent implementation
> Wave: DISCUSS
> Date: 2026-05-27
> Scoring formula: Opportunity = Importance + max(Importance − Satisfaction, 0)
> Scale: 1–10 per dimension; Opportunity scores 10–20

---

## Scoring Table

| Job ID | Job Name | Importance (1–10) | Satisfaction (1–10) | Opportunity Score | Priority |
|--------|----------|--------------------|---------------------|-------------------|----------|
| JOB-04 | credential-isolation | 10 | 3 | **17** | P0 — Blocking gate |
| JOB-07 | agent-operations | 9 | 2 | **16** | P0 — Delivery path |
| JOB-08 | agent-livesync | 8 | 1 | **15** | P1 — Parity blocker |
| JOB-09 | agent-auditproof | 7 | 2 | **12** | P2 — Compliance audit |

**Reference: existing embyr-rs jobs for context**

| Job ID | Job Name | Importance | Satisfaction | Opportunity | Notes |
|--------|----------|------------|--------------|-------------|-------|
| JOB-01 | sdk-compat | 9 | 3 | 15 | Covered by embyr-rs WS slices S01–S09 |
| JOB-03 | live-sync | 9 | 4 | 14 | Covered by embyr-rs slices S06–S08; agent variant = JOB-08 |

---

## Scoring Rationale

### JOB-04 (Importance: 10 / Satisfaction: 3 → Score: 17)

**Importance = 10**: JOB-04 is the single reason `backend_mode=agent` exists. Without it, FinOps-type customers cannot use embyr at all (policy violation). Binary importance.

**Satisfaction = 3**: The agent binary exists (mTLS server starts, Postgres probe works) but all 8 RPCs return `Unimplemented`. Zero storage operations succeed. Satisfaction is marginally above 1 only because the wiring skeleton (mTLS + Postgres probe) is proven.

**Opportunity = 10 + (10–3) = 17**: Highest-opportunity job in the feature. Every storage operation gap directly suppresses satisfaction.

### JOB-07 (Importance: 9 / Satisfaction: 2 → Score: 16)

**Importance = 9**: DevOps operators cannot deploy what they cannot configure and verify. A binary that starts but gives no deployment feedback has no operational utility.

**Satisfaction = 2**: Config loading exists (`AgentConfig::from_env`) and the binary exits on missing vars. However: no `EMBYR_AGENT_MAX_CONNS` support, no structured startup log, no readiness signal beyond "no error exit." Satisfaction is 2/10 — config skeleton only.

**Opportunity = 9 + (9–2) = 16**: Second-highest. Closing this job unblocks the deployment verification loop.

### JOB-08 (Importance: 8 / Satisfaction: 1 → Score: 15)

**Importance = 8**: Without Subscribe, agent-mode projects cannot use `onSnapshot`. For real-time collaborative apps (the primary embyr use case per JOB-03), this is a production blocker.

**Satisfaction = 1**: Subscribe RPC is not even declared in the proto (the current storage_agent.proto omits it). Satisfaction = 1 (theoretical awareness only).

**Opportunity = 8 + (8–1) = 15**: Third-highest. Blocked by proto gap and requires Postgres LISTEN integration.

### JOB-09 (Importance: 7 / Satisfaction: 2 → Score: 12)

**Importance = 7**: Audit-proof evidence is needed by compliance-first tenants (P4) but is not blocking deployment. It is blocking contract renewal for some customers.

**Satisfaction = 2**: System DB schema is correct (`backend_pg_creds_enc` is NULL for agent mode). However, agent logs do not yet emit mTLS handshake details, and no negative DSN-logging test exists.

**Opportunity = 7 + (7–2) = 12**: Fourth-highest. Addressed by log discipline (no DSN in logs) and by system DB evidence — not by new features.

---

## Prioritization Decision

**P0 jobs (JOB-04, JOB-07)** drive all slices in the walking skeleton (S01A: GetDocument via agent through the wiring chain) and lifecycle slices (S06A: startup probe, graceful shutdown).

**P1 job (JOB-08)** drives the Subscribe slice (S05A). Must ship before agent mode can be marked production-ready for real-time apps.

**P2 job (JOB-09)** drives log discipline (no DSN leakage) and the negative test requirement that appears as an AC on the startup probe slice (S06A).
