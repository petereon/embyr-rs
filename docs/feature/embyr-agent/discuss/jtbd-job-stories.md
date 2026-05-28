# JTBD Job Stories — embyr-agent

> Feature: embyr-agent full StorageAgent implementation
> Wave: DISCUSS
> Date: 2026-05-27
> Extends: docs/product/jobs.yaml (JOB-04, adds JOB-07, JOB-08, JOB-09)

---

## JOB-04 (extended): Credential-Isolation — Keep database credentials inside my network perimeter

**Job story (original)**: When my security policy prohibits DB credentials from leaving my VPC, I want to
deploy the embyr agent alongside my Postgres instance, so I can use the Firestore SDK without exposing
credentials to any third party.

**Persona**: P4 — Riley (Compliance-first Tenant / DevSecOps Lead, e.g. Riley Nakamura at FinOps Corp)

| Dimension | Detail |
|---|---|
| Functional | embyr agent runs in-VPC; holds DSN in env; proxies all 8 storage RPCs; embyr SaaS never receives DSN |
| Emotional | Feel secure I will pass my next SOC2 / ISO 27001 audit; feel the architecture is principled and auditable |
| Social | Demonstrate to auditors and board that zero credentials traverse the cloud boundary |

**Gaps in original JOB-04** (discovered during embyr-agent brownfield analysis):
- Original job covered credential isolation as a single deployment act. It does NOT address:
  - What happens after deployment: daily operations, upgrades, monitoring the agent health
  - How change notifications flow through agent mode (Subscribe streaming RPC)
  - How a security auditor verifies the "no credentials leave VPC" invariant at runtime

---

## JOB-07: Agent-Operations — Deploy, configure, and verify the agent binary in production

**Job story**: When I need to put the embyr agent into production in my Kubernetes cluster (or bare-metal
VPC), I want to configure it via environment variables, verify it connects successfully to both Postgres
and the embyr SaaS, and know immediately if the startup fails, so I can complete the deployment in under
30 minutes and hand off a green health check to my SRE team.

**Persona**: P4 — Riley (DevSecOps Lead deploying to company VPC at FinOps Corp)

| Dimension | Detail |
|---|---|
| Functional | `EMBYR_AGENT_*` env vars configure everything; agent exits non-zero with named missing var; agent logs "listening on :9191" and "connected to Postgres" on success; embyr SaaS admin API accepts `backend_mode=agent` with endpoint + CA PEM |
| Emotional | Feel in control — no guessing whether the agent is actually running; feel the startup errors are clear, not cryptic |
| Social | Be able to show the SRE team a clean deploy log without Slack escalations |

**Forces**:
- Push: Without the agent, Riley cannot deploy embyr at all (company policy prohibits credential egress)
- Pull: A working startup probe + structured log means Riley can demo "it works" in the first deploy session
- Anxiety: "What if the TLS certs are wrong and I get a cryptic openssl error at 3am?" — clear startup errors mitigate this
- Habit: Riley runs `kubectl logs <pod>` to check deployment health; agent must emit machine-readable structured JSON logs

---

## JOB-08: Agent-LiveSync — Receive real-time document changes through the agent without polling

**Job story**: When SDK clients use `onSnapshot` on a project backed by `backend_mode=agent`, I want
the agent to forward Postgres NOTIFY events to the embyr SaaS via the Subscribe streaming RPC, so
collaborative features work identically to direct-mode projects without the SDK knowing the difference.

**Persona**: P1 — Alex (SDK Developer, whose app's project was configured by Riley with `backend_mode=agent`)
Secondary: P4 — Riley (who must ensure LiveSync works before signing off the production migration)

| Dimension | Detail |
|---|---|
| Functional | Agent maintains a Postgres LISTEN connection; on committed write, pushes DocChange over Subscribe stream; embyr SaaS fans out to Listen targets; overflow (>64 buffered events) triggers RESET; on stream disconnection, SaaS reconnects with exponential backoff 1s→30s |
| Emotional | Alex feels the real-time UX is identical to Firestore (no awareness of agent mode). Riley feels confident the change pipeline is reliable — not a fragile bespoke pub-sub |
| Social | Alex can demo real-time collaboration to stakeholders without caveats about "agent mode limitations" |

**Forces**:
- Push: Without Subscribe, agent mode requires polling — degraded experience vs Firestore; clients lose real-time UX
- Pull: Subscribe streaming with reconnect means agent mode is a full production peer of direct mode
- Anxiety: "What if the Subscribe stream drops and changes are lost silently?" — RESET + re-snapshot is the safety net
- Habit: Alex tests real-time with Firestore Emulator; wants identical behavior from agent mode integration tests

---

## JOB-09: Agent-AuditProof — Prove to security auditors that no credential leaves the VPC boundary

**Job story**: When my external security auditor asks me to demonstrate the "zero credential egress"
claim, I want to show them the agent's network traffic log, the embyr SaaS system DB entry
(`backend_mode=agent`, no DSN stored), and the mTLS handshake log, so I can close the audit finding
without writing a remediation plan.

**Persona**: P4 — Riley (now presenting to external auditor at FinOps Corp's annual SOC2 review)

| Dimension | Detail |
|---|---|
| Functional | System DB `projects` row shows `backend_mode=agent`, NULL `backend_pg_creds_enc`; agent logs show mTLS handshake details at DEBUG; agent environment (DSN) is never echoed in any log line; agent GRPC wire logs contain no DSN strings |
| Emotional | Feel confident walking into the audit room; feel the evidence is unambiguous and self-documenting |
| Social | Be seen by the auditor as operating a mature, evidence-based security posture |

**Forces**:
- Push: Without audit evidence, a verbal "we don't store DSNs" claim is insufficient for SOC2 Type II
- Pull: A system DB row + network capture showing no DSN = concrete, non-repudiable evidence
- Anxiety: "What if a bug in the agent logs the DSN at DEBUG level?" — this must be tested (negative test: grep for DSN string in logs)
- Habit: Auditors expect to see a config file or DB record; agent system DB row serves as the record
