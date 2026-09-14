# ADR-080: Collection-Group Query Index — Trigger-Maintained Column, Resumable Backfill, Schema-Capability Probe

## Status: Accepted

## Context

Finding #17 (High) of `docs/product/production-readiness-audit-2026-09-08.md`: all four mirrored
`all_descendants=true` predicate-construction sites in `crates/embyr-pg-storage/src/backend_adapter.rs`
(`run_query` 598-608; `run_aggregation_query`'s `Count`/`Sum`/`Avg` arms 811-820/859-868/910-919 — line
numbers point-in-time per DISCUSS's own re-verification) build `collection_path = $1 OR collection_path
LIKE '%/$1'`. The leading `%` defeats every existing and future btree, including every real composite
index `composite-index-real-creation` (ADR-072) now provisions — every collection-group query
sequential-scans the entire project's `documents` table. DISCUSS (`docs/feature/collection-group-query-
index/feature-delta.md`) confirmed this is a pure performance gap (SPEC.md 913-921 semantics already
correct) and locked three hard requirements DESIGN must jointly satisfy: (US-01) a real index-assisted
plan for schema-current databases with zero result-set change; (US-02) backfilling existing documents
never blocks a live customer's concurrent reads/writes, and is safely resumable with zero double-
processing of already-backfilled rows; (US-03) a customer database that has not received this feature's
`migrations/customer/` migration — a genuinely indefinite, customer-DBA-controlled window, since no
fleet-wide re-apply mechanism exists (ADR-022) — must never regress to a wrong result or a hard error.

DISCUSS surveyed four mechanism candidates (Question A) without locking one: a reversed-string expression
index, a trigram/GIN index (`pg_trgm`), a separate `collection_id` column, or a partial/expression index
on a computed last-segment extraction. It also flagged, as DESIGN's own verification duty, whether `ALTER
TABLE ... ADD COLUMN ... GENERATED ALWAYS AS (...) STORED` requires a full-table rewrite (it does, in
every Postgres version this codebase's `migrations/` currently target — computing a generated
expression for every existing row is definitionally a rewrite, taking an `ACCESS EXCLUSIVE` lock for the
statement's duration; Postgres 18's *virtual* generated columns avoid this, but no authoritative minimum
supported customer-Postgres-version document exists in this codebase (DISCUSS's own finding) to justify
assuming that floor).

## Decision

### A — Indexable representation: a plain, nullable `collection_id` column, populated by a `BEFORE INSERT` trigger — not `GENERATED ... STORED`, not an expression-only index

**Column, not `GENERATED ALWAYS AS (...) STORED`.** `ALTER TABLE documents ADD COLUMN collection_id
VARCHAR(1500)` with **no default and no `GENERATED` clause** is a metadata-only catalog change in every
Postgres version this codebase supports — confirmed: a nullable column with no default has never required
a table rewrite, independent of the PG-18 virtual-generated-column question DISCUSS flagged. This is the
one candidate shape that carries zero minimum-version risk, resolving DISCUSS's own NFR without first
needing to establish the (currently absent) supported-Postgres-versions document.

**Population via a `BEFORE INSERT` trigger, not Rust write-path code.** `INSERT INTO documents` exists at
FIVE call sites in `backend_adapter.rs` (347, 398, 519, 1252, 1312) — a larger surface than the four
mirrored `LIKE` sites this very finding exists because of. Populating `collection_id` in Rust at each site
would recreate, at one more site, the exact copy-paste-drift risk ADR-040 § 2's own comment already warns
about for the four `LIKE` sites. A Postgres trigger removes the duplication class by construction: it
fires for every `INSERT` regardless of which of the five (or any future) call site issued it, requires
zero Rust code change at any write site, and is unaffected by `embyr-agent`'s independent binary version
skew (the trigger lives in the database, not in either binary). `collection_path` is confirmed immutable
per-row after insert (`handler.rs:151`, `segments.join("/")`; none of the five `DO UPDATE`/`UPDATE`
clauses in `backend_adapter.rs`, including the two upsert arms at 1256-1258 and 1316-1318 and the
soft-delete `UPDATE` at 1276-1280, ever assigns `collection_path`), so `BEFORE INSERT` alone is sufficient
— no `BEFORE UPDATE` trigger is needed, and `collection_id` never needs recomputation after its one
insert-time write.

Extraction rule: `regexp_replace(NEW.collection_path, '^.*/', '')` — greedy `.*` consumes up to the LAST
`/`, leaving the final path segment; a bare top-level `collection_path` (no `/`) is returned unchanged.
Confirmed safe against every `collection_path` this codebase constructs: `handler.rs:151` builds nested
paths via `segments.join("/")` — no empty segments, no leading/trailing slash, single `/` separator,
exactly the shape this regex assumes.

**Not a pure expression index (option 4) with no column at all.** A `CREATE INDEX CONCURRENTLY ... ON
documents (project_id, (regexp_replace(collection_path, '^.*/', '')))` avoids adding a column entirely —
Postgres auto-maintains expression indexes on every write, and `CONCURRENTLY` itself computes+indexes
existing rows without a separate backfill step, which looked initially more attractive than a column +
trigger + separate backfill loop. **Rejected specifically against AC-CGI-08(d)**: `CREATE INDEX
CONCURRENTLY`, if interrupted (the `PostgresBackendAdapter::new(&dsn)` connection drops mid-build, or
`embyr-db-prep` is killed), leaves an `INVALID` index with no resumable state — the only recovery is `DROP
... CONCURRENTLY` + a full rebuild from scratch, which re-scans and re-computes every row, including ones
already covered by the failed attempt. AC-CGI-08(d) requires "already-backfilled documents are not
redundantly reprocessed on resume," verified by the resumed run's own row-touch count excluding
already-completed documents — an index-only rebuild cannot satisfy this by construction. A real column,
backfilled via an explicit `WHERE collection_id IS NULL` loop (Decision B), resumes for free: already-set
rows no longer match the loop's own `WHERE` clause, so resumption touches zero already-completed rows,
trivially satisfying AC-CGI-08(d). This is the deciding factor over the otherwise-simpler expression-index
design.

**Not `pg_trgm`.** Requires `CREATE EXTENSION` (superuser or extension-allowlist privilege a BYOC
customer's own managed-Postgres provider may restrict), carries GIN's own larger index size and write
amplification, and — unlike a real equality-shaped column — does not convert the collection-group
predicate into the same shape `composite-index-real-creation`'s own composite indexes already use,
forfeiting the "closes the compounding relationship at its root" opportunity DISCUSS's own Question A
named. No open-source-license concern (`pg_trgm` ships in core Postgres `contrib`, BSD-style Postgres
license) — rejected purely on the privilege/version-availability grounds DISCUSS's own NFR raised, not on
licensing.

**Not a reversed-string expression index.** Same write-time-population problem as the plain column (needs
either a trigger or per-call-site Rust code to maintain a reversed string), with none of the column's
benefit of composing directly, as a literal leading column, with `composite-index-real-creation`'s own
composite-index DDL shape (ADR-072 Decision D) for any future collection-group-aware composite index —
named as a real but explicitly out-of-scope future opportunity (DISCUSS's own Out-of-Scope: "Extending
this fix to any other query shape").

### B — Backfill: resumable, throttled, `FOR UPDATE SKIP LOCKED` batched `UPDATE` loop — no job-queue framework, no async orchestration

```sql
UPDATE documents d
SET collection_id = regexp_replace(d.collection_path, '^.*/', '')
FROM (
    SELECT ctid FROM documents
    WHERE project_id = $1 AND collection_id IS NULL
    LIMIT $2                      -- batch_size, working default 1000
    FOR UPDATE SKIP LOCKED
) sub
WHERE d.ctid = sub.ctid;
```

Each batch is its own short transaction (never one unbounded transaction for the whole backfill). The
loop sleeps a throttle interval (working default 50ms) between batches, then re-runs the same statement,
terminating when a batch affects zero rows. `FOR UPDATE SKIP LOCKED` means the backfill never waits on a
row a concurrent production write already holds — it defers that row to a later batch instead of blocking
— directly satisfying AC-CGI-06's "no write is ever blocked waiting on a lock the backfill itself holds."
Resumability (AC-CGI-08) falls out of the `WHERE collection_id IS NULL` predicate with zero additional
bookkeeping: an interrupted-then-resumed run re-issues the identical query, which by construction only
ever matches not-yet-backfilled rows — no cursor, checkpoint, or idempotency token needed.

**No async job-queue, no `tokio::spawn`.** Unlike ADR-072's composite-index build (customer-triggered via
an HTTP `CreateIndex` request that must return before the build finishes), this backfill has exactly one
caller shape: a one-shot, already-blocking CLI process (`embyr-db-prep`, run once by a customer's DBA who
expects to watch it run to completion) or a fresh-project provisioning path where the backfill is a
guaranteed no-op (zero pre-existing rows, loop terminates on its first, empty batch). Both call sites can
run the loop synchronously to completion; no request/response API needs a `status` column or polling
endpoint the way `composite_indexes` does. This is a deliberate, evidence-based divergence from ADR-072's
own async-task pattern — reusing it here would add orchestration machinery this feature's actual caller
shape does not need (DISCUSS's own Out-of-Scope already rules out a generic job-queue framework).

Reuses ADR-022's single-embed-point discipline: the backfill loop is a new method on
`PostgresBackendAdapter` (the same crate `migrate()` already lives in, for the same reason — both
`embyr-server`'s `provision.rs` and the dependency-minimal `embyr-db-prep` crate need it, and
`embyr-pg-storage` is the one crate both already depend on), invoked from the same call sites `migrate()`
already is, immediately after `migrate()` succeeds — no new call-site fragmentation beyond what already
exists.

### C — Schema-capability probe: cached, monotonic-true, TTL'd-false — not per-query try/catch

`PostgresBackendAdapter` gains one cached capability check, queried once per adapter instance (one
instance per customer database, confirmed shared identically by `embyr-server` and `embyr-agent`'s
`StorageAgent`) via `information_schema.columns` (a catalog lookup, not a table scan):

```sql
SELECT 1 FROM information_schema.columns
WHERE table_name = 'documents' AND column_name = 'collection_id'
LIMIT 1;
```

Cache semantics are asymmetric, reflecting that this codebase's migrations are additive-only (ADR-022 —
no downgrade path exists): once a probe observes the column present, that result is cached **permanently**
for the adapter instance's lifetime (a schema migration never un-happens). Once a probe observes the
column absent, that result is cached with a short TTL (working default 30s) and re-checked on next use
after expiry — this is what lets a customer's mid-flight `embyr-db-prep` run be picked up automatically
(AC-CGI-11: "no restart, redeploy, or explicit per-customer cutover required") without paying a
probe-per-query cost in the converged, eventual steady state where the whole fleet has migrated (AC-CGI-13's
5% overhead ceiling — a cached boolean read is effectively free).

**Not per-query try/catch on `undefined_column` (SQLSTATE 42703).** Exception-driven control flow would
put error-handling machinery in the hot path of every collection-group query forever, including the
eventual (and intended-to-become-universal) fully-migrated steady state — the common case would forever
pay for handling the rare/transitional case. A cached, monotonic probe pays the detection cost once per
adapter lifetime for the common case, and bounded-by-TTL for the rare case.

### D — Query predicate: unifies "not yet migrated" and "migrated but not yet backfilled" into one hybrid shape; two purpose-built partial indexes

For a schema-current database (probe returns `Available`), all four mirrored call sites replace the
existing `all_descendants` arm with:

```sql
AND (
    collection_id = $N
    OR (collection_id IS NULL AND (collection_path = $N OR collection_path LIKE '%/' || $N))
)
```

This single shape is correct and index-assisted across the ENTIRE backfill lifecycle of a schema-current
database — not just after backfill completes — because AC-CGI-12 requires a partially-backfilled
collection's collection-group query to return every matching document regardless of each document's own
backfill state, and the fleet-wide server-code deploy cannot special-case "this customer finished
backfilling this specific collection" (that state is per-collection, per-document, and changes
continuously while `embyr-db-prep`/provisioning's own backfill loop runs). For a database where the probe
returns `Unavailable`, all four sites keep today's `collection_path = $N OR collection_path LIKE '%/' ||
$N` predicate, byte-for-byte unchanged (never references a column that does not exist).

Two purpose-built partial indexes, both `CREATE INDEX CONCURRENTLY IF NOT EXISTS` (never blocking, ADR-072
precedent reused directly, including its `indisvalid` re-check + best-effort `DROP ... CONCURRENTLY`
cleanup on a dropped-connection failure), built immediately after `migrate()` and BEFORE the backfill loop
runs (not after) — so AC-CGI-02's index-assisted-plan requirement holds from the moment a database becomes
schema-current, not only once backfill later completes:

```sql
CREATE INDEX CONCURRENTLY IF NOT EXISTS documents_collection_group_idx
    ON documents (project_id, collection_id)
    WHERE NOT deleted AND collection_id IS NOT NULL;

CREATE INDEX CONCURRENTLY IF NOT EXISTS documents_collection_group_pending_idx
    ON documents (project_id)
    WHERE NOT deleted AND collection_id IS NULL;
```

The first index serves the `collection_id = $N` arm directly (an ordinary equality lookup, the same shape
as the existing `documents_project_collection_idx`). The second bounds the fallback arm's scan to just the
not-yet-backfilled subset of the project — the set this index covers shrinks to empty as `collection_id
IS NULL` rows are backfilled (standard, automatic Postgres partial-index membership maintenance on
`UPDATE`, no application code involved) — so a `collection_path LIKE '%/' || $N` filter recheck against
that bounded subset is never a sequential scan of the whole `documents` table, satisfying AC-CGI-02 even
mid-backfill. Building both indexes before the backfill loop (rather than after) means early-backfill rows
are touched twice (once by the initial `pending` index's full build, again as the backfill loop moves them
out of it) — an accepted, minor efficiency cost, traded for AC-CGI-02 holding continuously instead of only
after full backfill completes.

Both indexes, and the backfill loop, live in `PostgresBackendAdapter` (`embyr-pg-storage`) rather than in
`embyr-server`'s `adapters/composite_index_builder.rs` (where ADR-072's own equivalent logic lives) —
deliberate placement divergence, because this feature's index/backfill preparation has the SAME two
consumers `migrate()` already has (`embyr-server`'s provisioning path AND the dependency-minimal
`embyr-db-prep` crate, which cannot import `embyr-server`), whereas ADR-072's composite-index build is a
single-consumer, per-project, admin-API-triggered concern with no `embyr-db-prep` involvement at all.

## Component Boundaries (new/changed)

- `migrations/customer/0002_collection_group_index.sql` (**new**, transactional — the schema change
  itself; NOT the `CONCURRENTLY` statements, which cannot run inside a migration transaction, mirroring
  ADR-072's own established constraint): `ALTER TABLE documents ADD COLUMN collection_id VARCHAR(1500)`;
  `CREATE FUNCTION documents_set_collection_id() RETURNS trigger ...`; `CREATE TRIGGER
  documents_collection_id_biu BEFORE INSERT ON documents FOR EACH ROW EXECUTE FUNCTION
  documents_set_collection_id()`. First trigger in this codebase's `migrations/customer/` — named
  explicitly as a new mechanism class, justified above (Decision A) specifically by the five-call-site
  duplication risk a Rust-side population would reintroduce.
- `crates/embyr-pg-storage/src/backend_adapter.rs` (**changed**): + `backfill_collection_id(&self,
  batch_size: u32, throttle: Duration) -> Result<BackfillSummary, CoreError>` (Decision B's loop); +
  `ensure_collection_group_indexes(&self) -> Result<(), CoreError>` (Decision D's two `CONCURRENTLY`
  builds + `indisvalid` recheck, reusing ADR-072's cleanup pattern); + a `SchemaCapabilityProbe` field
  (Decision C's cached probe) read by `run_query` and all three `run_aggregation_query` arms; all four
  mirrored `all_descendants` predicate blocks updated identically (Decision D), preserving the
  byte-for-byte-copy convention ADR-040 § 2 already established for this exact code shape.
- `crates/embyr-pg-storage/src/encoding/query.rs` (**changed**): + `SchemaCapability` enum
  (`Unknown`/`Available`/`Unavailable { checked_at: Instant }`) and a pure `fn is_probe_stale(checked_at:
  Instant, ttl: Duration) -> bool` — the one new pure, mutation-testable decision function this feature
  adds (per DISCUSS's own reapplied mutation-testing lesson), unit-tested without a database alongside the
  existing `append_filter`/`order_by_expr` pure builders.
- `crates/embyr-db-prep` (**changed**): after its existing `PostgresBackendAdapter::migrate()` call, adds
  calls to `ensure_collection_group_indexes()` then `backfill_collection_id()`, logging per-batch progress
  via the existing `tracing` setup (no new metrics/observability infrastructure). Both new calls are
  idempotent and safe to re-run if the DBA re-invokes the binary after an interruption.
- `crates/embyr-server/src/admin/handlers/provision.rs` (**changed**): the same two new calls added
  immediately after each of the three existing `migrate()` call sites (three `backend_mode` branches) —
  a no-op in practice for fresh projects (zero pre-existing rows), kept unconditional rather than
  special-cased, matching this codebase's existing preference for one code path over a conditional one.

**Zero changes**: `crates/embyr-server/src/grpc/handler.rs`'s `RunQuery`/`RunAggregationQuery` proto
translation; `composite-index-real-creation`'s own `CreateIndex`/`DeleteIndex` mechanism and the
`FAILED_PRECONDITION` gate; `embyr-agent`'s own `StorageAgent` (delegates to the same, now-updated
`PostgresBackendAdapter` methods — confirmed by DISCUSS's own reading, no independent fix needed);
`crates/embyr-core` (no IO crate enters it; the trigger is pure SQL, the probe/backfill/index logic stays
in `embyr-pg-storage`).

## Enforcement

- `deny.toml`/CI (existing, unmodified) continues to guarantee `embyr-core` stays IO-free — this feature
  adds zero code there.
- `SchemaCapability::is_probe_stale` gets direct unit-test coverage (no DB needed): TTL boundary
  (just-under vs just-over), `Available` never treated as stale regardless of elapsed time.
- A DISTILL-wave acceptance test asserts the trigger fires correctly (insert a row without specifying
  `collection_id`, read it back, assert the derived value) and that a document written mid-backfill
  already carries a populated `collection_id` (AC-CGI-07) — both real, not mocked, per this session's own
  Strategy A discipline.
- `cargo-mutants`, budgeted at QUALITY_GATE per this session's own established discipline, targets
  `is_probe_stale` and the backfill loop's own batch-termination condition (`WHERE collection_id IS NULL`
  match count reaching zero).
- Existing regression coverage (AC-CGI-04/05) — non-collection-group queries and
  `composite-index-real-creation`'s own ACs — is unaffected by construction: this feature touches only the
  `all_descendants=true` branch of the four mirrored sites.

## Consequences

**Positive**: closes production-readiness audit finding #17 (High) — collection-group queries become
index-assisted for schema-current databases (US-01), existing customer data backfills without ever
blocking production writes or risking double-processing on resume (US-02), and the fleet-wide code
release is decoupled from any individual customer's own migration timeline with zero correctness
regression, verified continuously (not just in the fully-converged end state) across the entire backfill
lifecycle (US-03). Reuses three existing mechanisms (`CREATE INDEX CONCURRENTLY` + `indisvalid` recheck
from ADR-072; the single-embed-point discipline from ADR-022; the byte-for-byte mirrored-site convention
from ADR-040) rather than inventing new infrastructure classes, apart from the one genuinely new
mechanism this feature's own risk profile justifies (a database trigger, Decision A).

**Negative / accepted trade-offs**: this is the first trigger in `migrations/customer/` — a new
mechanism class for this codebase, justified specifically against the five-call-site write-path
duplication risk, not adopted by default preference. Building both partial indexes before the backfill
loop (rather than after) costs a second touch on early-backfilled rows, accepted to keep AC-CGI-02
continuously true rather than only true post-backfill. The `Unavailable`-state probe's 30-second TTL is a
working default (not evidenced against real customer migration-adoption telemetry) — a customer who
completes migration mid-window waits up to 30s for the fast path to engage, never a correctness issue
(the fallback predicate is always correct), only a bounded, self-resolving performance-transition delay.
The `documents_collection_group_pending_idx` partial index has no forced removal step; it is safe (and
recommended, not required) to `DROP INDEX CONCURRENTLY` it once fleet telemetry shows a given customer's
backfill has fully completed — named as a candidate operational-hygiene follow-up, not built here, mirroring
`composite-index-real-creation`'s own precedent of naming, not building, its own deferred symmetry facet.
