# Outcome KPIs — embyr-agent

> Feature: embyr-agent full StorageAgent implementation
> Wave: DISCUSS
> Updated: 2026-05-27

---

## Feature Objective

Enable security-conscious tenants (P4, DevSecOps) to deploy the embyr-agent in their VPC and route all Firestore SDK operations through it, with SDK parity to direct-mode projects, real-time change delivery within 2 seconds, and zero credential egress proven by audit-grade evidence.

---

## Outcome KPIs

| # | Who | Does What | By How Much | Baseline | Measured By | Type |
|---|-----|-----------|-------------|----------|-------------|------|
| KPI-1 | P4 — Riley (DevSecOps) | Completes first agent deployment from "configure env vars" to first SDK operation succeeding | In ≤ 30 minutes | N/A (feature doesn't exist) | Integration test wall-clock time; dogfood deploy log timestamp delta | Leading |
| KPI-2 | P4 — Riley | Passes SOC2 audit item "zero credential egress" without remediation plan | 100% pass rate on first attempt (zero open findings) | 0% (no agent, no evidence) | SOC2 audit report; CI negative test (grep sentinel DSN = 0 matches) | Leading |
| KPI-3 | P1 — Alex | SDK operations (setDoc, getDoc, query, runTransaction) succeed on agent-backed projects | 100% success rate (parity with direct mode) | 0% today (all RPCs Unimplemented) | Integration parity test suite: same ops against direct-mode and agent-mode | Leading |
| KPI-4 | P1 — Alex | `onSnapshot` fires within 2 seconds of committed writes on agent-backed project | P99 latency < 2s under ≤100 writes/sec | N/A (Subscribe doesn't exist) | Integration test: write → measure time-to-callback in test harness | Leading |
| KPI-5 | P4 — Riley | Agent startup validates Postgres connectivity before accepting any RPC | 100% of startups probe Postgres first | 0% (listener opens before probe today) | Startup log ordering test: "connected to Postgres" must precede "listening on :9191" | Leading |

---

## Metric Hierarchy

- **North Star**: KPI-3 — agent-mode project is 100% operationally equivalent to direct-mode (all SDK operations succeed). This is the observable proxy for JOB-04's core value ("use the Firestore SDK without exposing credentials to any third party").
- **Leading Indicators**: KPI-1 (deploy success), KPI-4 (real-time parity), KPI-5 (startup probe ordering)
- **Guardrail Metrics**:
  - No log line at any level contains a Postgres DSN string (hard invariant — audited by KPI-2)
  - gRPC `write-read round-trip` latency via agent stays within 50ms of direct-mode on LAN (SDK parity)
  - Zero in-flight RPCs lost on graceful SIGTERM shutdown (all complete or receive Unavailable, never silently dropped)

---

## Measurement Plan

| KPI | Data Source | Collection Method | Frequency | Owner |
|-----|-------------|-------------------|-----------|-------|
| KPI-1 | Integration test log timestamps | CI test output; dogfood deploy log | Per release | embyr-agent team |
| KPI-2 | CI negative test (grep sentinel) | Automated test in `tests/features/` | Every PR, every merge | embyr-agent team |
| KPI-3 | Parity integration test suite | Run same test suite against direct-mode + agent-mode Postgres; compare results | Per merge | embyr-agent team |
| KPI-4 | Integration test harness | Write commit timestamp → onSnapshot callback timestamp delta in test | Per merge to main | embyr-agent team |
| KPI-5 | Log ordering test | CI test asserts log line order | Per PR | embyr-agent team |

---

## Hypothesis

We believe that implementing all 8 StorageAgent RPCs + Subscribe streaming + graceful lifecycle for the embyr-agent binary will achieve full operational equivalency with direct-mode projects for security-conscious tenants.

We will know this is true when:
- P4 (Riley) deploys the agent in ≤30 minutes and passes the SOC2 audit check with zero remediation items
- P1 (Alex) runs his existing Firebase SDK test suite against an agent-backed project with 100% passing tests
- `onSnapshot` fires within 2 seconds of committed writes, matching direct-mode behavior
