# ttl-policies — Firestore-style TTL deletion (go-live finding P5)

Status: implemented. Single-pass feature (no multi-wave ceremony).

## Behaviour

- Policy = `(project, collection group, top-level Timestamp field)`, one per collection group
  (`ttl_policies`, migration 0050, `ON DELETE CASCADE` from `projects`; also deleted by
  `SoftDeletePurgeSweeper` once a soft-deleted project's grace window has elapsed).
- Eligible: field holds a `Timestamp` strictly before now. Missing / null / any other type: never deleted.
- Deletes are real deletes through `begin_transaction` + `commit_transaction`, so the
  existing per-path `pg_notify` fires and live Listen streams see the removal. Subcollections of a deleted
  document are not touched. Security rules are bypassed (as in Firestore).
- Each delete carries an `update_time` precondition: a document rewritten between the sweeper's read and
  its delete (TTL pushed out) is kept. A failed batch falls back to one-by-one; changed documents are
  skipped and re-evaluated next cycle.

## Control plane

`GET`/`PUT /admin/v1/projects/:project_id/ttl_policies`, `DELETE .../ttl_policies/:collection_group`
(`handlers/ttl_policies.rs`). Same auth as `indexes` (session cookie or admin API key; operator bearer is not
accepted by these neighbours either). Admin+ changes, any role lists, foreign project is 404, invalid input is 422.
`field_path` goes through `validate_field_path` (the SQL-interpolation guard of audit #11/#26) and must be
top-level; `collection_group` must be a single collection id. Pure rules live in
`embyr-core/src/domain/ttl.rs`.

## Sweeper

`sweepers/ttl_sweeper.rs`. Env: `EMBYR_TTL_SWEEP_INTERVAL_SECS` (default 3600),
`EMBYR_TTL_MAX_DELETES_PER_RUN` (default 10000 per policy per sweep); both validated positive integers in
`config.rs`, listed in `.env.example`. Batches of 500 via a typed collection-group `field < now` query.
Single-flight across replicas with advisory lock `embyr_ttl_sweep`; the connection holding the lock also
runs the policy query (one system-pool connection pinned per cycle) and the first tick is delayed 60 s, to
avoid the pool starvation that hit the hourly retention sweeper (PR #39). Skips suspended/deleted projects;
per-project errors and a 300 s per-project time budget never stop the sweep.

Metrics: `embyr_ttl_deleted_total`, `embyr_ttl_sweep_errors_total` (also
`embyr_sweeper_error_total{sweeper="ttl_sweeper"}`, already alerted on),
`embyr_ttl_skipped_projects_total{reason="agent_mode"}`, `embyr_ttl_last_run_timestamp_seconds`.
No new alert rule: neighbouring sweepers have none for "not running".

## Limits

- Deletion latency is up to one sweep interval plus backlog drain time; not Firestore's exact
  "within 24 h" machinery. Per-run cap bounds work per policy; the remainder waits for the next sweep.
- Top-level fields only (the query layer reads a dotted path as one literal key).
- `agent` mode unsupported: `AgentBackendAdapter::run_query` hardcodes `all_descendants: false` and drops
  `limit`. The API answers 409; the sweeper skips and counts.
- `direct_pg` projects with only a legacy ECIES DSN (`backend_pg_dsn_enc IS NULL`) are skipped with a warning
  (ADR-055 gap shared with `TransactionSweeper`).
- If the collection-group column backfill is incomplete (`collection_id IS NULL` rows), the adapter falls back to
  a `LIKE` predicate whose `_`/`%` act as wildcards; the sweeper filters results to the exact group, so it can
  only under-delete, never over-delete.
- Each committed batch writes one `transactions` row (reclaimed by `TransactionSweeper` retention).
- TTL deletions are not metered as customer operations.

## Tests

- Unit (local): `embyr-core` `domain::ttl` (eligibility, group match, batch limit, validation) and
  `ttl_sweeper` stub-adapter tests (batching, per-run cap, group guard, abort fallback).
- Docker acceptance (CI only; compile-checked locally): `tests/ttl_policies/acceptance/`
  `ttl01_policy_admin_api.rs` (roles, 404 foreign project, 422 invalid path/group, 409 agent, upsert/delete) and
  `ttl02_sweeper.rs` (eligibility matrix, disabled/suspended, subcollection survives, cap, project isolation,
  live Listen subscriber sees the removal).
