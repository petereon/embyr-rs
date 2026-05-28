# JTBD Four Forces Analysis — embyr-agent

> Feature: embyr-agent full StorageAgent implementation
> Wave: DISCUSS
> Date: 2026-05-27

---

## JOB-07: Agent-Operations (Deploy, Configure, Verify)

| Force | Evidence | Detail |
|-------|----------|--------|
| **Push** (frustration with current state) | Blocking deployment | Without a working agent, Riley's entire Firestore migration is blocked. `backend_mode=agent` is the only approved path per company security policy. The alternative (direct_pg) exposes DSN to SaaS — policy violation. |
| **Pull** (desired future state) | Deploy once, run forever | Agent starts, logs success, registers with embyr SaaS, stays running. Riley can hand off a Kubernetes Deployment spec and let the SRE team operate it. No manual steps after initial cert registration. |
| **Anxiety** (adoption concern) | TLS complexity at 3am | mTLS cert management is the #1 operational risk. Riley fears: wrong CA path → silent TLS handshake failure → no traffic, no error, no clue. Mitigation: agent exits non-zero with named error if cert file is missing/unreadable; logs TLS handshake result at INFO on first connection. |
| **Habit** (inertia to overcome) | `kubectl logs` is the mental model | Riley verifies every deployment by checking pod logs. Agent must emit structured JSON logs (not plain text) to be compatible with Loki/Splunk log aggregation pipelines already in use at FinOps Corp. |

**Strongest demand-creating force**: Push (blocking deployment gate)
**Strongest demand-reducing force**: Anxiety (TLS configuration complexity)

---

## JOB-08: Agent-LiveSync (Real-time changes via Subscribe)

| Force | Evidence | Detail |
|-------|----------|--------|
| **Push** | Degraded UX without real-time | If Subscribe is missing, SDK's `onSnapshot` either falls back to polling (Firebase SDK behavior) or shows stale data. Collaborative features break. Alex's app looks "slow" compared to Firestore. |
| **Pull** | Transparent agent parity | Alex's integration tests pass identically against Firestore Emulator and agent mode. No `backend_mode`-conditional code paths in the application. |
| **Anxiety** | "Changes are silently lost" | Riley and Alex both fear that the Subscribe stream drops and no one notices until a user reports stale data. Mitigation: overflow flag triggers RESET → re-snapshot guarantees consistency; embyr SaaS reconnection with exponential backoff limits the gap window. |
| **Habit** | Emulator test harness | Alex's test suite runs against the Firebase Emulator. Switching to agent mode requires zero test changes — behavior must be identical. This constrains the Subscribe implementation to produce DocChange events in the exact same order and format as Postgres NOTIFY events in direct mode. |

**Strongest demand-creating force**: Pull (transparent parity)
**Strongest demand-reducing force**: Anxiety (silent change loss)

---

## JOB-09: Agent-AuditProof (Security audit evidence)

| Force | Evidence | Detail |
|-------|----------|--------|
| **Push** | Open audit finding | Riley has an open SOC2 finding: "credential management for external database connections is not documented." The finding blocks a contract renewal. |
| **Pull** | Closed audit finding, no remediation plan | The auditor accepts: (1) system DB row with NULL backend_pg_creds_enc; (2) network traffic capture showing no DSN in plaintext; (3) mTLS handshake log. The finding is closed with zero remediation items. |
| **Anxiety** | "A debug log accidentally prints the DSN" | If any log line at any level contains the DSN string, the audit evidence is invalidated. Mitigation: negative test case — start agent with a known DSN containing the string `DO-NOT-LOG`; grep all log output at all levels; assert zero matches. |
| **Habit** | Auditors read PDFs, not terminal output | Riley must export evidence to a PDF appendix. Structured JSON logs + system DB row printout + wireshark capture are the expected artifact format. The agent does not need to produce a PDF; it needs to produce machine-parseable evidence. |

**Strongest demand-creating force**: Push (open audit finding, contract at risk)
**Strongest demand-reducing force**: Habit (auditors need evidence in a standard format, not a live demo)

---

## JOB-04 (extended): Credential-Isolation Forces Summary

| Force | Detail |
|-------|--------|
| **Push** | Security policy absolutely prohibits DSN egress. No workaround. Deployment is blocked until agent mode works end-to-end. |
| **Pull** | Every storage operation (CRUD, queries, transactions, change subscriptions) works through the agent with no behavioral difference from direct mode. |
| **Anxiety** | "The agent binary is a black box in my VPC — what if it opens outbound connections I don't know about?" Mitigation: agent opens exactly two connections: one to Postgres (inbound to VPC from agent); one TLS listener on :9191 (accepts inbound from embyr SaaS only). No other outbound connections. |
| **Habit** | Riley's team uses Helm charts for all deployments. Agent env-var config is compatible with Kubernetes Secrets mounted as environment variables — no custom config file format to learn. |
