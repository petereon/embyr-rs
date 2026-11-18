# Migration Policy

Closes go-live assessment item O2 (`docs/product/gaps.md`): the
forward-only migrations in `migrations/` run on every replica start, and until now there was no
written rule about what a migration may do or how a bad release is rolled back.

## How migrations run today

- `migrations/*.sql` are System DB migrations, embedded into the `embyr-server` binary at
  compile time (`sqlx::migrate!("../../migrations")`) and applied by `SystemDb::migrate()` at
  startup (startup step 5, before any listener opens). A migration failure is a startup failure
  (exit 1).
- `migrations/customer/*.sql` are the customer-database schema, applied by provisioning /
  `embyr-db-prep` (see `runbook.md`), not by server start. The policy below applies to them
  identically.
- Migrations are **forward-only**: there are no `down` scripts and none will be written.
- sqlx takes a Postgres advisory lock while migrating, so several replicas starting at once are
  safe: one applies, the others wait and then see the migrations as already applied.
- Rolling deploys therefore always run for a while with **old code on the new schema** (old
  replicas still serving after a new replica has migrated) and, on rollback, indefinitely.

## The rule: N-1 compatibility via expand/contract

Release N's schema must work with release N-1's code, and release N's code must work with the
schema as N-1 left it. Achieve that by splitting every breaking change across releases:

| Phase | Release | Ships | Allowed in the migration |
|-------|---------|-------|--------------------------|
| Expand | N | new schema **plus** code that writes both old and new shape | `CREATE TABLE`, `ADD COLUMN` (nullable or with a constant `DEFAULT`), `CREATE INDEX`, new constraints that existing rows and old code satisfy |
| Migrate | N (or backfill job) | code reads the new shape; data backfilled in batches | `UPDATE` backfill, batched, outside a long single transaction |
| Contract | N+1 or later, only after N is the oldest image you would ever roll back to | code no longer touches the old shape | `DROP COLUMN`, `DROP TABLE`, `RENAME`, `SET NOT NULL`, `ALTER COLUMN ... TYPE` |

Concretely:

1. **Never drop or rename a table or column in the same release that changes the code using
   it.** Release N stops using it; release N+1 (or later) drops it.
2. **Never rename.** Add the new name, dual-write, backfill, switch reads, drop the old name
   two releases later. A rename is an instant outage for N-1 code.
3. **New columns must be nullable or have a constant default**, so N-1 code that does not know
   about them can still `INSERT`. `SET NOT NULL` only in the contract phase, after every writer
   (including N-1) populates the column.
4. **Never change a column type in place.** Add a new column of the new type and go through
   expand/contract. `ALTER COLUMN ... TYPE` rewrites the table under an `ACCESS EXCLUSIVE`
   lock and breaks N-1 queries that bind the old type.
5. **Don't add a new constraint or unique index that N-1 code can violate** unless the old code
   is already gone from the fleet.
6. **Large tables:** use `CREATE INDEX CONCURRENTLY` (in its own migration file starting with
   sqlx's `-- no-transaction` directive, as it cannot run inside the transaction sqlx wraps each
   file in), and backfill in batches. A
   migration holds up every replica's startup while it runs.
7. **Don't edit an applied migration file.** sqlx stores a checksum per version and refuses to
   start on a mismatch. Fix forward with a new file.
8. **Number files in order** (`NNNN_description.sql`, next free number). Two PRs picking the
   same number is a merge conflict to resolve by renumbering the one that has not shipped.

## Rollback

Rollback is **redeploying the previous immutable image** (`release-process.md` →
Rollback) against the schema as it now stands. Schema changes are not undone: with the rules
above, N-1 code is compatible with N's schema by construction.

If the schema itself is damaged (a bad backfill, a wrongly applied contract step), rollback of
the image does not help; restore from backup/PITR per `backup-disaster-recovery.md`, accepting
the data loss window.

### Older binary on a newer schema

Both migrators (`SystemDb::migrate()` in `crates/embyr-server/src/adapters/system_db.rs` and the
customer-schema `MIGRATOR` in `crates/embyr-pg-storage/src/backend_adapter.rs`) set sqlx's
`ignore_missing`, so an older binary started against a database that has migrations it does not
embed boots normally instead of failing with "migration N was previously applied but is missing
in the resolved migrations". Rollback across a release that added an expand/contract-compliant
migration is therefore a plain redeploy of the previous image. Checksum validation of the
migrations the old binary does know is unchanged, and customer-database provisioning still
compares the DB's highest applied version against the binary's own (a DB at or ahead of the
binary is `Ready`).

## CI lint

`scripts/check-migration-policy.sh` runs on every pull request (job `Migration policy`) and
fails if a migration file **added** by the PR contains any of:

`DROP TABLE`, `DROP COLUMN`, `RENAME`, `ALTER COLUMN ... TYPE`, `SET NOT NULL`

Details: existing migrations are exempt (only files added versus the PR base are checked);
`--` comments are ignored when matching; matching is per line and case-insensitive, so a
statement split across lines can slip through. The lint is a backstop, not a substitute for
review. To intentionally do a contract-phase step, add a line to the file with a reason:

```sql
-- migration-policy: allow contract step for column foo, unused since v0.9.0 (#123)
ALTER TABLE bar DROP COLUMN foo;
```

An `allow` line without a reason does not count. Reviewers: an `allow` line is the cue to check
that the previous release no longer touches what is being dropped. Run it locally with
`scripts/check-migration-policy.sh origin/master`.

## How to test a migration

Before merging a PR that adds a migration:

1. **Fresh database:** the CI test shards already apply all migrations to an empty Postgres;
   a migration that does not apply cleanly fails them.
2. **Upgrade path:** apply the PR's migration on top of a copy of the previous schema that
   contains realistic data (a staging dump, not an empty DB). Check duration and locks on
   anything touching a large table (`pg_stat_activity` during the run).
3. **N-1 compatibility:** with the PR's schema in place, run the previous release's test
   suite or smoke-test the previous image (`ghcr.io/<owner>/embyr-server:sha-<previous>`)
   against it: it must read and write without errors.
4. **Contract steps:** grep the previous released tag for the dropped/renamed identifier
   (`git grep -n <name> <tag> -- crates`); it must find nothing.
5. **Customer-schema migrations** (`migrations/customer/`): additionally verify against a
   customer database provisioned by the previous version, since those are not migrated on
   server start.

## Cross-references

- [`release-process.md`](./release-process.md): rollback procedure and the version-bump rule
  for PRs touching `migrations/`.
- [`backup-disaster-recovery.md`](./backup-disaster-recovery.md): restore/PITR when schema or
  data is damaged.
- [`runbook.md`](./runbook.md): customer-database provisioning.
