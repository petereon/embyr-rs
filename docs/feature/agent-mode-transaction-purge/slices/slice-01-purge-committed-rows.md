# Slice 01: Purge Committed Transaction Rows in Agent-Mode Deployments (Walking Skeleton)

**Story**: US-01 | **Release**: 1 | **Estimate**: 0.5-1 day

## Goal

Extend the already-running `AgentTransactionSweeper` with a retention-window purge step for `'committed'` rows, mirroring `customer-db-transaction-sweeper` US-02's own pattern.

## IN Scope

- Extend `AgentTransactionSweeper::sweep_once` (or add a sibling method in the same spawn loop) with `DELETE FROM transactions WHERE status = 'committed' AND started_at < NOW() - retention_window`
- Confirm no interference with the existing active-row reclaim step or a concurrently-committing transaction

## OUT Scope

- Any proto or cross-binary wire change (none needed — this feature is entirely agent-local)
- Prometheus counter (optional, non-blocking, DESIGN's call)

## Learning Hypothesis

Disproves: extending the already-running sweeper with a second delete statement introduces a race against a `'committed'` row that transitions from a concurrent in-flight `commit()` call during the same sweep cycle.
Confirms (if it succeeds): the extension is a safe, additive change to an already-proven background-job shape.

## Acceptance Criteria

- [ ] A `'committed'` row older than the retention window is hard-deleted by the next sweep cycle
- [ ] A `'committed'` row within the retention window is never deleted
- [ ] A transaction committing concurrently with a running sweep cycle is not disturbed
- [ ] The existing active-row reclaim behavior is unaffected

## Dependencies

None (first and only slice). Extends already-shipped `AgentTransactionSweeper`.

## Effort Estimate

0.5-1 day.

## Reference Class

`customer-db-transaction-sweeper` US-02 (non-agent modes) — identical purge pattern, agent-local context (no DSN resolution needed).
