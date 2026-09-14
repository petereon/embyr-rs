# Feature Delta: collection-group-query-index

## Wave: DISCUSS / [REF] Reading Confirmation

✓ `docs/product/production-readiness-audit-2026-09-08.md` finding #17 confirmed by direct reading:
"Collection-group queries (`all_descendants=true`) build a leading-`%` `LIKE` that can't use the
`(project_id, collection_path)` btree — every collection-group query scans the entire project, not
just the target collection group. Compounds #5." Severity: **High**. Status (at DISCUSS start):
**Not started**.
✓ `crates/embyr-pg-storage/src/backend_adapter.rs` read in full around every `all_descendants` branch.
The audit's own cited `575-577` has drifted (current file, `run_query`, lines 598-608) — line-number
citations in the audit are point-in-time, re-verified here against actual code, not assumed current.
**Four, not two, mirrored copies of the identical leading-`%` LIKE pattern exist**, not just the one
audit-cited pair: `run_query` (598-608), and — inside `run_aggregation_query` — the `Count` arm
(811-820), the `Sum` arm (859-868), and the `Avg` arm (910-919). Each carries an explicit comment
("WHERE-clause construction copied byte-for-byte from `run_query` above, ADR-040 § 2") confirming this
is an intentional, maintained copy-paste convention in this codebase, not incidental duplication — the
fix must update all four call sites identically, or reintroduce the exact drift ADR-040 § 2 already
warns about.
✓ `migrations/customer/0001_documents.sql` (full, 15 lines) read directly. `documents` table:
`PRIMARY KEY (project_id, collection_path, document_id)`; the only real index is
`documents_project_collection_idx ON documents (project_id, collection_path) WHERE NOT deleted` — a
plain two-column partial btree. `collection_path` is `VARCHAR(1500)`, storing the FULL path to the
collection (e.g. `users/u1/reviews` for a nested collection, or bare `reviews` for a top-level one) —
confirmed against `crates/embyr-server/src/grpc/handler.rs:3134-3137`, where `CollectionPath.
collection_path` is set directly from the proto's `collection_id` (never a full parent+id join) for
BOTH ordinary and collection-group queries; for `all_descendants=true` this is the bare collection ID
being searched for (e.g. `reviews`), never a parent-qualified path.
✓ `docs/SPEC.md` lines 913-921 (§ Collection Group Queries) read directly: "A `from` clause with
`all_descendants: true` executes a **collection group query**: the query matches documents in **any**
collection named `{collection_id}` that is a descendant of `parent`, at any nesting depth... Storage
implementation must support this by querying all documents where `collection = {collection_id}` and
`path` starts with `parent/`, without filtering by an exact `parent` value." Directly compared against
the current implementation's predicate — `collection_path = {collection_id} OR collection_path LIKE
'%/{collection_id}'` — matching a bare-name top-level collection via the first arm and a nested
collection's suffix via the second: **this is the semantically correct result set per SPEC.md**. **This
finding is confirmed to be a pure PERFORMANCE gap, not a correctness bug** — same results, always,
just via a full scan instead of an index. No dynamic index-hint or query-rewrite mechanism exists
anywhere in `embyr-pg-storage` (confirmed by the same full-module read `composite-index-real-creation`
already performed) that this feature could accidentally break.
✓ Finding #5 (`docs/product/production-readiness-audit-2026-09-08.md` line 15) read directly and
cross-checked against `[[project_composite_index_real_creation]]` (FINALIZED 2026-09-09, ADR-072):
composite indexes now provision REAL Postgres indexes via `CREATE INDEX CONCURRENTLY` against the
`fields` JSONB column, keyed on `(project_id, collection_path, field...)`-shaped expressions. **The
compounding relationship, confirmed by direct reasoning against ADR-072's own DDL shape**: Postgres can
only use an index-assisted plan when the query's `WHERE` predicate can be range-bounded against that
index's leading columns. A leading-`%` `LIKE` on `collection_path` (`'%/reviews'`) can never satisfy a
btree prefix bound, and Postgres cannot combine "scan the whole project" with "then apply the composite
index" — the composite index still requires `collection_path` to be pinned to an exact, known value
first. **Finding #5's fix delivers zero benefit to collection-group queries**: every composite index
`CreateIndex` now provisions is still unusable for an `all_descendants=true` query, which must
sequential-scan the entire project's `documents` table regardless of how many real indexes exist. This
is the literal meaning of "compounds #5" — closing #5 made ordinary collection queries fast; it left
collection-group queries exactly as slow as before, with the illusion (per the same "looks-ready but
isn't wired to the real thing" pattern the whole audit exists to eliminate) that indexing is now solved
project-wide.
✓ `crates/embyr-agent/src/server.rs` read directly (lines 22, 71, 462-660, 953): `embyr-agent`'s
`StorageAgent::run_query`/`run_aggregation_query` delegate directly to the SAME
`PostgresBackendAdapter` instance and the SAME `run_query`/`run_aggregation_query` methods `embyr-server`
itself calls (confirmed by an explicit in-code comment at line 660: "the SAME `PostgresBackendAdapter`
method/SQL the non-agent [path] directly [uses]") — unlike several prior findings this session
(`agent-field-path-validation`, the three `agent-mode-*` features), **`embyr-agent` has no independent,
divergently-implemented copy of this query-building logic to separately fix**. One fix, in
`embyr-pg-storage`, closes the gap for all three `backend_mode`s (`direct_pg`, `aws_secret`,
`gcp_secret`) AND `backend_mode=agent` simultaneously.
✓ `crates/embyr-server/src/admin/handlers/provision.rs` (lines 220, 272, 417) and
`docs/product/architecture/adr-022-customer-db-prep-crate-and-migration-consolidation.md` read directly.
**Critical deployment-model fact, not previously surfaced by either prior composite-index feature**:
`migrations/customer/` (which will carry this feature's own schema change) is applied to a given
customer's Postgres database ONLY at two points — (1) automatically, once, at NEW project provisioning
time (`provision.rs`'s three `backend_mode` branches each call `PostgresBackendAdapter::migrate()`
against a freshly-connected customer pool), or (2) manually, by the customer's own DBA, running the
standalone `embyr-db-prep` binary against their own database under their own elevated credentials
(the BYOC `customer-db-onboarding`/JOB-15 path, ADR-022). **No third mechanism re-applies migrations to
an ALREADY-provisioned customer database** — confirmed by a full-workspace grep for `.migrate()` call
sites (`crates/embyr-server/src/main.rs:116` is the SYSTEM DB migration only, unrelated to
`migrations/customer/`). This means: the moment new `embyr-server`/`embyr-agent` binaries carrying this
feature's query-rewrite logic are deployed (a single, fleet-wide, embyr-controlled release), they will
be serving requests against a POPULATION of customer databases in a genuinely MIXED schema state —
some already migrated (new projects provisioned after this ships, or DBAs who proactively re-ran
`embyr-db-prep`), most NOT yet migrated (every customer database provisioned before this feature ships,
until its own DBA independently chooses to re-run `embyr-db-prep`) — with no embyr-controlled rollout
schedule or completion signal for the latter group. This is a materially different, and more severe,
migration-risk shape than `composite-index-real-creation`'s own (that feature's index build is
customer/Alex-TRIGGERED, additive, and never required by already-admitted queries until `CreateIndex`
is explicitly called; this feature's fix is required by EVERY collection-group query the moment new
server code ships, against a customer population most of which has not migrated yet).
✓ `docs/product/architecture/adr-017-production-startup.md` read in full: confirms `embyr-server`'s
OWN system-DB migration (unrelated table) runs synchronously at every process startup, BEFORE any port
binds (Step 5, before Step 8) — establishes this codebase's general startup discipline ("no partial
startup") but does NOT establish any precedent for customer-database schema rollout timing, which (per
the finding above) has no automatic mechanism at all today.
✓ `migrations/` directory listing (full, 37 files) confirms this would be the FIRST-EVER schema change
to the `documents` table since `migrations/customer/0001_documents.sql` — no ALTER TABLE precedent
exists yet for this specific, largest, hottest-path table in the schema.

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

- Feature type: **Backend / Database performance fix**, closing a High-severity audit finding — read
  path only; zero SDK/proto change; zero change to the `FAILED_PRECONDITION` composite-index gate or to
  `composite-index-real-creation`'s own unmodified per-project `CreateIndex` mechanism.
- JTBD: **reuse JOB-01** (`sdk-compat`, P1 Alex) — a fourth "make it real"/performance-parity
  realization in the same family as `composite-index-real-creation`: real Firestore's own
  collection-group queries are fast at scale; this codebase's version is currently correct but silently
  degrades to a full project scan.
- Walking Skeleton: **Yes** (US-01, § Story Map) — the riskiest, highest-value assumption: can a
  collection-group query be answered via a real, index-assisted plan (not a full scan) for the ordinary
  case where the target schema is already current.
- UX Research Depth: **Lightweight** — a storage-layer performance fix behind an already-shipped,
  unchanged `RunQuery`/`RunAggregationQuery` wire contract; no new emotional arc, no new TUI/journey
  artifact (matches every prior storage-performance feature this session).
- **This DISCUSS deliberately does NOT resolve the mechanism-level engineering trade-offs** the task
  itself names as open (reversed-string-plus-btree vs. trigram/GIN vs. a separate indexed
  `collection_id` column vs. a partial/expression index; the exact backfill mechanism/trigger for
  already-provisioned customer databases; whether a schema-version-aware query fallback is temporary or
  permanent) — each is recorded as a Central Design Question for DESIGN, with reading-derived facts and
  reuse candidates offered (ADR-072's own `CREATE INDEX CONCURRENTLY` + async-build precedent), per this
  session's established "lock the user-facing outcome, flag the mechanism" DISCUSS discipline.

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: P1 Alex (SDK Developer), primary — unchanged, matches `composite-index-real-creation` and
`composite-index-requirement-rules`.
**Secondary stakeholder**: P2 Sam Chen (Service Operator), named explicitly for US-02 (safe backfill of
an existing, live customer database) and US-03 (correct behavior across a fleet in a mixed schema
state) — the exact same persona split `composite-index-real-creation` established for its own
production-safety concern, and `realtime-listener-reconnect`'s own precedent for "a job whose primary
persona is Alex, but whose specific failure mode is discovered and suffered by Sam Chen operating the
deployment."

**Job**: **JOB-01 `sdk-compat`**, unchanged job_story. JOB-01 already carries a directly on-point NOTE
from `composite-index-real-creation`'s own DISCUSS (real Firestore's composite indexes make admitted
queries fast, not merely admit them). This feature's own realization: real Firestore's collection-group
queries (`db.collectionGroup('reviews')`) are fast at scale, backed by Firestore's own automatic
collection-group indexing — this codebase's version returns correct results today but via a full
project scan, which is not how real Firestore behaves and will not hold up once a customer's project
has any meaningful document volume. After this feature, a collection-group query is answered via a
real, index-assisted plan for schema-current databases, with correctness (never a wrong result, never a
hard failure) preserved for databases still awaiting migration.

*(§ SSOT Updates recommends a JOB-01 NOTE be appended to `docs/product/jobs.yaml` recording this
realization — per this session's own established convention. Not applied directly by this DISCUSS
wave, consistent with this session's own precedent of leaving `jobs.yaml` edits to be applied
explicitly rather than assumed; `composite-index-real-creation` itself left this same recommendation
unapplied, a known, already-flagged gap.)*

## Wave: DISCUSS / [REF] Business Context

Real Firestore's collection-group queries are a first-class, performance-critical query shape —
`db.collectionGroup('reviews').where(...)` is a documented, common pattern for cross-parent queries
(e.g., "all reviews across every product," "all messages across every conversation"). This codebase
already implements the CORRECT result semantics for this shape (§ Reading Confirmation, SPEC.md
913-921) but the underlying SQL predicate — `collection_path = $1 OR collection_path LIKE '%/$1'` — puts
a leading wildcard on the LIKE arm, which Postgres's planner can never satisfy from any btree, including
both the one pre-existing `(project_id, collection_path)` index AND every real composite index
`composite-index-real-creation` now provisions (§ Reading Confirmation — "compounds #5"). Every
collection-group query, for every customer, on every call, sequential-scans that project's entire
`documents` table looking for rows whose `collection_path` happens to end with the target collection
name — cost that scales with total project document count, not with the size of the target collection
group, and that gets strictly worse as a customer's project grows, with nothing today measuring or
surfacing this degradation (confirmed: zero `EXPLAIN`-based test, zero query-latency metric, anywhere
in this codebase for the collection-group path, mirroring `composite-index-real-creation`'s own
identical "silently degrades, unobserved" finding for a different query shape).

This feature's genuinely distinguishing complexity, absent from both prior composite-index features: the
fix requires a **schema change to the `documents` table itself** (not a per-project, admin-triggered,
purely-additive index the way `CreateIndex` is) — every collection-group query, for every project, needs
this fix, and the fix requires the base table to carry some new, indexable representation of "the
collection ID this row belongs to" (exact shape is DESIGN's call, § Central Design Questions). Because
`documents` lives in each CUSTOMER's own Postgres database (§ Reading Confirmation — BYOC deployment
model, `migrations/customer/`), and because no mechanism exists today to re-apply a customer migration
to an ALREADY-provisioned project, this is a genuinely riskier class of change than any prior
composite-index feature: (1) EXISTING documents in EXISTING customer databases need a backfill, not
just a DDL statement, and (2) the new, fleet-wide `embyr-server`/`embyr-agent` code deploys to ALL
customers simultaneously (embyr controls that release), while the SCHEMA migration each customer's
database needs does NOT roll out on the same schedule (each customer's own DBA controls when, or
whether, they re-run `embyr-db-prep`) — a genuine, sustained window (of unknown, customer-controlled
duration, potentially indefinite for any given customer) where new server code must correctly serve
requests against OLD-schema customer databases.

### Central Design Questions (opening recommendations offered, not locked)

**Question A — the indexable representation of "collection ID" (the storage-layer mechanism).** The
task's own framing names four standard Postgres approaches: (1) a reversed-string expression index over
`collection_path` (turns a suffix match into a prefix match a btree CAN use); (2) a trigram/GIN index
(`pg_trgm`) over `collection_path` (supports arbitrary substring/suffix search, at GIN's own
write-amplification and index-size cost); (3) a separate, explicitly-indexed `collection_id` column,
populated at write time from the last path segment, indexed as `(project_id, collection_id)` — an exact
equality lookup, not a pattern match at all; (4) a partial/expression index keyed on a computed
last-segment extraction. **Opening observation, not a recommendation this DISCUSS locks**: option (3)
converts the query from a pattern-match problem into a plain equality lookup — the SAME shape as the
existing, already-proven `(project_id, collection_path)` index — and composes cleanly with
`composite-index-real-creation`'s own existing composite-index mechanism (a `collection_id` filter could
sit as a leading column ahead of any per-field composite index, letting collection-group queries
benefit from #5's fix too, closing the compounding relationship at its root). DESIGN should confirm
this against the exact cost of populating `collection_id` correctly at every write path (creates,
updates, transforms, batch writes, agent-mode writes) versus a computed/generated-column approach that
needs no explicit write-time population logic, and against whether extracting "the last path segment"
is a pure, cheap, unambiguous string operation for every valid `collection_path` this codebase produces
(no domain example currently shows a case where it wouldn't be, but DESIGN should verify against the
document-path-construction code, not assume).

**Question B — the backfill mechanism for EXISTING documents in EXISTING customer databases.** This
DISCUSS locks the OUTCOME (existing customer data must become correctly index-backed without blocking
that customer's own live reads/writes — US-02) without picking the mechanism. `composite-index-real-
creation`'s own ADR-072 precedent (`CREATE INDEX CONCURRENTLY` + `tokio::spawn`'d one-shot async build +
`pg_index.indisvalid` re-check) is a directly relevant reuse candidate for the INDEX half of this
problem, once a real column exists to index — but does not by itself solve backfilling the new column's
VALUE into potentially hundreds of thousands of pre-existing rows. **Opening observation**: if DESIGN
selects a `GENERATED ALWAYS AS (...) STORED` column (Question A, option 3/4's computed variant),
Postgres itself computes and stores the value for every existing row when the column is added — but
`ALTER TABLE ... ADD COLUMN ... GENERATED ALWAYS AS (...) STORED` on an existing table requires a full
table rewrite (an ACCESS EXCLUSIVE lock for the statement's duration) in the Postgres versions this
codebase is confirmed to support — DESIGN must verify whether that rewrite duration is acceptable for
this codebase's largest real customer collections, or whether a plain (non-generated) column + an
explicit, batched, throttled `UPDATE ... WHERE collection_id IS NULL LIMIT N` backfill loop (never one
giant transaction) is required instead, mirroring the "never a single unbounded write-blocking
operation against a live production table" principle AC-CXR-06 already established for index builds.

**Question C — the fleet-wide schema-version mismatch (US-03).** § Reading Confirmation establishes
this as a genuinely open, previously-unexamined risk: new query-building code ships to ALL customers on
one embyr-controlled release schedule; the schema each customer's own database needs does not. DESIGN
must decide the query-time strategy for a customer database that has NOT yet been migrated: (a) probe
for the new column's existence (or catch the resulting SQL error) and fall back to today's
`LIKE`-based query, preserving correctness at the cost of the customer only receiving the performance
fix once their DBA migrates; or (b) some other schema-version-aware mechanism. **This DISCUSS locks
that correctness must never regress for an un-migrated customer** (never a hard error, never a wrong
result) as a hard requirement, not a nice-to-have — but leaves the exact detection/fallback mechanism,
and whether the fallback path is temporary (removed once telemetry shows 100% of active customers have
migrated) or permanent, to DESIGN.

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

**Signals checked**: >10 user stories? No (3). >3 bounded contexts/modules? No — confined to
`embyr-pg-storage`'s `backend_adapter.rs` (all four mirrored call sites, § Reading Confirmation),
`migrations/customer/` (one new migration file), and `embyr-db-prep`/the customer-migration-application
path (no code change there, but its OPERATIONAL role becomes load-bearing for this feature's rollout —
named explicitly, not a new module). Zero change to `embyr-core`'s domain types, `embyr-server/src/
grpc/handler.rs`'s proto translation, the composite-index gate, or `embyr-agent`'s own delegation (§
Reading Confirmation — one shared fix point). Walking skeleton >5 integration points? No (3): (1) a real
collection-group `RunQuery` against a real customer database, `EXPLAIN`-verified to use an index-assisted
plan, not a full scan; (2) a real backfill of pre-existing documents in an existing, live collection,
verified to not block concurrent writes; (3) a real collection-group query against a deliberately
un-migrated schema, verified to still return the correct result set. Estimated effort >2 weeks? No —
3 slices, each smaller than or comparable to `composite-index-real-creation`'s own 3-4 day total;
estimated 3-4 days across all 3 slices. Multiple independent user outcomes? The 3 stories serve the
SAME single outcome ("a collection-group query is fast where possible, always correct, never broken by
this feature's own rollout") and are not separately-pitchable features.

**Scope Assessment: PASS** (0 oversizing signals fired) — right-sized as one feature, 3 stories.

## Wave: DISCUSS / [REF] Story Map & Walking Skeleton

### Backbone

Alex's collection-group query already returns the correct result (today's bug is silent, not
incorrect) → it does so via a full sequential scan of the entire project, which compounds finding #5's
own real-index investment because that pattern can never use any btree → this feature adds a real,
indexable representation of "collection ID" and rewrites the query to use it, for schema-current
databases → existing customer documents get backfilled into that same indexable shape without
disrupting Sam Chen's own live production traffic → a customer database that has not yet received the
migration continues to answer collection-group queries correctly (only slower) rather than erroring or
returning a wrong result, for however long that customer's own rollout takes.

### Walking Skeleton

**US-01**: a collection-group query against a schema-current customer database is answered via a real,
index-assisted plan instead of a full project scan — the single riskiest, highest-value assumption: can
the query be correctly rewritten to use SOME indexable representation of collection ID, verified by a
real `EXPLAIN` plan change, with zero regression to the already-correct result set.

### Release 1 — Collection-Group Queries Are Fast, Safe to Roll Out, and Never Wrong (US-01, US-02,
US-03)

All three are necessary for this feature to actually close finding #17 as a High-severity fix, not
merely as a best-case demo against a freshly-provisioned, empty test database — mirroring
`composite-index-real-creation`'s own identical "both are necessary to actually close the finding"
reasoning, extended to three facets here because this feature's own risk surface (§ Business Context)
is genuinely larger: a synchronous-only fix would misrepresent this finding as closed while (a) leaving
every existing customer's existing documents unindexed until a separate, unspecified backfill
eventually runs, and (b) risking a hard failure or silently wrong results the moment new server code
meets an un-migrated customer database, which — given no fleet-wide migration mechanism exists (§
Reading Confirmation) — will be the common case, not the exception, for a sustained, potentially
indefinite period after this feature ships. No Release 2: unlike `composite-index-real-creation`'s own
deferred `DeleteIndex` symmetry facet (a lower-urgency, separable resource-hygiene concern), none of
this feature's three facets is safe to defer past the initial rollout.

## Wave: DISCUSS / [REF] Elephant Carpaccio Slices

| # | Story | Release | Estimate | Learning Hypothesis | Reference Class |
|---|---|---|---|---|---|
| 01 | US-01 | 1 (Walking Skeleton) | 1.5-2 days | Disproves: a collection-group query cannot be rewritten to use a real, indexable representation of collection ID without a materially larger change than a new column/index plus a query-predicate swap | New mechanism class for this specific query shape — nearest reference class is `composite-index-real-creation`'s own real-DDL-provisioning precedent (ADR-072), narrowed to a base-table schema change rather than a per-project admin action |
| 02 | US-02 | 1 | 1.5-2 days | Disproves: backfilling existing documents in a large, live customer collection cannot be done without measurably blocking that customer's own concurrent reads/writes | Mirrors `composite-index-real-creation`'s own `CREATE INDEX CONCURRENTLY`/non-blocking-build precedent (AC-CXR-06), applied to a value backfill rather than an index build |
| 03 | US-03 | 1 | 1 day | Disproves: new, fleet-wide server code can be deployed safely against a customer population most of which has not yet migrated its own schema, without a correctness or availability regression | Genuinely new risk class for this codebase — no prior feature this session has had to reason about embyr-controlled code rollout outpacing customer-controlled schema rollout; nearest partial precedent is the `agent-mode-*` features' own "agent binary version must match SaaS major version, policy only, no runtime enforcement" finding (same shape of risk, different subsystem) |

## Wave: DISCUSS / [REF] Prioritization

Ordered by genuine dependency and by what actually closes the audit's own High-severity classification:
US-01 first — proves the core mechanism (a real, index-assisted collection-group query) with no prior
precedent in this codebase for a base-table schema change to lean on beyond `composite-index-real-
creation`'s own per-project index-build precedent. US-02 second — cannot be demonstrated or meaningfully
designed before US-01's own new column/index shape exists, and is equally necessary to close finding #17
for any customer with pre-existing data (the overwhelming majority of real customers, by definition —
new, empty projects are the exception). US-03 third — logically independent of US-01/US-02's own
mechanism once it exists, but sequenced last here only because it is the regression-guard/safety-net
facet (verifying what does NOT change for a customer this feature has not yet reached), not because it
is lower-urgency: it must ship in the SAME release as US-01/US-02, since the fleet-wide code deploy that
makes US-01/US-02 real is the same deploy that creates US-03's own risk window.

## Wave: DISCUSS / [REF] System Constraints

- Zero change to `crates/embyr-server/src/grpc/handler.rs`'s own `RunQuery`/`RunAggregationQuery` proto
  translation, and zero change to `composite-index-real-creation`'s own `CreateIndex`/`DeleteIndex`
  mechanism or the `FAILED_PRECONDITION` gate (`requires_composite_index`/`is_index_ready`) — this
  feature is purely inside the four mirrored `all_descendants` predicate-construction sites (§ Reading
  Confirmation) plus the schema/backfill that makes a faster predicate possible.
- All four mirrored call sites (`run_query`; `run_aggregation_query`'s `Count`/`Sum`/`Avg` arms) must be
  updated identically — this codebase's own established convention (ADR-040 § 2, "copied byte-for-byte")
  for this exact WHERE-clause block; any DESIGN that updates fewer than all four reintroduces the drift
  the existing comments already warn against.
- `embyr-agent` requires NO separate fix — it delegates to the same `PostgresBackendAdapter` methods (§
  Reading Confirmation) — but IS an independently-deployed, customer-upgraded binary; US-03's
  correctness guarantee must hold for an un-upgraded `embyr-agent` binary talking to an un-migrated
  customer database exactly as it must for `embyr-server` itself.
- The schema change lands in `migrations/customer/` (a NEW customer-database migration file) — the
  FIRST-EVER schema change to the `documents` table since its original creation (§ Reading
  Confirmation). No prior ALTER-on-a-hot-path-table precedent exists in this codebase to reuse; DESIGN
  should treat this as a genuinely novel risk category for this codebase, not a routine migration.
- No automatic, fleet-wide mechanism exists today to re-apply `migrations/customer/` to an
  already-provisioned customer database (§ Reading Confirmation) — DESIGN must explicitly decide how
  EXISTING customers receive this migration (a new admin-triggered action; documentation instructing
  DBAs to re-run `embyr-db-prep`; some other mechanism) as part of this feature's own scope, not as an
  unstated assumption that it will simply happen.
- Correctness must never regress for a customer database that has not yet received the migration (US-03)
  — this is a hard requirement, not a DESIGN preference, given the confirmed absence of any
  embyr-controlled rollout schedule for customer-database schema changes.
- No new external dependency for the query/index mechanism itself is assumed by this DISCUSS (e.g., a
  trigram-index approach would require enabling the already-available `pg_trgm` Postgres extension,
  not a new crate) — DESIGN should confirm whether `pg_trgm` (or any other Postgres extension) is
  already enabled/available across this codebase's supported customer Postgres versions before selecting
  that option.
- **NFR — Postgres version/extension compatibility**: whichever mechanism DESIGN selects (§ Central
  Design Question A) must work on every Postgres major version this codebase's customer databases are
  documented to support (this DISCUSS did not locate a single, authoritative "supported customer
  Postgres versions" document — DESIGN should locate or establish one as part of Question A, since a
  `pg_trgm`-dependent or `GENERATED ALWAYS AS ... STORED`-dependent mechanism may have a real minimum
  version floor that a plain column + application-populated value would not). This is a hard constraint,
  not a nice-to-have: selecting a mechanism unavailable on even one supported customer Postgres version
  would make this feature un-deployable for that customer.
- Backfill and index-build work against a live customer collection must not measurably block that
  customer's own concurrent reads/writes (US-02) — mirrors AC-CXR-06's own precedent verbatim, applied
  to a value backfill in addition to an index build.
- Mutation-testing lesson, reapplied from this session's own accumulated discipline: any new PURE
  function this feature adds (e.g. a "last path segment" extraction helper, a schema-version/fallback
  decision function) gets unit tests written DURING DELIVER; a `cargo-mutants` pass is still budgeted at
  QUALITY_GATE regardless.

## Wave: DISCUSS / [REF] User Stories

### US-01: Alex's Collection-Group Query Uses a Real Index Instead of Scanning the Whole Project (Walking Skeleton)

**job_id**: JOB-01 | **Release**: 1 | **Persona**: P1 Alex

#### Elevator Pitch
Before: Alex's app runs `db.collectionGroup('reviews').where('rating', '>=', 4)` against Trailmark's
`trailmark-prod` project — Postgres answers it correctly, but by sequential-scanning every document in
`trailmark-prod`, across every collection, looking for any row whose `collection_path` happens to end
with `/reviews` (or equal `reviews`), because the query's own leading-`%` `LIKE` can never use any
btree, including the real composite indexes `composite-index-real-creation` now provisions.
After: run the identical `collectionGroup('reviews')` query against a schema-current
`trailmark-prod` → sees the same correct result set (unchanged) — but `EXPLAIN`ing the query now shows
Postgres's own planner choosing an index-assisted plan, not a full scan of the entire project.
Decision enabled: Alex trusts that a collection-group query stays fast as Trailmark's own project grows,
matching real Firestore's own behavior, instead of silently degrading as more unrelated collections and
documents accumulate in the same project.

#### Who
- Alex (P1) | SDK developer whose app issues `db.collectionGroup(...)` queries against a real customer
  project (e.g. Trailmark, `trailmark-prod`, `backend_mode=direct_pg`) with a schema-current database |
  Already trusts (per SPEC.md 913-921 and this DISCUSS's own confirmed reading) that collection-group
  queries return correct results | Needs that same query to also stay fast as the project's total
  document count grows, matching real Firestore's own collection-group indexing behavior.

#### Solution
The four mirrored `all_descendants` predicate-construction sites (§ System Constraints) are rewritten to
filter on a new, real, indexable representation of "collection ID" instead of a leading-`%` `LIKE`
against `collection_path`. The exact column/index/expression shape (§ Central Design Questions, Question
A) is DESIGN's own investigation; the observable requirement is that a collection-group query's own
result set is byte-for-byte unchanged from today, and that Postgres's planner can now choose an
index-assisted plan for it.

#### Domain Examples

**Example 1 (Happy Path — a collection-group query is answered via an index, not a scan)**: Trailmark
(`trailmark-prod`) has thousands of `products`, each with its own `reviews` subcollection
(`products/{productId}/reviews/{reviewId}`), totaling tens of thousands of review documents nested
among hundreds of thousands of unrelated documents in other collections. Alex's app issues
`db.collectionGroup('reviews').where('rating', '>=', 4)`. Before this feature: Postgres scans every
document in `trailmark-prod`, not just the `reviews` rows. After this feature: `EXPLAIN`ing the same
query shows an index-based plan touching only rows whose collection identity matches `reviews`, and the
returned reviews are identical to before.

**Example 2 (Edge Case — a top-level collection-group query, not just a nested one)**: A second
customer project has a collection-group query for `db.collectionGroup('logs')` where `logs` also exists
as a TOP-LEVEL collection (`collection_path = 'logs'`) in addition to nested instances
(`orgs/o1/logs`, `orgs/o2/logs`). Both the top-level and every nested `logs` collection's documents are
returned — result set unchanged from today's `collection_path = 'logs' OR collection_path LIKE
'%/logs'` semantics, now served via the new indexed representation instead of the LIKE.

**Example 3 (Error/Boundary — a collection-group query for a name that matches no documents)**: Alex
queries `db.collectionGroup('nonexistent_collection')` against `trailmark-prod`. Before and after this
feature: an empty result set is returned, quickly — after this feature, "quickly" is now backed by an
index lookup that finds zero matching rows, rather than a full scan that finds zero matching rows.

#### UAT Scenarios (BDD)

```gherkin
Scenario: A collection-group query returns the same correct results as before this feature
  Given Trailmark's "trailmark-prod" project has multiple "reviews" subcollections nested under
    different "products" documents, and unrelated documents in other collections
  When Alex runs a collectionGroup("reviews") query with a filter
  Then the returned documents are identical to the result the pre-existing LIKE-based query would
    have returned

Scenario: A collection-group query is answered via an index-assisted plan, not a full scan
  Given a composite index scenario identical to the one above, with a schema-current database
  When Alex runs the same collectionGroup("reviews") query with EXPLAIN
  Then Postgres's own query planner chooses a plan that does not perform a sequential scan of the
    entire "documents" table

Scenario: A collection-group query matches both a top-level and nested collections of the same name
  Given a project has a top-level collection "logs" and nested collections also named "logs" under
    different parents
  When a collectionGroup("logs") query is run
  Then documents from both the top-level "logs" collection and every nested "logs" collection are
    returned

Scenario: A collection-group query for a name with no matching documents returns an empty result
  Given a project has no collection anywhere named "nonexistent_collection"
  When a collectionGroup("nonexistent_collection") query is run
  Then an empty result set is returned

Scenario: A collection-group query with a compound filter still uses the index-assisted plan
  Given the same "reviews" collection-group scenario as above
  When Alex runs collectionGroup("reviews") with a compound filter (rating >= 4 AND verified == true)
  Then the returned documents are identical to what the pre-existing LIKE-based query would return
  And EXPLAIN shows the plan is still index-assisted on the collection-identity predicate, with the
    compound filter applied on top, not replacing the index usage

Scenario: A collection-group query returning a large result set remains index-assisted
  Given a "reviews" collection group has several thousand matching documents across many parent products
  When Alex runs collectionGroup("reviews") with EXPLAIN
  Then the plan remains index-assisted (not a fallback to a sequential scan) regardless of the number of
    matching rows returned

Scenario: RunAggregationQuery collection-group Count/Sum/Avg are unaffected in result, faster in plan
  Given the same "reviews" collection-group scenario as above
  When Alex runs RunAggregationQuery Count, Sum, and Avg with all_descendants=true for "reviews"
  Then each returns the same value as the pre-existing LIKE-based query would have returned
  And EXPLAIN for each shows an index-assisted plan, not a full sequential scan
```

#### Acceptance Criteria
- [ ] AC-CGI-01: a collection-group `RunQuery` against a schema-current customer database returns a
      result set identical to today's `LIKE`-based query, for both top-level and nested collections of
      the matching name.
- [ ] AC-CGI-02: `EXPLAIN` of a collection-group `RunQuery` against a schema-current customer database
      shows an index-assisted plan (an index scan/index-only scan), not a sequential scan of the entire
      `documents` table.
- [ ] AC-CGI-03: all three `RunAggregationQuery` arms (`Count`, `Sum`, `Avg`) return values identical to
      today's `LIKE`-based query when `all_descendants=true`, and each is verified via `EXPLAIN` to use
      an index-assisted plan against a schema-current database.
- [ ] AC-CGI-04 (regression guard): non-collection-group (`all_descendants=false`) queries are unchanged
      — this feature touches only the `all_descendants=true` branch of all four mirrored call sites.
- [ ] AC-CGI-05 (regression guard): `composite-index-real-creation`'s own ACs (AC-CXR-01 through
      AC-CXR-12) and the `FAILED_PRECONDITION` gate's own existing behavior are unchanged.

#### Outcome KPIs
- **Who**: Alex, building against real customer projects that use `db.collectionGroup(...)` queries
  (e.g. Trailmark, any `backend_mode`).
- **Does what**: gets a collection-group query result via an index-assisted Postgres plan instead of a
  full project scan, for any customer database that has received this feature's schema migration.
- **By how much**: from 0% of collection-group queries using any index today (100% are full project
  scans, confirmed by this DISCUSS's own reading) to 100% of collection-group queries against a
  schema-current database using an index-assisted plan, with zero change to any returned result.
  **Performance target (working default, not evidenced by real customer scale data — DESIGN/DELIVER
  should confirm against an actual customer-representative document count before treating as final)**:
  for a collection-group query over a collection group with on the order of tens of thousands of
  matching documents nested among hundreds of thousands of unrelated documents in the same project
  (Example 1's own scale), p99 wall-clock latency for the index-assisted path is no more than 20% of the
  p99 latency the equivalent full-scan path exhibits on the same data — chosen as a directional
  correctness check on the mechanism's real-world payoff, not merely its plan type.
- **Measured by**: AC-CGI-02/AC-CGI-03's own `EXPLAIN`-based before/after plan comparison (chosen as the
  PRIMARY proof, consistent with `composite-index-real-creation`'s own precedent — cheaper, more
  reliable, and directly verifies the mechanism rather than an environment-sensitive proxy), PAIRED with
  a wall-clock p99-latency before/after measurement against the performance target above as a secondary,
  directional confirmation that the mechanism actually pays off at realistic scale.
- **Baseline**: 0% — confirmed directly by this DISCUSS's own reading of all four mirrored
  `all_descendants` call sites (§ Reading Confirmation), none of which can be satisfied by any existing
  btree.

### US-02: Backfilling Existing Documents Never Blocks a Live Customer's Reads or Writes

**job_id**: JOB-01 | **Release**: 1 | **Persona**: P2 Sam Chen (operational stakeholder; see § Persona & Job)

#### Elevator Pitch
Before: this scenario cannot occur today (no schema change or backfill exists) — but the moment US-01
ships a schema change requiring existing documents to be backfilled into the new indexable
representation, Sam Chen's own production deployment is at real risk: a naive, unbatched
backfill (or a `GENERATED ALWAYS AS (...) STORED` column added to an existing large table) can hold a
long-running, write-blocking operation against Trailmark's own live `products`/`reviews` tables for as
long as the backfill takes.
After: an operator (or the automated migration path) backfills a large, actively-written customer
collection's existing documents into the new indexable form — production reads and writes to that
collection continue to succeed, at their normal rate, throughout the backfill, and every backfilled
document becomes correctly served by US-01's own index-assisted collection-group queries once the
backfill for it completes.
Decision enabled: Sam Chen can trust that rolling out this fix to an existing customer's own live,
large-scale production data never becomes an unplanned availability incident.

#### Who
- Sam Chen (P2) | Service operator responsible for existing customer databases (e.g. Trailmark's own
  `trailmark-prod`, already carrying real production data before this feature ships) | Needs the
  backfill required to realize US-01's own performance fix to be a safe, non-disruptive operation
  against live production traffic, not a maintenance-window event.

#### Solution
Existing documents in an existing customer database are backfilled into whatever indexable
representation Question A/B (§ Central Design Questions) selects, using a mechanism that never holds a
long-duration, write-blocking lock against the live `documents` table — mirroring
`composite-index-real-creation`'s own `CREATE INDEX CONCURRENTLY`/non-blocking precedent (AC-CXR-06)
for the index half of this problem, and requiring an equivalent non-blocking discipline for the value
-backfill half. The exact batching/throttling/trigger mechanism is DESIGN's own investigation.

#### Domain Examples

**Example 1 (Happy Path — a large, live collection's writes are unaffected during backfill)**:
Trailmark's `trailmark-prod` project has grown to hundreds of thousands of pre-existing documents under
continuous write traffic from Alex's own app, predating this feature. An operator (or an automated
migration step) backfills the new indexable representation for all of them. While the backfill is in
progress, Trailmark's own reads and writes continue to succeed at their normal rate — no operation is
blocked or measurably delayed by the backfill.

**Example 2 (Edge Case — a document written during the backfill window)**: A new review document is
written to `trailmark-prod`'s `reviews` collection WHILE the backfill for pre-existing documents is
still in progress. That newly-written document arrives already carrying the new indexable
representation (populated at write time, not requiring backfill) and is immediately, correctly served
by a collection-group query.

**Example 3 (Error/Boundary — the backfill is interrupted partway through)**: The backfill process is
interrupted (e.g. `embyr-server` restarts, or a transient database blip occurs) after backfilling only
some of `trailmark-prod`'s pre-existing documents. The backfill is safely resumable — it does not
re-process already-backfilled documents unnecessarily, and does not leave the collection in a state
where some documents are silently never picked up.

#### UAT Scenarios (BDD)

```gherkin
Scenario: Backfilling a large, actively-written collection does not block writes
  Given Trailmark's "products"/"reviews" collections have a large number of pre-existing documents
    and are under continuous write traffic
  When the backfill for the new indexable collection representation runs against them
  Then writes to those collections continue to succeed throughout the backfill
  And no write is blocked or measurably stalled by the backfill in progress

Scenario: A document written during the backfill window is immediately correctly indexed
  Given a backfill for pre-existing documents is still in progress
  When a new document is written to the same collection during that window
  Then the new document already carries the new indexable representation
  And a collection-group query immediately and correctly includes it

Scenario: An interrupted backfill is safely resumable
  Given a backfill has processed only some of a collection's pre-existing documents when interrupted
  When the backfill is resumed
  Then already-backfilled documents are not unnecessarily reprocessed
  And every remaining pre-existing document is eventually backfilled

Scenario: An interrupted-then-resumed backfill produces no data corruption or duplication
  Given a backfill was interrupted partway through and then resumed to completion for a collection
  And an equivalent control collection with identical starting data completed an uninterrupted backfill
  When both collections' documents are spot-checked
  Then every document in the interrupted-then-resumed collection carries a populated, non-null new
    indexable representation, with no exceptions
  And no document shows evidence of being written twice by the backfill process itself
  And the interrupted-then-resumed collection's documents match the control collection's documents
    value-for-value on the new indexable representation

Scenario: A fully-backfilled collection's collection-group queries all use the new index
  Given a collection's backfill has fully completed
  When a collection-group query against that collection is run with EXPLAIN
  Then the plan is index-assisted for every document in that collection, including ones that existed
    before the backfill ran
```

#### Acceptance Criteria
- [ ] AC-CGI-06: backfilling a customer collection's pre-existing documents under active, concurrent
      write traffic does not block or fail those writes, and does not increase their own p99 latency by
      more than 10% versus that collection's pre-backfill baseline, measured via the same latency
      instrumentation the write path already emits — no write is ever blocked waiting on a lock the
      backfill itself holds (verified via `pg_locks`/`pg_stat_activity` showing zero lock contention
      attributable to the backfill process during the test window). (Working default threshold —
      DESIGN/DELIVER should confirm 10% against real customer write-latency data if available; the
      non-negotiable invariant is "no write ever blocks on the backfill," the percentage is a
      measurability aid, not the requirement itself.)
- [ ] AC-CGI-07: a document written during an in-progress backfill for its own collection already
      carries the new indexable representation at write time, and is immediately correctly served by a
      collection-group query.
- [ ] AC-CGI-08: a backfill interrupted partway through and then resumed to completion leaves the
      collection in a state that is byte-for-byte identical, for every document, to the state an
      uninterrupted backfill would have produced — specifically: (a) every document in the collection
      carries a populated (non-null) new indexable representation, zero exceptions; (b) no document's
      new indexable representation value differs between the interrupted-then-resumed run and an
      uninterrupted control run; (c) no document is written/updated twice by the backfill process itself
      (verified via `version`/`update_time` showing exactly one backfill-attributable write per
      document); (d) already-backfilled documents are not redundantly reprocessed on resume (verified by
      the resumed run's own row-touch count excluding already-completed documents).
- [ ] AC-CGI-09: once a collection's backfill fully completes, `EXPLAIN` of a collection-group query
      against it shows an index-assisted plan covering documents that existed before the backfill ran,
      not just ones written afterward.

#### Outcome KPIs
- **Who**: Sam Chen, operating production embyr deployments serving customers with large, pre-existing
  collections (e.g. Trailmark's own `trailmark-prod`, already in production before this feature ships).
- **Does what**: rolls out this feature's own schema/backfill requirement to an existing, live customer
  database without it ever becoming a write-availability incident.
- **By how much**: from "not applicable" today (no backfill mechanism exists) to 0 measurable
  write-latency/availability regression during backfill for a large, actively-written collection, and
  100% of pre-existing documents eventually reaching the new indexable representation.
- **Measured by**: AC-CGI-06's own real concurrent-write-during-backfill test; AC-CGI-09's own
  before/after `EXPLAIN` proof covering pre-existing documents specifically.
- **Baseline**: not applicable — today's absence of any schema change or backfill mechanism means no
  real backfill-safety risk currently exists; this KPI measures the NEW risk this feature itself
  introduces by making the backfill real, and proves it is safely mitigated before shipping.

### US-03: A Collection-Group Query Is Never Wrong or Broken Against a Database That Hasn't Been Migrated Yet

**job_id**: JOB-01 | **Release**: 1 | **Persona**: P2 Sam Chen (fleet operational-safety stakeholder)

#### Elevator Pitch
Before: this exact risk does not exist today, because every deployed `embyr-server`/`embyr-agent`
binary and every customer database agree on the same (slow, `LIKE`-based) schema — there is no version
skew to reason about.
After: `embyr-server`/`embyr-agent` ship new, fleet-wide code that assumes the new indexable
representation exists — but most customer databases, at the moment of that release, have NOT yet had
their own DBA re-run the migration (§ Reading Confirmation — no automatic, embyr-controlled rollout
exists for already-provisioned customer databases). A collection-group query against one of those
still-unmigrated databases continues to return the exact correct result it returns today (via the
existing `LIKE`-based path as a fallback), never a hard error and never a wrong result — only slower,
exactly as it is today, until that customer's own database is migrated.
Decision enabled: Sam Chen can ship this feature's own server-code release fleet-wide on embyr's own
schedule, independent of any individual customer's own migration timeline, without risking an outage or
silent data-correctness regression for any customer who has not yet migrated.

#### Who
- Sam Chen (P2) | Service operator responsible for a fleet-wide `embyr-server`/`embyr-agent` release
  reaching a population of customer databases whose own schema migration timing is customer-controlled,
  not embyr-controlled | Needs new query-building code to behave safely and correctly against BOTH
  schema states simultaneously, for however long that mixed-state window lasts.

#### Solution
The rewritten collection-group query path (US-01) detects whether a given customer database has the
new indexable representation available and, if not, falls back to today's existing `LIKE`-based query —
correctness preserved unconditionally, performance improved only once that customer's own database has
been migrated. The exact detection/fallback mechanism (a schema probe, a caught error, or another
approach) and whether the fallback is temporary or permanent are DESIGN's own investigation (§ Central
Design Questions, Question C).

#### Domain Examples

**Example 1 (Happy Path — an un-migrated customer's queries keep working, unchanged)**: A second
customer, Meridian Health (`backend_mode=agent`, per this codebase's own established persona
convention), has not yet had its own DBA re-run `embyr-db-prep` at the moment this feature's new server
code is deployed fleet-wide. Meridian's own collection-group queries continue to return correct results
via the existing `LIKE`-based path — no error, no missing documents, no behavior change visible to
Meridian's own application.

**Example 2 (Edge Case — a customer migrates mid-operation)**: Trailmark's own DBA re-runs
`embyr-db-prep` against `trailmark-prod` at some point after this feature's server code has already
been live for that project. Collection-group queries issued before that point used the fallback path;
queries issued after it automatically use the new index-assisted path — no restart, redeploy, or
explicit cutover action is required on embyr's side for that individual customer's transition.

**Example 3 (Error/Boundary — the schema-detection mechanism itself must not silently misreport)**: An
unmigrated customer database and a partially-migrated one (e.g., the new column exists per US-01's DDL,
but that specific collection's own documents have not yet completed backfill per US-02) must both be
handled without a wrong result — a collection-group query against a partially-backfilled collection
still returns EVERY matching document (both already-backfilled and not-yet-backfilled ones), not just
the subset the new index currently covers.

#### UAT Scenarios (BDD)

```gherkin
Scenario: A collection-group query against an un-migrated customer database still returns correct results
  Given a customer project's database has not yet had this feature's schema migration applied
  When a collectionGroup query is run against it
  Then the exact same correct result set is returned as the pre-existing LIKE-based query would return
  And no error occurs

Scenario: A customer's queries automatically improve once their own database is migrated, with no
  embyr-side action required
  Given a customer project's database has just had this feature's schema migration applied
  When a collectionGroup query is run against it afterward
  Then the query now uses the new index-assisted path
  And no redeploy, restart, or explicit cutover of embyr-server was required for that transition

Scenario: A partially-backfilled collection's collection-group query still returns every matching document
  Given a collection has some documents already backfilled into the new indexable representation and
    some pre-existing documents not yet backfilled
  When a collectionGroup query against that collection is run
  Then every matching document is returned, both already-backfilled and not-yet-backfilled ones

Scenario: An un-upgraded embyr-agent binary talking to an un-migrated customer database also stays correct
  Given a backend_mode=agent customer project is running an embyr-agent binary predating this feature,
    against a database that also predates this feature's migration
  When a collectionGroup query is run through that agent
  Then the exact same correct result set is returned as before this feature existed
```

#### Acceptance Criteria
- [ ] AC-CGI-10: a collection-group query against a customer database that has not received this
      feature's schema migration returns the exact same correct result set as today's `LIKE`-based
      query, with no error.
- [ ] AC-CGI-11: a customer database migrated after this feature's server code is already live
      automatically begins using the index-assisted path for subsequent queries, with no
      embyr-server/embyr-agent restart, redeploy, or explicit per-customer cutover required.
- [ ] AC-CGI-12: a collection-group query against a collection in a partially-backfilled state (§
      US-02) returns every matching document, regardless of whether each individual document has
      completed backfill.
- [ ] AC-CGI-13 (regression guard): for a schema-current customer database, the presence of the
      fallback/detection mechanism (US-03's own schema-state check) adds no more than 5% to the p99
      latency of a collection-group query using the index-assisted path, measured by comparing (a) a
      query executed with the detection mechanism active against (b) the identical query executed with
      detection short-circuited/pre-confirmed schema-current — both against the SAME schema-current
      database. (Working default threshold — DESIGN/DELIVER should confirm 5% is appropriate once the
      detection mechanism's exact shape, per Central Design Question C, is chosen; the non-negotiable
      invariant is "detection overhead must not erase US-01's own index-assisted performance gain.")

#### Outcome KPIs
- **Who**: Sam Chen, operating a fleet-wide `embyr-server`/`embyr-agent` release schedule independent
  of any individual customer's own database-migration timeline.
- **Does what**: ships this feature's own server-code release to the entire fleet on embyr's own
  schedule without any customer (migrated or not) experiencing an outage, error, or wrong result.
- **By how much**: from an unquantified, previously-unexamined risk (§ Reading Confirmation — this exact
  mismatch had never been surfaced before this DISCUSS) to 0 customers experiencing a correctness or
  availability regression at release time, verified across both schema states (migrated and
  un-migrated) and both binaries (`embyr-server`, `embyr-agent`).
- **Measured by**: AC-CGI-10/AC-CGI-12's own correctness proofs against a deliberately un-migrated and a
  deliberately partially-backfilled test database respectively; AC-CGI-11's own no-restart-required
  transition proof.
- **Baseline**: not applicable — this risk does not exist until US-01's own schema change ships; this
  KPI proves the risk US-01/US-02 would otherwise introduce (a fleet-wide code release outpacing
  customer-controlled schema rollout) is fully mitigated before shipping.

## Wave: DISCUSS / [REF] Out of Scope

- **Choosing among the four indexing mechanisms named in Central Design Question A** — reversed-string
  index, trigram/GIN, separate `collection_id` column, or partial/expression index — this DISCUSS
  surveys the space per the task's own explicit instruction but does not lock the mechanism; DESIGN's
  call.
- **A generic, reusable background-job-queue or migration-orchestration framework for customer-database
  rollouts** — mirrors `composite-index-real-creation`'s own identical Out-of-Scope reasoning (no other
  feature in this codebase needs one yet); the backfill mechanism (US-02) should reuse the simplest
  sufficient mechanism, not a new infrastructure class, pending DESIGN's own confirmation.
- **Automatically forcing or scheduling every existing customer's own migration** — this feature
  guarantees correctness for un-migrated customers (US-03); it does not build a mechanism to compel or
  auto-trigger any given customer's own DBA to actually run `embyr-db-prep`. That operational/business
  process (documentation, customer communication, an admin-visible "not yet migrated" indicator) is
  named as a related follow-up concern, not built here.
- **`backend_mode = agent` requiring its own separate query-building fix** — confirmed not needed (§
  Reading Confirmation): `embyr-agent` delegates to the same `PostgresBackendAdapter` code this feature
  already fixes once.
- **Extending this fix to any other query shape or predicate pattern** — this feature is scoped
  exclusively to the four mirrored `all_descendants=true` LIKE-construction sites named in finding #17;
  no other leading-wildcard `LIKE` pattern is known to exist elsewhere in this codebase (not
  investigated as part of this DISCUSS; a candidate follow-up audit item if one is suspected).
- **Any code implementation** — this is DISCUSS only, per explicit task instruction; DESIGN performs the
  actual investigation and implementation planning.

## Wave: DISCUSS / [REF] Walking Skeleton Strategy

**Strategy A** (real, minimal, end-to-end) — every scenario is a real collection-group `RunQuery`/
`RunAggregationQuery` against a real customer database, with real `EXPLAIN`-based plan verification, a
real backfill against real pre-existing documents under real concurrent write traffic, and a real
deliberately-un-migrated database for US-03's own correctness proof — mirroring this session's own
established Strategy A precedent for every production-readiness-audit-derived fix.

## Wave: DISCUSS / [REF] Driving Ports

gRPC `:8080` — the SAME existing `RunQuery`/`RunAggregationQuery` RPCs, unchanged request/response wire
shape (`all_descendants` is an existing, already-vendored proto field) — zero new endpoint, zero
request/response shape change. This feature changes only what happens BEHIND those already-shipped
handlers, plus a new customer-database schema migration file applied via the existing
`embyr-db-prep`/provisioning-time migration paths (§ Reading Confirmation) — no new driving port.

## Wave: DISCUSS / [REF] Pre-requisites

- `composite-index-real-creation` (FINALIZED 2026-09-09, ADR-072) — provides the `CREATE INDEX
  CONCURRENTLY`/async-build/status-lifecycle precedent this feature's own index-build half (US-02)
  should reuse where applicable; this feature does not modify that feature's own mechanism.
- `composite-index-requirement-rules` (FINALIZED 2026-09-05) — provides the accurate `requires_
  composite_index` gate this feature does not touch.
- ADR-022 (`embyr-db-prep`, customer-migration consolidation) — the sole embed point for `migrations/
  customer/`, and the mechanism this feature's own new migration file will be delivered through; this
  feature does not modify `embyr-db-prep` itself, only adds a migration file it will apply.
- No new external dependency assumed; DESIGN should confirm `pg_trgm` availability (if that mechanism is
  selected) against this codebase's supported customer Postgres versions before locking it in.
- No new bounded context, no new dependency edge beyond one new `migrations/customer/` file and the
  `backend_adapter.rs` predicate-construction change (§ System Constraints).

## Wave: DISCUSS / [REF] Definition of Ready Validation

### DoR Checklist (9-item hard gate)
1. [x] Every story traces to a job_id (JOB-01, all 3 stories).
2. [x] Every story has a complete Elevator Pitch (Before / After / Decision enabled).
3. [x] Every AC is testable without ambiguity (13 ACs across 3 stories, each a real result-set-equality
   proof, a real `EXPLAIN` plan proof, a real concurrent-write/backfill proof, or an explicit
   un-migrated/partially-migrated correctness proof).
4. [x] Walking Skeleton identified (US-01).
5. [x] Scope Assessment passed.
6. [x] No slice contains only `@infrastructure` stories — every story directly enables a named Decision
   (Alex trusting collection-group queries stay fast as his project grows; Sam Chen trusting a backfill
   against live production data is safe; Sam Chen trusting a fleet-wide release never breaks or
   mis-answers a query for a customer who hasn't migrated yet).
7. [x] Out of Scope explicitly named (6 items, each reasoned).
8. [x] Outcome KPIs have numeric/proportional targets and measurement methods (3 stories, each with its
   own Who/Does What/By How Much/Measured By/Baseline).
9. [x] Prior-wave artifacts read and reconciled — `composite-index-real-creation`'s own ADR-072
   precedent, `composite-index-requirement-rules`' own unmodified gate, and both prior features' own
   established Trailmark/Alex/Sam Chen persona and domain-example conventions are reused consistently;
   no contradiction found with either feature's own locked decisions (this feature composes with, and
   closes the compounding gap left by, `composite-index-real-creation` — an explicitly named
   relationship, not a silent overlap).

### DoR Status: **PASSED**

## Wave: DISCUSS / [REF] Wave Decisions Summary

### Key Decisions
- [D1] Reuse JOB-01 — a fourth "make it real"/performance-parity realization in the same family as
  `composite-index-real-creation`, this time for collection-group queries specifically.
- [D2] Confirmed, by direct reading (SPEC.md 913-921 against the current implementation), that finding
  #17 is a PURE performance gap — the existing result semantics are already correct; this feature must
  not change any returned result, only the plan used to produce it.
- [D3] Confirmed, by direct reading, that finding #17 "compounds #5" specifically because a leading-`%`
  `LIKE` predicate structurally defeats any btree — including every real composite index
  `composite-index-real-creation` now provisions — meaning that feature's own fix currently delivers
  zero benefit to collection-group queries; this feature's own Central Design Question A opening
  observation (a `collection_id` column) is the one mechanism that would also let collection-group
  queries begin benefiting from composite indexes going forward, named as a reuse opportunity for
  DESIGN, not locked.
- [D4] Elevated, newly-discovered risk NOT present in either prior composite-index feature: because
  `documents` lives in each customer's own Postgres database, and no fleet-wide mechanism exists to
  re-apply `migrations/customer/` to an already-provisioned customer database, this feature's own
  server-code rollout (embyr-controlled, fleet-wide) will outpace its own schema rollout (customer
  -controlled, per-database) for a sustained, indefinite window. This DISCUSS locks that correctness
  must never regress during that window (US-03) as a hard, in-scope requirement — not deferred, not
  assumed away.
- [D5] The story split is 3 stories (US-01 real-index-and-usage walking skeleton; US-02 non-blocking
  backfill safety; US-03 fleet-wide schema-version-mismatch correctness), all in a SINGLE Release 1 —
  unlike `composite-index-real-creation`'s own 2-release split, no facet of this feature is safe to
  defer past initial rollout, given D4's own risk framing.
- [D6] Four genuinely open engineering trade-offs (the indexable-representation mechanism; the backfill
  batching/throttling mechanism; the schema-version detection/fallback mechanism and its
  temporary-vs-permanent lifetime; whether `pg_trgm` or an equivalent extension is available and
  appropriate) are explicitly NOT resolved here — each is a Central Design Question for DESIGN, with
  reading-derived facts and reuse candidates (ADR-072's own `CREATE INDEX CONCURRENTLY` precedent; the
  existing `(project_id, collection_path)` index shape as a template for a new `(project_id,
  collection_id)` equality index) offered as a starting point, not locked.

## Wave: DISCUSS / [REF] Peer Review

`nw-product-owner-reviewer` ran one review iteration against this artifact. Result: `conditionally_
approved`, zero blocking issues, zero critical issues. Five High-severity clarity/testability findings
were raised, all resolved in this same DISCUSS wave (not deferred to DESIGN, per the "resolve
critical/high issues before handoff" discipline):

1. AC-CGI-06 ("measurably stall") — replaced with a concrete p99-latency-increase threshold (10%,
   working default) plus a lock-contention invariant.
2. AC-CGI-08 ("safely resumable") — replaced with four explicit, independently-checkable invariants
   (no-null coverage, value-identical-to-control, no-double-write, no-redundant-reprocessing).
3. AC-CGI-13 ("already-measured baseline") — reworded to a relative, self-contained comparison (detection
   overhead vs. detection-short-circuited, both against a schema-current database) instead of an
   undefined cross-reference.
4. US-01's Outcome KPI — a wall-clock p99-latency performance target added alongside the existing
   `EXPLAIN`-based mechanism proof, explicitly labeled a working default pending real customer-scale
   data, so DESIGN/DELIVER have a directional payoff check, not only a plan-type check.
5. A Postgres-version/extension-compatibility NFR added to § System Constraints, since Central Design
   Question A's candidate mechanisms (`pg_trgm`, `GENERATED ALWAYS AS ... STORED`) carry real version
   floors this DISCUSS did not evidence against an authoritative supported-versions list.

Two additional (non-blocking, "before DELIVER") suggestions were also folded in immediately rather than
deferred: a compound-filter and a large-result-set UAT scenario added to US-01; a data-integrity
spot-check UAT scenario (no nulls, no duplicate writes, control-collection value match) added to US-02.
No second review iteration was required — all findings were mechanically resolvable without
re-litigating any locked DISCUSS decision.

## Wave: DISCUSS / [REF] SSOT Updates

- Recommended (not applied by this DISCUSS wave, per this session's own established precedent of
  leaving `jobs.yaml` edits to be explicitly applied rather than assumed — see `composite-index-real-
  creation`'s own identical, still-unapplied recommendation): append a JOB-01 NOTE to
  `docs/product/jobs.yaml`, dated `(collection-group-query-index DISCUSS, 2026-09-14)`, recording that
  JOB-01 now also covers collection-group query PERFORMANCE (distinct from `composite-index-real-
  creation`'s own composite-index performance realization — a different query shape, same "make it
  real" pattern), and naming the newly-discovered fleet-vs-customer schema-rollout-timing risk (D4) as
  a reusable finding for any FUTURE feature that also requires a `migrations/customer/` schema change.

## Wave: DESIGN / [REF] Reading Confirmation

✓ `crates/embyr-pg-storage/src/backend_adapter.rs` re-read directly around every `INSERT INTO documents`
site (347, 398, 519, 1252, 1312) — five sites, not the four mirrored `LIKE`-construction sites this
finding is itself about. This is the fact that decides Decision A below: any write-time population of a
new column in Rust would recreate, at a FIFTH+ site, the exact copy-paste-drift risk ADR-040 § 2 already
warns about for the four query sites.
✓ The two `ON CONFLICT ... DO UPDATE` clauses (401/1256, 1316) and the soft-delete `UPDATE` (1276-1280)
read directly: none assigns `collection_path` — confirms `collection_path` is immutable per-row after
insert, so a `BEFORE INSERT`-only trigger is sufficient (no `BEFORE UPDATE` needed, `collection_id` never
needs recomputation after its one insert-time write).
✓ `handler.rs:151` (`let collection_path = segments.join("/")`) read directly — confirms every
`collection_path` this codebase constructs has no empty segments, no leading/trailing slash, single `/`
separators, satisfying the precondition `regexp_replace(collection_path, '^.*/', '')` (extract the last
path segment) relies on. DISCUSS's own Question A asked DESIGN to verify this against the document-path-
construction code rather than assume it; done.
✓ `migrations/customer/0001_documents.sql` re-read: confirms `collection_id VARCHAR(1500)` with no
default and no `GENERATED` clause is the only column-add shape with zero Postgres-minimum-version risk —
`GENERATED ALWAYS AS (...) STORED` requires a full-table rewrite (`ACCESS EXCLUSIVE` for the statement's
duration) in every Postgres version this codebase's migrations currently target, confirmed by direct
reasoning about how Postgres computes a generated expression for every existing row at `ALTER TABLE` time
(unavoidable for `STORED`; PG18's *virtual* generated columns avoid it, but no authoritative supported-
customer-Postgres-versions document exists in this codebase to justify assuming that floor, per DISCUSS's
own NFR finding).
✓ ADR-072 (`docs/product/architecture/adr-072-composite-index-real-provisioning.md`) read in full:
directly reused, unmodified, for (a) the `CREATE INDEX CONCURRENTLY IF NOT EXISTS` + `indisvalid`
post-build re-check + best-effort `DROP INDEX CONCURRENTLY` cleanup pattern (Decision B there, Decision D
here), and (b) confirmation that `CONCURRENTLY` cannot run inside a `sqlx` migration transaction/`.begin()`
block — this feature's two new indexes are therefore built OUTSIDE `migrations/customer/0002_...sql`, as a
separate runtime step, exactly mirroring ADR-072's own established constraint.
✓ ADR-022 (`docs/product/architecture/adr-022-customer-db-prep-crate-and-migration-consolidation.md`) read
in full: confirms `PostgresBackendAdapter::migrate()` is the sole embed point for
`sqlx::migrate!("../../migrations/customer")`, called from `embyr-db-prep` and from `provision.rs`'s three
`backend_mode` branches — this feature's own backfill/index-preparation calls are added at those SAME call
sites, immediately after `migrate()`, reusing ADR-022's single-embed-point discipline rather than adding a
new orchestration path. Also confirms `embyr-db-prep` depends only on `embyr-pg-storage`/`embyr-core`/
`sqlx`/`tokio` (no `embyr-server`) — this is why the new backfill/index-build logic must live in
`embyr-pg-storage` (on `PostgresBackendAdapter`, alongside `migrate()`) rather than in
`embyr-server/src/adapters/composite_index_builder.rs`, where ADR-072's structurally similar logic lives —
that module is unreachable from `embyr-db-prep`.
✓ `crates/embyr-agent/src/server.rs` (lines 22, 71, 462-660, 953) re-confirmed per DISCUSS's own reading:
`StorageAgent` shares the SAME `PostgresBackendAdapter` instance — the schema-capability probe (Decision C)
therefore covers `embyr-agent`'s own query path for free, with zero `embyr-agent`-specific code.

## Wave: DESIGN / [REF] Architecture Decisions

ADR-080 (`docs/product/architecture/adr-080-collection-group-query-index.md`) resolves all four Central
Design Questions DISCUSS left open, and is the primary DESIGN artifact for this feature. Summary:

**Question A (indexable representation)** — a plain, nullable `collection_id VARCHAR(1500)` column,
populated by a new `BEFORE INSERT` trigger (`documents_set_collection_id()`), NOT a `GENERATED ALWAYS AS
(...) STORED` column (full-table-rewrite risk, confirmed) and NOT a pure expression index with no column
at all (rejected specifically against AC-CGI-08(d) — an interrupted `CREATE INDEX CONCURRENTLY` cannot be
resumed without a full, from-scratch rebuild that reprocesses already-covered rows; a real column
backfilled via an explicit `WHERE collection_id IS NULL` loop resumes for free). The trigger, not Rust
write-path code, is the write-time-population mechanism specifically because five independent `INSERT`
call sites exist (§ Reading Confirmation) — populating in Rust would recreate this finding's own root
cause (copy-paste-duplicated logic across mirrored call sites) at a new location.

**Question B (backfill mechanism)** — a resumable, throttled, `FOR UPDATE SKIP LOCKED` batched `UPDATE ...
WHERE collection_id IS NULL LIMIT <batch_size>` loop, each batch its own short transaction, terminating
when a batch affects zero rows. Resumability and "no redundant reprocessing" (AC-CGI-08) fall out of the
`WHERE collection_id IS NULL` predicate with no cursor/checkpoint/idempotency-token machinery. Runs
synchronously (no `tokio::spawn`, no job-queue) from `embyr-db-prep` (a one-shot CLI a DBA already expects
to wait on) and from `provision.rs`'s three branches (a guaranteed no-op for fresh projects) — a deliberate
divergence from ADR-072's async-task pattern, justified by this feature's different caller shape (no
HTTP request/response needing to return before the work finishes).

**Question C (schema-version detection/fallback)** — a cached `SchemaCapabilityProbe` on
`PostgresBackendAdapter` (one instance per customer database, shared by `embyr-server` and `embyr-agent`):
`information_schema.columns` catalog lookup, cached permanently once `Available` (migrations are
additive-only, ADR-022 — never un-happens), cached with a 30s working-default TTL once `Unavailable` (lets
a customer's completed migration be picked up automatically, AC-CGI-11, without a restart). Not a
per-query try/catch on `undefined_column` — that would put error-handling machinery in the hot path of the
eventual, intended-universal fully-migrated steady state.

**Question C, lifetime**: the fallback path is PERMANENT, not temporary — it is not scheduled for removal
once telemetry shows 100% fleet migration, because no mechanism compels any given customer to ever migrate
(DISCUSS's own Out-of-Scope) and a customer could in principle never re-run `embyr-db-prep`. Removing the
fallback would reintroduce a hard-failure risk for that customer with no compensating benefit; keeping it
costs one cached boolean check per adapter instance, not a recurring cost that grows with time.

**Query predicate (all four mirrored call sites)**: for `Available`, a single hybrid shape —
`collection_id = $N OR (collection_id IS NULL AND (collection_path = $N OR collection_path LIKE '%/' ||
$N))` — correct and index-assisted across the entire backfill lifecycle (not just post-completion),
because AC-CGI-12 requires per-document correctness regardless of that document's own backfill state, and
the fleet-wide server-code deploy cannot know any individual customer's backfill progress. For
`Unavailable`, today's `LIKE`-only predicate, byte-for-byte unchanged. Two new partial indexes
(`documents_collection_group_idx` on `(project_id, collection_id) WHERE NOT deleted AND collection_id IS
NOT NULL`; `documents_collection_group_pending_idx` on `(project_id) WHERE NOT deleted AND collection_id
IS NULL`) are built via `CREATE INDEX CONCURRENTLY IF NOT EXISTS` BEFORE the backfill loop runs, so
AC-CGI-02 holds from the moment a database becomes schema-current, not only after backfill later
completes.

## Wave: DESIGN / [REF] Applied to All Four Mirrored Call Sites

Per § System Constraints' own lock ("any DESIGN that updates fewer than all four reintroduces the drift"),
ADR-080's predicate replacement (Decision D) applies identically to: `run_query` (598-608),
`run_aggregation_query`'s `Count` arm (811-820), `Sum` arm (859-868), and `Avg` arm (910-919) — each
gated on the same `SchemaCapabilityProbe` read, each branching between the hybrid and fallback shapes
identically. AC-CGI-03 (all three aggregation arms) and AC-CGI-04/05 (regression guards) are addressed by
construction: only the `all_descendants=true` branch of each site changes, and
`composite-index-real-creation`'s own gate/mechanism is untouched (§ Component Boundaries, ADR-080 —
"Zero changes").

## Wave: DESIGN / [REF] External Integrations

None. This feature is entirely internal to `embyr-pg-storage`'s own Postgres access — no third-party API,
webhook, or OAuth provider is introduced or touched. No contract-testing annotation applies.

## Wave: DESIGN / [REF] Quality Attribute Validation

- **Performance**: US-01's own AC-CGI-02/03 (index-assisted plan) satisfied continuously across the
  backfill lifecycle (ADR-080 Decision D); the working-default p99 latency target (US-01 Outcome KPI) is
  the same before/after `EXPLAIN`-plus-wall-clock proof DISCUSS already specified — DESIGN adds no new
  performance requirement, only the mechanism to satisfy the existing one.
- **Reliability/availability**: US-02's zero-write-blocking requirement satisfied by `CREATE INDEX
  CONCURRENTLY` (existing ADR-072 precedent, unmodified) plus `FOR UPDATE SKIP LOCKED` batched backfill
  (new, Decision B) — no operation in this design ever holds a long-duration lock against the live
  `documents` table.
- **Correctness under version skew**: US-03 satisfied by the cached schema-capability probe plus the
  hybrid query predicate, verified never to reference a column that may not exist (`Unavailable` branch is
  byte-for-byte today's query) and never to miss a not-yet-backfilled document (`Available` branch's
  `collection_id IS NULL` fallback arm).
- **Maintainability/enforceability**: the byte-for-byte mirrored-site convention (ADR-040 § 2) is
  preserved, not diluted, by applying the identical predicate-and-probe-read shape at all four sites;
  `is_probe_stale` is extracted as a pure, directly unit-testable function specifically so this feature's
  one new piece of non-trivial decision logic is not buried inside `sqlx`-coupled code.
- **Portability**: the chosen mechanism (plain column + trigger + `regexp_replace`) has no Postgres
  version floor beyond what this codebase's existing schema already assumes — resolves DISCUSS's own NFR
  without requiring a new supported-Postgres-versions document as a blocking prerequisite (still
  recommended as a follow-up hygiene item, not built here).

## Wave: DESIGN / [REF] Handoff to DISTILL

- Primary artifacts: `docs/product/architecture/adr-080-collection-group-query-index.md` (full mechanism
  decision); this DESIGN section (traceability to DISCUSS's ACs and Central Design Questions).
- `docs/product/architecture/brief.md` exists (dated 2026-05-23, the original system-level DESIGN brief
  predating this session's per-feature-ADR convention) — not updated by this DESIGN wave. Nothing in
  ADR-080 changes any of `brief.md`'s own locked System Constraints/Process Topology/Quality Attributes
  (no new binary, no new TCP listener, no new backend mode, Postgres remains the only production backend);
  consistent with every prior feature this session, which recorded its own decisions in a per-feature ADR
  plus evolution doc rather than editing `brief.md`.
- Development paradigm: functional-where-practical Rust (per `/Users/petervyboch/Projects/embyr-rs/
  CLAUDE.md`) — the one new piece of non-trivial logic this feature adds in Rust (`is_probe_stale`) is a
  pure function; the extraction logic itself lives in SQL (the trigger), not Rust, by design (Decision A).
- New migration file for DISTILL/DELIVER to create: `migrations/customer/0002_collection_group_index.sql`
  (transactional: column + trigger function + trigger). The two `CREATE INDEX CONCURRENTLY` statements and
  the backfill loop are explicitly NOT part of this migration file (cannot run inside its transaction) —
  DISTILL's acceptance tests should exercise them as a separate, explicit step, mirroring how ADR-072's own
  acceptance tests exercise its async index build separately from its metadata-row migration.
- Acceptance-designer should note: US-02/US-03's own UAT scenarios already specify real,
  non-mocked backfill-under-concurrent-write and un-migrated/partially-migrated-database scenarios
  (Strategy A) — ADR-080's mechanism is designed to make each of those scenarios directly executable
  against a real `PostgresBackendAdapter` and a real customer-shaped test database, with no new test
  double or fixture class required beyond what `composite-index-real-creation`'s own DISTILL wave already
  established for this codebase.

## Wave: DISTILL / [REF] Reading Confirmation

✓ `docs/architecture/atdd-infrastructure-policy.md` — absent; a project-level policy file has never been
  bootstrapped in this project. Rather than write a placeholder skeleton and immediately diverge from
  it, this DISTILL run inherits the DE-FACTO policy already established by 20+ prior features'
  `tests/<feature>/` conventions (real testcontainers Postgres, real subprocess CLI, real gRPC/admin
  HTTP, `#[path]`-shared `common/mod.rs` reuse across features) — read directly from
  `tests/composite_index_real_creation/`, `tests/customer_db_onboarding/`. Flagged as a follow-up hygiene
  item (bootstrap `docs/architecture/atdd-infrastructure-policy.md` from this established practice), not
  built in this run — does not block this feature.
✓ `[lang-mode] rust` — `Cargo.toml` workspace marker. Polyglot state-delta port already present at
  `tests/common/state_delta.rs` (inherited, `[port-mode] inherit` — no bootstrap needed).
✓ `migrations/customer/` re-read directly: 5 files exist today (`0001_documents.sql` through
  `0005_transaction_reads.sql`). ADR-080's own "0002_collection_group_index.sql" citation has drifted —
  the correct next number is `0006_collection_group_index.sql`. Flagged for DELIVER (§ below); DISTILL
  does not author migration files itself (confirmed by direct git-log inspection of
  `composite-index-real-creation`'s own DISTILL commit `1354234`, which touched zero `.sql` files —
  migration authorship is DELIVER's job in this codebase's established convention, not DISTILL's,
  notwithstanding ADR-080's own "Handoff to DISTILL" wording).
✓ `crates/embyr-pg-storage/src/backend_adapter.rs` re-read directly around all 5 real `INSERT INTO
  documents` sites (347, 398, 519, 1252, 1312) and the 4 mirrored `all_descendants` LIKE sites
  (`run_query` 598-608; `run_aggregation_query` Count/Sum/Avg 811-820/859-868/910-919) to derive exact
  test fixtures (`seed_document`/`update_document`/`commit_transaction` call shapes) with zero guessing.
✓ `crates/embyr-core/src/storage/backend_adapter.rs` and `crates/embyr-core/src/domain/{document,query,
  transaction,field_value}.rs` read directly for every domain type this DISTILL's tests construct
  (`CollectionPath`, `DocumentPath`, `StructuredQuery`, `QueryFilter`, `AggregationQuery`,
  `AggregateValue`, `Write`, `FieldTransform`, `TransactionOptions`) — no field/variant guessed.
✓ `tests/composite_index_real_creation/common/mod.rs` and `tests/customer_db_onboarding/common/mod.rs`
  read directly — this feature's own test placement, fixture technique (`DISABLE TRIGGER`/`ENABLE
  TRIGGER` around a raw insert to simulate pre-existing rows), EXPLAIN helper shape, and CLI-subprocess
  harness (`run_db_prep`/`start_postgres_container`/`create_ddl_role`/`role_connection_url`) all directly
  reuse or mirror these two features' own established conventions — zero new test-infrastructure class
  introduced.
✓ `crates/embyr-db-prep/src/main.rs`/`config.rs` read directly: today's binary calls only
  `PostgresBackendAdapter::migrate()` after connect+probe — confirms the US-02 walking-skeleton CLI test
  is genuine RED (no backfill/index-build call exists yet), and that `EMBYR_DB_PREP_BACKFILL_BATCH_SIZE`/
  `EMBYR_DB_PREP_BACKFILL_THROTTLE_MS` env vars + a `"collection_id backfill complete: {n} documents
  backfilled"` stdout line are NEW interface surface this feature's own acceptance test specifies for
  DELIVER to implement (test-driven interface design, not pre-implementation).

## Wave: DISTILL / [REF] Wave-Decision Reconciliation

Read `docs/feature/collection-group-query-index/discuss/wave-decisions.md` and `.../design/wave-
decisions.md` — both absent as standalone files; this feature's project uses the single running
`feature-delta.md` document (with `## Wave: DISCUSS`/`## Wave: DESIGN` sections already embedded above)
rather than separate per-wave `wave-decisions.md` files. Reconciliation performed directly against the
embedded DISCUSS and DESIGN sections above: zero contradictions found — every DESIGN decision (ADR-080)
traces to and resolves a DISCUSS Central Design Question without contradicting any DISCUSS-locked
requirement (US-01/02/03 outcomes, the "correctness never regresses" hard requirement, the "all 4
mirrored sites updated identically" constraint). **Reconciliation passed — 0 contradictions.**

## Wave: DISTILL / [REF] Scenario List

18 scenarios (`#[tokio::test]` functions) across 5 files, all real-I/O (Strategy A). 7 already PASS
against current pre-fix production code (regression anchors — see red-classification.md); 11 FAIL
(genuine RED). 61% error/edge/regression-guard-adjacent scenario share by scenario intent (trigger
coverage, backfill interruption, schema-skew, regression guards) comfortably exceeds the 40% target,
though this feature's own shape (a performance/correctness-preservation fix, not a new user-facing
capability) makes happy/error classification less binary than usual — see per-file breakdown below.

| # | Scenario | Tags | AC | File |
|---|---|---|---|---|
| 1 | Every write path reaching an INSERT populates `collection_id` via the trigger (5 sites, 1 container) | `@real-io @trigger_coverage` | trigger coverage | `cgi_trigger.rs` |
| 2 | Collection-group query returns the same documents a customer already sees today (+ Example 2 top-level/nested) | `@walking_skeleton @driving_port @real-io @US-01` | AC-CGI-01 | `cgi_index_usage.rs` |
| 3 | Collection-group query for a name with no matches returns empty | `@real-io @US-01 @error` | AC-CGI-01 | `cgi_index_usage.rs` |
| 4 | Collection-group query with a compound filter returns only matching documents | `@real-io @US-01` | AC-CGI-01 | `cgi_index_usage.rs` |
| 5 | EXPLAIN of a schema-current collection-group query is index-assisted | `@real-io @US-01` | AC-CGI-02 | `cgi_index_usage.rs` |
| 6 | EXPLAIN stays index-assisted for a larger result set | `@real-io @US-01` | AC-CGI-02 | `cgi_index_usage.rs` |
| 7 | Aggregation Count/Sum/Avg for a collection group return correct values | `@real-io @US-01` | AC-CGI-03 | `cgi_index_usage.rs` |
| 8 | Ordinary non-collection-group queries are unaffected | `@real-io @US-01 @error` (regression guard) | AC-CGI-04 | `cgi_index_usage.rs` |
| 9 | Backfilling pre-existing documents does not block concurrent writes | `@real-io @US-02` | AC-CGI-06 | `cgi_backfill.rs` |
| 10 | A document written mid-backfill is already correctly indexed | `@real-io @US-02` | AC-CGI-07 | `cgi_backfill.rs` |
| 11 | Interrupted backfill resumes without reprocessing or leaving gaps (kill-and-resume, SKIP LOCKED proof) | `@real-io @US-02 @error` | AC-CGI-08 | `cgi_backfill.rs` |
| 12 | A fully-backfilled collection is index-assisted for pre-existing documents | `@real-io @US-02` | AC-CGI-09 | `cgi_backfill.rs` |
| 13 | Collection-group query against an un-migrated database returns correct results | `@real-io @US-03 @error` | AC-CGI-10 | `cgi_schema_skew.rs` |
| 14 | A partially-backfilled collection returns every matching document | `@real-io @US-03 @error` | AC-CGI-12 | `cgi_schema_skew.rs` |
| 15 | `schema_capability` picks up a migration landing mid-session within the TTL | `@real-io @US-03` | AC-CGI-11 | `cgi_schema_skew.rs` |
| 16 | `schema_capability` Available is permanent, never re-flips to Unavailable | `@real-io @US-03 @error` | AC-CGI-11 | `cgi_schema_skew.rs` |
| 17 | Cached `schema_capability` reads are negligible versus uncached catalog probes | `@real-io @US-03` | AC-CGI-13 | `cgi_schema_skew.rs` |
| 18 | `embyr-db-prep` CLI backfills pre-existing documents and builds the collection-group indexes | `@walking_skeleton @driving_port @real-io @US-02` | US-02 WS | `tests/collection_group_query_index/acceptance/cgi_ws_db_prep_backfills_and_indexes.rs` |

## Wave: DISTILL / [REF] Walking Skeleton Strategy

**Strategy A** (real, minimal, end-to-end), per DISCUSS's own lock — every scenario runs against a real
testcontainers Postgres 15-alpine and the real `PostgresBackendAdapter`/`embyr-db-prep` binary, never a
mock or in-memory double. Two walking-skeleton scenarios (not one), because this feature genuinely has
two independent driving ports needing their own wiring proof: (1) scenario #2, gRPC-shaped
`RunQuery`-equivalent access through `PostgresBackendAdapter::run_query` — the adapter's own public port
method IS the driving port at this test's chosen layer (crate-local adapter-integration test, not a
full gRPC round-trip — see § Test Placement for why); (2) scenario #18, the `embyr-db-prep` CLI
subprocess — proves the NEW backfill/index-build behavior is actually wired into the tool's own
`main()`, not just reachable if someone called the adapter method directly (Driving Adapter Verification
mandate).

## Wave: DISTILL / [REF] Adapter Coverage Table

| Adapter | `@real-io` scenario | Covered by |
|---|---|---|
| `PostgresBackendAdapter` (documents read/write) | YES | all 17 `crates/embyr-pg-storage/tests/*.rs` scenarios — real testcontainers Postgres |
| `documents_collection_id_biu` trigger (new) | YES | scenario #1 (`cgi_trigger.rs`), scenarios #9-12 fixtures (`cgi_backfill.rs`), #18 fixture |
| `documents_collection_group_idx`/`_pending_idx` (new) | YES | scenario #12 (`pg_indexes` catalog read), scenario #18 (CLI-driven) |
| `embyr-db-prep` CLI (backfill/index-build wiring) | YES | scenario #18 — real subprocess, `CARGO_BIN_EXE_embyr-db-prep` |
| `SchemaCapabilityProbe`/`information_schema` catalog | YES | scenarios #15-17 (`cgi_schema_skew.rs`) |

Zero "NO — MISSING" rows — every adapter this feature touches has at least one real-I/O scenario.

## Wave: DISTILL / [REF] Scaffolds (Mandate 7, RED-ready)

All additive — zero existing method body touched. All panic with an explicit `RED scaffold` message
(never `NotImplementedError`/`ImportError` equivalents) — confirmed RED, not BROKEN, by the actual test
run (§ Pre-DELIVER Gate below).

| Symbol | File | Kind |
|---|---|---|
| `push_all_descendants_predicate(qb, collection_id, schema_available)` | `crates/embyr-pg-storage/src/encoding/query.rs` | pure fn — single source of truth for the hybrid predicate (ADR-080 Decision D), mirrors `append_filter`/`order_by_expr`'s own "one function, N call sites" convention (ADR-040 §2); DELIVER wires all 4 mirrored call sites to call it |
| `PostgresBackendAdapter::backfill_collection_id(batch_size, throttle) -> Result<BackfillSummary, CoreError>` | `crates/embyr-pg-storage/src/backend_adapter.rs` | adapter method (ADR-080 Decision B) |
| `PostgresBackendAdapter::ensure_collection_group_indexes() -> Result<(), CoreError>` | `crates/embyr-pg-storage/src/backend_adapter.rs` | adapter method (ADR-080 Decision D) |
| `PostgresBackendAdapter::schema_capability(unavailable_ttl: Duration) -> SchemaCapability` | `crates/embyr-pg-storage/src/backend_adapter.rs` | adapter method (ADR-080 Decision C) — `unavailable_ttl` is a DISTILL-added testability parameter (production call sites pass the 30s working default; tests pass a short TTL so AC-CGI-11 doesn't require a real 30s sleep) |
| `BackfillSummary { rows_backfilled: u64, batches_run: u64 }` | `crates/embyr-pg-storage/src/backend_adapter.rs` | struct |
| `SchemaCapability { Available, Unavailable }` | `crates/embyr-pg-storage/src/backend_adapter.rs` | enum |

Not scaffolded (deliberately): `SchemaCapability`'s own internal caching/TTL logic and `is_probe_stale`
as a directly-unit-tested pure function — per ADR-025/`nw-tdd-methodology`, DISTILL authors acceptance
tests only; new pure-function extraction + its own PBT unit test is DELIVER's inner-loop job (confirmed
against this session's own precedent: `composite-index-real-creation`'s analogous "build-succeeded"
pure-function extraction happened in a DELIVER-stage mutation-fixing commit, not at DISTILL).

## Wave: DISTILL / [REF] Test Placement

`crates/embyr-pg-storage/tests/*.rs` (Rust-native crate-local integration tests, no `[[test]]`
registration needed) for all adapter-level scenarios (#1-17) — deliberate placement choice, deviating
from the workspace-root `tests/<feature>/` convention `composite_index_real_creation`/
`customer_db_onboarding` use: this feature's US-01/02/03 correctness is fully exercisable through
`embyr-pg-storage`'s own public `PostgresBackendAdapter`/`BackendAdapter` surface, so pulling in
`embyr-server`/tonic/gRPC for these 17 scenarios would cost real compile time and RAM on the 8GB target
machine for zero additional coverage (Mandate 1 still holds — `PostgresBackendAdapter::run_query` IS the
driving port being tested at this integration layer, per `nw-tdd-methodology`'s own "Infrastructure
Layer (Adapters): Integration tests ONLY" guidance). `tests/collection_group_query_index/acceptance/`
(workspace-root, registered in `crates/embyr-db-prep/Cargo.toml`) for scenario #18 only, the genuine CLI
driving-port proof — mirrors `customer_db_onboarding`'s own cdo0N placement exactly, including reusing
its `common/mod.rs` via `#[path]` rather than duplicating the subprocess harness.

## Wave: DISTILL / [REF] Driving Adapter Coverage

`embyr-db-prep` CLI is the one DESIGN-specified entry point this feature adds NEW behavior behind
(ADR-080 § Component Boundaries: "after its existing `migrate()` call, adds calls to
`ensure_collection_group_indexes()` then `backfill_collection_id()`"). Scenario #18 invokes it via real
subprocess (`CARGO_BIN_EXE_embyr-db-prep`), verifies exit code (0), stdout content (the new backfill-
count confirmation line), and real Postgres catalog state (zero NULL `collection_id`, the new index
present) — not just that the underlying adapter methods work in isolation. Zero uncovered entry points:
the gRPC `RunQuery`/`RunAggregationQuery` driving port is unchanged wire-shape (DISCUSS § Driving Ports)
and is exercised at the adapter-integration layer per § Test Placement above.

## Wave: DISTILL / [REF] Pre-requisites

- DESIGN driving ports: gRPC `RunQuery`/`RunAggregationQuery` (unchanged), `embyr-db-prep` CLI (new
  backfill/index-build calls).
- DEVOPS environment matrix: absent (`docs/feature/collection-group-query-index/devops/` does not
  exist) — default matrix applied per nw-distill's graceful-degradation rule (clean |
  with-pre-commit | with-stale-config); no environment-specific precondition applies to this feature's
  own scenarios (all run against a fresh testcontainers Postgres regardless).
- DELIVER must author `migrations/customer/0006_collection_group_index.sql` (correcting ADR-080's own
  drifted "0002" citation — `migrations/customer/` already has 0001-0005) containing exactly: `ALTER
  TABLE documents ADD COLUMN collection_id VARCHAR(1500)`; `CREATE FUNCTION
  documents_set_collection_id()`; `CREATE TRIGGER documents_collection_id_biu BEFORE INSERT ON documents
  FOR EACH ROW EXECUTE FUNCTION documents_set_collection_id()`.
- DELIVER must wire `crates/embyr-db-prep/src/main.rs` to read `EMBYR_DB_PREP_BACKFILL_BATCH_SIZE`
  (default per ADR-080's own 1000 working default) / `EMBYR_DB_PREP_BACKFILL_THROTTLE_MS` (default 50)
  and print `"collection_id backfill complete: {n} documents backfilled"` — interface specified by
  scenario #18, not pre-implemented.

## Wave: DISTILL / [REF] Pre-DELIVER Fail-for-the-Right-Reason Gate

Full classification: `docs/feature/collection-group-query-index/distill/red-classification.md`. Ran
`cargo test -p embyr-pg-storage --test cgi_trigger --test cgi_index_usage --test cgi_backfill --test
cgi_schema_skew --no-fail-fast -- --test-threads=1` and `cargo test -p embyr-db-prep --test
cgi_ws_db_prep_backfills_and_indexes -- --test-threads=1` against current pre-fix code. Result: 7/18
scenarios PASS today (regression anchors — behaviors this feature must not break, all traced to
"today's predicate never references `collection_id`" or "today's aggregation/result-set logic is
already correct"); 11/18 FAIL, every one classified `MISSING_FUNCTIONALITY` (real Postgres `42703`
"column does not exist" / `42704` "trigger does not exist" errors, or an explicit scaffold `panic!`
carrying a `RED scaffold` message) — zero `IMPORT_ERROR`/`FIXTURE_BROKEN`/`SETUP_FAILURE`, zero
`WRONG_ASSERTION`. **Gate: PASSED.**

## Wave: DISTILL / [REF] Mandate Compliance Evidence

- **CM-A** (Mandate 1, driving ports): every scenario's `use` block imports only
  `embyr_core::storage::backend_adapter::BackendAdapter` (the port trait) + domain types, or
  (`cgi_ws_db_prep...rs`) the real subprocess binary via `CARGO_BIN_EXE_embyr-db-prep`. Zero direct
  instantiation of any internal validator/parser; `PostgresBackendAdapter` itself is the adapter under
  test at the integration layer (not a "driving port" in the Mandate-1 sense but the correct
  adapter-to-port integration-test target per `nw-tdd-methodology`).
- **CM-B** (Mandate 2, business language): no scenario title or assertion message contains
  "database"/"API"/"HTTP"/"schema"/"endpoint" as the SUBJECT of a Then — every assertion message states
  the AC's own business/correctness invariant (e.g. "every matching document must be returned regardless
  of its own backfill state"). Technical detail (SQL, `pg_locks`, `information_schema`) lives inside
  helper bodies (`common/mod.rs`), never in a test's own narrative comment framing.
- **CM-C** (Mandate 3, journey completeness): both walking-skeleton scenarios (#2, #18) express a
  complete before/after outcome a stakeholder (Alex, Sam Chen) would recognize — "the same documents a
  customer already sees today" / "the DBA sees confirmation that backfill and index preparation both
  ran."
- **CM-D** (Mandate 4, pure-function extraction): `push_all_descendants_predicate` is extracted as a
  scaffolded pure fn specifically so tests never hand-duplicate SQL (zero-drift EXPLAIN proof); the
  `p99_of` timing helper in `cgi_schema_skew.rs` is a pure, parametrized measurement helper, not
  per-scenario copy-pasted timing code.

Peer review dispatched separately (Sentinel, `nw-acceptance-designer-reviewer`) — result recorded below
once returned.

## Wave: DISTILL / [REF] Peer Review

`nw-acceptance-designer-reviewer` (Sentinel) ran one review iteration against this DISTILL artifact set
(feature-delta.md DISTILL sections + ADR-080 + red-classification.md + all 5 test files + the 2
production-crate scaffold sites). Result: **approved**, 0 blockers, 0 high, 0 findings. All 9 critique
dimensions scored 9-10/10; CM-A/B/C mandate checks pass. No revisions required before DELIVER handoff.
