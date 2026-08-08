# Slice DRL-05 — Provisioning Integration

**Feature:** distributed-rate-limiting
**Slice:** DRL-05 of DRL-05
**Estimate:** 0.5 days
**Stories:** US-DRL-04 — Provisioning integration — new projects start with a rate bucket
**Depends on:** DRL-01 (migration `0018_rate_buckets.sql` + FK must exist before INSERT can succeed)

---

## Goal

Update `provision_project` (called by `POST /admin/v1/projects`) to insert a `rate_buckets` row with `tokens = capacity` inside the same transaction as the `projects` row. Verify FK CASCADE removes the row on hard-delete. No cold-start burst window.

## Learning Hypothesis

Disproves: "The rate_buckets INSERT inside the provision transaction creates a lock ordering issue or deadlock with concurrent provision calls."
Confirms if succeeds: Concurrent `POST /admin/v1/projects` calls for different projects complete without deadlock; each produces exactly one `rate_buckets` row with `tokens = capacity`; hard-delete cascades cleanly.

## IN Scope

- Locate the `provision_project` code path (admin handler or `SystemDb` method called by admin handler)
- Inside the transaction block that inserts the `projects` row, add:
  ```sql
  INSERT INTO rate_buckets (project_id, tokens, last_refill)
  VALUES ($1, $2, now())
  ```
  where `$1` = `project_id`, `$2` = current `capacity` value (read from `EMBYR_RATE_LIMIT_RPS` at startup, passed in as parameter or read from config)
- Verify the transaction is already open at this point (the `projects` INSERT must be in the same transaction)
- If `provision_project` currently uses auto-commit, wrap both INSERTs in an explicit `BEGIN … COMMIT` block
- No new `provision_project` function signature change that would break existing admin API tests

## OUT Scope

- Per-project rate limits (D4 explicitly: operator-wide only)
- Soft-delete hook (soft-delete sets `deleted_at`; CASCADE fires on hard-delete by sweeper — no code change needed for soft-delete)
- Backfilling `rate_buckets` rows for existing projects (separate ops task, not part of this feature — handled by running a one-time INSERT SELECT in the migration or as an ops runbook)

## Acceptance Criteria

- AC-DRL-04: `POST /admin/v1/projects` results in a `rate_buckets` row with `tokens = capacity` and `last_refill` within 1s of creation time
- AC-DRL-04: if the `rate_buckets INSERT` fails, the transaction rolls back and no `projects` row exists for that `project_id`
- AC-DRL-04: hard-deleting the `projects` row (as the sweeper does) cascades to `rate_buckets` — verified by direct SQL assertion
- AC-DRL-04: newly provisioned project's first gRPC request is handled by the distributed UPDATE path (not lazy in-process bucket creation)
- New acceptance test `us_drl_04_provisioning.rs`: provision project via admin API + system DB; verify `rate_buckets` row; hard-delete; verify cascade

## Dependencies

- DRL-01: migration `0018_rate_buckets.sql` applied — FK `rate_buckets.project_id → projects.id ON DELETE CASCADE` is active
- DRL-02: distributed `check()` path exists — first gRPC call must hit the `UPDATE rate_buckets` path, not the old `HashMap` lazy-creation path
- Existing admin API test infrastructure (`provision_project` function in `us_14_rate_limiting.rs`) must still work — it bypasses the admin HTTP endpoint and calls the system DB directly; it must also insert a `rate_buckets` row after this slice lands (or the test helper is updated to call the updated `provision_project`)

## Effort Estimate

0.5 days. Adding one SQL INSERT inside an existing transaction is ~10 LOC. The main effort is (1) verifying the transaction boundary in `provision_project`; (2) passing `capacity` value through the call stack; (3) updating the test helper `provision_project` in `us_14_rate_limiting.rs` to also insert the `rate_buckets` row so existing tests do not break.

## Backfill Note (ops, not in-scope for this slice)

Existing projects in production will not have `rate_buckets` rows after the migration. On first gRPC request to such projects, the distributed `UPDATE rate_buckets … WHERE project_id = $1 AND tokens >= 1` will return 0 rows and fall back to the per-instance bucket. This is correct and safe — the per-instance bucket enforces 1× limit. A one-time `INSERT INTO rate_buckets SELECT id, $capacity, now() FROM projects WHERE id NOT IN (SELECT project_id FROM rate_buckets)` ops runbook should be documented in the DESIGN wave handoff.
