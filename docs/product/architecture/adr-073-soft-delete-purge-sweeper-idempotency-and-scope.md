# ADR-073: Soft-Delete Purge Sweeper — Column-Null Idempotency Guard and SystemDb-Only Scope

## Status

Accepted

## Context

`docs/product/production-readiness-audit-2026-09-08.md` finding #6 (Blocker): the documented 168-hour
soft-delete purge sweeper does not exist. `delete_project` (`crates/embyr-server/src/admin/handlers/
lifecycle.rs:165-196`) sets `projects.status = 'deleted'` and `projects.deleted_at = now()`, but
`deleted_at` is write-only — nothing in the codebase ever reads it again (confirmed by repo-wide grep,
`docs/feature/soft-delete-purge-sweeper/feature-delta.md` § Reading Confirmation). A soft-deleted
project's ECIES/AES-GCM-encrypted credentials — `ecies_encrypted_dsn`, `backend_pg_dsn_enc`,
`agent_tls_bundle_enc`, all `BYTEA` columns on the `projects` row itself (`migrations/0001_initial_
schema.sql:8`, `0015_projects_admin_columns.sql:4`, `0005_agent_endpoint.sql:5`) — persist unchanged
forever, contradicting the admin UI's own unmodified "168h grace window before data purge" promise
(`crates/embyr-admin-ui/src/views/db_detail/mod.rs:280-283`).

DISCUSS locked the outcome (the three columns are genuinely nulled once, and only once, 168h/7 days
after `deleted_at`) and one mechanism-level finding directly answered by an FK investigation (a hard
`DELETE FROM projects` is unsafe — `access_rule_history`'s append-only invariant has no `ON DELETE
CASCADE`, `sdk_api_keys` rows are deliberately preserved in revoked form). DISCUSS also identified,
directly from reading both existing background sweepers, that this feature's own target data lives
entirely in the system DB — a structural fact this ADR's Decision 2 turns on.

Two decisions in this feature had genuine rejected alternatives worth recording. The remaining design
choices (config variable naming, interval default, advisory-lock key string, metrics naming) follow
established codebase precedent directly, with no competing candidate to reject, and are recorded as
consequences here rather than separately debated.

## Decision

### Decision 1 — Idempotency guard: the nulled columns are their own "already purged" marker

The purge `UPDATE`'s own `WHERE` clause includes `AND (ecies_encrypted_dsn IS NOT NULL OR backend_pg_
dsn_enc IS NOT NULL OR agent_tls_bundle_enc IS NOT NULL)`. An already-purged row (all three columns
already `NULL`) fails this predicate on every subsequent cycle — `rows_affected() == 0`, no error, no
redundant write, no redundant metric increment. No new column, no new migration.

### Decision 2 — SystemDb-only scope, mirroring `CapUsageRefresher` not `TransactionSweeper`

`crates/embyr-server/src/sweepers/` has two existing background tasks sharing an identical interval-
loop + `pg_try_advisory_lock`/`pg_advisory_unlock` shape (`advisory_lock_key`, `sweepers/mod.rs:18-27`,
reused unchanged). They differ structurally: `CapUsageRefresher` (ADR-020) operates entirely against
`SystemDb` — a single query per cycle, zero customer-database connection. `TransactionSweeper` (ADR-054)
additionally loops over every PG-reachable project and opens a separate connection to each project's OWN
customer database, because its target data (the `transactions` table) lives there, not in the system DB.

This feature's target data — the three sensitive columns above — lives directly on the `projects` row in
the system DB (confirmed by direct migration read). The sweeper is therefore built as a single `SystemDb`-
only `UPDATE` per cycle: no per-project enumeration, no DSN resolution, no `AwsSecretFetcher`/
`GcpSecretFetcher` dependency, no `backend_mode` filter (unlike `TransactionSweeper`'s own necessary
`backend_mode IN ('direct_pg', 'aws_secret', 'gcp_secret')` filter, needed only because those three modes
require a resolvable customer DSN — this feature has no such requirement and must NOT exclude `agent`
mode, since `agent_tls_bundle_enc` is exactly the credential class `agent`-mode projects hold).

## Consequences

### Positive

- Zero schema migration (all three target columns and `deleted_at` already exist).
- Zero new external dependency; `chrono`, `sqlx`, `metrics`, `tracing` are already workspace dependencies
  used identically by the two existing sweepers.
- Smallest possible blast radius: one new file (`sweepers/soft_delete_purge_sweeper.rs`), three files
  gaining a small, precedented addition (`sweepers/mod.rs`, `config.rs`, `main.rs`). Zero change to
  `embyr-core`, `embyr-pg-storage`, `embyr-admin-ui`, or any admin HTTP route.
- The `WHERE`-clause idempotency guard is proven correct by the same construction both existing sweepers
  already rely on (`set_project_status`'s `WHERE status IN (...)`; `TransactionSweeper`'s reclaim/purge
  `WHERE` clauses) — a well-understood, already-battle-tested idiom in this codebase, not a novel one.
- A skipped cycle (advisory lock held elsewhere, or zero eligible rows) is provably harmless — the next
  tick picks up any genuinely-eligible row unchanged, mirroring both existing sweepers' own "no error on
  a benign skip" discipline.

### Negative / accepted residuals

- The `agent_tls_bundle_enc`/`backend_pg_dsn_enc`/`ecies_encrypted_dsn` purge is column-level, not
  row-level — the `projects` row, its `id`/`status`/`deleted_at`/`account_id`/timestamps, and every
  FK-referencing child row (`sdk_api_keys`, `access_rule_history` and siblings, `daily_project_metrics`)
  persist indefinitely. This is the DISCUSS-locked outcome (a hard row delete is unsafe — see Context),
  not a gap this ADR introduces; a soft-deleted project's non-credential metadata remains visible to any
  future direct-DB inspection forever. Accepted: the finding this feature closes is specifically about
  encrypted-credential persistence, not row persistence.
- No `restore`/`undelete` mechanism exists or is added — the grace window today protects nothing except
  "don't purge before 168h," a fact named explicitly in DISCUSS's own Out of Scope and unchanged by this
  ADR.

## Alternatives Considered

**For Decision 1 (idempotency guard):**
1. **New `purged_at TIMESTAMPTZ` marker column** — rejected. Requires a migration this feature's own
   scope explicitly excludes (DISCUSS confirmed all target columns already exist and no new one is
   needed), to record a fact the three existing nullable columns already encode for free.
2. **Column-null state as the marker (selected)** — a `WHERE col IS NOT NULL` guard on the same `UPDATE`
   makes the statement naturally idempotent and self-limiting with zero additional state.

**For Decision 2 (scope):**
1. **Reuse `TransactionSweeper`'s per-project customer-DB connection loop shape** — rejected. This
   feature's target data never lives in a customer database; adopting that shape would add DSN
   resolution, per-project connection overhead, and an unnecessary `backend_mode` filter that would
   incorrectly exclude `agent`-mode projects from having their `agent_tls_bundle_enc` purged — a direct
   regression against this feature's own scope.
2. **A single `SystemDb`-only `UPDATE` per cycle, mirroring `CapUsageRefresher` (selected)** — the
   evidently-simplest mechanism structurally correct for where this feature's own data actually lives.

## Enforcement

No new automated enforcement mechanism beyond the regression tests named in the feature-delta.md DESIGN
handoff (`docs/feature/soft-delete-purge-sweeper/feature-delta.md` § Regression Guards Carried Forward) —
this is a single-file, new-module addition reusing an existing, already-enforced sweeper shape
(`advisory_lock_key`, already covered by `sweepers/mod.rs`'s own unit tests) with no new port/adapter
boundary and no new external dependency.
