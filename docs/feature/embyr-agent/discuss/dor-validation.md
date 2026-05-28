# Definition of Ready Validation — embyr-agent

> Wave: DISCUSS
> Updated: 2026-05-27
> Validated against: feature-delta.md + slice briefs (S01A–S06A)

---

## DoR Checklist (per task instructions — 8-item hard gate)

### US-A01 — GetDocument via agent

| DoR Item | Status | Evidence |
|----------|--------|----------|
| 1. Problem statement clear, domain language | PASS | "All StorageAgent RPCs return Unimplemented; Riley cannot test whether the agent actually proxies anything." Real person, real pain. |
| 2. User/persona with specific characteristics | PASS | P4 Riley Nakamura, DevSecOps Lead, FinOps Corp |
| 3. 3+ domain examples with real data | PASS | finops-prod project, orders/ord-2026-001, users/riley; 3 examples covering happy path, edge, error |
| 4. UAT scenarios in Given/When/Then (3–7) | PASS | 3 scenarios: round-trip, not-found, invalid name |
| 5. AC derived from UAT | PASS | 4 ACs mapping to scenarios; each cites SPEC.md |
| 6. Right-sized (1–3 days, 3–7 scenarios) | PASS | ≤1 day, 3 scenarios, S01A |
| 7. Technical notes: constraints/dependencies | PASS | Depends on mTLS skeleton (done); testcontainers required |
| 8. Dependencies resolved or tracked | PASS | mTLS skeleton: DONE (step 09-01); embyr_proto: DONE |

**DoR Status: PASSED**

---

### US-A02 — Write operations via agent

| DoR Item | Status | Evidence |
|----------|--------|----------|
| 1. Problem statement clear | PASS | "SDK writes to agent-backed projects return Unimplemented; Alex's app cannot write any data." |
| 2. Persona | PASS | P1 Alex Chen, SDK Developer |
| 3. 3+ domain examples | PASS | setDoc, updateDoc with mask, OCC conflict with real field values |
| 4. UAT scenarios (3–7) | PASS | 5 scenarios including @property |
| 5. AC derived from UAT | PASS | 7 ACs; each cites SPEC.md |
| 6. Right-sized | PASS | ≤1 day, 5 scenarios, S02A |
| 7. Technical notes | PASS | Dependencies: S01A; field transforms table from SPEC.md |
| 8. Dependencies tracked | PASS | S01A listed; currently in planned sequence |

**DoR Status: PASSED**

---

### US-A03 — Query operations via agent

| DoR Item | Status | Evidence |
|----------|--------|----------|
| 1. Problem statement clear | PASS | "Query RPCs return Unimplemented from agent; dashboards using getDocs(query) are broken" |
| 2. Persona | PASS | P1 Alex Chen |
| 3. 3+ domain examples | PASS | filtered query, collection group, invalid field path |
| 4. UAT scenarios (3–7) | PASS | 4 scenarios |
| 5. AC derived from UAT | PASS | 7 ACs; each cites SPEC.md |
| 6. Right-sized | PASS | ≤1 day, 4 scenarios, S03A |
| 7. Technical notes | PASS | BatchGetDocuments proto gap flagged; index enforcement deferred |
| 8. Dependencies tracked | PASS | S01A, S02A listed |

**DoR Status: PASSED**

---

### US-A04 — Transaction lifecycle via agent

| DoR Item | Status | Evidence |
|----------|--------|----------|
| 1. Problem statement clear | PASS | "runTransaction fails with Unimplemented on agent-mode projects; concurrent-write scenarios are unsafe" |
| 2. Persona | PASS | P1 Alex Chen (order processing, inventory management) |
| 3. 3+ domain examples | PASS | Clean commit, OCC conflict + retry, read document deleted during tx |
| 4. UAT scenarios (3–7) | PASS | 3 scenarios |
| 5. AC derived from UAT | PASS | 6 ACs; each cites SPEC.md §Transactions |
| 6. Right-sized | PASS | ≤1 day, 3 scenarios, S04A |
| 7. Technical notes | PASS | OCC latency hypothesis; depends on S02A |
| 8. Dependencies tracked | PASS | S01A, S02A listed |

**DoR Status: PASSED**

---

### US-A05 — Real-time change subscription via Subscribe

| DoR Item | Status | Evidence |
|----------|--------|----------|
| 1. Problem statement clear | PASS | "onSnapshot never fires for agent-mode projects; real-time features are broken" |
| 2. Persona | PASS | P1 Alex Chen (real-time dashboard) |
| 3. 3+ domain examples | PASS | Live update, Subscribe overflow + RESET, agent disconnect + reconnect |
| 4. UAT scenarios (3–7) | PASS | 4 scenarios including @property |
| 5. AC derived from UAT | PASS | 7 ACs; each cites SPEC.md |
| 6. Right-sized | PASS | ≤1 day (after SPIKE), 4 scenarios, S05A |
| 7. Technical notes | PASS | Pre-slice SPIKE required; proto extension required; Postgres NOTIFY trigger needed |
| 8. Dependencies tracked | PASS | S01A, S02A listed; SPIKE listed |

**DoR Status: PASSED**

---

### US-A06 — Agent startup probe and graceful shutdown

| DoR Item | Status | Evidence |
|----------|--------|----------|
| 1. Problem statement clear | PASS | "Agent opens gRPC listener even when Postgres is unreachable; startup failure not clearly signaled" |
| 2. Persona | PASS | P4 Riley Nakamura, SOC2 audit context |
| 3. 3+ domain examples | PASS | Clean startup, SIGTERM during in-flight RPC, DSN-never-in-logs audit |
| 4. UAT scenarios (3–7) | PASS | 5 scenarios |
| 5. AC derived from UAT | PASS | 8 ACs; each cites SPEC.md or Invariant 13 |
| 6. Right-sized | PASS | ≤1 day, 5 scenarios, S06A |
| 7. Technical notes | PASS | AgentConfig gaps (max_conns, log_level, listen_addr validation) explicitly listed |
| 8. Dependencies tracked | PASS | S01A required; config gaps identified in current codebase |

**DoR Status: PASSED**

---

## Feature-Level DoR (per task instructions — 8-item checklist)

| Item | Status | Evidence |
|------|--------|----------|
| 1. Every user story has a testable AC | PASS | Each US has 3–8 ACs citing SPEC.md section |
| 2. Every story traces to a job_id | PASS | US-A01: JOB-04; US-A02/A03/A04: JOB-01; US-A05: JOB-08; US-A06: JOB-07 |
| 3. Every slice has a complete slice brief | PASS | S01A–S06A all have briefs in docs/feature/embyr-agent/slices/ |
| 4. All 5 Elephant Carpaccio taste tests applied | PASS | All 5 taste tests documented in story-map.md; all PASS |
| 5. Walking skeleton slice identified and estimated ≤6h | PASS | S01A: GetDocument end-to-end, estimated ≤6 hours |
| 6. Elevator Pitch on every non-@infrastructure story | PASS | All 6 stories have Before/After/Decision-enabled triplets |
| 7. Dependencies between slices explicit | PASS | Each slice brief lists dependencies; execution order in story-map.md |
| 8. Spec-traceability: each AC cites SPEC.md section | PASS | All slice briefs and story ACs cite SPEC.md section |

**Feature-Level DoR Status: PASSED**

---

## Open Design Questions (for DESIGN wave)

| ID | Question | Impact |
|----|----------|--------|
| OQ-1 | BatchGetDocuments: extend proto vs. N parallel GetDocument calls in AgentAdapter? | S03A implementation strategy |
| OQ-2 | Should EMBYR_AGENT_MAX_CONNS also apply to the dedicated LISTEN connection (Subscribe), or is the LISTEN connection always a single separate connection outside the pool? | S05A connection management |
| OQ-3 | Drain timeout on SIGTERM: should it be configurable via env var (EMBYR_AGENT_SHUTDOWN_TIMEOUT) or hardcoded at 30s? | S06A config surface |

These are NOT blocking DoR — they are flagged for DESIGN wave resolution.
