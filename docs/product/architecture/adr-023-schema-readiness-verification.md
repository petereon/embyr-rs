# ADR-023: Schema-Readiness Verification — sqlx Migration-Tracking Table + Role-Scoped SELECT Grant

## Status

Accepted. **Revised** after a targeted security review of this ADR (see § Alternatives Considered,
Alternative 4, and § Review Resolution) — the original `GRANT ... TO PUBLIC` mechanism is
superseded by a role-parameterized grant, discovered at runtime and scoped to exactly the DML role
being onboarded.

## Context

US-02 (`customer-db-onboarding`) requires `embyr-server` to detect, at `POST /admin/v1/projects`
(`backend_mode=direct_pg`) time, whether the submitted database is (a) fully prepped, (b) not
prepped at all, or (c) prepped to a stale schema version — and to do so **without requiring or
attempting any elevated (DDL) privilege on the submitted connection string** (AC-02-05). DISCUSS's
Handoff Package flags this explicitly: "confirm any chosen mechanism (e.g., `information_schema`
queries) is actually available to an ordinary DML-only role before committing to it."

The reference precedent named in DISCUSS's Pre-requisites is `SystemDb::probe()`
(`crates/embyr-server/src/adapters/system_db.rs`): a `SELECT 1` liveness check plus an
`information_schema.tables` existence check for the system DB's `projects` table, refusing to
proceed if the schema isn't initialized.

A second, harder constraint: `provision.rs`'s existing `direct_pg` branch must keep working
**unchanged** for customers who submit a full-privilege DSN against a not-yet-migrated database
(AC-02-06, no regression) — today's automatic `sqlx::migrate!` attempt must still fire in that
case. Any verification step must therefore *enrich* the existing failure path, not replace or gate
it, because read-only schema introspection alone cannot determine whether a given credential has
DDL rights — only attempting DDL can determine that, and that attempt already exists today.

## Decision

Add `PostgresBackendAdapter::verify_schema_readiness()` to `embyr-pg-storage`, returning a new pure
domain type in `embyr-core` (IO-free, per the project's `deny.toml` boundary):

```
enum SchemaReadiness {
    Ready { schema_version: i64 },
    NotPrepped { missing_tables: Vec<String> },
    Stale { expected_version: i64, found_version: i64 },
}
```

### Mechanism

Read sqlx's own migration-bookkeeping table, `_sqlx_migrations` — auto-created by any
`sqlx::migrate!().run()` call, including today's default provisioning path; no new table *type* is
introduced. The check is a plain `SELECT version, success FROM _sqlx_migrations ORDER BY version` —
pure DML, no schema-introspection privilege beyond ordinary table `SELECT`.

Compare the highest successfully-applied version found against the compiled-in `Migrator`'s own
highest embedded version (the identical single-sourced embed established by ADR-022):

- `_sqlx_migrations` absent or unreadable → `NotPrepped { missing_tables }` (naming the expected
  application tables — `documents`, `transactions` — as the missing element; the caller cannot
  cleanly distinguish "genuinely never prepped" from "prepped but tracking table unreadable"
  without deeper introspection, and both cases warrant the identical remediation: re-run the prep
  step).
- `0 < found_version < expected_version` → `Stale { expected_version, found_version }`.
- `found_version >= expected_version`, all rows `success = true` → `Ready { schema_version:
  found_version }`. Note: **not** strict equality — see Consequences.

### Making `_sqlx_migrations` readable by the DML-only role only (revised — see § Review Resolution)

**This mechanism was revised after a targeted security review of this ADR.** The original
DESIGN-wave proposal (`GRANT SELECT ON _sqlx_migrations TO PUBLIC`, as a static migration file) is
superseded by a role-parameterized grant, recorded in full under § Alternatives Considered,
Alternative 4.

Postgres's `GRANT ... TO <role>` requires the *granting* session (Elena's elevated role, the owner
of `_sqlx_migrations`) to name the target role as a literal identifier — there is no bind-parameter
form of `GRANT`, and `sqlx::migrate!`'s embedded `.sql` files are fixed, checksummed, unparameterized
text resolved at compile time. A role-scoped grant therefore **cannot** live in
`migrations/customer/` as static SQL — the target role's name is not known at migration-authoring
time, and sqlx migration files cannot template it in. **Migration
`0003_grant_schema_readiness_read.sql` is removed from this design entirely** — the grant becomes a
runtime step, performed only by `embyr-db-prep`, never a tracked migration. (This is also a
correctness improvement independent of the security fix: a privilege-administration statement was
never a good fit for the schema-*version* tracking migration set to begin with — removing it keeps
`expected_version` cleanly meaning "count of schema-shape migrations," unchanged at `2`.)

**Revised mechanism, in full:**

1. `DbPrepConfig::from_env()` gains a second, **optional** environment variable,
   `EMBYR_DB_PREP_DML_ROLE_DSN` — the connection string for the DML-only role Elena is about to
   hand her ops team (per US-01's own domain example, she already possesses this artifact around
   the same time she runs the prep step; the tool does not require her to type or transcribe the
   Postgres-internal role *name*, only to supply the connection string she already has).
2. If `EMBYR_DB_PREP_DML_ROLE_DSN` is absent, the prep tool applies migrations as today and skips
   the grant step, printing an explicit informational note (not an error — migration success is
   still reported via the existing, unchanged success message) that read-verification access was
   not established and `verify_schema_readiness()` will report `NotPrepped` until a grant is
   performed.
3. If present, after `PostgresBackendAdapter::migrate()` succeeds, the tool opens a brief,
   short-lived connection using `EMBYR_DB_PREP_DML_ROLE_DSN` and calls a new adapter method,
   `PostgresBackendAdapter::discover_current_user(&self) -> Result<String, CoreError>`, which runs
   `SELECT current_user` and immediately closes the connection. This is the *only* query ever run
   against that connection — mirrors `StartupProbe`'s existing "never log the DSN" convention
   (`crates/embyr-agent/src/probe.rs`): the DML-role DSN is read once, used once, never logged, and
   the resulting *role name* (not the DSN) is the only value carried forward.
4. The tool then calls a second new adapter method,
   `PostgresBackendAdapter::grant_schema_readiness_read(&self, role_name: &str) -> Result<(), CoreError>`,
   executed against the **elevated** connection (the only connection that structurally holds grant
   authority, since `_sqlx_migrations` is owned by Elena's elevated role by virtue of having created
   it). This method performs two round trips, deliberately avoiding hand-rolled Rust-side SQL
   identifier quoting in favor of Postgres's own trusted quoting function:
   ```sql
   -- round trip 1: server-side identifier quoting via a plain, safely-parameterized SELECT
   SELECT format('%I', $1::text);   -- bound: role_name — returns a safely double-quoted identifier

   -- round trip 2: the quoted identifier is interpolated (not bound — GRANT has no bind-parameter
   -- form) into a plain GRANT statement
   GRANT SELECT ON _sqlx_migrations TO <quoted-identifier-from-round-trip-1>;
   ```
   Using Postgres's own `format('%I', ...)` — rather than a Rust-side quoting routine — for the
   identifier-escaping step means a pathological or unusual role name (embedded quotes, reserved
   words, mixed case) cannot produce a SQL-injection-shaped `GRANT` statement; the escaping is
   delegated to the database engine's own trusted implementation of identifier quoting.
5. `GRANT` remains naturally idempotent — re-running `embyr-db-prep` (interrupted-run resume,
   AC-01-03, or a deliberate re-run) re-executes steps 3–4 harmlessly if the grant already exists.
6. `_sqlx_migrations` still contains only migration bookkeeping (version, description, checksum,
   timestamp) — no customer data — but is now readable by **exactly one** role: the DML role Elena
   names, not every role on the database.

### `provision.rs`'s `direct_pg` branch (extended, not replaced)

After the existing `probe_customer_db()` connectivity check:

1. Call `verify_schema_readiness()`.
2. **`Ready`** → skip the `sqlx::migrate!` call entirely — no DDL attempted against a DML-only
   credential (AC-02-01, AC-02-05) → proceed to the existing project `INSERT`.
3. **`NotPrepped` / `Stale`** → attempt the **existing, unchanged** `PostgresBackendAdapter::migrate()`
   call (ADR-022). This preserves today's default, full-privilege-DSN auto-migrate path exactly
   (AC-02-06 — no regression).
   - Migrate succeeds (the DSN did have DDL rights after all) → proceed normally, exactly as today.
   - Migrate fails → return the specific error body — `customer_db_not_prepped` (naming
     `verify_schema_readiness()`'s missing tables) or `customer_db_schema_stale` (naming expected
     and found versions) — instead of today's generic `backend_unavailable`.

`verify_schema_readiness()` never gates or blocks the existing auto-migrate path; it only enriches
the error message shown when that path fails, exactly as it already can today. The DDL-privilege
question ("is this DSN DDL-capable or not?") is still answered empirically by attempting the
migration — verification does not need to determine privilege level itself (which a read-only
check structurally cannot do), only schema *state*, which a read-only check can determine safely.

## Alternatives Considered

### Alternative 1: A dedicated marker table

Add a new final migration creating `_embyr_schema_marker(version INT, applied_at TIMESTAMPTZ)`
instead of reusing sqlx's own `_sqlx_migrations`.

**Rejected because:** this introduces a second, parallel bookkeeping mechanism that itself must
stay in sync with sqlx's own migration count — exactly the kind of drift-prone duplication ADR-022
exists to eliminate. Reusing sqlx's own tracking table means there is exactly one record of "what's
applied," maintained by sqlx itself, not by feature-specific code.

### Alternative 2: Classify the existing migrate-attempt failure only (no separate read-only check)

Attempt `sqlx::migrate!` first (as today) and rely solely on classifying its failure (via
`SQLSTATE`, e.g. `42501` insufficient_privilege) to distinguish not-prepped from stale, without a
separate read-only verify step.

**Rejected because:** SQLSTATE `42501` on a pending-migration DDL statement cannot, by itself,
distinguish "not prepped at all" from "prepped to a stale version" — both fail the same way
(insufficient privilege to create/alter the next pending object). AC-02-03 requires naming *both*
the expected and found schema version, which needs the read-only state comparison
`verify_schema_readiness()` provides; a failure-classification-only approach cannot produce that
detail.

### Alternative 3: `information_schema.tables` presence check only (no `_sqlx_migrations` involvement)

Check for `documents`/`transactions` table presence directly, matching `SystemDb::probe()`'s exact
mechanism, with no reference to sqlx's migration-tracking table at all.

**Considered strongly** — it is the literal precedent DISCUSS's Pre-requisites names — but
**rejected as the primary mechanism** because it degrades silently for any future migration that
`ALTER`s an existing table rather than adding a new one. The current 2-migration set happens to add
exactly one new table per migration, but that is an accident of the current schema, not a
guaranteed future property. `_sqlx_migrations`-based version comparison remains correct regardless
of what any individual migration's DDL contains. Table-presence checking is retained as the
*diagnostic detail* inside the `NotPrepped` variant's `missing_tables` field (naming `documents`/
`transactions` specifically when absent) but is not the versioning source of truth.

### Alternative 4: `GRANT SELECT ON _sqlx_migrations TO PUBLIC` (original DESIGN-wave proposal)

The DESIGN wave's original proposal for this ADR made `_sqlx_migrations` readable by granting
`SELECT` to `PUBLIC` (every role on the database), via a static migration file
(`migrations/customer/0003_grant_schema_readiness_read.sql`). Rationale at the time: `PUBLIC` grants
require no role-name discovery at all — the simplest mechanism that avoids a manual grant step, and
the table itself carries no customer data (only migration bookkeeping), so the exposure was judged
low-risk.

**Raised in a targeted security review of this ADR** (post-DESIGN-wave, pre-DISTILL): the review
approved the ADR overall but flagged this specific choice as a genuinely worthwhile, non-blocking
improvement. The review's point: this feature's entire framing is privilege separation for
enterprise/regulated customers operating under least-privilege governance. Shipping an avoidably
broad `PUBLIC` grant — when a role-scoped grant costs nothing extra in engineering effort and
achieves the identical zero-manual-typing property via runtime self-discovery — undercuts the
feature's own selling point the moment a customer's security team audits the resulting schema. A
customer who adopts this feature specifically *because* they don't trust broad grants would find a
`GRANT ... TO PUBLIC` in their own database produced by the tool that promised privilege separation.

**Resolution: superseded by the role-parameterized mechanism** documented in § Mechanism above.
`PUBLIC` is rejected in favor of granting `SELECT` on `_sqlx_migrations` to exactly the DML role
being onboarded, discovered at runtime via a brief secondary connection using that role's own
credential (`EMBYR_DB_PREP_DML_ROLE_DSN`, § Mechanism steps 1–4). This is not a cost-free change —
see § Consequences, Negative — but the review's judgment, endorsed here, is that the tightened
privilege model is worth the small added input surface for a feature whose value proposition is
privilege minimization itself. This resolution is recorded here explicitly so the `PUBLIC` grant is
not silently reintroduced by a future edit that rediscovers "but PUBLIC needs zero role-name
discovery" without also rediscovering why that trade-off was rejected.

## Consequences

### Positive

- Reuses sqlx's own bookkeeping — zero new tracking-table maintenance code.
- Matches `SystemDb::probe()`'s established hard-gate-verification shape. This check *is*
  `embyr-server`'s probe of an external substrate before acting on it (Earned Trust: "can I trust
  this database's claimed schema state before I provision against it") — the provisioning-time
  analogue of a startup probe, run on every `direct_pg` provisioning request rather than once at
  process start.
- Does not require the customer to manually type, transcribe, or look up the DML role's
  Postgres-internal name — it is self-discovered via `SELECT current_user` against a credential
  Elena already possesses, avoiding both a manual `GRANT` step *and* a transcription-error surface.
- The resulting grant is scoped to exactly one role, not every role on the database — directly
  reinforces this feature's own privilege-separation value proposition rather than working against
  it (see Alternative 4).
- Identifier interpolation into the dynamic `GRANT` statement is delegated to Postgres's own
  `format('%I', ...)`, not a hand-rolled Rust quoting routine — closes the SQL-injection-shaped risk
  a naively-interpolated role name would otherwise open.
- Generalizes cleanly to `aws_secret`/`gcp_secret` modes (deferred per DISCUSS's Out of Scope) with
  zero new adapter code — those branches already call `probe_customer_db()` +
  `PostgresBackendAdapter::migrate()` on a fetched DSN; adopting `verify_schema_readiness()` is the
  same call, same branch shape. The grant step remains `embyr-db-prep`-only regardless of backend
  mode, so this generalization is unaffected.

### Negative / Trade-offs

- `found_version >= expected_version` (not strict equality) is a deliberate choice to avoid
  blocking provisioning when a DBA has run a *newer* prep-tool build than the currently-deployed
  `embyr-server` expects (forward-compatible during rolling deploys). Consequence: a genuinely
  incompatible *future* schema change (e.g., a migration that removes or renames a column
  `embyr-server` still reads) would currently read as `Ready` rather than `Stale`, because this
  ADR's model only compares migration *counts*, not schema *compatibility*. Flagged as an open
  question in the feature-delta — not resolved here, and not a risk for the current additive-only
  migration set.
- The role-parameterized grant is **not** a free lunch relative to `PUBLIC`: it requires
  `embyr-db-prep`'s input contract to grow from one required env var (`EMBYR_DB_PREP_DSN`) to one
  required plus one *optional* env var (`EMBYR_DB_PREP_DML_ROLE_DSN`). If Elena runs the prep tool
  without the DML role's connection string in hand yet (e.g., she preps the database before her ops
  team has created the `embyr_app` role), the grant step is skipped and `verify_schema_readiness()`
  will report `NotPrepped` at provisioning time even though the schema itself is fully applied,
  until the prep tool is re-run (idempotently) with `EMBYR_DB_PREP_DML_ROLE_DSN` supplied. This is
  judged an acceptable, honestly-documented trade-off — re-running the idempotent prep tool is cheap
  — but it is a real behavior change from the rejected `PUBLIC`-grant design, which had no such
  ordering dependency.
- The grant step opens one additional short-lived Postgres connection during a prep run (using the
  DML role's credential, solely to run `SELECT current_user`). This connection is never logged and
  is used for nothing else — mirrors the existing `StartupProbe` DSN-handling convention — but it is
  additional surface relative to the single-connection `PUBLIC`-grant design.

## Enforcement

- Integration test (DISTILL wave, testcontainers, mirrors `SystemDb::probe()`'s existing test
  pattern in `crates/embyr-server/src/adapters/system_db.rs`): connect `verify_schema_readiness()`
  as a role granted only `SELECT`/`INSERT`/`UPDATE`/`DELETE` on `documents`/`transactions` (no
  `CREATE`), against each of the three schema states (fresh/empty, partially migrated, fully
  migrated), asserting the correct `SchemaReadiness` variant in each case without the check itself
  ever triggering a permission-denied error.
- **Negative privilege test (added per security review):** after `embyr-db-prep` runs with
  `EMBYR_DB_PREP_DML_ROLE_DSN` set to role `embyr_app`, assert that (a) `embyr_app` CAN
  `SELECT * FROM _sqlx_migrations`, and (b) a *second*, distinct role (`embyr_app_other`, granted no
  privileges by the test fixture) CANNOT — asserts `permission denied for table _sqlx_migrations`.
  This is the test that would have caught a `PUBLIC` grant regression: with `PUBLIC`, (b) would
  incorrectly succeed.
- Integration test: run `embyr-db-prep`'s full flow (migrate + discover + grant) twice against the
  same database and the same `EMBYR_DB_PREP_DML_ROLE_DSN`, assert the second run is a no-op —
  `PostgresBackendAdapter::migrate()`'s existing idempotency (unchanged) plus the new
  `grant_schema_readiness_read()`'s idempotency (re-granting an already-granted privilege is a
  Postgres no-op, not an error).
- Integration test: run `embyr-db-prep` with `EMBYR_DB_PREP_DML_ROLE_DSN` **absent**, assert
  migration success is still reported via the unchanged success message, the informational
  grant-skipped note is printed, and a subsequent `verify_schema_readiness()` call (using a DML-only
  role with no grant) reports `NotPrepped` rather than erroring unhandled.
- `cargo-mutants -p embyr-core --filter schema_readiness` targets the pure `SchemaReadiness`
  comparison logic. Per-feature mutation gate per project `CLAUDE.md`.
