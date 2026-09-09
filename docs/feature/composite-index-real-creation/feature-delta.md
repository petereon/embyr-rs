# Feature Delta: composite-index-real-creation

## Wave: DISCUSS / [REF] Prior Wave Consultation — Reading Confirmation

✓ `docs/product/production-readiness-audit-2026-09-08.md` finding #5 confirmed by direct reading:
"Composite-index feature never creates a real Postgres index — `IndexManager`/`create_composite_index`
only read/write a metadata row (`status='ready'` set on INSERT, no build step). Zero GIN/expression
index exists on the `fields` JSONB column; every 'index ready' filtered/sorted query is actually a
full sequential scan." Location cited: `crates/embyr-server/src/adapters/index_manager.rs:19-31`;
`crates/embyr-server/src/admin/handlers/composite_indexes.rs:118-123`;
`migrations/customer/0001_documents.sql:12-14`. Severity: **Blocker**. Status (at DISCUSS start):
**Not started**.
✓ `crates/embyr-server/src/adapters/index_manager.rs` (full, 32 lines) — `IndexManager::
is_index_ready(project_id, collection_id)` runs a `SELECT count(*) FROM composite_indexes WHERE
project_id = $1 AND collection_path = $2 AND status = 'ready'` against the **system DB** only — a
pure metadata-existence check, unchanged since it was built. Confirms directly: nothing in this
function ever touches a customer database or issues DDL.
✓ `crates/embyr-server/src/admin/handlers/composite_indexes.rs` (full, 266 lines) — `create_composite_
index` inserts a `composite_indexes` row via `INSERT ... VALUES ($1, $2, $3::jsonb, 'ready') ON
CONFLICT ... DO UPDATE ...` (lines 118-123) — `status` is hardcoded to the literal `'ready'` in the SQL
text itself, unconditionally, on every call. `list_composite_indexes`/`delete_composite_index` also
operate exclusively against `state.system_db.pool()` — confirmed, no code path in this file ever opens
a connection to a customer database.
✓ `migrations/customer/0001_documents.sql` (full, 14 lines) — the ONLY real index on `documents` is
`documents_project_collection_idx ON documents (project_id, collection_path) WHERE NOT deleted` — a
plain btree over two scalar columns, zero JSONB coverage of the `fields` column any composite-index
filter/orderBy predicate actually touches.
✓ `crates/embyr-server/src/grpc/handler.rs` lines 3190-3245 (`handle_run_query`'s composite-index gate)
re-read directly, confirming the task's own framing precisely: `requires_composite_index`/`is_index_
ready` run strictly BEFORE `adapter.run_query(...)` (line 3242) and gate ONLY on whether a READY
metadata row exists — the real query then executes via `run_query` **unconditionally**, regardless of
whether any physical Postgres index backs it. This is a Firestore-PARITY simulation gate, not a
query-planning necessity — confirmed by direct read, not assumed (matches `composite-index-
requirement-rules`'s own identical finding, re-confirmed here independently).
✓ `crates/embyr-pg-storage/src/encoding/query.rs` (`append_field_filter` lines 45-130+, `order_by_expr`
lines 335-346) read directly to verify the task's own request to check for a hidden dynamic
index-hint mechanism. Confirmed: filter predicates are built as JSONB text-extraction expressions —
`fields->'{field}'->>'v' {op} {bound value}` (scalar comparisons), with `push_value_equality`/`push_
array_contains`/etc. for other operators — and `order_by_expr` builds `fields->'{field}'->>'v' {ASC|
DESC}` as raw (non-parameterized) SQL text; a separate `order_by_expr_bigint` helper exists for an
explicit `::bigint` cast variant. **No dynamic index-hint, query-rewrite, or index-selection mechanism
of any kind exists anywhere in this crate** (confirmed by reading the full query-building module, not
merely grepping) — Postgres's own query planner picks whatever plan the ACTUAL indexes present on
`documents` support, exactly as the task's own framing predicted. This directly confirms the nuance:
finding #5 is a pure PERFORMANCE gap (every filtered/sorted query already executes correctly today via
a full scan or whatever the planner picks from the ONE existing btree above), not a correctness bug —
the `FAILED_PRECONDITION` Firestore-parity gate is unaffected by this feature and continues to work
exactly as `composite-index-requirement-rules` left it.
✓ `crates/embyr-core/src/domain/query.rs::validate_field_path` (lines 16-24) read directly — the
existing charset gate (`^[a-zA-Z_][a-zA-Z0-9_.]*$`) audit finding #26 names as "the one gate" the
entire query-filter path already funnels every `field_path` through before SQL interpolation. This is
the direct reuse candidate for sanitizing the SAME `field_path` values (`CreateCompositeIndexBody.
fields[].field`, already user-supplied via `firestore-composite-indexes-admin-api`'s own shipped
request body) before they are ever interpolated into a DDL statement — DDL identifiers/expressions
cannot be bound via `push_bind` in Postgres at all (unlike WHERE-clause values), so this is a
different, and in some ways harder, injection surface than the one `validate_field_path` was
originally built to close; named explicitly as an open question for DESIGN (§ Central Design
Questions, Question A), not silently assumed to be already solved by reuse alone.
✓ `crates/embyr-server/src/sweepers/transaction_sweeper.rs` (full, 344 lines) read directly — the
**only existing precedent in this codebase for embyr-server opening a connection directly to a
CUSTOMER database without holding a live api_key** (matches this feature's own need exactly: `Create
Index`'s admin-session auth context, like the sweeper's own background-task context, never has an
api_key in scope). Confirms three directly-reusable pieces: (1) `resolve_dsn_without_api_key` — a
per-`backend_mode` (`direct_pg`/`aws_secret`/`gcp_secret`) DSN-resolution function already handling
`AwsSecretFetcher`/`GcpSecretFetcher`/`decrypt_with_rotation` correctly, returning `None` (never a
panic) on any failure; (2) `PostgresBackendAdapter::new(&dsn)` — the existing one-off-connection-pool
constructor the sweeper already uses to reach a customer DB; (3) `backend_mode = "agent"` is excluded
at the caller level (never reaches a DSN-resolution branch), matching the established "agent-mode is a
structurally different wall" pattern from `JOB-01`'s own agent-mode NOTEs. This is a strong reuse
candidate for THIS feature's own DDL-execution path, flagged in § Central Design Questions (Question
C) rather than locked, since the sweeper's own function is currently a periodic-enumeration helper
(`sweep_one_project`, called from an interval loop over EVERY reachable project) and this feature needs
a per-request, ONE-project, one-shot action — the underlying DSN-resolution/connect logic is the reuse
target, not necessarily the sweeper's own interval/enumeration wrapper.
✓ `migrations/0003_composite_indexes.sql` (referenced via `firestore-composite-indexes-admin-api`'s
own feature-delta.md, § Reading Confirmation there) — `composite_indexes.status` is a plain `VARCHAR
DEFAULT 'ready'`, no CHECK constraint restricting its value set. Confirms this feature needs a genuine
schema change (at minimum, a documented value set covering "not yet real," "real and working," and "a
build attempt failed" — exact naming is DESIGN's call) to correctly represent a build lifecycle that
does not exist in the schema today.
✓ `docs/feature/firestore-composite-indexes-admin-api/feature-delta.md` read in full. Confirmed
Resolution 3 ("metadata-only, locked... Real index provisioning for query performance is a separate,
unevidenced future concern... named, deferred, out of this feature's own scope") and its own § Out of
Scope entry ("Actual Postgres index provisioning for query performance — this feature is metadata-only
... no domain example or metric in this codebase currently shows composite-query latency as a problem.
Named, deferred, no candidate feature id assigned.") — **this feature is that deferred item, now
assigned a candidate id.** Also confirmed Resolution 4 ("no dependency-safety check on `DeleteIndex`")
and Resolution 5 ("idempotent duplicate-create") are unchanged, reused verbatim by this feature — no
contradiction, this feature layers onto them, does not re-litigate them.
✓ `docs/feature/composite-index-requirement-rules/feature-delta.md` read in full. Confirmed its own
identical § Out of Scope entry ("Real Postgres index provisioning for query performance — unchanged
from `firestore-composite-indexes-admin-api`'s own identical Out-of-Scope entry; this feature widens
the GATE only, never touches `run_query`'s own already-fully-functional execution") and its own
Discovered Gap (`append_field_filter` does not support every `FilterOp` — `In`/`NotIn`/`ArrayContains`/
`ArrayContainsAny` panic) — since resolved by the later `firestore-query-filter-operator-support`
feature per `MEMORY.md`; confirmed via the same `encoding/query.rs` read above that all these operators
now have real match arms (lines 97-130+), so this feature's own EXPLAIN-based usage proof (§ US-01)
is not blocked by that historical gap.
✓ `docs/product/jobs.yaml` read in full (all 20 jobs). JOB-01 (`sdk-compat`, persona P1 Alex) already
carries two directly-relevant NOTEs from the two prior composite-index features (§ Persona & Job below)
— both explicitly name this exact gap as deferred future scope under the same job. No candidate job
fits better: JOB-11/JOB-12 (Sam Chen's fairness/observability jobs) are about cluster-wide rate-fairness
and metrics respectively, neither of which this finding touches; JOB-13 (`production-deployment`) is
about safe, fail-fast STARTUP/request-admission-time configuration (per `realtime-listener-reconnect`'s
own explicit reasoning against reusing JOB-13 for a background-mechanism concern) — this finding is
about whether a query that IS admitted then executes efficiently, squarely JOB-01's own "behave
identically to real Firestore" territory (real Firestore's composite indexes exist precisely to make
admitted compound queries fast, not merely to gate them).
✓ `docs/feature/realtime-listener-reconnect/feature-delta.md` and
`docs/feature/stripe-webhook-body-limit/feature-delta.md` read in full to confirm this project's own
established single-file, Tier-1-only `feature-delta.md` convention (`## Wave: DISCUSS / [REF]
{Section}` headings, no standalone `acceptance-criteria.md`/`outcome-kpis.md`/`story-map.md` files),
its "lock what evidence answers, flag genuine trade-offs as Central Design Questions for DESIGN" DISCUSS
discipline, and its "four/three tightly-coupled facets can be one story, or several independently
-demonstrable slices, depending on whether each is separately shippable" story-sizing judgment — applied
directly to this feature's own story split (§ Story Map & Walking Skeleton).

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

- Feature type: **Backend / Reliability-performance fix** — layers real DDL provisioning onto an
  existing, already-shipped, unmodified admin CRUD surface and an existing, unmodified Firestore-parity
  gate; zero SDK/data-plane proto change.
- JTBD: **reuse JOB-01** (`sdk-compat`, P1 Alex) — the "make it real" pattern, the third feature in this
  exact composite-index sequence (admin CRUD surface → requirement-detection accuracy → now, real
  provisioning), each explicitly flagged as the next deferred item by the one before it.
- Walking Skeleton: **Yes** (US-01, § Story Map) — the single riskiest, highest-value assumption: can a
  real `CREATE INDEX` be safely built from user-supplied field paths and executed against a real
  customer database, end-to-end, with Postgres's own planner then actually using it.
- UX Research Depth: **Lightweight** — an admin-API-internal provisioning mechanism layered onto an
  already-shipped 3-verb CRUD surface, one already-fully-profiled persona (Alex, 10+ prior features),
  no new emotional arc, no new TUI/journey artifact warranted (matches both sibling composite-index
  features' own precedent).
- **This DISCUSS deliberately does NOT resolve the four genuinely open engineering trade-offs the task
  itself names** (SQL-injection-safe DDL construction mechanism; `CREATE INDEX CONCURRENTLY` vs. plain;
  the async-build mechanism/status-lifecycle shape; the exact Postgres index DDL shape) — each is
  recorded as a Central Design Question for DESIGN, with reading-derived facts and reuse candidates
  offered, per this session's own established `realtime-listener-reconnect` precedent for "lock the
  user-facing outcome, flag the mechanism."

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: P1 Alex (SDK Developer), primary — unchanged, matches both prior composite-index features.
**Secondary stakeholder**: P2 Sam Chen (Service Operator), named explicitly for US-02 (non-blocking
build on a large, live collection) — the production-safety concern of "does creating an index break my
running deployment" is squarely Sam Chen's own operational worldview, mirroring the exact
`persona`/secondary-stakeholder split `realtime-listener-reconnect` established for JOB-03 (a job whose
primary recorded persona is Alex, but whose specific failure mode is discovered and suffered by Sam
Chen operating the deployment).

**Job**: **JOB-01 `sdk-compat`**, unchanged job_story, third realization in this exact sequence. JOB-01
already carries two directly on-point NOTEs:
1. `firestore-composite-indexes-admin-api`'s NOTE, closing the "no way to ever create an index" gap.
2. `composite-index-requirement-rules`'s NOTE, closing the "the gate doesn't catch every documented
   trigger shape" gap.

Both NOTEs' own feature-delta.md documents independently and explicitly deferred THIS exact gap ("real
Postgres index provisioning for query performance... no domain example or metric in this codebase
currently shows composite-query latency as a problem... named, deferred, no candidate feature id
assigned") — this feature is that assignment. This feature's own realization of JOB-01: an index Alex
creates via `CreateIndex` today satisfies the `FAILED_PRECONDITION` gate and detects the correct trigger
shape (both already real, per the two prior features) — but the underlying query it unblocks still
executes as a full sequential scan forever, which is not how real Firestore's own composite indexes
behave (they exist specifically to make the now-admitted query fast). After this feature, `CreateIndex`
provisions a real, working Postgres index, and Alex's query is measurably faster once it exists — not
merely no-longer-rejected.

## Wave: DISCUSS / [REF] Business Context

Real Firestore's composite indexes serve two distinct purposes: (1) a query-admission GATE (compound
queries are rejected until a matching index exists) and (2) the actual PERFORMANCE MECHANISM that makes
an admitted compound query fast at scale. This codebase's prior two features built (1) completely and
correctly. This feature closes (2) — the gap the production-readiness audit's finding #5 names.

Today, `create_composite_index` (`composite_indexes.rs:118-123`) writes ONLY a `composite_indexes`
metadata row to the **system DB**, with `status` hardcoded to the literal `'ready'` in the SQL text
itself — no build step of any kind ever runs. `IndexManager::is_index_ready` reads that same row. Real
query execution (`adapter.run_query(...)`, `handler.rs:3242`) proceeds unconditionally once the gate
passes, against whatever indexes ACTUALLY exist on the customer's `documents` table — today, only the
one pre-existing `(project_id, collection_path)` partial btree (§ Reading Confirmation), which provides
zero coverage for any `fields->'x'->>'v'` predicate a composite-index query needs. Every "index ready"
filtered/sorted query is, and remains, a full sequential scan of that project's entire `documents`
table for that collection — silently, since nothing observes or reports this today (confirmed: zero
`EXPLAIN`-based test, zero query-latency metric, anywhere in this codebase for the `RunQuery` path).

**This is confirmed, independently, to be a pure PERFORMANCE gap, not a correctness bug** (§ Reading
Confirmation) — the Firestore-parity `FAILED_PRECONDITION` gate this feature layers onto is unaffected
and continues to work exactly as `composite-index-requirement-rules` left it; no dynamic index-hint or
query-rewrite mechanism exists anywhere in `embyr-pg-storage` that this feature could accidentally
break or duplicate.

### Central Design Questions (opening recommendations offered, not locked)

**Question A — SQL-injection-safe DDL construction.** `field_path` values arrive as user-supplied
strings via `CreateCompositeIndexBody` (already-shipped, unchanged shape). Postgres cannot bind DDL
identifiers/expressions via `push_bind` the way `append_field_filter` already safely binds WHERE-clause
VALUES — a `CREATE INDEX ... ON documents ((fields->>'{field_path}'), ...)`-shaped statement requires
the `field_path` text itself to be interpolated into DDL. **Opening recommendation**: reuse `embyr_
core::domain::query::validate_field_path` (the SAME charset gate — `^[a-zA-Z_][a-zA-Z0-9_.]*$` —
audit finding #26 already names as "the one gate" the query-filter path funnels every field_path
through) to reject any `field_path` outside that charset BEFORE any DDL text is built, for every field
in the request. This closes the injection surface (the accepted charset contains no quote, semicolon,
comment-sequence, or SQL-metacharacter of any kind) without inventing a second, parallel validator —
but DESIGN should confirm this charset is sufficiently AND correctly restrictive for DDL-expression
context specifically (not merely WHERE-clause-value context, a different call site with a different
threat shape), and should decide the exact quoting/escaping wrapper Rust code around the accepted text
(e.g., whether nested JSON path segments like `a.b.c` need per-segment handling in the generated
expression).

**Question B — `CREATE INDEX CONCURRENTLY` vs. plain `CREATE INDEX`.** Plain `CREATE INDEX` runs inside
an implicit transaction and briefly locks the table against writes; `CONCURRENTLY` avoids that lock but
cannot run inside a transaction, takes substantially longer, and can leave an `INVALID` index behind if
it fails partway. This DISCUSS locks the OUTCOME (a large, live collection's writes must not be blocked
by index creation — AC-CXR-06) without picking the mechanism. `CONCURRENTLY` is the evidently-necessary
choice to satisfy that outcome for large collections (a plain `CREATE INDEX` structurally cannot, by
Postgres's own documented locking behavior) — named as the strong default expectation, not a hard lock,
since DESIGN may find a reason (e.g. a size threshold below which plain `CREATE INDEX`'s brief lock is
acceptable and simpler) this DISCUSS did not evidence.

**Question C — the async-build mechanism and status-lifecycle shape.** Today's `status` is a bare
`VARCHAR DEFAULT 'ready'` with no CHECK constraint and no build step (§ Reading Confirmation) — this
feature needs `status` to genuinely represent "not yet real," "real and working," and "a build attempt
failed," at minimum. The task's own framing asks specifically whether building a whole job-queue
infrastructure is warranted, or whether a `tokio::spawn`'d task fired at `CreateIndex` time (mirroring
this codebase's own already-existing `TransactionSweeper`/`CapUsageRefresher` background-task shape,
minus their periodic-interval/multi-project-enumeration wrapper — this is a per-request, one-shot,
single-project action, not a periodic sweep) is the pragmatic, sufficiently-correct fix. **Opening
recommendation**: reuse `transaction_sweeper.rs`'s own `resolve_dsn_without_api_key`-shaped DSN
resolution (§ Reading Confirmation — the only existing precedent for embyr-server reaching a customer
DB without a live api_key, matching `CreateIndex`'s own session-based admin auth) plus `PostgresBackend
Adapter::new(&dsn)` inside a `tokio::spawn`'d one-shot task fired synchronously from `create_composite_
index`'s own handler body, with the row's own `status` column tracking the transition — NOT a new
generic job-queue (no other feature in this codebase needs one yet; ponytail discipline: reuse an
existing shape before inventing a new mechanism class). DESIGN should confirm this against the actual
concurrency/crash-recovery needs (e.g., what happens to a `'building'` row if `embyr-server` restarts
mid-build — a genuinely separate question this DISCUSS does not evidence a need to solve, since no
domain example currently shows a customer being harmed by a stale `'building'` row surviving a restart
beyond it needing to eventually become observably distinguishable from `'ready'`, which AC-CXR-07
already requires).

**Question D — the real Postgres index DDL shape.** § Reading Confirmation directly confirms
`append_field_filter`/`order_by_expr` build predicates as `fields->'{field}'->>'v' {op}` (text
extraction) and orderBy as the same shape, with a SEPARATE `order_by_expr_bigint` helper for an explicit
`::bigint`-cast variant used elsewhere for integer fields. **Postgres only uses an expression index when
a query's own expression syntactically matches the indexed expression** — so the index DDL must be built
from the SAME expression form the query path actually emits for each field's runtime value shape, not a
generic guess. This DISCUSS does not know, and did not investigate, whether every field in a composite
index spec is uniformly text-compared or whether some fields need the `::bigint`-cast form to actually
be used by the planner for numeric-typed data — a genuine open question DESIGN must resolve by tracing
`append_field_filter`'s full match arms against the exact value types real composite-index queries use,
not by assumption. A plain multi-column expression btree (mirroring the ordering `missing_index_fields`
already computes — filtered fields first, then orderBy fields, per `composite-index-requirement-rules`'
own DESIGN) is the evidently-correct family (a GIN index serves containment/array queries, not the
equality/range/sort predicates these composite-index triggers are about) — named as the strong default,
not locked, since DESIGN should confirm against the exact operator mix each trigger shape produces.

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

**Signals checked**: >10 user stories? No (3). >3 bounded contexts/modules? No — confined to
`embyr-server`'s own admin handler (`composite_indexes.rs`), a new DDL-construction/execution module
(new file, `embyr-server`), and a `composite_indexes` schema migration (new `status` value set / CHECK
constraint) — zero change to `embyr-core`, `embyr-pg-storage`'s `run_query`/`encoding/query.rs`
(confirmed: this feature builds a SEPARATE DDL-generation path informed by, but not executing through,
the existing WHERE/ORDER-BY query-building functions), or `handler.rs`'s own `RunQuery` gate (`requires_
composite_index`/`is_index_ready` remain unmodified — this feature only makes the row they already read
truthful). Walking skeleton >5 integration points? No (3): (1) a real `CreateIndex` call against a real
customer database producing a real, verifiable `pg_indexes` entry; (2) an `EXPLAIN` of a real,
previously-full-scan `RunQuery` now showing an index-based plan; (3) a real field-path value outside the
accepted charset rejected before any DDL executes. Estimated effort >2 weeks? No — 3 slices; the two
genuinely novel engineering mechanisms (safe dynamic DDL construction; non-blocking large-collection
build) are each independently scoped and each smaller than `realtime-listener-reconnect`'s own single
2-3-day story; total estimate 3-4 days across all 3 slices, well under 2 weeks. Multiple independent
user outcomes? The 3 stories are each independently demonstrable (§ Story Map) but all serve the SAME
single "the composite index Alex creates is a real, working, safely-built index, and stays that way
through delete" outcome — never separately-pitchable features.

**Scope Assessment: PASS** (0 oversizing signals fired) — right-sized as one feature, 3 stories.

## Wave: DISCUSS / [REF] Story Map & Walking Skeleton

### Backbone

Alex's query is admitted (gate already real) but silently runs as a full scan forever (today's bug) →
Alex creates the SAME index he already could via `CreateIndex` (unchanged request shape) → this time a
real Postgres index is built, and `status` only claims `'ready'` once it genuinely is → the SAME query,
re-run, is provably using the new index (not merely no-longer-rejected) → for a large, actively-written
collection, that build never blocks Sam Chen's own production writes → if Alex later deletes the index,
the real underlying Postgres index goes with it, not just the metadata row.

### Walking Skeleton

**US-01**: real DDL creation + truthful status + provable usage, for the ordinary (small/typical)
collection case — the single riskiest, highest-value assumption: can dynamic DDL be built safely from
user input and actually get used by Postgres's own planner, end-to-end, with zero change to the
existing, unmodified `RunQuery` gate or `run_query` execution path.

### Release 1 — The Index Is Real, and Safe to Build on a Live Production Collection (US-01, US-02)

Both are necessary for this feature to actually close finding #5 as a Blocker, not merely as a
best-case demo — a synchronous, blocking `CREATE INDEX` would reproduce the audit's own "looks done but
isn't wired to the real thing" pattern for any customer at meaningful scale, exactly the failure mode
this whole audit exists to eliminate.

### Release 2 — Deleting an Index Doesn't Leave an Orphaned Real Index Behind (US-03)

Sequenced last: deleting is meaningless before creation is real (mirrors `firestore-composite-indexes-
admin-api`'s own identical create-before-delete sequencing rationale), and an orphaned real index is a
slow-burn resource/write-overhead cost, not an immediate correctness or safety risk — lower urgency than
Release 1's two facets.

## Wave: DISCUSS / [REF] Elephant Carpaccio Slices

| # | Story | Release | Estimate | Learning Hypothesis | Reference Class |
|---|---|---|---|---|---|
| 01 | US-01 | 1 (Walking Skeleton) | 1.5-2 days | Disproves: dynamic, user-input-driven DDL construction cannot be made injection-safe and actually usable by Postgres's planner without a materially larger mechanism than reusing `validate_field_path` | New mechanism class for this codebase — nearest reference class is `transaction_sweeper`'s own "connect directly to a customer DB without an api_key" precedent, combined with the query-filter path's own `validate_field_path` reuse |
| 02 | US-02 | 1 | 1-1.5 days | Disproves: a genuinely non-blocking, status-truthful build (CONCURRENTLY-shaped) needs a new job-queue class of infrastructure beyond a `tokio::spawn`'d one-shot task, mirroring this codebase's own existing background-task shape | Mirrors `TransactionSweeper`/`CapUsageRefresher`'s own already-proven `tokio::spawn` background-task pattern, narrowed to a one-shot per-request action |
| 03 | US-03 | 2 | 0.5 day | Disproves: dropping the real index on delete needs new DSN-resolution/connection machinery beyond what US-01/US-02 already built — confirmatory, reuses the same connection path | Mirrors `firestore-composite-indexes-admin-api`'s own `delete_composite_index`'s already-simple shape, extended with the same customer-DB connection US-01 introduces |

## Wave: DISCUSS / [REF] Prioritization

Ordered by genuine dependency and by what actually closes the audit's own Blocker classification: US-01
first — proves the single most uncertain, highest-risk mechanism (safe dynamic DDL against a real
customer database) with no prior precedent in this codebase to lean on beyond the sweeper's own
DSN-resolution helper. US-02 second — cannot be demonstrated or even meaningfully designed before US-01's
own DDL-construction mechanism exists, and is equally necessary to actually close finding #5 for any
customer at real scale (a synchronous-only fix would misrepresent the finding as closed while still
being unsafe for the exact "large collection" scenario the finding's own audit language names). US-03
last — depends on US-01/US-02's own customer-DB connection mechanism existing, and is the lowest-urgency
of the three (an orphaned real index is a resource-hygiene concern, not a Blocker-severity risk on its
own).

## Wave: DISCUSS / [REF] System Constraints

- Zero change to `crates/embyr-server/src/grpc/handler.rs`'s own `RunQuery`/`requires_composite_index`/
  `is_index_ready` call sites, and zero change to `crates/embyr-pg-storage/src/encoding/query.rs`'s
  existing WHERE/ORDER-BY builders — this feature is purely upstream (a new DDL-construction/execution
  path informed by, but not sharing code with, the existing query-execution builders) and purely
  downstream (making the `composite_indexes.status` row those unmodified call sites already read
  truthful).
- `composite_indexes.status`'s value set widens beyond the current bare `'ready'` default — a real
  schema/migration change is required; the exact value set (naming, whether a CHECK constraint is added)
  is DESIGN's own choice.
- DDL identifiers/expressions cannot be bound via `push_bind` — every `field_path` must be validated
  through a charset gate BEFORE interpolation into DDL text (§ Central Design Questions, Question A);
  this is a hard requirement, not a DESIGN preference.
- `backend_mode = agent` is out of scope for real DDL provisioning — `embyr-server` has no existing
  mechanism to open a direct Postgres connection to an agent-fronted customer database (mirrors the
  already-established "agent-mode is a structurally different wall" pattern from `realtime-listener-
  reconnect`/JOB-01's own agent-mode NOTEs); `CreateIndex` against an agent-mode project should continue
  to behave sanely (DESIGN's call on the exact response — e.g. a clean rejection vs. metadata-only
  fallback — but must not silently claim `'ready'` for a project this feature cannot actually reach).
- No new external dependency — DDL execution reuses `sqlx` (already pinned) and the existing `Postgres
  BackendAdapter`/DSN-resolution primitives already proven by `transaction_sweeper.rs`.
- Mutation-testing lesson, reapplied from this session's own accumulated discipline (4c/4d/4e,
  `composite-index-requirement-rules`, `realtime-listener-reconnect`): any new PURE function this
  feature adds (e.g. the DDL-string-construction function, the field-path-validation wrapper) gets unit
  tests written DURING DELIVER; a `cargo-mutants` pass is still budgeted at QUALITY_GATE regardless.

## Wave: DISCUSS / [REF] User Stories

### US-01: Alex's Composite Index Is a Real, Working Postgres Index — Not Just a Metadata Row (Walking Skeleton)

**job_id**: JOB-01 | **Release**: 1 | **Persona**: P1 Alex

#### Elevator Pitch
Before: `POST /admin/v1/projects/trailmark-prod/indexes` with `{"collection_path": "products",
"fields": [{"field": "category", "order": "ASC"}, {"field": "score", "order": "DESC"}]}` returns 200
with `status: "ready"` immediately — but zero Postgres index actually exists; Alex's previously-rejected
`RunQuery` now succeeds only because the gate is satisfied, and executes as a full sequential scan of
every document in `products` every single time, forever, with no way for Alex to know that.
After: run the identical `CreateIndex` call → sees the same 200 response shape (unchanged, per §
Reading Confirmation), but `status` reaches `"ready"` only once a real Postgres index has actually
finished building against `trailmark-prod`'s own database — and re-running the identical, previously
-full-scan `RunQuery` now shows (via `EXPLAIN`) Postgres's own planner choosing an index-based plan.
Decision enabled: Alex trusts that creating an index makes his query genuinely fast, not merely
no-longer-rejected — he does not need to separately verify or work around a scan that "succeeded" but
never actually got faster.

#### Who
- Alex (P1) | SDK developer whose app issues a compound filter+orderBy query against a real customer
  project (e.g. Trailmark, `trailmark-prod`, `backend_mode=direct_pg`) | Already trusts (per the two
  prior composite-index features) that `CreateIndex` unblocks his query's admission | Needs that same
  call to also make the query fast, matching real Firestore's own behavior, not merely satisfy a gate.

#### Solution
`CreateIndex` builds and executes a real `CREATE INDEX` DDL statement against the CUSTOMER database
(not the system DB) for the requested `(collection_path, fields)` spec, using field-path values
validated through the existing `validate_field_path` charset gate before any DDL text is constructed.
The `composite_indexes.status` row transitions to a genuinely truthful `'ready'` state only once that
real index exists and is usable — never synchronously on INSERT before any build step has run. The
exact DDL construction/quoting mechanism and status-transition timing for THIS (ordinary-collection)
case are DESIGN's own investigation (§ Central Design Questions A, C, D).

#### Domain Examples

**Example 1 (Happy Path — a real index gets built and used)**: Trailmark (`trailmark-prod`,
`backend_mode=direct_pg`) has a `products` collection with a few thousand documents. Alex's app issues
`db.collection('products').where('category', '==', 'electronics').orderBy('score', 'desc')` — a query
`composite-index-requirement-rules` already correctly rejects `FAILED_PRECONDITION` until an index
exists. Alex calls `CreateIndex` with `{"collection_path": "products", "fields": [{"field": "category",
"order": "ASC"}, {"field": "score", "order": "DESC"}]}`. Before this feature: the row is `'ready'`
instantly, but the re-run query is a full scan of every `products` document for `trailmark-prod`. After
this feature: a real Postgres index is built against `trailmark-prod`'s own database, `status` becomes
`'ready'` once it genuinely exists, and `EXPLAIN`ing the re-run query shows an index-based plan, not a
sequential scan.

**Example 2 (Edge Case — a field path attempting SQL injection is rejected, not executed)**: A malformed
or malicious `CreateIndex` request names a field as `category"; DROP TABLE documents; --` instead of a
well-formed field name. Before this feature: this scenario cannot occur, because no DDL is ever built
from `field_path` at all. After this feature: the request is rejected before any DDL text is
constructed or executed — `validate_field_path`'s own existing charset (`^[a-zA-Z_][a-zA-Z0-9_.]*$`)
does not accept the quote/semicolon/comment sequence, so no malformed or dangerous statement is ever
sent to the customer database.

**Example 3 (Error/Boundary — the customer database is unreachable mid-request)**: Trailmark's own
Postgres instance is briefly unreachable (a transient network blip) at the moment Alex calls
`CreateIndex`. Before this feature: this scenario cannot meaningfully fail, since no real connection to
the customer database is ever attempted. After this feature: the row's own `status` reflects that the
real index was not actually built — never silently reported `'ready'` when it is not, and never left
ambiguous between "still working on it" and "genuinely failed" (mirrors the same distinguishability
requirement `realtime-listener-reconnect` established for a different failure surface).

#### UAT Scenarios (BDD)

```gherkin
Scenario: Creating a composite index results in a real Postgres index existing on the customer database
  Given Trailmark's project "trailmark-prod" has a "products" collection with real documents
  When Alex calls CreateIndex for collection_path "products" with fields
    [{"field": "category", "order": "ASC"}, {"field": "score", "order": "DESC"}]
  Then a real Postgres index matching those fields exists on trailmark-prod's own "documents" table
  And this is verified by directly querying trailmark-prod's own index catalog, not merely the
    composite_indexes metadata row

Scenario: The index status only claims ready once the real index genuinely exists
  Given Alex has just called CreateIndex for a new composite index
  When the real index build has not yet finished
  Then the composite_indexes row's status is not "ready"
  And status becomes "ready" only after the real Postgres index has actually finished building

Scenario: A query matching the new index is provably faster, not merely no longer rejected
  Given a composite index exists and its status is "ready"
  When Alex runs the exact RunQuery shape the index was created for, with EXPLAIN
  Then Postgres's own query planner chooses a plan that uses the new index
  And the same query, run against an equivalent unindexed collection, still shows a full sequential scan

Scenario: A field path outside the safe charset is rejected before any DDL is built or executed
  Given a CreateIndex request names a field containing a quote, semicolon, or SQL comment sequence
  When Alex submits that request
  Then the request is rejected with a clean error before any DDL statement is constructed or sent
    to the customer database

Scenario: The existing CreateIndex request/response shape and gating behavior are unchanged
  Given firestore-composite-indexes-admin-api's own existing acceptance tests
  When this feature's changes are applied
  Then AC-CIX-01 through AC-CIX-04 (idempotent duplicate create, Owner/Admin-only, 200 response shape,
    the RunQuery gate itself) all still pass unmodified
```

#### Acceptance Criteria
- [ ] AC-CXR-01: `CreateIndex` against a real customer database (an ordinary, typically-sized
      collection) results in a real Postgres index object existing on that collection's underlying
      storage — verified by directly querying the customer database's own index catalog for an index
      matching the created composite index's fields, not merely the `composite_indexes` metadata row.
- [ ] AC-CXR-02: the row's `status` reaches a genuinely-ready value only AFTER the real index has
      actually finished building — never synchronously on INSERT before any build step has run.
- [ ] AC-CXR-03: once ready, an `EXPLAIN` of the exact `RunQuery` shape the index was created for shows
      Postgres's planner choosing a plan that uses the new index (an index scan/index-only scan), not a
      full sequential scan of `documents`.
- [ ] AC-CXR-04: a `field_path` value containing characters outside the set the existing `validate_
      field_path` gate already accepts is rejected with a clean error before any DDL is constructed or
      executed — never interpolated into a DDL statement unsanitized.
- [ ] AC-CXR-05 (regression guard): `firestore-composite-indexes-admin-api`'s own existing behavior
      (AC-CIX-01 through AC-CIX-04) and `composite-index-requirement-rules`'s own existing behavior
      (AC-CIR-01 through AC-CIR-07) are unchanged.

#### Outcome KPIs
- **Who**: Alex, building against real customer projects using compound filter+orderBy queries (e.g.
  Trailmark, `direct_pg`/`aws_secret`/`gcp_secret` backend modes).
- **Does what**: gets a genuinely faster query once he creates a matching composite index — not merely
  a query that is no longer rejected.
- **By how much**: from 0% of "ready" composite indexes backed by a real Postgres index today (100% are
  metadata-only, per this DISCUSS's own confirmed reading) to 100% of indexes reported `'ready'` being
  backed by a real, planner-verified-in-use Postgres index.
- **Measured by**: AC-CXR-01's own direct catalog-query proof; AC-CXR-03's own `EXPLAIN`-based
  before/after plan comparison (chosen over a wall-clock timing assertion — cheaper and more reliable,
  per the task's own explicit guidance).
- **Baseline**: 0% — confirmed directly by this DISCUSS's own reading of `composite_indexes.rs:118-123`
  (`status` hardcoded to the literal `'ready'` in the INSERT statement itself, no build step anywhere).

### US-02: Building a Composite Index Never Blocks Writes to a Large, Live Collection

**job_id**: JOB-01 | **Release**: 1 | **Persona**: P2 Sam Chen (operational stakeholder; see § Persona & Job)

#### Elevator Pitch
Before: this scenario cannot occur today (no real DDL is ever built) — but the moment US-01 ships a
naive synchronous `CREATE INDEX`, Sam Chen's own production deployment would be at real risk: a
`CREATE INDEX` against a large, actively-written `products` collection can hold a write-blocking lock for
as long as the build takes, and today's `status='ready'`-on-INSERT bug would ALSO mean Sam Chen has no
way to observe whether an index is still building or has genuinely finished.
After: Alex (or an operator on Sam Chen's team) creates a composite index against a large, live
collection under active write traffic — the build proceeds without blocking or measurably stalling those
writes, and the row's own `status` distinguishes "still building" from "ready to use" from "the build
failed," so Sam Chen is never left guessing whether it is safe yet to rely on.
Decision enabled: Sam Chen can trust that enabling a new composite index in production never becomes an
unplanned write-availability incident for the collection it targets, and can tell — without guessing —
whether a given index is done building.

#### Who
- Sam Chen (P2) | Service operator running `embyr-server` in production for customers with large,
  actively-written collections (e.g. Trailmark's own `products` collection at real scale) | Needs index
  creation to be a safe, observable, non-disruptive operation against live production traffic, not a
  maintenance-window event.

#### Solution
The real index build (US-01's own DDL-execution mechanism) is structured so that it does not hold a
write-blocking lock on the target collection for the duration of the build, and the `composite_indexes.
status` column carries a genuine build-lifecycle value set (at minimum: building, ready, and a distinct
failure state) that a caller polling `ListIndexes` can rely on. The exact mechanism (`CREATE INDEX
CONCURRENTLY` vs. an alternative; the async-build wrapper shape) is DESIGN's own investigation (§
Central Design Questions B, C).

#### Domain Examples

**Example 1 (Happy Path — a large collection's writes are unaffected during build)**: Trailmark's
`products` collection has grown to hundreds of thousands of documents under continuous write traffic
from Alex's own app. An operator calls `CreateIndex` for a new `(category, score)` composite index.
While the build is in progress, Trailmark's own writes to `products` continue to succeed at their normal
rate — no write is blocked or measurably delayed by the index build.

**Example 2 (Edge Case — status genuinely distinguishes building from ready)**: The same `CreateIndex`
call above is immediately followed by a `ListIndexes` call. Before the build finishes, the returned
`status` is a value clearly distinct from `"ready"` — an operator polling cannot mistake "still working
on it" for "safe to rely on now." Once the build genuinely completes, the SAME row's `status` transitions
to `"ready"`, and the `RunQuery` gate begins admitting matching queries that were previously rejected.

**Example 3 (Error/Boundary — the build genuinely fails)**: Mid-build, Trailmark's own Postgres instance
becomes briefly unreachable (the same class of transient blip `realtime-listener-reconnect` names for a
different code path). The build attempt fails. The row's `status` reflects a state distinguishable from
both `"building"` and `"ready"` — Sam Chen is never left interpreting a permanently-stuck `"building"`
row as either "still in progress" or "silently fine."

#### UAT Scenarios (BDD)

```gherkin
Scenario: Creating an index against a large, actively-written collection does not block writes
  Given Trailmark's "products" collection has a large number of documents and is under continuous
    write traffic
  When an operator creates a new composite index for that collection
  Then writes to "products" continue to succeed throughout the build
  And no write is blocked or measurably stalled by the index build in progress

Scenario: The status distinguishes building from ready
  Given a composite index build has just started and has not yet finished
  When an operator calls ListIndexes for that project
  Then the returned status for that index is a value clearly distinguishable from "ready"

Scenario: Status genuinely transitions to ready once the build completes
  Given a composite index build eventually finishes successfully
  When an operator calls ListIndexes again after that point
  Then the status is now "ready"
  And a RunQuery matching that index's own field/order shape, previously rejected, now succeeds

Scenario: A genuinely failed build is distinguishable from both building and ready
  Given a composite index build fails partway through (e.g. the customer database becomes
    unreachable mid-build)
  When an operator calls ListIndexes for that project
  Then the returned status for that index is neither "building" nor "ready"
```

#### Acceptance Criteria
- [ ] AC-CXR-06: creating a composite index against a collection under active, concurrent write traffic
      does not block, fail, or measurably stall those writes while the index build is in progress.
- [ ] AC-CXR-07: while a composite index is building, its `status` is a value distinguishable from both
      `'ready'` and a terminal failure state — a caller polling `ListIndexes` can tell "still building"
      apart from "ready to use."
- [ ] AC-CXR-08: once the build genuinely completes, `status` transitions to a value distinguishable as
      ready, and the `RunQuery` gate (`is_index_ready`) begins admitting matching queries that were
      previously rejected — proving the transition is truthful, not merely time-delayed.
- [ ] AC-CXR-09: if the build genuinely fails, the row's `status` reflects a state distinguishable from
      both "building" and "ready" — never silently stuck, never falsely reported ready.

#### Outcome KPIs
- **Who**: Sam Chen, operating production embyr deployments serving customers with large, actively
  -written collections (e.g. Trailmark).
- **Does what**: enables a new composite index in production without it ever becoming a write
  -availability incident, and can always tell whether a given index build is still in progress, done, or
  failed.
- **By how much**: from "unmeasured/unknown" today (no real build exists to measure) to 0 measurable
  write-latency/availability regression during index build, for a large collection under active traffic,
  and 100% of index-status queries correctly distinguishing building/ready/failed.
- **Measured by**: AC-CXR-06's own real concurrent-write-during-build test; AC-CXR-07/08/09's own
  status-transition proofs against a real build cycle.
- **Baseline**: not applicable — today's `status='ready'`-on-INSERT bug means no real build, and
  therefore no real write-blocking risk, currently exists; this KPI measures the NEW risk this feature
  itself introduces by making the build real, and proves it is safely mitigated before shipping.

### US-03: Deleting a Composite Index Removes the Real Postgres Index, Not Just the Metadata Row

**job_id**: JOB-01 | **Release**: 2 | **Persona**: P2 Sam Chen (operational resource-hygiene stakeholder)

#### Elevator Pitch
Before: `DELETE /admin/v1/projects/:project_id/indexes/:index_id` (already shipped, unchanged by this
story's own predecessors) removes only the `composite_indexes` metadata row — once US-01/US-02 make
`CreateIndex` provision a REAL Postgres index, deleting only the metadata row would leave that real
index behind forever: an orphaned resource silently consuming disk and slowing every future write to
that collection, with zero record anywhere that it still exists.
After: run the identical `DeleteIndex` call → sees the same response (unchanged) — but this time the
real underlying Postgres index is also genuinely gone from the customer database, not merely the
metadata row that used to (inaccurately) describe it.
Decision enabled: Sam Chen can trust that deleting an index actually reclaims the real resources it was
consuming, and never has to separately audit customer databases for indexes the admin API itself no
longer knows about.

#### Who
- Sam Chen (P2) | Service operator responsible for the real resource footprint (disk, write overhead) of
  every customer database `embyr-server` manages | Needs `DeleteIndex` to be truthful about what it
  actually removes, mirroring the same "no orphaned resource left behind" concern the audit's own
  findings #4 and #6 already name for other subsystems.

#### Solution
`DeleteIndex` drops the real underlying Postgres index (when one exists) from the customer database, in
addition to removing the `composite_indexes` metadata row it already removes today. The exact DROP
mechanism (plain `DROP INDEX` vs. `DROP INDEX CONCURRENTLY`) and its ordering relative to the metadata
-row deletion are DESIGN's own investigation, mirroring Central Design Question B for the deletion path.

#### Domain Examples

**Example 1 (Happy Path — deleting a ready index removes the real one too)**: Trailmark's `products`
collection has a `'ready'` composite index (a real Postgres index exists, per US-01). Alex, no longer
needing it, calls `DeleteIndex`. Before this story: only the metadata row disappears; the real index
silently remains. After this story: both the metadata row AND the real Postgres index are gone —
verified directly against the customer database's own index catalog.

**Example 2 (Edge Case — deleting an index that is still building)**: An operator calls `DeleteIndex`
for a composite index whose build (US-02) has not yet finished. The delete succeeds without error,
regardless of whether a real (possibly partial or `INVALID`) index object already exists for it at that
point.

**Example 3 (Error/Boundary — the underlying real index was never successfully created)**: A composite
index's own build previously failed (US-02's own failure state). An operator deletes it. The delete
succeeds cleanly — there is no real index to drop, and the absence of one is not treated as an error.

#### UAT Scenarios (BDD)

```gherkin
Scenario: Deleting a ready composite index removes the real Postgres index, not just the metadata row
  Given Trailmark's "trailmark-prod" project has a composite index with status "ready" and a real
    Postgres index backing it
  When Alex calls DeleteIndex for that index
  Then the composite_indexes metadata row is removed
  And the real Postgres index is also removed from trailmark-prod's own database, verified directly
    against its own index catalog

Scenario: Deleting an index that is still building does not error
  Given a composite index build is still in progress
  When an operator calls DeleteIndex for that index
  Then the delete succeeds without error

Scenario: Deleting an index whose build previously failed does not error
  Given a composite index's own build previously failed and no real Postgres index exists for it
  When an operator calls DeleteIndex for that index
  Then the delete succeeds without error

Scenario: A subsequent query that newly requires the deleted index fails again, unchanged
  Given a composite index has just been deleted, and it was the only ready index for its collection
  When a query newly requiring that exact index is run
  Then the query fails FAILED_PRECONDITION again, exactly as if the index had never been created
```

#### Acceptance Criteria
- [ ] AC-CXR-10: deleting a composite index whose `status` is ready (a real index exists) removes BOTH
      the `composite_indexes` metadata row AND the real Postgres index object from the customer
      database.
- [ ] AC-CXR-11: deleting a composite index that is still building or has failed does not error,
      regardless of whether a real (possibly partial) index object exists yet.
- [ ] AC-CXR-12 (regression guard): a subsequent query that newly requires the deleted index fails
      `FAILED_PRECONDITION` again, unchanged from `firestore-composite-indexes-admin-api`'s own AC-CIX
      -08 guarantee — now also true because the real underlying index storage is genuinely gone, not
      merely the metadata row.

#### Outcome KPIs
- **Who**: Sam Chen, operating production embyr deployments whose customer databases accumulate
  composite indexes over the account's lifetime.
- **Does what**: never accumulates orphaned, silently-resource-consuming real Postgres indexes that the
  admin API itself no longer has any record of.
- **By how much**: from 0% of deletes removing the real underlying resource (once US-01 ships, every
  delete would otherwise orphan 100% of ready indexes) to 100% of deletes removing both the metadata row
  and any real index that exists for it.
- **Measured by**: AC-CXR-10's own direct catalog-query proof (before and after delete).
- **Baseline**: not applicable — the real-index-orphaning risk this story closes does not exist until
  US-01 ships; this KPI proves the risk US-01/US-02 would otherwise introduce is fully closed before
  this feature is considered done.

## Wave: DISCUSS / [REF] Out of Scope

- **Multi-index query planning sophistication** (choosing among several candidate indexes for a query
  that could be served by more than one) — Postgres's own native planner already handles this once a
  real index exists; no new logic needed or proposed here.
- **Index rebuild-on-schema-change** — no domain evidence any customer's own field-value types change
  in a way requiring index rebuild; not promised by either prior composite-index feature.
- **`UpdateIndex`** — carried forward unchanged from `firestore-composite-indexes-admin-api`'s own
  identical Out-of-Scope entry; real Firestore itself has no such verb.
- **Per-field-set granularity in `IndexManager::is_index_ready`** (today: ANY ready index for a
  collection unblocks ANY query needing one for that collection) — carried forward unchanged from both
  prior composite-index features' own identical Out-of-Scope entry; a pre-existing behavior this feature
  neither builds nor changes.
- **Simulating real Firestore's own client-facing async `CREATING` console UX** — distinct from this
  feature's own `'building'` status, which reflects REAL work now genuinely happening (unlike the fake
  delay `firestore-composite-indexes-admin-api`'s own Resolution 2 explicitly rejected building for zero
  real work) — named explicitly to avoid confusing the two.
- **`backend_mode = agent`** — `embyr-server` has no existing mechanism to open a direct Postgres
  connection to an agent-fronted customer database; mirrors the already-established agent-mode
  structural-wall pattern elsewhere in JOB-01's own history. `CreateIndex` against an agent-mode project
  must not silently claim a real index exists when it cannot — DESIGN decides the exact response shape.
- **Distributed/multi-`embyr-server`-instance coordination for concurrent `CreateIndex` calls against
  the same index spec from different server replicas** — no domain evidence of multi-instance deployment
  being exercised anywhere in this codebase's own tests today; DESIGN may note it as a related follow-up
  if its chosen mechanism happens to expose a natural coordination point, but it is not required by this
  feature's own ACs.
- **A generic, reusable background-job-queue infrastructure** — the task's own explicit ask is to
  evaluate the simplest correct mechanism first; this DISCUSS presumptively scopes OUT building new
  generic job-queue infrastructure (no other feature in this codebase needs one yet), pending DESIGN's
  own confirmation that a `tokio::spawn`'d one-shot task is sufficient (§ Central Design Question C).
- **Any code implementation** — this is DISCUSS only, per explicit task instruction; DESIGN performs the
  actual investigation and implementation planning.

## Wave: DISCUSS / [REF] Walking Skeleton Strategy

**Strategy A** (real, minimal, end-to-end) — every scenario is a real `CreateIndex`/`DeleteIndex` call
against a real customer database, with real `EXPLAIN` plan verification and real concurrent-write
testing during build, mirroring this session's own established Strategy A precedent for every
production-readiness-audit-derived fix.

## Wave: DISCUSS / [REF] Driving Ports

Admin HTTP `:9090` — the SAME 3 existing routes `firestore-composite-indexes-admin-api` already shipped
(`POST/GET /admin/v1/projects/:project_id/indexes`, `DELETE /admin/v1/projects/:project_id/indexes/
:index_id`) — zero new endpoint, zero request/response shape change. This feature changes only what
happens BEHIND those already-shipped handlers.

## Wave: DISCUSS / [REF] Pre-requisites

- `firestore-composite-indexes-admin-api` (FINALIZED 2026-09-04) — provides the admin CRUD surface and
  `composite_indexes` table this feature makes real, unmodified request/response shapes.
- `composite-index-requirement-rules` (FINALIZED 2026-09-05) — provides the accurate `requires_
  composite_index`/`missing_index_fields` gate this feature does not touch.
- `firestore-query-filter-operator-support` (per `MEMORY.md`) — provides real SQL translation for every
  `FilterOp` this feature's own `EXPLAIN`-based usage proof (US-01) depends on being executable at all.
- `transaction_sweeper.rs`'s own `resolve_dsn_without_api_key`/`PostgresBackendAdapter::new(&dsn)`
  precedent — the reuse candidate for this feature's own customer-DB connection mechanism (§ Central
  Design Question C), not itself modified by this feature.
- No new external dependency, no new bounded context, no new dependency edge beyond a new DDL
  -construction module and a `composite_indexes` schema migration.

## Wave: DISCUSS / [REF] Definition of Ready Validation

### DoR Checklist (9-item hard gate)
1. [x] Every story traces to a job_id (JOB-01, all 3 stories).
2. [x] Every story has a complete Elevator Pitch (Before / After / Decision enabled).
3. [x] Every AC is testable without ambiguity (12 ACs across 3 stories, each a real DDL/DB-catalog
   proof, a real `EXPLAIN` plan proof, a real concurrent-write proof, or an explicit regression guard
   against the two prior composite-index features' own existing ACs).
4. [x] Walking Skeleton identified (US-01).
5. [x] Scope Assessment passed.
6. [x] No slice contains only `@infrastructure` stories — every story directly enables a named
   Decision (Alex trusting his query is genuinely fast; Sam Chen trusting production writes are safe
   during build; Sam Chen trusting delete reclaims real resources).
7. [x] Out of Scope explicitly named (9 items, each reasoned).
8. [x] Outcome KPIs have numeric/proportional targets and measurement methods (3 stories, each with its
   own Who/Does What/By How Much/Measured By/Baseline).
9. [x] Prior-wave artifacts read and reconciled — both prior composite-index features' own explicit
   "deferred, no candidate feature id assigned" Out-of-Scope entries are the direct origin of this
   feature; no contradiction found with either feature's own locked Resolutions (metadata-only status
   reversed here is the EXPLICIT, named point of this feature, not an accidental contradiction).

### DoR Status: **PASSED**

## Wave: DISCUSS / [REF] Wave Decisions Summary

### Key Decisions
- [D1] Reuse JOB-01 — the third "make it real" realization in this exact composite-index sequence, both
  prior features' own feature-delta.md explicitly named this as their own deferred follow-up.
- [D2] This feature reverses `firestore-composite-indexes-admin-api`'s own locked Resolution 3
  ("metadata-only") — explicitly, by design, named as the assigned follow-up that Resolution's own text
  anticipated ("a separate, unevidenced future concern... named, deferred, out of this feature's own
  scope"), not a silent contradiction.
- [D3] DeleteIndex must also remove the real underlying DDL index when one exists (US-03) — locked as
  in-scope, not deferred, because leaving a real index behind after "delete" is a genuine new orphaned
  -resource risk THIS feature itself would introduce if left unaddressed, mirroring the audit's own
  recurring "leaks forever" pattern (findings #4, #6).
- [D4] Four genuinely open engineering trade-offs (SQL-injection-safe DDL construction mechanism;
  `CONCURRENTLY` vs. plain; the async-build mechanism/status-lifecycle shape; the exact index DDL
  expression shape) are explicitly NOT resolved here — each is a Central Design Question for DESIGN,
  with reading-derived facts and reuse candidates (`validate_field_path`; `transaction_sweeper.rs`'s own
  DSN-resolution precedent; `missing_index_fields`' own field-ordering convention) offered as a starting
  point, not locked.
- [D5] The story split is 3 stories (US-01 real-creation-and-usage walking skeleton; US-02 non-blocking
  -build safety; US-03 delete symmetry) rather than 1 combined story or 4 separate facet-stories — each
  is independently demonstrable (mirrors the task's own explicit (a)-(d) facet framing while applying
  `realtime-listener-reconnect`'s own "combine only genuinely inseparable facets" judgment: (a)+(b)+(c)
  are inseparable for the ordinary-collection case — a created-but-never-proven-in-use index is not a
  meaningful separate deliverable — while (d)'s large-collection safety mechanism and delete-symmetry
  are each a structurally distinct, separately demonstrable capability).

### Requirements Summary
- Primary need: `CreateIndex` has satisfied the Firestore-parity gate since 2026-09-04, but has never
  provisioned a real Postgres index — every "ready" composite-index query still executes as a full
  sequential scan, silently, forever. This feature closes that gap, safely, for both ordinary and large
  live collections, and symmetrically on delete.
- Walking skeleton scope: US-01 — real DDL creation, truthful status, and `EXPLAIN`-proven usage, for
  the ordinary-collection case.
- Feature type: Backend / Reliability-performance fix.

### Constraints Established
- Zero change to `handler.rs`'s own `RunQuery`/gating call sites, and zero change to `encoding/query.rs`'s
  existing WHERE/ORDER-BY query-execution builders.
- `field_path` values must pass the existing `validate_field_path` charset gate before any DDL
  interpolation — a hard requirement, not a DESIGN preference.
- Large-collection index builds must not block writes — the exact mechanism is DESIGN's own choice, but
  the outcome is locked.
- `backend_mode = agent` is out of scope.

### Upstream Changes
- None to `jobs.yaml` applied in this DISCUSS (recommended NOTE text below, for FINALIZE — mirrors
  `realtime-listener-reconnect`'s own precedent of some NOTEs landing at FINALIZE rather than DISCUSS).
  Recommended NOTE for JOB-01: "JOB-01 now also covers REAL composite-index provisioning — `CreateIndex`
  provisions an actual Postgres index against the customer database (safely, without blocking writes on
  large live collections), and `DeleteIndex` removes it symmetrically, closing the third and final gap
  in the composite-index sequence (admin CRUD surface → requirement-detection accuracy → real
  provisioning). Same job, same persona, not a new job. Closes production-readiness-audit finding #5."

## Wave: DISCUSS / [REF] Next Wave

**Handoff To**: nw-solution-architect (DESIGN wave)
**Deliverables**: this feature-delta.md, 4 Central Design Questions (A-D) with reading-derived facts and
reuse candidates offered but not locked, 5 locked Decisions (D1-D5), 3-story/2-release plan, 12 ACs
(AC-CXR-01 through AC-CXR-12) to design executable scenarios against. DESIGN's own investigation scope:
(1) the exact DDL-construction/quoting mechanism for `field_path` values (Question A); (2) `CREATE INDEX
CONCURRENTLY` vs. an alternative for large-collection safety (Question B); (3) the exact async-build
mechanism and `composite_indexes.status` value set/schema change (Question C); (4) the exact Postgres
index DDL expression shape that will actually be used by the planner for each field's runtime value type
(Question D, informed by `append_field_filter`'s full match-arm set).

## Wave: DESIGN / [REF] Reading Confirmation (beyond DISCUSS's own)

✓ `crates/embyr-core/src/domain/query.rs::validate_field_path` (lines 1-33) re-read directly: charset
`^[a-zA-Z_][a-zA-Z0-9_.]*$`, no quote/semicolon/comment-sequence acceptable. Confirms Question A's
opening recommendation is sufficient AS-IS.
✓ `crates/embyr-pg-storage/src/encoding/query.rs` (`append_field_filter`, `push_value_equality`,
`push_scalar_comparison`, `order_by_expr`, full file) read directly, side-by-side with
`crates/embyr-pg-storage/src/backend_adapter.rs::run_query` (lines 557-582). Confirms: (1) `run_query`'s
own WHERE clause leads with bound (non-expression) `project_id = $1 AND collection_path = $2`, trailing
`AND NOT deleted` as a literal boolean, exactly matching `documents_project_collection_idx`'s own shape;
(2) `Equal`/`NotEqual` (`push_value_equality`) compare the WHOLE stored JSON value
(`fields->'{field}' {=|!=} $1::jsonb`), type-agnostic across every `FieldValue` variant; (3) `ORDER BY`
(`order_by_expr`) always extracts `fields->'{field}'->>'v' {ASC|DESC}` — a DIFFERENT expression shape
from (2); (4) range comparisons (`push_scalar_comparison`) need a THIRD, per-type-cast shape not
derivable from `CreateCompositeIndexBody` (no value-type information in the wire request). These four
facts directly drive ADR-072 Decision D.
✓ `crates/embyr-server/src/sweepers/transaction_sweeper.rs` (full, 344 lines) re-read directly. Confirms
`resolve_dsn_without_api_key` + its 3 per-`backend_mode` helpers are extractable verbatim (no
sweeper-specific state referenced inside them beyond the `SweeperProjectRow` parameter, itself a
plain-data struct) — the extraction ADR-072 Decision C locks is a behavior-preserving move, not a
rewrite.
✓ `crates/embyr-server/src/adapters/system_db.rs` (`SweeperProjectRow`, `get_project_backend_mode`,
`get_project_for_auth` read directly, full signatures) — confirms `get_project_backend_mode`'s own
"fold ownership check + field into one query" pattern (ADR-036 Decision 5) is the direct precedent for
this feature's new `get_project_pg_connect_info`.
✓ `crates/embyr-server/src/admin/state.rs` (full) + `crates/embyr-server/src/admin/router.rs` (lines
100-169) + `crates/embyr-server/src/main.rs` (lines 230-280) read directly. Confirms: `UserAdminState`
currently has NO `aws_secret_fetcher`/`gcp_secret_fetcher` fields (only `OperatorState` does); the
composition root passes `aws_secret_fetcher: None, gcp_secret_fetcher: None` to BOTH `build_router` and
`TransactionSweeper::spawn` TODAY, already a documented, accepted gap (ADR-054 § D7's own comment,
re-read verbatim: "`FirestoreService`'s own live gRPC request-serving path... already hardcodes the same
`None, None`... this sweeper's wiring is independent and does not worsen that gap"). This feature
inherits, not introduces, that gap for `aws_secret`/`gcp_secret` composite-index provisioning.
✓ `migrations/0003_composite_indexes.sql` (full) + `migrations/customer/0001_documents.sql` (full)
re-read directly — confirms `status` is `VARCHAR(20) DEFAULT 'ready'` with NO CHECK constraint (schema
change is additive, no backfill needed, every existing row is already `'ready'`) and confirms the exact
existing index (`documents_project_collection_idx (project_id, collection_path) WHERE NOT deleted`) this
feature's own new index mirrors the leading-columns/partial-predicate convention of.
✓ `crates/embyr-server/src/grpc/handler.rs` lines 576-689 (`requires_composite_index`,
`collect_filter_fields`, `missing_index_fields`) read directly, full bodies — confirms the exact 3
trigger shapes (`order_by.len() >= 2`; `IN` + range-on-different-field; single-filter +
single-orderBy-on-different-field) and confirms `missing_index_fields`' own "filter fields (hardcoded
Asc) first, then real orderBy fields (real direction)" convention, which ADR-072 Decision D's positional
role-inference generalizes for the ONE shape (single/multi equality + one trailing sort field) this
feature's own ACs exercise, and explicitly does NOT claim solved for the other two shapes (named
limitation, ADR-072 Decision D).

## Wave: DESIGN / [REF] Locked Decisions (Questions A-D)

Full reasoning, alternatives considered (2+ each), and consequences: `docs/product/architecture/adr-072-
composite-index-real-provisioning.md`. Summary:

- **[A] DDL safety**: reuse `validate_field_path`'s existing charset gate as the sole guard, applied to
  every `field_path` before any DDL text is built. No `quote_ident` wrapper needed for `field_path`
  (never used as a raw SQL identifier — only as a string-literal argument to the `->` JSONB operator,
  same as existing production WHERE/ORDER-BY code). `quote_ident`-shaped care IS applied to the
  **index name**, which is server-generated (`cix_<hex(uuid)>`), never derived from user input.
- **[B] `CREATE INDEX CONCURRENTLY`**, locked. Confirmed necessary (AC-CXR-06 structurally unreachable
  with plain `CREATE INDEX`). Failure/`INVALID` detection: after the DDL statement returns, a SEPARATE
  `pg_index.indisvalid` check is the authoritative ready/failed decision (catches a mid-build connection
  drop the client-side `Ok`/`Err` alone would miss). On failure: best-effort `DROP INDEX CONCURRENTLY IF
  EXISTS` before writing `status = 'failed'`, freeing the deterministic name for retry.
- **[C] Async-build mechanism**: `tokio::spawn`'d one-shot task fired from `create_composite_index`'s own
  handler body, reusing `transaction_sweeper.rs`'s `resolve_dsn_without_api_key` (extracted to a new
  shared module, `adapters/customer_db_connect.rs`) + `PostgresBackendAdapter::new(&dsn)`. NOT a generic
  job queue (DISCUSS's own scoping-out confirmed correct). `composite_indexes.status` widens to
  `building` / `ready` / `failed` (CHECK constraint, migration `0036`). Retry is via idempotent re-POST
  (`ON CONFLICT ... DO UPDATE SET status = CASE WHEN status = 'failed' THEN 'building' ELSE status END`)
  — explicitly NOT via self-healing crash-recovery; a stuck `'building'` row (process restart mid-build)
  recovers via the already-designed US-03 delete-then-recreate path, zero new machinery. `backend_mode =
  agent` (and any future unrecognized mode) falls through the SAME `resolve_dsn_without_api_key` `None`
  path as any other unreachable-DB case — no special-case code, naturally produces `'failed'`, never
  silently `'ready'`.
- **[D] Index DDL expression shape**, locked: `(project_id, collection_path, (fields->'{f_1}'), ...,
  (fields->'{f_{n-1}}'), (fields->'{f_n}'->>'v') {ASC|DESC}) WHERE NOT deleted` — literal leading
  `project_id`/`collection_path` columns (matching `run_query`'s own bound predicates), all fields
  except the last built as whole-value jsonb expressions (matching `push_value_equality`'s
  `Equal`/`NotEqual` shape, type-agnostic), the LAST field built as text-extraction + direction (matching
  `order_by_expr`'s shape exactly). Provably correct for the walking skeleton's own worked example and
  every "N equality fields + 1 trailing sort field" trigger shape. Explicitly named, NOT silently
  claimed correct, for two rarer shapes (2+ orderBy with no filter; `IN` + range-filter landing last) —
  deferred, matches DISCUSS's own already-locked "multi-index query planning sophistication" Out-of-Scope
  entry.

## Wave: DESIGN / [REF] Component Boundaries and Schema

See ADR-072 § Component Boundaries for the full new/changed file list:
- New: `adapters/customer_db_connect.rs` (extracted DSN resolution), `adapters/composite_index_ddl.rs`
  (pure DDL-string builders, DB-free unit-testable), `adapters/composite_index_builder.rs` (one-shot
  build-task orchestration), `migrations/0036_composite_indexes_status_lifecycle.sql`.
- Changed (behavior-preserving extraction only): `sweepers/transaction_sweeper.rs`.
- Changed (feature behavior): `admin/handlers/composite_indexes.rs` (`create_composite_index`'s INSERT/
  `ON CONFLICT`/spawn; `delete_composite_index`'s new best-effort real-index-drop step, ordered BEFORE
  the existing metadata-row DELETE so a drop failure leaves the metadata row — and therefore visibility
  and retryability — intact rather than orphaning an untracked real index), `admin/state.rs`
  (`UserAdminState` gains the 2 secret-fetcher fields `OperatorState` already has), `adapters/system_db.rs`
  (+`get_project_pg_connect_info`, +`update_composite_index_status`).
- Zero change (confirmed, matches DISCUSS's own System Constraints): `embyr-core` (no IO crate added;
  `validate_field_path` reused unmodified), `embyr-pg-storage/src/encoding/query.rs`,
  `grpc/handler.rs`'s `RunQuery`/gate call sites.

**C4 Container-level note**: this feature adds no new deployable container or external system — it adds
a new OUTBOUND edge from the existing `embyr-server` container to the existing "Customer PostgreSQL"
external system (already drawn in this codebase's system diagrams via `PostgresBackendAdapter`'s
existing use for live query/write traffic and `TransactionSweeper`'s existing background-task use) — a
second, symmetric use of an already-modeled integration point, not a new one. No new C4 diagram is
warranted (matches this session's own established precedent of skipping diagram churn for
internal-component-only features touching an already-fully-diagrammed edge — mirrors
`composite-index-requirement-rules`' and `firestore-composite-indexes-admin-api`'s own identical choice).

## Wave: DESIGN / [REF] External Integration Annotation

**Customer PostgreSQL** (per-project, `direct_pg`/`aws_secret`/`gcp_secret` backend modes): this feature
adds a SECOND write-capable DDL path (`CREATE`/`DROP INDEX CONCURRENTLY`) alongside the existing
document-CRUD path (`PostgresBackendAdapter`) and the existing `TransactionSweeper` background-task path
— all three already share the SAME connection/adapter primitives; no new integration surface, no new
contract-testing recommendation beyond what those existing paths already carry (a customer's own
Postgres instance is infrastructure this codebase owns and provisions, not a third-party API with an
independently-evolving contract — consumer-driven contract testing, per `nw-architecture-patterns`'
own guidance, applies to third-party APIs/webhooks/OAuth providers with independent release cycles, not
to a customer-operated instance of an open-source database this codebase already fully controls the
schema and DDL vocabulary for).

## Wave: DESIGN / [REF] Quality Attribute Validation

- **Reliability**: build failures (connect drop, `INVALID` index, agent-mode, missing secret-fetcher
  wiring) all converge on one terminal `'failed'` state, never a silent false `'ready'` (Earned Trust —
  the build task is itself the "probe" that empirically demonstrates the real index exists and is valid
  before ever reporting success; `wire (INSERT 'building') -> probe (CREATE INDEX CONCURRENTLY + indisvalid
  check) -> use (status='ready' unlocks the RunQuery gate)` mirrors the composition-root "wire then probe
  then use" invariant at request-lifecycle scope rather than process-startup scope).
- **Performance**: AC-CXR-03's `EXPLAIN`-based proof is the direct measurement; Decision D's expression
  shape is chosen specifically to be syntactically matchable by Postgres's planner for the proven shape,
  not merely "an index that exists."
- **Maintainability/testability**: `composite_index_ddl.rs`'s pure functions are unit-testable without a
  database (mirrors `encoding/query.rs`'s own `QueryBuilder::sql()`-based test pattern) — the
  injection-safety property (AC-CXR-04) and the exact-expression-match property (Decision D) are both
  verifiable in a fast, DB-free unit test, with the full real-Postgres proof reserved for the acceptance
  suite (Strategy A, per DISCUSS).
- **Security**: no new attack surface beyond the already-analyzed field-path-into-DDL-text injection
  vector (Question A) — closed by charset reuse; no new secret material introduced (DSN
  resolution/decryption reuses `transaction_sweeper.rs`'s already-audited path unmodified).

## Wave: DESIGN / [REF] Enforcement Tooling

Rust workspace: existing `deny.toml`-enforced `embyr-core` IO-freedom (unmodified, zero new import
there). New pure functions (`composite_index_ddl.rs`) get direct unit-test coverage plus the session's
own standing `cargo-mutants` QUALITY_GATE pass (per DISCUSS's own System Constraints, reapplied here) —
no new enforcement tool class introduced; matches this codebase's established Rust-workspace convention
of `deny.toml` (import-boundary enforcement) + `cargo-mutants` (test-quality enforcement), not a
per-feature bespoke tool.

## Wave: DESIGN / [REF] Quality Gate Self-Check

- [x] Requirements traced to components (12 ACs -> named files/functions above).
- [x] Component boundaries with clear responsibilities (5 new/changed adapter-layer files, 1 handler
  file, 1 migration; zero domain/query-execution-layer change).
- [x] Technology choices in ADR-072 with 2+ alternatives each (Questions A-D, all 4 sub-decisions).
- [x] Quality attributes addressed (reliability/performance/maintainability/security, above).
- [x] Dependency-inversion / simplest-solution compliance: reuse over new infrastructure at every
  decision point (charset gate over new validator; extracted DSN-resolution over duplicated logic;
  `tokio::spawn` one-shot over job queue; existing idempotent-INSERT pattern over new retry machinery).
- [x] C4: no new diagram warranted (reasoned above, matches sibling-feature precedent); existing
  Container-level edge (embyr-server -> Customer PostgreSQL) is reused, not newly drawn.
- [x] Integration patterns specified (Decision B/C: connection lifecycle, standalone-statement
  requirement, terminal-status write-back).
- [x] OSS preference: zero new dependency (`sqlx`, already pinned, is the only crate touched).
- [x] AC behavioral, not implementation-coupled (all 12 ACs describe observable DB-catalog/EXPLAIN/
  status-polling outcomes, never internal function names).
- [x] External integration annotated (Customer PostgreSQL, reasoned above — no contract-testing
  recommendation warranted, reasoned explicitly rather than silently omitted).
- [x] Architectural enforcement tooling named (`deny.toml` + `cargo-mutants`, existing, reused).
- [x] Peer review — APPROVED, iteration 1, condition resolved same-pass. See below.

## Wave: DESIGN / [REF] Peer Review

Reviewer: `solution-architect-reviewer`, invoked against this DESIGN section + ADR-072, using
`nw-sa-critique-dimensions`' 5-dimension framework (bias detection, ADR quality, completeness,
feasibility, priority validation).

**Iteration 1 result**: `conditionally_approved`. 0 critical, 0 high issues. 2 medium (Decision D's
2 named limitations not yet reflected in AC-level language; the aws_secret/gcp_secret inherited gap
— both already explicitly disclosed in ADR-072, reviewer confirmed disclosure adequate, not a bias/
scope-dodge), 2 low (migration SQL not spelled out verbatim; no self-healing crash-recovery, already
explicitly justified). Priority validation: Q1=YES (Finding #5/Blocker, evidenced directly), Q2=ADEQUATE,
Q3=CORRECT, Q4=JUSTIFIED. Reviewer independently re-verified (not merely trusted) the expression-shape
claims against `encoding/query.rs`, the DSN-resolution extraction against `transaction_sweeper.rs`, and
the WHERE-clause structure against `backend_adapter.rs` — all confirmed accurate.

**Single condition for full approval**: encode Decision D's 2 named limitations directly into the AC
handoff to DISTILL, not just ADR prose, so a future acceptance scenario cannot silently imply broader
planner-usage coverage than this feature actually proves. **Resolved**: added as a binding, explicit
instruction in § Next Wave below (new "Peer-review condition" paragraph) — DISTILL is required to
encode it as an executable regression-guard scenario or an explicit "not asserted" AC note, not merely
carry it forward as prose.

**Status: APPROVED** (condition satisfied in this same DESIGN pass — no iteration 2 needed, matching
this session's own established bar of 0 critical/high findings, fully resolved in one iteration, set by
the 4 already-closed sibling Blocker features).

## Wave: DESIGN / [REF] Next Wave

**Handoff To**: nw-acceptance-designer (DISTILL wave)
**Deliverables**: this feature-delta.md (DISCUSS + DESIGN sections), ADR-072 (all 4 locked sub-decisions
+ component boundaries + schema), 12 ACs to design executable Given-When-Then scenarios against. Every
AC is a real-database proof (catalog query, `EXPLAIN` plan, concurrent-write timing, or status-polling
sequence) — Strategy A (real, minimal, end-to-end) applies unchanged from DISCUSS. Key implementation
notes for acceptance-designer/software-crafter: (1) the exact SQL shapes in ADR-072 Decisions B/D are
contracts, not suggestions — DELIVER must reproduce them verbatim, since correctness depends on exact
syntactic match against `encoding/query.rs`'s own emitted expressions; (2) `backend_mode=direct_pg`
(Trailmark, every worked example) is the only mode with an end-to-end-wired secret-fetcher path today —
acceptance tests for `aws_secret`/`gcp_secret` modes should assert the `'failed'`-not-`'ready'` outcome,
not index-usage (pre-existing, inherited gap, ADR-054 § D7); (3) `composite_index_ddl.rs`'s pure
functions warrant direct unit tests written during DELIVER, per this session's own standing
mutation-testing discipline.

**Peer-review condition (resolved here, binding on DISTILL)**: `solution-architect-reviewer` iteration 1
conditionally approved this DESIGN pending one explicit callout — ADR-072 Decision D's expression-shape
convention (positional equality-vs-sort role inference) is proven ONLY for the shape this feature's own
ACs exercise (N equality-filter fields + exactly one trailing sort field). DISTILL MUST encode this
boundary directly into the executable scenarios, not merely leave it in prose here: add a new
regression-guard scenario (or an explicit "not asserted" note on AC-CXR-01/02/03's own Given-When-Then)
stating that this feature's acceptance suite does NOT exercise, and does NOT claim correct, (a) a
composite index built from 2+ genuine `orderBy` fields with zero filter, or (b) an `IN` + range-filter
shape where the range-filtered field lands last in `fields[]`. Both remain real Postgres indexes that
exist and pass `CreateIndex`/`DeleteIndex`'s own ACs, but may not be selected by the planner for those
two specific shapes — a named, deferred follow-up (ADR-072 Decision D), not a regression, since neither
shape has any real index today either.

## Wave: DISTILL / [REF] Test Files and RED-State Verification

`nw-acceptance-designer` wrote 3 acceptance test files plus shared fixtures, then its own final
report deferred without confirming RED state or appending this section — the orchestrator
independently ran and confirmed all three files below directly against the unfixed code before
handing off to DELIVER, per this session's standing "verify a subagent's own claim, don't just
trust it" discipline.

- `tests/composite_index_real_creation/acceptance/cxr01_real_index_creation_and_usage.rs` — 4
  scenarios: `creating_an_index_builds_a_real_postgres_index_that_the_planner_uses` (AC-CXR-01/02/03
  walking skeleton — proves a real Postgres index exists via `pg_indexes`/`pg_index.indisvalid` and
  is planner-selected via `EXPLAIN`), `a_field_path_outside_the_safe_charset_is_rejected_before_any_ddl_runs`
  (AC-CXR-04), `a_query_against_a_still_building_index_is_rejected_exactly_as_before` (AC-CXR-05,
  Firestore-parity gate regression guard), `a_two_orderby_no_filter_query_still_executes_once_an_index_exists_planner_usage_not_asserted`
  (the peer-review-condition scenario — encodes Decision D's own named limitation explicitly, not
  just in prose).
- `tests/composite_index_real_creation/acceptance/cxr02_non_blocking_build.rs` — 4 scenarios:
  `creating_an_index_against_a_large_live_collection_does_not_block_concurrent_writes` (AC-CXR-06/07),
  `listing_indexes_immediately_after_create_shows_a_status_distinct_from_ready` (AC-CXR-07),
  `status_genuinely_transitions_to_ready_and_unblocks_the_matching_query` (AC-CXR-08),
  `a_build_against_an_unreachable_backend_mode_ends_in_a_distinct_failed_status` (AC-CXR-09).
- `tests/composite_index_real_creation/acceptance/cxr03_delete_removes_real_index.rs` — 4 scenarios:
  `deleting_a_ready_index_removes_the_real_postgres_index_too` (AC-CXR-10),
  `deleting_an_index_that_is_still_building_does_not_error` (AC-CXR-11, crash-recovery path),
  `deleting_an_index_whose_build_previously_failed_does_not_error` (AC-CXR-11),
  `a_query_newly_requiring_the_deleted_index_fails_again_unchanged` (AC-CXR-12, regression guard).

**`[[test]]` registrations**: added to `crates/embyr-server/Cargo.toml` for all 3 files (confirmed
present, correctly scoped, no unrelated targets touched).

**Migration 0036 does NOT yet exist** — DESIGN specified its shape (widen `composite_indexes.status`
CHECK constraint to `building`/`ready`/`failed`) but did not write the file. This is DELIVER's job.

**RED-state verification (orchestrator-run, all 3 files, `--test-threads=1`, Docker cleaned before
each)**:
- `cxr01`: 3/8 failed — `creating_an_index_builds_a_real_postgres_index_that_the_planner_uses` (got
  `status='ready'` synchronously, no build step), `a_field_path_outside_the_safe_charset_is_rejected_before_any_ddl_runs`
  (got 200 OK, no validation), `a_query_against_a_still_building_index_is_rejected_exactly_as_before`
  (status is never `'building'` today). 5/8 passed (regression guards + the peer-review-condition
  scenario, correctly green since they don't depend on new functionality). All failures classified
  MISSING_FUNCTIONALITY, matching finding #5 exactly.
- `cxr02`: 3/8 failed — `a_build_against_an_unreachable_backend_mode_ends_in_a_distinct_failed_status`,
  `creating_an_index_against_a_large_live_collection_does_not_block_concurrent_writes`,
  `listing_indexes_immediately_after_create_shows_a_status_distinct_from_ready` — all because
  `status` is always synchronously `'ready'`, never `'building'`/`'failed'`. MISSING_FUNCTIONALITY.
- `cxr03`: 2/8 failed — `deleting_a_ready_index_removes_the_real_postgres_index_too` (precondition
  failed: no real index exists to delete), `deleting_an_index_whose_build_previously_failed_does_not_error`
  (precondition failed: build never genuinely fails today). MISSING_FUNCTIONALITY.
- Full workspace `cargo check --workspace --tests` confirmed clean (only pre-existing, unrelated
  warnings) — the 3 new test files compile correctly against today's schema/API surface without
  needing migration 0036 to exist yet (they fail at runtime/assertion, not compile time).

**Handoff to DELIVER**: implement per ADR-072's 4 locked decisions exactly; write migration 0036;
turn all 8 currently-failing scenarios green while keeping the 13 already-green ones green.
