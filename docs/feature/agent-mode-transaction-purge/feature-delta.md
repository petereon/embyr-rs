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

---

## Wave: DESIGN / [REF] Prior Wave Consultation — Reading Confirmation

✓ `crates/embyr-agent/src/sweeper.rs` (full, 53 lines) — re-confirmed as read by DISCUSS: `AgentTransactionSweeper::sweep_once` runs one `DELETE FROM transactions WHERE started_at < NOW() - $1 * INTERVAL '1 second' AND status = 'active'`, bound to `ttl_secs: i64`. `spawn()` loops on `interval`, logs and continues on error. `new(pool, ttl_secs, interval)` — three params today.
✓ `crates/embyr-agent/src/server.rs::run` (lines 809-838, targeted) — confirmed spawn wiring: `AgentTransactionSweeper::new(pool, 60, Duration::from_secs(30))` at lines 830-834, right after `StorageAgentService::new(config.project_id, storage, bridge)` moves `config.project_id` out of `config` at line 826. `config.listen_addr` is still read (by reference) at line 837, confirming partial-move-then-later-field-read is already the established pattern in this function — the new `config.transaction_retention_days` read at the sweeper-construction site is not a new risk.
✓ `crates/embyr-agent/src/config.rs` (full, 97 lines) — confirmed `AgentConfig` exists with an `EMBYR_AGENT_*`-prefixed env var convention, `parse_optional_u32`/`parse_optional_u64` helpers already established (`EMBYR_AGENT_MAX_CONNS`, `EMBYR_AGENT_SHUTDOWN_TIMEOUT_SECS`). No retention-window-equivalent var exists yet. This is the extension point for the new config value — a new `parse_optional_i64` helper is needed (existing helpers are `u32`/`u64` only).
✓ `docs/product/architecture/adr-054-transaction-sweeper-raw-access-and-sweep-sql.md` (full) — direct precedent for the purge SQL shape (`DELETE ... WHERE status IN (...) AND started_at < $cutoff`, retention window as a runtime-bound parameter, reclaim/purge kept as two separate statements per cycle, not merged). Its own D2 explicitly derives the `'expired'` status value from `commit_transaction`'s own reactive-expiry branch — see the finding below, which extends that same derivation one step further than ADR-054 itself did.
✓ `crates/embyr-pg-storage/src/backend_adapter.rs` (targeted, lines 900-1260) — **independently re-verified rather than trusted from DISCUSS's own citation**. `PostgresBackendAdapter` (shared verbatim between `direct_pg` and `embyr-agent`) writes FOUR distinct `transactions.status` values, not two: `'active'` (insert default, line ~906), `'expired'` (`commit_transaction` reactive 60s-window check, line 944 — fires when a client calls `Commit` after the TTL has elapsed but before the sweeper's own next tick reclaims the row), `'committed'` (`commit_transaction` success path, line 1230), `'rolled_back'` (`rollback_transaction`, lines 1253-1254). **Correction to DISCUSS's own System Constraints** ("Purge targets the existing `'committed'` value only"): `'expired'` and `'rolled_back'` are also existing, reachable, currently-never-purged status values in agent-mode's own `transactions` table — DISCUSS's own Reading Confirmation cited `commit_transaction`'s success path only, not its reactive-expiry branch or `rollback_transaction`. Full reasoning: ADR-058.
✓ `tests/acceptance/embyr_agent/us_a04_transactions.rs` (targeted, lines 290-310) — confirmed the only other call site of `AgentTransactionSweeper::new`/`sweep_once`. It constructs its own sweeper instance directly and only asserts `sweep_once().await.expect(...)` does not error — never inspects the numeric return value. Confirms a combined reclaim+purge row count return is safe (no existing assertion depends on the value's composition), and confirms the constructor signature change (new `retention_days` param) has exactly one other call site to update.
✓ `crates/embyr-agent/Cargo.toml` (full) — confirmed `chrono` is already a workspace dependency of `embyr-agent` (both `[dependencies]` and `[dev-dependencies]`), and no metrics/prometheus crate is present — relevant to two DESIGN decisions (SQL-side vs. Rust-side date arithmetic; deferring the optional Prometheus counter). See ADR-058.

No contradictions with DISCUSS's own scope decisions beyond the status-vocabulary correction above, which is a data-fact correction (verified against shared code), not a re-litigation of any DISCUSS judgment call.

---

## Wave: DESIGN / [WHY] Escalations — None, One Correction

DISCUSS's own Handoff Package states zero open design questions. This DESIGN wave confirms that holds for every judgment call (extension point, SQL shape, cycle interaction, config mechanism) **except one correction, not an escalation**: the purge SQL's status vocabulary must cover `'committed'`, `'expired'`, and `'rolled_back'` — not `'committed'` alone — because the latter two are reachable via the identical shared `PostgresBackendAdapter` code DISCUSS itself cited for the `'committed'` path, and leaving them un-purged would not meet the feature's own stated Outcome KPI (bounded `transactions` table growth). This is resolved directly (§ ADR-058), not escalated to the user, because it is a verifiable fact about existing code, not a trade-off requiring a decision among options.

---

## Wave: DESIGN / [WHY] Reuse Analysis

| Component | Reuse or New | Rationale |
|---|---|---|
| `AgentTransactionSweeper` (`crates/embyr-agent/src/sweeper.rs`) | **EXTEND** | Add `retention_days: i64` field + constructor param; add a second `DELETE` statement to `sweep_once`. No new struct, no new module. |
| `AgentConfig` (`crates/embyr-agent/src/config.rs`) | **EXTEND** | Add `transaction_retention_days: i64` field, read from a new env var; add one new `parse_optional_i64` helper alongside the existing `u32`/`u64` ones (same pattern, new numeric width). |
| `server::run` (`crates/embyr-agent/src/server.rs`) | **EXTEND** | Update the one existing `AgentTransactionSweeper::new(...)` call site to pass `config.transaction_retention_days`. |
| `chrono` dependency | **REUSE, unchanged** | Already present in `embyr-agent`'s `Cargo.toml`; not newly added, and not used by this change either (SQL-side arithmetic chosen instead — § ADR-058 D1). |
| Prometheus counter | **NOT BUILT (deferred)** | `embyr-agent` exposes no metrics endpoint today (verified: no metrics crate dependency, no `/metrics` route). Out of this feature's own confirmed scope per DISCUSS's own Handoff Package note; named as a follow-up in ADR-058, not built speculatively. |
| Retention-window purge PATTERN (`DELETE ... WHERE status IN (...) AND started_at < cutoff`) | **REUSE (pattern, not code)** | Mirrors `customer-db-transaction-sweeper` US-02 / ADR-054 D2 one level down — agent-local pool, no DSN resolution, no cross-project enumeration. |

Verdict: **entirely EXTEND**, as DISCUSS's own Scope Assessment predicted ("no existing alternative" bar trivially met — the only existing sweeper for this table in this crate is `AgentTransactionSweeper` itself). Zero new files, zero new component.

---

## Wave: DESIGN / [WHY] Architecture Decision — Purge SQL, Status Vocabulary, and Retention Config

Full decision, verification, and alternatives analysis: `docs/product/architecture/adr-058-agent-transaction-sweeper-purge-sql-and-retention-config.md`. Summary:

**Purge SQL** (second statement inside `sweep_once`, kept separate from the existing reclaim `DELETE` per ADR-054's own two-statement precedent):

```sql
DELETE FROM transactions
WHERE status IN ('committed', 'expired', 'rolled_back')
  AND started_at < NOW() - $1 * INTERVAL '1 day'
```

`$1` binds `retention_days: i64`. SQL-side interval arithmetic (matching the existing reclaim query's own idiom in this file), not Rust-side `chrono` computation (ADR-054's own sibling-crate style, evaluated and not carried over — see ADR-058 D1 for why).

**Status vocabulary**: `'committed'`, `'expired'`, `'rolled_back'` — corrected up from DISCUSS's own `'committed'`-only framing after direct re-verification of `PostgresBackendAdapter` (shared with `direct_pg`) showed all three are reachable, existing, currently-unpurged terminal states in agent-mode. `'active'` remains exclusively owned by the unchanged reclaim step.

**Retention config**: new `retention_days: i64` parameter on `AgentTransactionSweeper::new` (breaking constructor change, two in-repo call sites to update: `server.rs`, `tests/acceptance/embyr_agent/us_a04_transactions.rs`), sourced from a new `AgentConfig.transaction_retention_days: i64` field, env var `EMBYR_AGENT_TRANSACTION_RETENTION_DAYS` (optional, default `30`) — `EMBYR_AGENT_*` prefix to match this file's own 100%-consistent existing convention (deliberately not reusing the non-agent sibling's bare `EMBYR_TRANSACTION_RETENTION_DAYS` name — different crate, different config namespace, zero operational benefit to matching it literally). Default value `30` mirrors the sibling's default for cross-backend-mode behavioral parity.

**`sweep_once` return value**: signature unchanged (`Result<u64, sqlx::Error>`); now returns `reclaimed_rows + purged_rows` summed. Verified safe against both existing call sites (neither inspects the numeric value).

**Cycle/race safety** (AC 3, "a transaction committing concurrently with a running sweep cycle is not disturbed"): satisfied structurally, not by new guard code — the purge `WHERE` clause matches on `started_at < NOW() - retention_days * INTERVAL '1 day'`, and any row transitioning to a terminal status during the current cycle has a `started_at` at most tens of seconds old (bounded by the 60s TTL/commit window), five-plus orders of magnitude inside the retention window (default 30 days). No explicit lock or transaction isolation is needed for this property — same reasoning ADR-054 D2 already established for the non-agent sibling's identical concurrent-commit boundary case.

**Component list for the crafter**:
1. `crates/embyr-agent/src/sweeper.rs` — extend `AgentTransactionSweeper` (new field, new constructor param, second `DELETE` in `sweep_once`).
2. `crates/embyr-agent/src/config.rs` — extend `AgentConfig` (new field, new env var, new `parse_optional_i64` helper).
3. `crates/embyr-agent/src/server.rs` — update the one `AgentTransactionSweeper::new(...)` call site.
4. `tests/acceptance/embyr_agent/us_a04_transactions.rs` — update the one other `AgentTransactionSweeper::new(...)` call site (compile-fix; DISTILL/DELIVER owns whether/how to extend this file's own assertions or add a new acceptance test file for this feature's own AC).

No proto change, no new crate, no new file beyond the two docs this DESIGN wave adds (this ADR, this section).

**External integrations**: none. Agent-local Postgres only, already-probed (`crates/embyr-agent/src/probe.rs`, unchanged).

**Peer review**: not performed — session standing methodology for this feature (per orchestrator instruction) skips the `solution-architect-reviewer` sub-agent dispatch for these 4 sibling features; the orchestrator verifies DESIGN output directly against the code.

**Next**: orchestrator dispatches `nw-software-crafter` directly for delivery (single slice, no DISTILL, no roadmap.json, no execution-log.json, per session standing methodology).
