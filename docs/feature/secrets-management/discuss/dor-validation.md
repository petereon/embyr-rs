# Definition of Ready Validation — secrets-management

> Wave: DISCUSS
> Updated: 2026-08-09
> Validated against: `feature-delta.md`, `story-map.md`, `user-stories.md`, `outcome-kpis.md`

---

## DoR Checklist (9-item hard gate, plus job_id + Elevator Pitch)

### US-SM-01 — Admin key sourced from AWS/GCP Secrets Manager at startup

| DoR Item | Status | Evidence |
|----------|--------|----------|
| 1. Problem statement clear, domain language | PASS | "EMBYR_ADMIN_KEY ... can only be set as a literal environment variable. Every audit flags the literal value sitting in the deployment manifest." |
| 2. User/persona with specific characteristics | PASS | Sam Chen (P2), Service operator deploying for secrets-manager-policy customers |
| 3. 3+ domain examples with real data | PASS | 5 examples: AWS ARN, GCP resource name, CI fallback, ambiguous-source error, IAM-denied fetch error |
| 4. UAT scenarios in Given/When/Then (3–7) | PASS | 6 scenarios |
| 5. AC derived from UAT | PASS | 7 ACs, each traceable to a scenario |
| 6. Right-sized (1–3 days, 3–7 scenarios) | PASS | 1 day, 6 scenarios |
| 7. Technical notes: constraints/dependencies | PASS | Walking skeleton; generalizes fetchers; optional-adapter pattern noted |
| 8. Dependencies resolved or tracked | PASS | No dependency — this is the walking skeleton |
| 9. Outcome KPIs defined with measurable targets | PASS | KPI-1 target: 100% of new deployments avoid literal admin-key value (0% today) |
| job_id present | PASS (JOB-14) |
| Elevator Pitch present | PASS — Before/After/Decision-enabled triplet with a real entry point (`cargo run -p embyr-server` / deployment manifest) and observable output (server logs, HTTP 200/401) |

**DoR Status: PASSED**

---

### US-SM-02 — Encryption key sourced from AWS/GCP Secrets Manager at startup

| DoR Item | Status | Evidence |
|----------|--------|----------|
| 1. Problem statement clear, domain language | PASS | "EMBYR_ENCRYPTION_KEY has the same literal-env-var-only limitation ... a leaked manifest value is a wider blast radius." |
| 2. User/persona with specific characteristics | PASS | Sam Chen (P2), same context as US-SM-01 |
| 3. 3+ domain examples with real data | PASS | 5 examples including a real OIDC client_secret value and a real invalid-length error message |
| 4. UAT scenarios in Given/When/Then (3–7) | PASS | 6 scenarios |
| 5. AC derived from UAT | PASS | 8 ACs, each traceable to a scenario |
| 6. Right-sized (1–3 days, 3–7 scenarios) | PASS | 0.5–1 day, 6 scenarios |
| 7. Technical notes: constraints/dependencies | PASS | Builds on US-SM-01's raw-fetch method; call sites explicitly unmodified |
| 8. Dependencies resolved or tracked | PASS | Depends on US-SM-01 (raw-string fetch method) — tracked |
| 9. Outcome KPIs defined with measurable targets | PASS | KPI-1 (shared with US-SM-01) |
| job_id present | PASS (JOB-14) |
| Elevator Pitch present | PASS |

**DoR Status: PASSED**

---

### US-SM-03 — Rotate EMBYR_ENCRYPTION_KEY without orphaning existing encrypted data

| DoR Item | Status | Evidence |
|----------|--------|----------|
| 1. Problem statement clear, domain language | PASS | "Rotating it ... permanently orphans every existing ... row, because Aes256Gcm::new_from_slice(&state.encryption_key) uses exactly one key with no fallback." |
| 2. User/persona with specific characteristics | PASS | Sam Chen (P2), responding to a suspected leak or scheduled rotation |
| 3. 3+ domain examples with real data | PASS | 4 examples: Maria Santos (existing TOTP user), Diego Ramirez (post-rotation enrollment), corrupted-ciphertext case, operational-boundary case |
| 4. UAT scenarios in Given/When/Then (3–7) | PASS | 6 scenarios |
| 5. AC derived from UAT | PASS | 7 ACs, each traceable to a scenario |
| 6. Right-sized (1–3 days, 3–7 scenarios) | PASS | 1–1.5 days, 6 scenarios |
| 7. Technical notes: constraints/dependencies | PASS | Explicitly documents the write-only status of 2 of 3 AES-GCM sites; explicitly defers lazy re-encryption with rationale |
| 8. Dependencies resolved or tracked | PASS | Soft dependency on US-SM-02 for secrets-manager-sourced `_PREVIOUS`; plain-env-var rotation independently deliverable — tracked |
| 9. Outcome KPIs defined with measurable targets | PASS | KPI-2 target: 0 orphaned rows per rotation (vs. 100% today) |
| job_id present | PASS (JOB-14) |
| Elevator Pitch present | PASS — Before/After/Decision-enabled triplet; "sees" clause is an observable sign-in success (HTTP-level outcome), not internal state |

**DoR Status: PASSED**

---

### US-SM-04 — Rotate EMBYR_ADMIN_KEY without a hard cutover outage

| DoR Item | Status | Evidence |
|----------|--------|----------|
| 1. Problem statement clear, domain language | PASS | "requires coordinating every operator client ... to update simultaneously with the restart, because operator_auth_middleware accepts exactly one valid token value." |
| 2. User/persona with specific characteristics | PASS | Sam Chen (P2), responsible for admin-key rotation |
| 3. 3+ domain examples with real data | PASS | 4 examples: mid-rotation dual-validity, leaked-key hard cutover, stale-token-after-close, ambiguous-window error |
| 4. UAT scenarios in Given/When/Then (3–7) | PASS | 6 scenarios |
| 5. AC derived from UAT | PASS | 7 ACs, each traceable to a scenario |
| 6. Right-sized (1–3 days, 3–7 scenarios) | PASS | 0.5–1 day, 6 scenarios |
| 7. Technical notes: constraints/dependencies | PASS | Single-function extension point named (`operator_auth_middleware`); `OperatorState` field addition named |
| 8. Dependencies resolved or tracked | PASS | Soft dependency on US-SM-01 for secrets-manager-sourced `_PREVIOUS` only; otherwise independent — tracked |
| 9. Outcome KPIs defined with measurable targets | PASS | KPI-3 target: 0 unplanned outages caused by admin-key rotation |
| job_id present | PASS (JOB-14) |
| Elevator Pitch present | PASS |

**DoR Status: PASSED**

---

## Feature-Level DoR

| Item | Status | Evidence |
|------|--------|----------|
| 1. Every user story has a testable AC | PASS | Each US has 7–8 ACs, each traceable to a UAT scenario |
| 2. Every story traces to a job_id | PASS | All 4 stories → JOB-14 |
| 3. Scope assessed and right-sized (or split approved) | PASS | `story-map.md` Scope Assessment: PASS — 4 stories, 3 bounded contexts, 3.5–4.5 days |
| 4. Walking skeleton identified | PASS | US-SM-01, rationale in `story-map.md` |
| 5. Elevator Pitch on every non-@infrastructure story | PASS | All 4 stories have Before/After/Decision-enabled triplets with real entry points and observable output |
| 6. Dependencies between stories explicit | PASS | US-SM-02 depends on US-SM-01 (hard); US-SM-03/US-SM-04 have soft dependencies on US-SM-01/02 for secrets-manager-sourced `_PREVIOUS` only, called out explicitly in each story's Technical Notes |
| 7. Priority rationale documented | PASS | `story-map.md` § Priority Rationale — risk-severity-based ordering (data-loss risk before availability risk) |
| 8. Outcome KPIs defined at feature level | PASS | `outcome-kpis.md` — 3 leading KPIs + 1 guardrail KPI, north star identified |
| 9. Shared artifacts tracked with single source of truth | PASS | `shared-artifacts-registry.md` — 12 artifacts, 5 integration validation rules |

**Feature-Level DoR Status: PASSED**

---

## Honesty Check (Anti-Pattern Detection)

| Anti-Pattern | Checked | Result |
|--------------|---------|--------|
| Implement-X | All 4 stories start from Sam Chen's audit/rotation pain, not "implement secrets manager integration" | PASS |
| Generic data | Real names (Maria Santos, Diego Ramirez), real ARNs, real secret-shaped values used throughout | PASS |
| Technical AC | ACs describe observable behavior (HTTP status codes, exit codes, log content) not implementation ("use AES-GCM") | PASS |
| Technical scenario titles | All scenario titles describe business outcomes ("Both current and previous admin tokens are accepted"), not internals ("middleware checks OR condition") | PASS |
| Oversized story | All 4 stories ≤ 6 scenarios, ≤ 1.5 days | PASS |
| Overclaimed scope | US-SM-03 explicitly states 2 of 3 AES-GCM sites are write-only today rather than claiming full 3-site rotation testing | PASS — verified against actual codebase, not assumed |

---

## Open Design Questions (for DESIGN wave)

| ID | Question | Impact |
|----|----------|--------|
| OQ-1 | Should the rotation-aware decrypt helper live in `embyr-server::adapters` (new small module) or as an associated function on `ServerConfig`/`UserAdminState`? | US-SM-03 implementation location |
| OQ-2 | Should `EMBYR_ADMIN_KEY_PREVIOUS` and `EMBYR_ENCRYPTION_KEY_PREVIOUS` each get their own `_AWS_SECRET_ARN`/`_GCP_SECRET_NAME` variants (4 more env vars), or should the `_PREVIOUS` value only ever be a plain env var? | US-SM-03/US-SM-04 config surface size — the DISCUSS-wave stories assume `_PREVIOUS` is sourceable via the same mechanism as the primary key, but DESIGN may simplify if operational complexity outweighs the benefit |
| OQ-3 | Should there be an operator-facing warning (log line, not a hard error) when `_PREVIOUS` has been configured for longer than some threshold, as a nudge to close the window? | Out of scope per D-SM-7, but worth flagging as a possible low-cost addition |

These are NOT blocking DoR — they are flagged for DESIGN wave resolution.
