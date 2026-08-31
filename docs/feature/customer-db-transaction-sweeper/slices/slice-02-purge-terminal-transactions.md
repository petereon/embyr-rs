# Slice 02 — Terminal-State Transaction Rows Are Purged

**Story**: US-02 | **Release**: 2 | **Estimate**: 1 day | **job_id**: JOB-12 (observability, extends)

## Goal

The same sweep cycle additionally hard-deletes `'committed'`/`'expired'` `transactions` rows older than a retention window — the piece that actually bounds table growth (Slice 01 alone only relabels rows; it never deletes). Sam Chen can observe purge activity via `/metrics`.

## IN scope

- Second raw SQL statement, same cycle, same customer DB, run after Slice 01's own reclaim step: `DELETE FROM transactions WHERE status IN ('committed', 'expired') AND started_at < now() - <retention_window>`.
- Reuse of Slice 01's own DSN-resolution dispatch unchanged — no new mode-specific branching.
- Increment `embyr_transaction_sweeper_purged_total` (counter registered in Slice 01, incremented here).

## OUT scope

- Any change to the reclaim behavior itself (Slice 01, unchanged).
- Exact retention-window numeric default (DESIGN/DEVOPS call, mirrors `SessionCleaner`'s own 30-day precedent as a reference point, not a committed value).
- `backend_mode=agent` (same exclusion as Slice 01 — inherited, not re-decided).

## Learning Hypothesis

Disproves: "hard-deleting terminal-state rows introduces a race against a row that is about to be legitimately re-read (e.g., an operator debugging a recent transaction) within the retention window."

## Acceptance Criteria

- [ ] A `'committed'` row older than the retention window is hard-deleted by the next sweep cycle
- [ ] An `'expired'` row older than the retention window is hard-deleted by the next sweep cycle
- [ ] A terminal-state row within the retention window is never deleted
- [ ] Purge behavior is identical across `direct_pg`, `aws_secret`, `gcp_secret` (no mode-specific branching)
- [ ] `embyr_transaction_sweeper_purged_total` increments once per row deleted, observable via `GET :9090/metrics`

## Dependencies

Depends on Slice 01 shipping first — reuses its DSN-resolution dispatch and raw customer-DB SQL access path unchanged.

## Reference Class

`docs/product/architecture/brief.md`'s own documented `SessionCleaner` design ("Hard delete — no soft-delete needed; sessions contain no user-generated content") — the direct precedent for hard-deleting a purely-operational bookkeeping row past a retention window in this codebase.

## Pre-Slice SPIKE

Not required — same rationale as Slice 01; the mechanism (DSN resolution) is proven by Slice 01 itself before this slice starts.
