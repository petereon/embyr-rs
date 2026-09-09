# ADR-072: Composite Index Real Provisioning — Safe DDL, Non-Blocking Build, Symmetric Delete

## Status: Accepted

## Context

Finding #5 (Blocker) of `docs/product/production-readiness-audit-2026-09-08.md`: `create_composite_index`
(`crates/embyr-server/src/admin/handlers/composite_indexes.rs:118-123`) writes a `composite_indexes`
metadata row with `status` hardcoded to the SQL literal `'ready'` on every INSERT — no Postgres DDL is
ever built or executed. `IndexManager::is_index_ready` reads only that row. The Firestore-parity
`FAILED_PRECONDITION` admission gate (`handler.rs`, `requires_composite_index`/`is_index_ready`,
unmodified by this feature) is correct and unaffected — every admitted "ready" compound query then runs
as a full sequential scan forever, since the only real index on `documents`
(`migrations/customer/0001_documents.sql`) is `(project_id, collection_path) WHERE NOT deleted`, with
zero coverage of the `fields` JSONB column.

DISCUSS (`docs/feature/composite-index-real-creation/feature-delta.md`) locked the user-facing outcome
(3 stories, 12 ACs) and handed DESIGN four open engineering trade-offs (Questions A-D). This ADR resolves
all four, plus the schema/component-boundary decisions needed to implement them.

## Decision

### A — DDL-safety: reuse `validate_field_path`'s charset gate; no `quote_ident` needed

Every `field_path` in `CreateCompositeIndexBody.fields[]` is validated via the EXISTING
`embyr_core::domain::query::validate_field_path` (`^[a-zA-Z_][a-zA-Z0-9_.]*$`) before any DDL text is
built — identical reuse to the WHERE/ORDER-BY builders. Read directly (`query.rs` lines 45-346): every
existing SQL-generation call site interpolates `field_path` as a **string-literal argument to the `->`
JSONB operator** (`fields->'{field_path}'->>'v'`, `fields->'{field_path}'`) — never as a raw SQL
identifier. The accepted charset contains no `'`, `;`, `--`, `/*`, or any other SQL metacharacter, so
direct interpolation into a `'{field_path}'` string-literal position is injection-safe by the SAME
argument this codebase's own production WHERE/ORDER-BY code already relies on (unparameterized, proven
safe today). No nested-path per-segment handling is needed: existing code treats a dotted path
(`a.b.c`) as one flat top-level JSONB key, verified directly by reading `append_field_filter`/
`order_by_expr` — this feature's DDL builder does the same, zero new convention.

`quote_ident`-style identifier quoting IS still needed, but only for the **index name** — a raw SQL
identifier, never derived from user input (see Decision D, naming scheme: `cix_<hex(uuid)>`, generated
server-side from `composite_indexes.id`). No crate dependency added; the fixed `cix_` prefix + hex UUID
is by construction a valid, injection-free identifier.

**Alternatives considered**:
- A generic identifier-quoting wrapper (hand-rolled `quote_ident`, or a crate) around `field_path` —
  rejected: `field_path` is never used as a raw identifier anywhere in this DDL (same as existing
  WHERE/ORDER-BY code), so this would be unused defensive machinery for a threat shape that doesn't
  exist at this call site.
- A second, DDL-specific field-path validator — rejected: `validate_field_path`'s charset is the SAME
  threat shape (string-literal interpolation into JSONB-operator SQL text), already proven, one root
  gate is simpler and avoids drift between two validators of the same property.

### B — `CREATE INDEX CONCURRENTLY`, with `INVALID`-index detection and cleanup

Confirmed: `CONCURRENTLY` is required — AC-CXR-06 (build must not block writes to a large, live
collection) is structurally unreachable with plain `CREATE INDEX` (Postgres holds a
write-blocking lock for the statement's duration). Locked:

- Build statement: `CREATE INDEX CONCURRENTLY IF NOT EXISTS {name} ON documents (...)` (Decision D for
  the expression list), executed as a standalone statement on a **fresh, single-purpose connection**
  (never inside a `sqlx` transaction/`.begin()` block — `CONCURRENTLY` cannot run in one).
- After the statement returns (`Ok` or `Err`), the build task ALWAYS re-checks validity with a separate
  query: `SELECT indisvalid FROM pg_index WHERE indexrelid = '{name}'::regclass` — this catches the case
  where the DDL connection itself drops mid-build (server-side leaves an `INVALID` index; the client
  never gets a clean `Err`). `Ok` from the statement + `indisvalid = true` -> `ready`. Any other
  combination (statement `Err`, `indisvalid = false`, or the validity query itself failing/timing out) ->
  `failed`.
- On `failed`: best-effort `DROP INDEX CONCURRENTLY IF EXISTS {name}` (ignore errors, log
  `tracing::warn!`) BEFORE writing `status = 'failed'` — guarantees the deterministic name is free for a
  future retry (Decision C) without a manual operator DROP.

**Alternatives considered**:
- Plain `CREATE INDEX` gated behind a collection-size threshold ("small collections can tolerate the
  lock") — rejected: no domain evidence of a safe threshold, and DISCUSS's own AC-CXR-06 language
  ("large, live collection") names the exact case a threshold would need to get right; `CONCURRENTLY`'s
  extra cost (longer build, two-transaction internal implementation) is strictly cheaper than getting a
  threshold wrong in production.
- Ignoring `INVALID`-index risk (trust the `Err`/`Ok` return alone) — rejected: this is precisely the
  documented Postgres failure mode DISCUSS's own task framing named; silently trusting `Ok` without the
  `indisvalid` re-check would let a connection-drop-mid-build masquerade as success.

### C — Async build: `tokio::spawn` one-shot task, reusing `transaction_sweeper.rs`'s DSN-resolution; no job queue

**Extract, don't duplicate**: `resolve_dsn_without_api_key` and its three per-`backend_mode` helpers
(`resolve_aws_secret_dsn`/`resolve_gcp_secret_dsn`/`resolve_direct_pg_dsn`) move from
`crates/embyr-server/src/sweepers/transaction_sweeper.rs` into a new shared module,
`crates/embyr-server/src/adapters/customer_db_connect.rs`, generalized to take a small `PgConnectInfo`
struct (`{ backend_mode, backend_secret_arn, backend_secret_gcp, backend_pg_dsn_enc }` — the same shape
`SweeperProjectRow` already has, renamed/relocated since it is no longer sweeper-specific).
`transaction_sweeper.rs` imports it instead of defining it locally — zero behavior change there,
confirmed by keeping the function signatures and `None`-on-any-failure/`tracing::warn!`-per-branch
contract byte-identical. This is the ONLY existing precedent in this codebase for reaching a customer DB
without a live `api_key`; this feature is its second consumer, which is what justifies extracting it now
(rule of three not needed — DISCUSS's own task explicitly asked to evaluate the extraction).

**Trigger and lifecycle**: `create_composite_index`'s handler body, after the `INSERT ... RETURNING`
(Decision, schema below), inspects the returned `status`. If it is `'building'` (true for both a fresh
INSERT and a reset-from-`'failed'` retry — see the `ON CONFLICT` clause below), it fetches the project's
`PgConnectInfo` (new `SystemDb::get_project_pg_connect_info(project_id, account_id)` — folds an
ownership-scoped lookup into one query, mirroring `get_project_backend_mode`'s own established
"fold ownership + field into one query" pattern, ADR-036 Decision 5) and fires
`tokio::spawn(build_composite_index(...))` — a **one-shot** task, not an interval loop: resolve DSN,
connect via `PostgresBackendAdapter::new(&dsn)`, run the DDL (Decision B), write the terminal
`composite_indexes.status` (new `SystemDb::update_composite_index_status(id, status)`) via the
**system DB** pool the handler already holds (`state.system_db`, cloned into the spawned task). If
`status` is `'ready'` (no-op conflict on an already-good index) or the row is otherwise unaffected, no
task is spawned — never rebuilds a working index.

**No generic job-queue infrastructure**: no other feature in this codebase needs one; a
`tokio::spawn`'d one-shot task fired at request time, writing its own terminal state back via a single
`UPDATE ... WHERE id = $1`, is materially simpler and sufficient at this codebase's actual scale (one
`embyr-server` process, no multi-instance coordination — explicitly out of scope per DISCUSS).

**Status value set** (schema, migration `0036_composite_indexes_status_lifecycle.sql`): widen
`composite_indexes.status` from a bare, unconstrained `VARCHAR DEFAULT 'ready'` to
`VARCHAR(20) DEFAULT 'building' CHECK (status IN ('building', 'ready', 'failed'))`. No backfill needed —
every existing row was already written as `'ready'` (confirmed by DISCUSS's own reading), which remains
a valid value under the new CHECK.

**Retry via idempotent re-POST, not self-healing crash-recovery**: `create_composite_index`'s own
`INSERT ... ON CONFLICT (project_id, collection_path, fields) DO UPDATE` clause becomes:

```sql
INSERT INTO composite_indexes (project_id, collection_path, fields, status)
VALUES ($1, $2, $3::jsonb, 'building')
ON CONFLICT (project_id, collection_path, fields)
DO UPDATE SET status = CASE WHEN composite_indexes.status = 'failed' THEN 'building'
                            ELSE composite_indexes.status END
RETURNING id::text AS id, project_id, collection_path, fields, status, created_at
```

The `CASE` (not a `WHERE`-conditional `DO UPDATE`) guarantees the clause always "fires" so `RETURNING`
always yields a row (mirrors the existing handler's own established
`DO UPDATE SET collection_path = EXCLUDED.collection_path` no-op-self-update trick, same guarantee, same
reason). A duplicate POST against a `'ready'` index is a true no-op (never rebuilds). A duplicate POST
against a `'failed'` index resets it to `'building'` and the handler re-spawns a build — this IS the
"operator retries via the existing endpoint's own idempotency" mechanism DISCUSS asked DESIGN to
confirm.

**Explicit, locked answer to the crash-recovery question**: a `'building'` row does NOT self-heal or
resume if `embyr-server` restarts mid-build (no supervisory mechanism watches for this). It remains
`'building'` — visible, honest, never silently `'ready'` — until an operator notices (AC-CXR-07 already
requires `'building'` to be observably distinguishable from `'ready'`) and recovers via the ALREADY-
DESIGNED US-03 delete path: `DeleteIndex` on a `'building'` row succeeds without error (AC-CXR-11,
best-effort drops any partial/invalid real index), after which a fresh `POST` starts a clean build under
a new row `id` (and therefore a new deterministic index name). This reuses only mechanisms this ADR
already builds — zero new "resume" machinery — matching DISCUSS's own explicitly offered simpler
alternative.

**`backend_mode = agent` handled for free**: `resolve_dsn_without_api_key`'s existing `other => None`
match arm (already logs `tracing::warn!` and returns `None`) already covers any `backend_mode` outside
`{aws_secret, gcp_secret, direct_pg}` — `agent` falls through it unchanged. A `None` DSN makes the build
task write `status = 'failed'` through the exact same path as any other unreachable-database case. No
special-case code added; Alex sees `'building'` immediately (200 response), then `'failed'` on the next
`ListIndexes` poll — consistent with AC-CXR-09, and consistent with the already-established
"agent-mode is a structurally different wall" pattern (never silently `'ready'`).

**Known, accepted, pre-existing gap inherited, not introduced**: `main.rs`'s composition root passes
`aws_secret_fetcher: None, gcp_secret_fetcher: None` to EVERY consumer today (`build_router`,
`TransactionSweeper::spawn`) — an already-documented gap (ADR-054 § D7). `UserAdminState` gains the same
two `Option<Arc<_>>` fields (currently absent — `OperatorState` has them, `UserAdminState` does not),
wired `None, None` at the composition root identically to every existing consumer. `direct_pg` projects
(every worked example in this feature's own stories — Trailmark) are unaffected; `aws_secret`/
`gcp_secret` projects get a clean `'failed'` status via the same `None`-DSN path, not a silent `'ready'`
— this feature neither fixes nor worsens the pre-existing gap, and never misrepresents its status.

**Alternatives considered**:
- Generic background-job-queue table + worker pool — rejected per DISCUSS's own explicit scoping-out;
  no second consumer exists in this codebase, and the one-shot-task shape above is materially simpler
  while meeting every named AC.
- Self-healing resume-on-restart (a startup scan for stale `'building'` rows, re-driving their builds) —
  rejected for THIS feature: no domain example shows a customer harmed by a stale `'building'` row
  surviving a restart (DISCUSS's own finding); the existing delete-and-recreate path already recovers it
  with zero new code. Named as a candidate follow-up if a real incident ever demonstrates the gap
  matters at this codebase's actual (single-instance) scale.

### D — Index DDL expression shape: literal `(project_id, collection_path)` prefix + role-split field expressions

Read `crates/embyr-pg-storage/src/encoding/query.rs` and `backend_adapter.rs::run_query` (line 557-582)
side by side. Three facts drive this decision:

1. `run_query`'s own WHERE clause always leads with bound, non-expression predicates:
   `project_id = $1 AND collection_path = $2 ... AND NOT deleted` (confirmed directly, `backend_adapter.rs`
   lines 568-582) — matching the EXISTING `documents_project_collection_idx (project_id, collection_path)
   WHERE NOT deleted` convention.
2. **Equality/inequality filters** (`Equal`/`NotEqual`, and by extension `In`/`NotIn`, which reduce to
   the same primitive) compare the field's WHOLE stored JSON value, uniformly for every `FieldValue`
   type: `fields->'{field}' {=|!=} $1::jsonb` (`push_value_equality`) — no `->>'v'` extraction, no
   type-specific cast.
3. **`ORDER BY`** always extracts the unwrapped scalar as TEXT: `fields->'{field}'->>'v' {ASC|DESC}`
   (`order_by_expr`) — a DIFFERENT expression shape from (2), and range-comparison filters
   (`<`/`<=`/`>`/`>=`) need a THIRD, type-specific-cast shape (`push_scalar_comparison`:
   `(fields->'{field}'->>'v')::bigint`, `::float8`, `::boolean`, `ROW(...)` for `Timestamp`, `decode(...,
   'base64')` for `Bytes`) that cannot be determined from `CreateCompositeIndexBody` alone (it carries no
   value-type information).

**Locked expression shape** — for a `composite_indexes.fields` array `[f_1, ..., f_n]` (`n >= 1`):

```sql
CREATE INDEX CONCURRENTLY IF NOT EXISTS cix_{hex(id)}
  ON documents (
    project_id,
    collection_path,
    (fields->'{f_1.field}'),          -- ... one such column per f_1..f_{n-1}, "equality role"
    (fields->'{f_{n-1}.field}'),
    (fields->'{f_n.field}'->>'v') {f_n.order: ASC|DESC}   -- "sort role", last field only
  )
  WHERE NOT deleted;
```

- **Leading `(project_id, collection_path)`**: plain (non-expression) columns, matching `run_query`'s own
  bound-parameter predicates exactly — safe under Postgres's generic (not just custom) prepared-statement
  plans, unlike a partial-index predicate keyed off a bound parameter would be.
- **`WHERE NOT deleted`**: a partial-index predicate is safe here specifically because `run_query` emits
  it as a literal boolean condition (`AND NOT deleted`), not a bound parameter — constant-to-constant
  matching is provable by the planner at every plan-generic level, mirroring the existing
  `documents_project_collection_idx`'s own identical partial predicate.
- **All fields except the last** (`f_1..f_{n-1}`): built as `(fields->'{field}')` — the SAME expression
  `push_value_equality` emits for `Equal`/`NotEqual` (and, transitively, `In`/`NotIn`), and, being
  whole-JSON-value comparison, is **type-agnostic** — correct for every `FieldValue` variant Alex might
  filter on, no runtime-type knowledge needed at index-build time.
- **The last field only** (`f_n`): built as `(fields->'{field}'->>'v') {ASC|DESC}`, using the field's own
  declared `order` — the SAME expression `order_by_expr` emits, syntactically.
- This directly and provably covers the walking skeleton's own worked example (US-01 Example 1:
  `category == 'electronics'` [equality role] `ORDER BY score DESC` [sort role]) and, more generally,
  every composite index shape `requires_composite_index`'s BASE rule triggers (N equality filter fields +
  exactly one orderBy field on a different field) — the dominant, most common real-world composite-index
  shape, and the ONLY shape any of this feature's own ACs (AC-CXR-01/02/03) exercise or assert against.

**Explicit, named limitation (not silently assumed solved)**: `composite_indexes.fields[]` carries no
per-field operator/role tag (unchanged wire contract, per DISCUSS's own lock) — this convention infers
role purely positionally (last = sort, rest = equality). It is **not** proven correct, and is explicitly
NOT claimed correct, for two rarer trigger shapes neither this feature's stories nor its ACs exercise:
(a) 2+ genuine `orderBy` fields with zero filter (`requires_composite_index`'s own `order_by.len() >= 2`
rule) — this convention would misbuild all-but-the-last such field as equality-shaped, and the index
would not be used to satisfy that field's own sort; (b) the `IN` + range-filter-on-a-different-field
shape (`AC-CIR-04`) when the range-filtered field lands last — it needs a type-cast range expression
(`push_scalar_comparison`'s shape), not the sort-role text-extraction this convention would build. Both
are named, deferred gaps, consistent with DISCUSS's own already-locked Out-of-Scope entry "multi-index
query planning sophistication" — a candidate follow-up feature, not a regression this feature introduces
(neither shape has any real index today either).

**Alternatives considered**:
- A GIN index on the whole `fields` JSONB column — rejected: GIN serves containment (`@>`) queries;
  none of the composite-index trigger shapes (equality/range/sort) benefit from containment semantics,
  and DISCUSS's own reading confirms no containment-based query exists on this path.
- Requiring a NEW `role`/`operator` field on `CreateCompositeIndexBody` to remove the positional
  ambiguity entirely — rejected: DISCUSS explicitly locked "the existing CreateIndex request/response
  shape ... are unchanged" (US-01's own regression-guard scenario); changing the wire contract is out of
  this feature's scope. Named as the clean fix for the two limitation cases above, for a future feature.

## Component Boundaries (new/changed)

- `crates/embyr-server/src/adapters/customer_db_connect.rs` (**new**): `PgConnectInfo` struct +
  `resolve_dsn_without_api_key` and its 3 helpers, extracted verbatim from `transaction_sweeper.rs`
  (behavior-preserving move).
- `crates/embyr-server/src/adapters/composite_index_ddl.rs` (**new**): pure functions —
  `index_name_for(id: Uuid) -> String`, `build_create_index_sql(id, collection_fields) -> Result<String,
  CoreError>`, `build_drop_index_sql(id) -> String` — each field validated via `validate_field_path`
  before any DDL text is built (AC-CXR-04). Zero IO; unit-testable without a database (mirrors
  `encoding/query.rs`'s own `QueryBuilder::sql()`-based pure-SQL-shape test pattern).
- `crates/embyr-server/src/adapters/composite_index_builder.rs` (**new**): `spawn_build(system_db,
  connect_info, index_id, collection_path, fields) -> JoinHandle<()>` — the one-shot task: resolve DSN ->
  connect -> run DDL -> `indisvalid` check -> best-effort cleanup on failure -> write terminal status via
  `SystemDb::update_composite_index_status`.
- `crates/embyr-server/src/adapters/system_db.rs` (**changed**): + `get_project_pg_connect_info(project_id,
  account_id) -> Result<Option<PgConnectInfo>, CoreError>` (ownership-scoped, mirrors
  `get_project_backend_mode`); + `update_composite_index_status(id: Uuid, status: &str) -> Result<(),
  CoreError>`.
- `crates/embyr-server/src/admin/handlers/composite_indexes.rs` (**changed**): `create_composite_index`'s
  INSERT SQL (status literal + `ON CONFLICT` clause, per Decision C) and post-insert spawn call;
  `delete_composite_index` gains a best-effort real-index-drop step BEFORE the existing metadata-row
  DELETE (ordering deliberate: on a drop failure, the metadata row is left in place — visible, retryable
  — rather than orphaning a real index with no record of it at all, mirroring the audit's own recurring
  "leaks forever" concern, findings #4/#6).
- `crates/embyr-server/src/admin/state.rs` (**changed**): `UserAdminState` gains
  `aws_secret_fetcher: Option<Arc<AwsSecretFetcher>>`, `gcp_secret_fetcher: Option<Arc<GcpSecretFetcher>>`
  (structurally required for `direct_pg`/`aws_secret`/`gcp_secret` dispatch; wired `None, None` at the
  composition root, matching the existing fleet-wide gap noted above).
- `crates/embyr-server/src/sweepers/transaction_sweeper.rs` (**changed**, behavior-preserving): imports
  `PgConnectInfo`/`resolve_dsn_without_api_key` from the new `adapters::customer_db_connect` module
  instead of defining them locally.
- `migrations/0036_composite_indexes_status_lifecycle.sql` (**new**): `status` default `'building'` +
  `CHECK (status IN ('building', 'ready', 'failed'))`.

**Zero changes**: `crates/embyr-core` (no IO crate ever enters it — `validate_field_path` is reused
unmodified), `crates/embyr-pg-storage/src/encoding/query.rs` (WHERE/ORDER-BY builders, unmodified —
this feature's DDL builder is informed by, but does not call into, these functions), `handler.rs`'s
`RunQuery`/`requires_composite_index`/`is_index_ready` call sites (unmodified — this feature only makes
the row they already read truthful).

## Enforcement

- `deny.toml`/CI (existing, unmodified) continues to guarantee `embyr-core` never imports `tokio`/`sqlx`
  — this feature adds zero code there.
- `composite_index_ddl.rs`'s pure DDL-string builders get direct unit-test coverage (no DB needed) proving:
  every field passes through `validate_field_path` before any string is built; the generated SQL text is
  byte-identical to the shape locked in Decision D (regression guard against silent expression drift
  versus `encoding/query.rs`'s own emitted shapes); a field outside the accepted charset is rejected
  before any DDL string exists (AC-CXR-04, satisfiable as a pure function test, no acceptance-test
  DB round-trip required for this specific guarantee).
- `cargo-mutants`, budgeted at QUALITY_GATE per this session's own established discipline, targets the
  new pure functions (`composite_index_ddl.rs`) and the widened `ON CONFLICT` `CASE` logic.

## Consequences

**Positive**: closes production-readiness audit finding #5 (Blocker) — `CreateIndex` provisions a real,
planner-usable Postgres index; `status` becomes truthful across its full lifecycle; large-collection
builds never block writes; `DeleteIndex` no longer orphans real indexes. Reuses 3 existing mechanisms
(`validate_field_path`, `transaction_sweeper`'s DSN resolution, the existing idempotent-INSERT pattern)
rather than inventing new infrastructure classes.

**Negative / accepted trade-offs**: the positional equality-vs-sort role inference (Decision D) is a
named, bounded gap for 2 rarer trigger shapes — flagged, not hidden, and matches the scope DISCUSS
itself locked. A `'building'` row does not self-heal after a process crash — recoverable only via the
already-existing delete-and-recreate path, an explicit, simpler, deliberately-chosen trade-off. The
pre-existing `aws_secret`/`gcp_secret` DSN-fetcher gap (ADR-054 § D7) now also applies to real
composite-index provisioning — inherited, not introduced, and never silently misreported (`'failed'`,
never `'ready'`).
