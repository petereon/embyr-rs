# Evolution: collection-group-query-index

**Date:** 2026-09-14
**Feature:** Collection-group queries (`all_descendants=true`) now use a real Postgres
index instead of scanning every document in the project. Adds a `collection_id` column
(populated by a BEFORE INSERT trigger), a resumable batched backfill for existing data,
and a cached schema-version probe so query code stays correct on not-yet-migrated
customer databases while getting the fast path once they're current.
**ADR:** `docs/product/architecture/adr-080-collection-group-query-index.md`

## This closes finding #17 from `docs/product/production-readiness-audit-2026-09-08.md`

Compounds finding #5 (`composite-index-real-creation`) — that work gave real per-field
indexes to ordinary queries but delivered zero benefit here, since a leading-`%` LIKE
defeats any btree regardless of how many composite indexes exist.

## Business Context

`RunQuery`/`RunAggregationQuery` with `all_descendants=true` built `collection_path = X
OR collection_path LIKE '%/X'` — semantically correct (confirmed against
`docs/SPEC.md:913-921`, this was a pure performance gap, not a correctness bug) but
unindexable: a leading `%` defeats btree prefix matching, forcing a full sequential scan
of every document in the project to answer a query meant to touch one collection group.
Existed at 4 mirrored call sites (`run_query` + all 3 `run_aggregation_query` arms).

The complicating factor, absent from every recent finding: this is a BYOC deployment —
`documents` lives in each CUSTOMER's own Postgres, and there's no fleet-wide mechanism to
re-apply schema migrations to an already-provisioned customer database. embyr's
server-code releases are fleet-wide; schema rollout is customer-controlled. That created
a genuine, potentially-indefinite window where new query code has to serve OLD-SCHEMA
customer databases correctly, not just during a migration.

## Key Decisions

| Decision | Verdict |
|---|---|
| Indexing mechanism | Plain nullable `collection_id VARCHAR(1500)` column + `BEFORE INSERT` trigger — NOT a `GENERATED ALWAYS AS (...) STORED` column, which was confirmed to require a full-table rewrite under an `ACCESS EXCLUSIVE` lock on every Postgres version this codebase supports. A plain nullable column with no default is metadata-only/instant. |
| Population mechanism | A trigger, not Rust write-path code — 5 independent `INSERT INTO documents` call sites exist (not the 4 the finding named), and populating in Rust would recreate this finding's own root cause (copy-paste-duplicated logic) at a new site. |
| Backfill | Resumable, `FOR UPDATE SKIP LOCKED` batched `UPDATE ... WHERE collection_id IS NULL LIMIT N`, each batch its own explicit transaction, runs synchronously inside `embyr-db-prep` (a one-shot CLI a DBA already waits on) — no job queue. |
| Query correctness during migration | A hybrid predicate (`collection_id = $N OR (collection_id IS NULL AND <old LIKE>)`) keeps every query correct and index-assisted for already-backfilled rows AND correct-but-unindexed for not-yet-backfilled rows, for the ENTIRE backfill lifecycle — not just before/after. |
| Schema-version detection | A cached `SchemaCapabilityProbe` — permanent cache once `Available` (migrations are additive-only, can't regress), TTL'd cache once `Unavailable` (auto-pickup without a server restart when a customer migrates later). |
| ADR | New ADR-080 — the trigger is a genuinely new mechanism class for this codebase, and the schema-migration/BYOC-skew handling needed real justification against specific ACs. |

## Steps Completed

Full nWave subagent pipeline (DISCUSS=`nw-product-owner`, DESIGN=`nw-solution-architect` with peer review, DISTILL=`nw-acceptance-designer` with peer review, DELIVER=`nw-software-crafter`):

1. **DISCUSS**: confirmed pure-performance framing against SPEC, found the 4-call-site (not 2) scope, surfaced the BYOC schema-skew risk as the standout requirement. 3 stories (index-usage, backfill-safety, schema-skew-correctness), all Release 1 — none safely deferrable given the risk. DoR 9/9 after resolving 5 High clarity gaps inline during peer review.
2. **DESIGN**: rejected the generated-column approach with a concrete rewrite-lock justification, found a 5th INSERT call site, designed the trigger/backfill/schema-probe mechanisms, wrote ADR-080. Peer review: approved, 0 critical/high, 1 iteration.
3. **DISTILL**: 18 scenarios (20 raw test functions) across 5 new test files, all real-I/O (testcontainers Postgres, no mocks). Tested resumability via a genuine kill-and-resume against a live backfill task, verifying byte-identical row state and non-redundant row counts on resume. Tested schema-skew via a fixture that applies every migration except the last (mirroring `customer_db_onboarding`'s own technique), and TTL auto-pickup via a parametrized short TTL rather than sleeping 30+ real seconds. Caught DESIGN's own migration-numbering error (ADR-080 said "0002," the real next number was 0006). Interrupted once by a session rate limit near the very start, cleanly resumed from scratch. Peer review: approved, 0 blockers, 9-10/10 across all 9 dimensions.
4. **DELIVER**: implemented exactly as designed (commit `b32ebfb`). Found and fixed a real bug via TDD: an autocommit statement let an aborted caller's in-flight backfill batch commit silently uncounted, breaking the exact-resume-count assertion — fixed with explicit per-batch `tx.begin()`/`commit()`. Disclosed one scoped tradeoff (`SET enable_seqscan = off` on a provisioning-scoped connection, with a `ponytail:` comment naming the `SET LOCAL` upgrade path) and one honest scope gap (`provision.rs` wiring left untouched, no test covers it). 18/18 scenarios green, backfill suite re-run 3x to rule out lucky timing.
5. **Independent verification**: fresh subagent gave both flagged risk areas real independent judgment rather than a rubber stamp — confirmed the `enable_seqscan` tradeoff is safe by current caller topology (not a structural guarantee) by tracing every call site workspace-wide, and confirmed the `provision.rs` gap is real but medium severity: `migrate()` itself now calls `ensure_collection_group_indexes()`, so the column/indexes always land — the actual gap is narrower, `backfill_collection_id()` is never called from `provision.rs`, meaning a `Stale`-reprovisioned database with pre-existing rows stays permanently un-backfilled for those rows until someone runs `embyr-db-prep` manually. Correctness is never at risk (NULL rows fall back correctly); only the performance benefit fails to materialize for that slice of data.
6. **QUALITY_GATE**: found 3 genuine gaps on the FIRST pass (16/24 caught, 3 missed) — all in the highest-risk pieces: the backfill's own `batches_run` counter (never asserted, could silently stay stuck at 0), an inlined invalid-index-rebuild check (a leftover `indisvalid=false` index with a no-op `IF NOT EXISTS` re-run could be silently accepted as built), and the TTL cache's `Unavailable`-branch freshness guard (answers stayed correct but the cost benefit was unverified). All 3 fixed — one via a real root-cause extraction (`index_build_succeeded`, mirroring `composite_index_builder.rs`'s own `build_succeeded` pattern) plus a direct truth-table unit test, the other two via tightened/added assertions. Final run: 21/21 viable mutants caught, 5 correctly unviable. Commit `9b35229`.

## Lessons Learned

1. **This is the riskiest feature closed this session so far** (real schema migration, a new Postgres trigger, a resumable batched backfill, BYOC version-skew handling) — and the nWave pipeline scaled up cleanly: more DISCUSS/DESIGN depth, more DISTILL scenarios, a deeper independent-verification pass that gave genuine second opinions rather than a checklist pass, and QUALITY_GATE finding real gaps precisely in the highest-risk pieces (not noise). The pipeline doesn't need a different process for higher-risk work, just more care applied at each existing step.
2. **QUALITY_GATE finding gaps concentrated in the exact pieces flagged as highest-risk during independent verification is a good signal, not a coincidence** — the schema_capability TTL branch and the backfill's own counter were both areas the verification pass had already scrutinized closely (even though it didn't find bugs there, it correctly identified them as the parts needing the most scrutiny). Worth treating independent-verification's own risk-ranking as a hint for where mutation testing is most likely to find something real.
3. **A test helper/verification pass that traces every call site workspace-wide** (rather than trusting a docstring's claim) is what turned "is this safe?" into a defensible, specific answer ("safe by caller topology, not structural guarantee") instead of a guess — this generalizes the session's now-established "verify claims against source, not prose" discipline to safety/soundness judgments, not just correctness claims.

## Key Files

- `migrations/customer/0006_collection_group_index.sql` (new) — column, trigger, 2 partial indexes.
- `crates/embyr-pg-storage/src/encoding/query.rs` — `push_all_descendants_predicate`.
- `crates/embyr-pg-storage/src/backend_adapter.rs` — `backfill_collection_id`, `ensure_collection_group_indexes`, `schema_capability`, `index_build_succeeded`.
- `crates/embyr-db-prep/src/main.rs`, `config.rs` — backfill wiring, env-var config.
- `crates/embyr-pg-storage/tests/cgi_{trigger,index_usage,backfill,schema_skew}.rs` (new).
- `docs/product/architecture/adr-080-collection-group-query-index.md`.
- `docs/feature/collection-group-query-index/deliver/mutation/mutation-report.md`.

## Follow-Up Work

- **`provision.rs` backfill gap (real, medium severity, not blocking)**: `backfill_collection_id()` is never called from `crates/embyr-server/src/admin/handlers/provision.rs`. New-project provisioning is unaffected (zero pre-existing rows). But the `SchemaReadiness::Stale` re-provisioning path can reach a database with real pre-existing document data that stays permanently un-backfilled for those specific rows until a DBA runs `embyr-db-prep` manually — correctness is never at risk (the hybrid predicate falls back correctly), only the performance benefit fails to materialize for that data. Recommend adding a `backfill_collection_id()` call alongside the existing `migrate()` calls in `provision.rs`. **RESOLVED** 2026-09-19 (tracked as finding #48) — `provision.rs`'s `Stale`/`NotPrepped` branch now spawns `backfill_collection_id()` in the background (fire-and-forget, matching `composite_index_builder`'s own precedent for slow admin-triggered Postgres work) right after `migrate()` succeeds.
- Finding #18+ (High) remain — next in audit order.
