# ADR-054: TransactionSweeper — Raw Customer-DB Access Mechanism, Sweep SQL, and Cycle Shape

## Status

Accepted

## Context

`customer-db-transaction-sweeper` (DISCUSS, feature-delta.md) closes a confirmed
resource leak: `BeginTransaction` inserts a `transactions` row; only a
LATER call referencing the same `transaction_id` ever reactively expires it
(`commit_transaction`'s own 60-second check,
`crates/embyr-pg-storage/src/backend_adapter.rs:919-946`). A proactive
background sweeper (`TransactionSweeper`) must enumerate every PG-reachable
customer database and run maintenance SQL against a table
(`transactions`) that sits outside `BackendAdapter`'s own document-CRUD trait
surface (`get_document`, `create_document`, `begin_transaction`/
`commit_transaction`, etc. — no bulk-maintenance method exists or should be
force-fitted onto that surface).

DISCUSS escalated two questions to DESIGN, both resolved here: (1) how the
sweeper reaches raw SQL against a customer DB without going through
`BackendAdapter`, and (2) the exact sweep SQL and cycle shape given the
"many databases, not one" wrinkle `CapUsageRefresher` (the only prior sweeper
built) never had to handle.

### Precedents re-verified directly (not trusted from DISCUSS's citation alone)

1. **`PostgresBackendAdapter::pool()` already exists**
   (`crates/embyr-pg-storage/src/backend_adapter.rs:86-88`) — `pub fn pool(&self)
   -> &PgPool`, doc-commented "Expose the internal pool for use by
   `PostgresNotifyListener`." A second, pre-existing consumer of raw-pool
   access already lives in this codebase. DISCUSS's own Reading Confirmation
   did not cite this accessor.
2. **`resolve_customer_db_adapter`** (`crates/embyr-server/src/adapters/project_auth.rs`,
   client-auth-hosted-identity feature, full 174 lines) — a function DISCUSS's
   own Reading Confirmation never cited, and the single most load-bearing find
   for this ADR. It already solves an almost-identical problem: resolve a
   project's DSN (aws_secret/gcp_secret fetchers unchanged, `direct_pg` via
   ECIES — that feature has a live `api_key`, this one never does), build
   `PostgresBackendAdapter::new(&dsn)`, and return the CONCRETE
   `Arc<PostgresBackendAdapter>` type specifically so its own caller can issue
   raw SQL against `hosted_identity_accounts` — a table outside
   `BackendAdapter`'s surface, exactly the same class of problem
   `transactions` maintenance SQL presents here. Its own module doc explains
   why: "callers need the CONCRETE `PostgresBackendAdapter` type to run raw
   SQL... via its own `pool()` accessor."
3. **`CapUsageRefresher`** (`crates/embyr-server/src/sweepers/cap_usage_refresher.rs`,
   full 225 lines) — works directly against `SystemDb`'s own concrete pool,
   never through any trait, for the identical class of reason: a sweeper is
   infrastructure-shaped, not request-shaped.
4. **`docs/product/architecture/brief.md`'s own original, greenfield
   Application Architecture** (lines 890, 1308, 1570 — none cited by DISCUSS's
   own Reading Confirmation, which only read the later admin-api-v2
   `QueryLogSweeper`/`SessionCleaner` sections at lines 2152-2224). The
   ORIGINAL day-one design already named `TransactionSweeper` as a planned
   `embyr-server::sweepers` component (line 890) and its agent-side
   counterpart `AgentTransactionSweeper` (line 1308), with AD-A05 (line 1570)
   already stating: "Embyr SaaS cannot sweep agent-local transactions (it has
   no direct Postgres access to the customer VPC DB)." This feature is not
   inventing a new component — it is building an already-planned one, and the
   agent-mode exclusion was architected in from the start, not discovered
   during this feature's own DISCUSS.
5. **ADR-041's own "default-provided trait body" precedent** — considered and
   found NOT to transfer to this decision (see Alternatives Considered).

## Decision

### D1 — Raw access mechanism: Option (a), concrete `PostgresBackendAdapter::pool()`, zero trait change

The sweeper resolves a project's DSN (§ D3), builds
`PostgresBackendAdapter::new(&dsn)` (existing constructor, unchanged), and
issues raw SQL directly against `adapter.pool()` (existing accessor,
unchanged — literal reuse, zero new methods on `PostgresBackendAdapter`).

**Zero `BackendAdapter` trait change. Zero `AgentBackendAdapter` change of any
kind** — verifiable via `git diff crates/embyr-server/src/adapters/agent_backend.rs`
showing nothing, mirroring ADR-041/ADR-048's own "verifiable zero blast
radius" discipline. This is stronger than "the new methods are unused" — no
new methods exist for it to (not) implement, because `AgentBackendAdapter` is
never constructed by the sweeper at all: `backend_mode = 'agent'` is excluded
at the `SystemDb` enumeration query itself (§ D4), not filtered per-adapter.

### D2 — Sweep SQL

**Slice 01 (reclaim)** — no bind parameters, the threshold is a compile-time
constant (see rationale below):

```sql
UPDATE transactions
SET status = 'expired'
WHERE status = 'active'
  AND started_at < now() - interval '60 seconds'
```

The `60 seconds` literal is deliberately identical to `commit_transaction`'s
own existing constant (`crates/embyr-pg-storage/src/backend_adapter.rs:937`,
`chrono::Duration::seconds(60)`) — not invented. It is implemented as a named
Rust constant (`const ABANDONMENT_THRESHOLD_SECS: i64 = 60`) in
`transaction_sweeper.rs`, **not** exposed as an environment variable. This is
deliberate: making it independently configurable would let it drift out of
lockstep with `commit_transaction`'s own hardcoded value, silently breaking
the design intent that this sweeper is "a proactive, cross-project
generalization of an already-established semantic, not a new one" (DISCUSS §
System Constraints).

**Slice 02 (purge)** — retention window IS a runtime parameter (bound, not
baked into the SQL string), because unlike the abandonment threshold it has no
sibling constant elsewhere to drift out of sync with — it is a pure
operational tuning knob:

```sql
DELETE FROM transactions
WHERE status IN ('committed', 'expired')
  AND started_at < $1
```

`$1` is computed once per cycle in Rust: `Utc::now() - chrono::Duration::days(retention_days)`,
where `retention_days` comes from `EMBYR_TRANSACTION_RETENTION_DAYS` (default
`30`, mirroring `SessionCleaner`'s own documented 30-day retention precedent,
`docs/product/architecture/brief.md` admin-api-v2 § Background Tasks).

**No new migration.** `migrations/customer/0002_transactions.sql` has no
completion timestamp, only `started_at`. Using `started_at` as the purge
anchor is an accepted approximation: a `'committed'` row's true completion
time is always within `ABANDONMENT_THRESHOLD_SECS` (60s) of `started_at` by
construction (`commit_transaction` itself rejects/expires anything that takes
longer); an `'expired'` row's true terminal time is at most one sweep interval
after crossing the 60s threshold. Both skews are bounded in the tens of
seconds to low minutes; the retention window (default 30 days) is five orders
of magnitude larger. A `terminal_at`/`completed_at` column was considered and
rejected as unrequested schema-migration risk for a skew this immaterial, on a
low-severity bookkeeping table (see ADR context: DISCUSS explicitly framed
this whole feature as "low severity ... pure storage growth").

### D3 — DSN resolution without an api_key

New function `resolve_dsn_without_api_key` (private, `transaction_sweeper.rs`
— not extracted to a shared module now; see § Consequences for the natural
future extraction point):

- `aws_secret` / `gcp_secret`: `AwsSecretFetcher::get_dsn(&arn)` /
  `GcpSecretFetcher::get_dsn(&resource_name)`, unchanged, zero api_key
  involved (both fetchers already internally TTL-cache DSNs per
  ARN/resource-name, so repeated cycles do not necessarily re-fetch over the
  network every time).
- `direct_pg`: `backend_pg_dsn_enc` (new read call site — see ADR-055 for the
  `IS NULL` coverage-gap decision) → `decrypt_with_rotation(&encryption_key,
  encryption_key_previous.as_ref(), &enc)` (existing helper,
  `crates/embyr-server/src/adapters/encryption.rs`, unchanged) →
  `String::from_utf8`.
- Any failure (missing arn/resource-name, fetch error, decrypt/AEAD failure,
  malformed UTF-8, `backend_pg_dsn_enc IS NULL`) → `tracing::warn!` + `None`
  → caller skips the project, continues the cycle. Mirrors
  `CapUsageRefresher`'s own per-account `continue`-on-error discipline
  exactly.

### D4 — Enumeration query

New `SystemDb::list_pg_reachable_projects()`:

```sql
SELECT id, backend_mode, backend_secret_arn, backend_secret_gcp, backend_pg_dsn_enc
FROM projects
WHERE backend_mode IN ('direct_pg', 'aws_secret', 'gcp_secret')
```

Deliberately **no `status` filter**. Sweeping is not a security-sensitive
per-request check the way `authenticate()`'s own `ProjectStatus` gate is; a
`suspended`/`deleted` project's customer DB, if already torn down, simply
fails to connect and is skipped via the existing continue-on-error path — no
new status-branching rule is needed. New row type `SweeperProjectRow` (id,
backend_mode, backend_secret_arn, backend_secret_gcp, backend_pg_dsn_enc),
mirrors `ProjectAuthRow`'s existing shape minus the auth-only fields
(`api_key_hash_*`, `ecies_encrypted_dsn`, agent fields).

### D5 — Cycle shape: ONE cycle-level advisory lock, sequential per-project loop, connect-per-project-per-cycle

Mirrors `CapUsageRefresher` almost exactly, with one wrinkle: the inner loop
body now connects to a DIFFERENT customer database per iteration instead of
querying `SystemDb` only.

- **One advisory lock per cycle** (`pg_try_advisory_lock(fnv1a_hash("embyr_transaction_sweep"))`),
  held on ONE `PoolConnection` acquired from `SystemDb`'s pool across the
  WHOLE cycle (enumerate + every project's sweep), exactly like
  `CapUsageRefresher`'s own lock/unlock pair. **Not** a per-project lock.
- **Sequential, not concurrent, per-project iteration** — a plain `for` loop
  with `.await` per project, no `join_all`/`FuturesUnordered`. This directly
  satisfies DISCUSS's own System Constraint ("never a full concurrent
  fan-out"), for free, as a consequence of not reaching for a concurrency
  combinator — not an extra guard to write.
- **Connect-per-project-per-cycle, no connection caching.** Each project's
  `PostgresBackendAdapter` (5-max-connection pool, existing default,
  unchanged) is constructed, used for its 1-2 sequential statements, and
  dropped (Rust `Drop` closes the pool) before the next project. See
  Alternatives Considered for why this beats caching.
- **Continue-on-error per project** — a connect failure, a query failure, or
  an unresolvable DSN all skip that project and move to the next; no error
  aborts the cycle for other projects (AC, both slices).

### D6 — Metrics

`metrics::counter!("embyr_transaction_sweeper_reclaimed_total").increment(rows_affected)`
/ `"embyr_transaction_sweeper_purged_total"`, incremented by each SQL
statement's own `PgQueryResult::rows_affected()` — never a flat `+1` per
project — so "increments once per row actually transitioned" (both slices'
AC) holds exactly. No `project_id` label (DISCUSS § System Constraints,
mirrors `embyr_rate_limit_requests_total`'s own documented high-cardinality
caution).

### D7 — Composition root wiring

```
TransactionSweeper::spawn(
    system_db: Arc<SystemDb>,
    aws_secret_fetcher: Option<Arc<AwsSecretFetcher>>,
    gcp_secret_fetcher: Option<Arc<GcpSecretFetcher>>,
    encryption_key: [u8; 32],
    encryption_key_previous: Option<[u8; 32]>,
    sweep_interval: Duration,
    retention_days: i64,
) -> tokio::task::JoinHandle<()>
```

New env vars (mirroring `EMBYR_CAP_CHECK_INTERVAL_SECS`'s exact `config.rs`
pattern): `EMBYR_TRANSACTION_SWEEP_INTERVAL_SECS` (default `300` — 5 minutes;
sequential sweeping at this cadence is trivial background load even at the
documented ~10,000-project ceiling, per `brief.md`'s own capacity section),
`EMBYR_TRANSACTION_RETENTION_DAYS` (default `30`). `main.rs` spawns
`_transaction_sweeper` immediately after `_cap_usage_refresher`, reusing
`Arc::clone(&system_db)` and `cfg.encryption_key`/`cfg.encryption_key_previous`
(already parsed and threaded elsewhere — zero new secret plumbing).

**Finding, named explicitly, not silently absorbed**: `main.rs`'s own
`FirestoreService` construction today hardcodes `aws_secret_fetcher: None,
gcp_secret_fetcher: None` (lines 164-165) — `aws_secret`/`gcp_secret`
backend_mode DSN resolution is **not actually wired for the live gRPC
request-serving path in production today either**. This is a pre-existing
gap, confirmed by direct reading, unrelated to and out of scope for this
feature. This feature's own sweeper wiring is independent (its own
`Option<Arc<...>>` params, constructed fresh in `main.rs` using the identical
pattern already used in `config.rs::fetch_from_secret_manager`) and does not
worsen that gap — but any deployment where these fetchers remain
unconfigured will see the sweeper silently skip every `aws_secret`/
`gcp_secret` project, via the same continue-on-error path, mirroring the
pre-existing request-path gap's own shape. `GcpSecretFetcher::new` requires a
bearer token; reusing `EMBYR_GCP_ACCESS_TOKEN` (already read in `config.rs`
for a different purpose — admin-key/encryption-key secrets-manager bootstrap)
is the pragmatic zero-new-config-surface choice, provided operator IAM scope
covers customer-project secrets too (an operational concern, same class
already flagged as OQ-01 in `brief.md`'s original architecture for
`GcpSecretFetcher` generally — not re-litigated here).

## Alternatives Considered

### D1 alternative — `BackendAdapter` trait methods (`reclaim_orphaned_transactions`/`purge_old_transactions`)

Rejected. `BackendAdapter`'s trait purpose is backend-POLYMORPHIC
document-CRUD/query/transaction dispatch, resolved at PER-REQUEST time
(`Arc<dyn BackendAdapter>`, chosen dynamically by `authenticate()` based on
whichever project is calling). The sweeper's own dispatch is structurally
different: it filters to PG-reachable `backend_mode`s BEFORE constructing any
adapter, at the `SystemDb` enumeration query (§ D4) — it never constructs an
`AgentBackendAdapter` and never needs runtime polymorphism across adapter
types for this operation.

ADR-041's own "default-provided trait body returning an existing `CoreError`
variant" precedent was explicitly considered as a way to avoid forcing
`AgentBackendAdapter` to hand-write a rejection — but that precedent applies
where genuine per-request runtime polymorphism exists (`RunAggregationQuery`
IS dispatched via `dyn BackendAdapter` depending on which project calls it,
so a default body is reachable code on a real call path). It does not
transfer here: even a default-body trait method would sit as permanently
dead, un-exercised surface on `AgentBackendAdapter`, for zero abstraction
benefit, and would falsely imply agent-mode COULD structurally support
sweeping — when `brief.md`'s own original architecture (AD-A05) already
establishes it categorically cannot (no direct Postgres access to the
customer VPC DB at all).

### D5 alternative — per-project advisory locks

Rejected. The reclaim/purge SQL is naturally idempotent — two instances
concurrently running the identical `UPDATE`/`DELETE` against the same
customer DB converge to the same end state regardless of interleaving. The
only actual race the lock needs to prevent is DOUBLE-COUNTING the Prometheus
counters across simultaneously-racing instances, which a single cycle-level
lock already fully prevents — exactly as `CapUsageRefresher`'s own doc
comment already establishes for the structurally analogous "avoid redundant
computation across instances, not required for correctness" concern.
Per-project locks would require inventing N lock keys (derived from
`project_id`) for no correctness gain.

### D5 alternative — cache connections across cycles

Rejected. The sweeper cannot reuse `CredentialCache`: its cache key
structurally requires `api_key_blake3`, which the sweeper never has, by
design (DISCUSS § System Constraints: "the sweeper must never hold a live
api_key"). Building a NEW parallel DSN-keyed pool cache is unrequested
complexity for a background task running once per multi-minute interval,
where a few-ms TCP+auth connect cost is immaterial — and connect-per-cycle
directly serves the documented per-instance Postgres connection-ceiling
caution (`brief.md`'s own capacity section) by never holding N idle
customer-DB connections between cycles.

## Consequences

**Positive**: zero new trait surface; zero touched `AgentBackendAdapter`
(verifiable, not just claimed); reuses two already-existing accessors
(`PostgresBackendAdapter::pool()`, `decrypt_with_rotation`) and one
near-identical existing pattern (`resolve_customer_db_adapter`) almost
verbatim; the sweep-interval/sequential-loop shape structurally satisfies the
"never full concurrent fan-out" constraint without any extra guard code;
metrics are exact (row-count-based), not approximate.

**Negative**: the sweeper's own raw SQL against `transactions` is invisible
in `BackendAdapter`'s own trait surface — a future reader auditing "everything
`PostgresBackendAdapter` can do" via the trait alone would miss it; mitigated
by this ADR plus inline doc comments at the `pool()` call site, mirroring
`SystemDb::pool()`'s and `PostgresBackendAdapter::pool()`'s own existing
"expose sparingly, prefer typed methods" doc-comment discipline.
`resolve_dsn_without_api_key` is NOT extracted to a shared module now — if
`TombstoneSweeper`/`DeletedProjectSweeper` (both already named in `brief.md`'s
original `embyr-server::sweepers` component table, line 890, never built)
are eventually delivered, they will want the identical api-key-free DSN
dispatch; that is the natural extraction point, named here as a follow-up,
not built speculatively now (YAGNI).
