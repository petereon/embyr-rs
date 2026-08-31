# agent-mode-transaction-purge — Feature Delta

**Wave**: DISCUSS | **Agent**: Luna (nw-product-owner) | **Date**: 2026-08-31
**Status**: Ready for DESIGN handoff — zero escalated open questions. See § Handoff Package.
**Upstream**: No DISCOVER/DIVERGE wave ran — commissioned directly by the orchestrator as one of 4 confirmed `backend_mode=agent` parity gaps, originally framed as "embyr-agent has zero sweeper code of its own." **Investigation corrects that framing** (§ Reading Confirmation) — a sweeper already exists and already reclaims orphaned active transactions; the real, narrower gap is that it never purges terminal `'committed'` rows, an identical gap to the one `customer-db-transaction-sweeper`'s own US-02 closed for `direct_pg`/`aws_secret`/`gcp_secret`, deliberately excluding `backend_mode=agent` from its own scope (its own § System Constraints: "`backend_mode=agent` is excluded from the sweeper's own project-enumeration query entirely... `StorageAgent`'s own proto has no bulk/sweep-shaped RPC today").

---

## Wave: DISCUSS / [REF] Prior Wave Consultation — Reading Confirmation

✓ `crates/embyr-agent/src/sweeper.rs` (full, 53 lines) — **corrects the orchestrator's own framing**: `AgentTransactionSweeper` already exists, is already spawned in production (`crates/embyr-agent/src/server.rs::run`, lines 828-835: 60-second TTL, 30-second sweep interval), and already runs `sweep_once()` — `DELETE FROM transactions WHERE started_at < NOW() - $1 * INTERVAL '1 second' AND status = 'active'`. This is a HARD DELETE of abandoned `'active'` rows, not a two-phase mark-then-purge — functionally stronger than `customer-db-transaction-sweeper`'s own US-01 (which only marks `'expired'`, deferring deletion to US-02's separate retention window). The task's original grep (`"transaction_sweeper"`/`"orphaned"`) missed this because the file is named `sweeper.rs`, not `transaction_sweeper.rs`, and the code uses `'expired'`-adjacent language ("expired active transaction records") rather than the word "orphaned."
✓ `crates/embyr-agent/src/server.rs::run` (lines 809-859, targeted) — confirmed the sweeper is wired into the production binary's own startup path (not a dead/unused module), spawned unconditionally alongside the gRPC listener.
✓ `migrations/customer/0002_transactions.sql` (referenced via `customer-db-transaction-sweeper`'s own § Reading Confirmation, schema shared by `PostgresBackendAdapter` across ALL backend modes including agent, since `embyr-agent` uses the identical `PostgresBackendAdapter`) — confirmed schema: `transaction_id UUID PRIMARY KEY, project_id VARCHAR(63), status VARCHAR(20) DEFAULT 'active', started_at TIMESTAMPTZ DEFAULT now()`. `commit_transaction` (shared crate, § sibling feature `agent-mode-field-transforms`'s own Reading Confirmation) sets `status = 'committed'` on successful commit — identically for direct_pg and agent-mode, since both call the same `PostgresBackendAdapter::commit_transaction`.
✓ `crates/embyr-agent/src/sweeper.rs`'s own `sweep_once` query — confirmed it targets `status = 'active'` ONLY. **`'committed'` rows are never touched by any existing code path in `embyr-agent`** — they accumulate forever, identically to the gap `customer-db-transaction-sweeper` US-02 closed for the other three backend modes.
✓ `docs/feature/customer-db-transaction-sweeper/feature-delta.md` (targeted, § System Constraints, § US-02) — confirmed direct precedent: `SessionCleaner`'s own "hard delete, no soft-delete needed, contains no user-generated content" pattern, and the exact retention-window mechanics (backdated `started_at`, purge query shape) this feature mirrors, one level down (agent-local, not cross-project System-DB enumeration).
✓ `docs/SPEC.md` § Agent gRPC Protocol (line 498) — confirmed the documented (aspirational) protocol summary lists "Transaction lifecycle (Begin, GetForTransaction, Commit, Rollback, Sweep)" — including "Sweep" as a named capability. The ACTUAL implementation took a different, arguably better shape than an RPC-triggered sweep: an autonomous agent-LOCAL background interval loop, requiring no trigger from `embyr-server` at all (§ System Constraints — documentation/implementation divergence noted, not treated as a defect).
✓ `docs/product/jobs.yaml` — confirmed no job covers this narrowly; mirrors `customer-db-transaction-sweeper`'s own JOB-12 resolution.

No contradictions found between this feature's scope and prior evidence, beyond the orchestrator's own initial "zero sweeper" framing, which is explicitly corrected above.

---

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated, with one correction noted)

| # | Decision | Value |
|---|---|---|
| 1 | Feature Type | Backend/Infrastructure — extends an EXISTING agent-local background sweeper with a purge step, mirroring `customer-db-transaction-sweeper`'s own US-02 for the other three backend modes |
| 2 | Walking Skeleton | YES — one story IS the walking skeleton; the reclaim mechanism already exists (not part of this feature's own scope), only the purge step is new |
| 3 | UX Research Depth | Lightweight — Sam Chen gets a small extension to an already-planned observable surface pattern (mirrors `customer-db-transaction-sweeper`'s own Prometheus-counter precedent) |
| 4 | JTBD Analysis | Confirmed: **extends JOB-12 (observability)**, NOT JOB-01 — same reasoning as `customer-db-transaction-sweeper` itself: zero SDK-observable behavior change, Alex never sees this |

**Correction to orchestrator framing**: the task's premise ("embyr-agent has zero sweeper code of its own") does not hold — `AgentTransactionSweeper` exists and already reclaims orphaned active transactions, arguably more aggressively (hard-delete, no retention window) than the direct_pg/aws/gcp sweeper's own two-phase mark-then-purge design. This feature's real scope is narrower than originally framed: add a purge step for terminal `'committed'` rows only.

---

## Wave: DISCUSS / [REF] Pre-requisites

- `AgentTransactionSweeper` (`crates/embyr-agent/src/sweeper.rs`) — ships already, already spawned in production; this feature extends its own `sweep_once()` (or adds a sibling method) with a purge step, reusing the identical `pool`/`interval` fields.
- `customer-db-transaction-sweeper` US-02 (shipped, non-agent modes) — the retention-window purge PATTERN (`DELETE ... WHERE status IN (...) AND started_at < now() - retention_window`) this feature mirrors, one level down (agent-local Postgres, no DSN resolution needed — the agent already holds its own live pool).
- No dependency on the other 3 sibling `backend_mode=agent` features (confirmed independent — this is the ONLY one of the 4 with zero proto/wire-protocol changes, § System Constraints).

---

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: **P2 — Sam Chen (Service Operator / Platform Engineer)**, same persona as `customer-db-transaction-sweeper`'s own US-02 — this time monitoring an agent-mode deployment's own local customer Postgres (inside the customer's VPC, e.g., Meridian Health's own infrastructure) rather than a System-DB-enumerated fleet.

**job_id decision**: `JOB-12` (`observability`), extended not new — identical resolution to `customer-db-transaction-sweeper`.

---

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

| Signal | Threshold | This feature | Fired? |
|---|---|---|---|
| User stories | >10 | 1 (US-01) | **NO** |
| Bounded contexts / modules | >3 | 1 — BC-2 Document Storage, agent-local transaction bookkeeping | **NO** |
| Walking Skeleton integration points | >5 | 2 — (1) extend `AgentTransactionSweeper::sweep_once` (or add a sibling purge method) targeting `status = 'committed'` past a retention window, (2) optional Prometheus counter if `embyr-agent` exposes a metrics endpoint (§ Handoff Package Escalation 1) | **NO** |
| Estimated effort | >2 weeks | ~0.5-1 day, single slice | **NO** |
| Independent shippable outcomes | multiple unrelated | **NO** | **NO** |

**0 of 5 signals fired. Verdict: PASS — single feature, single story, right-sized. Smallest and lowest-risk of the 4 sibling features** — no proto changes, no cross-version compatibility question (§ System Constraints), purely an extension to an already-running agent-local background job.

---

## Wave: DISCUSS / [REF] Journey (Lightweight, per Decision 3)

**Sam's mental model**: Sam already knows (from `customer-db-transaction-sweeper`'s own delivery) that `direct_pg`/`aws_secret`/`gcp_secret` customer databases have both reclaim AND purge running. Sam has no equivalent visibility into whether Meridian Health's own agent-mode deployment (`embyr-agent` running inside Meridian Health's VPC) has the same protection — and today it does not: committed transaction rows accumulate forever in that deployment's local Postgres, with only the active-row reclaim already in place.

**Emotional arc** (Problem Relief, narrow scope): **Start** — mild unease once Sam realizes agent-mode deployments were left out of the fleet-wide purge work. **Middle** — Sam confirms the agent's own sweeper now purges committed rows past retention, mirroring the other three backend modes' own behavior. **End** — relief; every backend mode now has consistent, bounded transaction-table growth, with no backend-mode-specific gap remaining.

**Shared artifact**: the `transactions.status` state machine (`'active'` → `'committed'`, hard-deleted directly from `'active'` if abandoned) — reused unchanged; no new status value introduced, mirroring `customer-db-transaction-sweeper`'s own constraint.

**Failure modes**: a `'committed'` row within the retention window must be left untouched | a `'committed'` row is created WHILE a purge cycle is running (must not race) | the agent's own Postgres connection is briefly unavailable during a sweep cycle (existing `tracing::warn!` log-and-continue behavior, unchanged).

---

## Wave: DISCUSS / [REF] Story Map & Walking Skeleton

**Persona**: P2 Sam Chen | **Goal**: Every agent-mode deployment's own local `transactions` table has bounded row-count growth, matching the guarantee `customer-db-transaction-sweeper` already gives the other three backend modes.

### Backbone

| A. Purge Terminal Rows |
|---|
| Hard-delete `'committed'` rows older than the retention window **[WS]** |
| Leave rows within the retention window untouched **[WS]** |

### Walking Skeleton

The existing `AgentTransactionSweeper`'s own sweep cycle, after its already-shipped active-row reclaim step, additionally runs `DELETE FROM transactions WHERE status = 'committed' AND started_at < NOW() - retention_window`. This is Slice 01, US-01 — the entire feature.

---

## Wave: DISCUSS / [REF] WS Strategy

**Strategy: B (Real, Narrow Slice)** — real Postgres, a real `'committed'` row backdated past the retention window, mirroring `customer-db-transaction-sweeper` US-02's own precedent exactly (which itself mirrored `SessionCleaner`'s testing approach for time-based retention).

---

## Wave: DISCUSS / [REF] Elephant Carpaccio Slices

| Slice | Story | Release | Estimate | Learning Hypothesis (disproves) | Production-data taste test |
|---|---|---|---|---|---|
| 01 (WS) | US-01 | 1 | 0.5-1 day | Extending the already-running `AgentTransactionSweeper` with a second delete statement introduces a race against a `'committed'` row that transitions from a concurrent in-flight `commit()` call during the same sweep cycle | Real Postgres, a real `'committed'` row backdated past retention, plus a concurrently-committing transaction during the same sweep cycle, asserting the backdated row is purged and the concurrent commit completes normally without interference |

**Total estimate: 0.5-1 day, single slice.**

**Taste tests applied**: 4+ components — 1-2 components (extend an existing sweeper method, optional metric) — PASS, clearly thin. New-abstraction-first — no new abstraction, pure extension of an existing one — PASS. Falsifiable hypothesis — PASS. Production data only — PASS. No identical-except-scale slices — N/A, single slice.

---

## Wave: DISCUSS / [REF] Prioritization

| Priority | Slice | Target Outcome | Rationale |
|---|---|---|---|
| 1 | Slice 01 (WS) | Agent-mode's own local `transactions` table stops growing unboundedly from committed rows | Single-slice feature; no sequencing decision needed |

---

## Wave: DISCUSS / [REF] System Constraints

- **No new `transactions.status` value.** Purge targets the existing `'committed'` value only — `'active'` rows are already handled by the existing reclaim step (out of this feature's own scope, already shipped).
- **No proto or wire-protocol change of any kind.** This is the ONLY one of the 4 sibling `backend_mode=agent` features with zero cross-binary surface change — `embyr-server` is not involved at all; the sweeper is entirely internal to `embyr-agent`'s own process. **The cross-version graceful-degradation question that applies to the other 3 sibling features (§ their own Escalation sections) does NOT apply here** — there is nothing for an older/newer version mismatch to affect.
- **Reuses the retention-window purge PATTERN from `customer-db-transaction-sweeper` US-02**, not its code (different crate, different DSN-resolution context — the agent already holds a live pool, no DSN resolution needed at all).
- **Retention window default value is a DESIGN/DEVOPS tuning question**, not committed here — mirrors `customer-db-transaction-sweeper`'s own precedent (its own OQ-CP-3 equivalent).

---

## Wave: DISCUSS / [REF] Driving Ports

This feature has **no client-invocable driving port** — identical framing to `customer-db-transaction-sweeper` itself.

| Port | Trigger | Notes |
|---|---|---|
| `AgentTransactionSweeper::sweep_once` (extended) | `tokio::time::interval` tick, already running in production | Existing spawn point (`crates/embyr-agent/src/server.rs::run`, lines 828-835), unchanged signature — this feature adds a second `DELETE` statement to the existing sweep cycle, not a new spawn point |

No new gRPC RPC, no new REST route. Sam Chen never calls anything to trigger this feature.

---

## Wave: DISCUSS / [REF] User Stories

<!-- markdownlint-disable MD024 -->

### US-01: Committed Transaction Rows Are Purged in Agent-Mode Deployments, Bounding Table Growth

**job_id**: JOB-12 (observability) — extends, "make it real" pattern, mirrors `customer-db-transaction-sweeper` US-02

#### Problem

Sam Chen already knows `customer-db-transaction-sweeper` bounds `transactions` table growth for `direct_pg`/`aws_secret`/`gcp_secret` customer databases — but that feature explicitly excluded `backend_mode=agent` (no connection model to reach it remotely). Meridian Health's own agent-mode deployment already reclaims abandoned `'active'` rows (existing `AgentTransactionSweeper`), but every successfully `'committed'` transaction row persists forever — the identical unbounded-growth gap `customer-db-transaction-sweeper` US-02 closed for the other three backend modes, just never extended to agent mode.

#### Who

- Sam Chen, Service Operator/Platform Engineer | Monitoring Meridian Health's own agent-mode deployment (their local Postgres, inside their VPC) alongside Trailmark/Acme's own direct_pg/aws_secret/gcp_secret deployments | Wants consistent, bounded transaction-table growth across every backend mode, not a backend-mode-specific gap

#### Solution

The existing `AgentTransactionSweeper`'s own sweep cycle gains a second step: hard-delete `'committed'` transaction rows older than a retention window, mirroring `customer-db-transaction-sweeper` US-02's own purge pattern.

#### Elevator Pitch

Before: Meridian Health's agent-mode `transactions` table accumulates every successfully-committed transaction row forever — no deletion path exists for `'committed'` rows in `embyr-agent`'s own sweeper.
After: an operator runs `SELECT count(*) FROM transactions WHERE status = 'committed'` against Meridian Health's own Postgres → sees the count stabilize at a bounded, retention-window-scoped value instead of growing indefinitely, after the next sweep cycle runs.
Decision enabled: Sam decides agent-mode deployments no longer need a manual cleanup script or an escalation when a customer's own DBA flags table bloat — the sweeper now behaves identically to every other backend mode.

#### Domain Examples

##### 1: Happy Path — A `'committed'` row past the retention window is purged

A Meridian Health transaction committed successfully 45 days ago (`status = 'committed'`, `started_at` 45 days in the past). The retention window is 30 days. The next sweep cycle's new purge step hard-deletes the row.

##### 2: Edge Case — A `'committed'` row still within the retention window

A Meridian Health transaction committed 10 days ago. The retention window is 30 days. The purge step leaves it untouched this cycle.

##### 3: Boundary — A row committing during the same sweep cycle is not disturbed

A transaction transitions from `'active'` to `'committed'` (via a real, concurrent `commit()` call) WHILE the sweep cycle's purge step is running. The purge step's own `WHERE started_at < NOW() - retention_window` clause does not match this just-committed row (its `started_at` is recent); it completes normally and is picked up by a future cycle once it ages past retention.

#### UAT Scenarios (BDD)

##### Scenario: A committed transaction row past the retention window is purged
```gherkin
Given meridian-health-prod's local transactions table has a row with status "committed" and started_at 45 days ago
And the retention window is 30 days
When the next sweep cycle runs
Then that transaction row no longer exists in the transactions table
```

##### Scenario: A committed transaction row within the retention window is left untouched
```gherkin
Given meridian-health-prod's local transactions table has a row with status "committed" and started_at 10 days ago
And the retention window is 30 days
When the next sweep cycle runs
Then that transaction row still exists in the transactions table
```

##### Scenario: A concurrently-committing transaction is not disturbed by the same sweep cycle
```gherkin
Given a transaction is actively being committed via a real commit() call
When the sweep cycle's purge step runs concurrently
Then the in-flight commit completes normally
And the newly-committed row is not purged in the same cycle, since its started_at is recent
```

##### Scenario: The existing active-row reclaim behavior is unaffected by the new purge step
```gherkin
Given meridian-health-prod has a row with status "active" and started_at 20 minutes ago, older than the abandonment TTL
When the next sweep cycle runs
Then that row is deleted by the existing reclaim step, exactly as it was before this feature
```

#### Acceptance Criteria

- [ ] A `'committed'` transaction row older than the retention window is hard-deleted by the next sweep cycle
- [ ] A `'committed'` transaction row within the retention window is never deleted
- [ ] A transaction committing concurrently with a running sweep cycle is not disturbed and completes normally
- [ ] The existing active-row reclaim behavior (already shipped) is unaffected by this feature's own purge addition

#### Outcome KPIs

- **Who**: Sam Chen (Service Operator)
- **Does what**: Any agent-mode deployment's own local `transactions` table row count stabilizes to a bounded value instead of growing indefinitely
- **By how much**: Steady-state row count bounded by `(sweep interval × cycles within retention window)` worth of committed rows, rather than unbounded lifetime accumulation
- **Measured by**: Operator-run `SELECT count(*) FROM transactions WHERE status = 'committed'` spot-check (not itself instrumented in v1, mirrors `customer-db-transaction-sweeper` US-02's own measurement plan)
- **Baseline**: 0 committed rows ever deleted today in agent-mode deployments — every `'committed'` row persists forever, by construction (confirmed, § Reading Confirmation)

#### Technical Notes (Optional)

- Extends `AgentTransactionSweeper::sweep_once` (or adds a sibling method called from the same spawn loop) — no new spawn point, no new configuration surface beyond an optional retention-window value.
- Zero proto or cross-binary wire change (§ System Constraints) — this feature does not touch `storage_agent.proto` or `agent_backend.rs`.
- Optional: if `embyr-agent` exposes (or later exposes) its own Prometheus endpoint, a `embyr_agent_transaction_sweeper_purged_total` counter would mirror `customer-db-transaction-sweeper`'s own observability precedent — flagged as an optional enhancement, not required for DoR (§ Handoff Package).

---

## Wave: DISCUSS / [REF] Outcome KPIs (Feature-Level Summary)

### Feature: agent-mode-transaction-purge

### Objective

Every agent-mode deployment's own local `transactions` table has bounded row-count growth, closing the last backend-mode-specific gap in `customer-db-transaction-sweeper`'s own fleet-wide reliability guarantee.

### Outcome KPIs

| # | Who | Does What | By How Much | Baseline | Measured By | Type |
|---|-----|-----------|-------------|----------|-------------|------|
| 1 | Any agent-mode deployment | `transactions` table `'committed'`-row count | Unbounded growth → bounded, steady-state | Unbounded (every committed row persists forever today) | Operator-run row-count spot-check | Lagging |

### Metric Hierarchy

- **North Star**: KPI #1 — the actual problem statement
- **Leading Indicators**: sweep cycle purge execution (not separately instrumented in v1 — see Technical Notes optional counter)
- **Guardrail Metrics**: purge must never delete a row within the retention window (data-safety guardrail, hard AC not a KPI) | purge must never interfere with a concurrently-committing transaction (correctness guardrail)

### Measurement Plan

| KPI | Data Source | Collection Method | Frequency | Owner |
|-----|------------|-------------------|-----------|-------|
| Committed row count | Agent-local Postgres | Manual operator query (v1) | Ad hoc | Sam Chen (operator) |

### Hypothesis

We believe that extending the already-running `AgentTransactionSweeper` with a retention-window purge step for `'committed'` rows will bound agent-mode deployments' own local `transactions` table growth, matching the guarantee already given to the other three backend modes.
We will know this is true when a spot-checked agent-mode deployment's committed-row count stabilizes rather than growing without bound.

---

## Wave: DISCUSS / [REF] Definition of Ready Validation

### Story: US-01

| DoR Item | Status | Evidence/Issue |
|----------|--------|----------------|
| Problem statement clear | PASS | Committed rows accumulate forever in agent-mode's own local Postgres, the identical gap `customer-db-transaction-sweeper` US-02 closed elsewhere |
| User/persona identified | PASS | Sam Chen, Service Operator, monitoring Meridian Health's agent-mode deployment |
| 3+ domain examples | PASS | Happy path (past-retention purge), edge case (within-retention no-op), boundary (concurrent commit during sweep) |
| UAT scenarios (3-7) | PASS | 4 scenarios |
| AC derived from UAT | PASS | 4 AC items |
| Right-sized | PASS | 0.5-1 day, 4 scenarios — smallest of the 4 sibling features |
| Technical notes | PASS | Exact extension point named, zero proto change confirmed |
| Dependencies tracked | PASS | No blocking dependency; extends already-shipped `AgentTransactionSweeper` |
| Outcome KPIs | PASS | Numeric framing and measurement method defined, mirrors `customer-db-transaction-sweeper` US-02's own KPI shape |

### DoR Status: PASSED

---

## Wave: DISCUSS / [REF] Handoff Package

No escalations. This is the only one of the 4 sibling `backend_mode=agent` features with zero open design questions requiring DESIGN-wave judgment — the extension point, retention pattern, and safety constraints are all fully determined by existing precedent (`AgentTransactionSweeper` itself, `customer-db-transaction-sweeper` US-02).

**Optional, non-blocking note for DESIGN**: whether to add a Prometheus counter mirrors `customer-db-transaction-sweeper`'s own observability precedent, contingent on whether `embyr-agent` exposes (or should expose) its own metrics endpoint at all — out of this feature's own confirmed scope, DESIGN's call whether to fold in.

### Handoff Confirmation

Next step (NOT performed by this agent): orchestrator dispatches `nw-solution-architect` for the DESIGN wave — lighter-weight than the other 3 sibling features given zero escalations; primary DESIGN task is confirming the exact SQL statement and its interaction with the existing reclaim query in the same sweep cycle.
