# Slice 01 — Orphaned Transaction Rows Are Reclaimed (Walking Skeleton)

**Story**: US-01 | **Release**: 1 | **Estimate**: 1.5 days | **job_id**: JOB-12 (observability, extends)

## Goal

A recurring background sweep proactively marks abandoned `'active'` `transactions` rows `'expired'`, across every reachable customer database, without requiring any later call to reference the same `transaction_id` — closing the `BatchWrite`/ADR-048 gap and the general any-abandoned-client gap. Sam Chen can observe reclaim activity via `/metrics`.

## IN scope

- New `TransactionSweeper` module (`crates/embyr-server/src/sweepers/transaction_sweeper.rs`), interval loop + `pg_try_advisory_lock`/`pg_advisory_unlock`, shape copied from `CapUsageRefresher`.
- New `SystemDb` query enumerating `projects` rows with `backend_mode IN ('direct_pg', 'aws_secret', 'gcp_secret')`.
- DSN resolution without any live api_key: `AwsSecretFetcher`/`GcpSecretFetcher::get_dsn` (aws_secret/gcp_secret, unchanged) and `backend_pg_dsn_enc` + `decrypt_with_rotation` under `EMBYR_ENCRYPTION_KEY` (direct_pg — first live read call site for this column).
- Silent skip (not error) of any `direct_pg` project with `backend_pg_dsn_enc IS NULL`.
- Raw SQL against each reachable customer DB: `UPDATE transactions SET status = 'expired' WHERE status = 'active' AND started_at < now() - <abandonment_threshold>`.
- New raw customer-DB SQL access path (§ Handoff Package Escalation 1 in feature-delta.md — DESIGN chooses the exact mechanism).
- Two new Prometheus counters on the already-installed recorder: `embyr_transaction_sweeper_reclaimed_total` (this slice), `embyr_transaction_sweeper_purged_total` (registered here, incremented in Slice 02).

## OUT scope

- Purge/hard-delete of terminal-state rows (Slice 02).
- `backend_mode=agent` (excluded at the enumeration query level — never attempted).
- Backfilling `backend_pg_dsn_enc` for pre-existing projects that have never PATCHed their DSN.
- Exact abandonment-threshold/interval numeric tuning (DESIGN/DEVOPS call, mirrors OQ-CP-3 precedent).

## Learning Hypothesis

Disproves: "reaching a customer database's `transactions` table without holding a live api_key is not actually possible today for any backend_mode using only already-shipped adapters."

## Acceptance Criteria

- [ ] A `'active'` row older than the abandonment threshold, in a `direct_pg` project with `backend_pg_dsn_enc` populated, is marked `'expired'` by the next sweep cycle
- [ ] A `'active'` row older than the abandonment threshold, in an `aws_secret` or `gcp_secret` project, is marked `'expired'` by the next sweep cycle, with zero live api_key involved
- [ ] A `'active'` row younger than the abandonment threshold is never modified
- [ ] A `direct_pg` project with `backend_pg_dsn_enc IS NULL` is skipped without aborting the cycle for other projects
- [ ] `backend_mode=agent` projects never appear in the sweep's own enumeration query
- [ ] Concurrent sweep attempts across multiple server instances are serialized by a Postgres advisory lock
- [ ] `embyr_transaction_sweeper_reclaimed_total` increments once per row transitioned to `'expired'`, observable via `GET :9090/metrics`

## Dependencies

None outstanding — all reused components (`CapUsageRefresher` shape, `AwsSecretFetcher`, `GcpSecretFetcher`, `backend_pg_dsn_enc`, `decrypt_with_rotation`, Prometheus recorder) already ship in this codebase today.

## Reference Class

`crates/embyr-server/src/sweepers/cap_usage_refresher.rs` — structural precedent for the sweeper shape itself (interval, advisory lock, per-row continue-on-error).

## Pre-Slice SPIKE

Not required — the central feasibility question (can the sweeper resolve a DSN without an api_key) was already resolved by direct code reading during DISCUSS (feature-delta.md § Reading Confirmation), not left as an open uncertainty for a SPIKE to answer.
