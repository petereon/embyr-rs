# Evolution: composite-index-real-creation

**Date:** 2026-09-09
**Feature:** `POST /admin/v1/projects/:project_id/indexes` now provisions a real, non-blocking
Postgres index in the customer database instead of only writing a metadata row that lies about
readiness.
**Job:** JOB-01 (`sdk-compat`) — reused, P1 Alex.
**ADRs:** ADR-072 (new) — composite index real provisioning (4 sub-decisions A-D).

## This closes finding #5 from `docs/product/production-readiness-audit-2026-09-08.md`

## Business Context

`create_composite_index` inserted a `composite_indexes` row with `status='ready'` synchronously,
on INSERT — no `CREATE INDEX` ever ran. The Firestore-parity gate (blocking compound
filter+orderBy queries until a matching index is "ready") already worked correctly and was NOT
the problem; the problem is that once "ready," every such query ran unindexed forever (a full
scan against the `fields` JSONB column), which does not scale past a small collection.

## Key Decisions (ADR-072)

| Decision | Verdict |
|---|---|
| A — DDL safety | Reuse `validate_field_path`'s existing charset gate on every field path before any DDL text is built. No `quote_ident` needed — field paths are only ever interpolated as string-literal args to the `->` JSONB operator, never as raw SQL identifiers (same threat shape as the already-proven WHERE/ORDER-BY builders) |
| B — `CREATE INDEX CONCURRENTLY` | Required (US-02's non-blocking-build AC). A separate `pg_index.indisvalid` check after the statement is the authoritative ready/failed signal — catches a mid-build connection drop the statement's own Ok/Err alone would miss |
| C — Async build | `tokio::spawn`'d one-shot task, reusing `transaction_sweeper.rs`'s `resolve_dsn_without_api_key` (extracted to shared `customer_db_connect.rs`) — no new job-queue infrastructure. `status` widens to `building`/`ready`/`failed`. Retry via idempotent re-POST, not crash self-healing — a stuck `building` row recovers via delete-then-recreate |
| D — Index expression shape | Positional equality-vs-sort role inference matching `encoding/query.rs`'s own emitted WHERE/ORDER-BY expressions exactly — explicitly proven only for N equality-filter fields + one trailing sort field; 2 rarer shapes (2+ orderBy no filter; IN/range landing last) named as a deferred, non-regression follow-up |

## Steps Completed

Full nWave subagent pipeline (DISCUSS=`nw-product-owner`, DESIGN=`nw-solution-architect` with peer
review, DISTILL=`nw-acceptance-designer`, DELIVER=`nw-software-crafter`) plus significant
orchestrator-led verification and one direct fix:

1. **DISCUSS**: reused JOB-01, confirmed via direct code reading (not assumed) that the gate is a
   pure Firestore-parity simulation and query execution already runs unconditionally — this is a
   performance finding, not a correctness bug. Found `transaction_sweeper.rs`'s own
   `resolve_dsn_without_api_key`/`tokio::spawn` pattern as the reuse candidate for DESIGN. 3 stories
   (US-01 walking skeleton, US-02 non-blocking build, US-03 DeleteIndex drops the real index too),
   12 ACs, DoR passed.
2. **DESIGN**: peer-reviewed (0 critical/high, 2 medium/2 low, all resolved in one pass — including
   a condition to encode Decision D's own named limitations directly into the AC handoff, not just
   ADR prose). Locked all 4 sub-decisions with concrete evidence (read `sqlx`/`encoding/query.rs`/
   `transaction_sweeper.rs` source directly).
3. **DISTILL**: wrote 12 acceptance scenarios across 3 files. The DISTILL subagent's own final
   report deferred without confirming RED state or documenting itself — the orchestrator
   independently ran and confirmed all 8 new-functionality scenarios correctly RED
   (MISSING_FUNCTIONALITY) against the unfixed code, then wrote the DISTILL wave section itself.
   Confirmed migration 0036 did not yet exist (DELIVER's job).
4. **A stray duplicate DISTILL subagent** (from an earlier, already-superseded dispatch under the
   same session) surfaced mid-flight, discovered the concurrent DELIVER commit had already landed,
   correctly self-detected the conflict, reverted its own debug edits, and stopped without touching
   production code — no orchestrator action needed, a genuinely clean self-resolution.
5. **DELIVER**: implemented per ADR-072 exactly — new `composite_index_ddl.rs` (pure DDL builders,
   6 unit tests), `customer_db_connect.rs` (extracted DSN resolution), `composite_index_builder.rs`
   (the async build task), handler + migration 0036 changes. Fixed several genuine pre-existing
   fixture gaps exposed by making the build real/async (documented honestly in its own commit, not
   glossed over): a missing `backend_pg_dsn_enc` column in a shared security-rules test fixture, an
   under-seeded `EXPLAIN`-plan test (3 rows can never make Postgres prefer an index over a Seq
   Scan), and 3 sibling tests that assumed synchronous index readiness needing to poll instead.
   Reported 12/12 scenarios green across repeated runs.
6. **Orchestrator's full-workspace regression**: initially hit 3 unrelated `custom_claims` test
   failures (`ConnectionReset`/`PoolTimedOut`) — traced to a genuine, independently-observed system
   memory-pressure event (the harness had just killed an unrelated wait-loop shell for low memory)
   rather than a real regression; confirmed via isolated rerun (7/7 clean) after Docker cleanup.
7. **QUALITY_GATE**: scoped mutation run (4 highest-value new-logic files). First attempt aborted
   on a flaky baseline (same memory-pressure class as step 6) — confirmed via isolated rerun, then
   relaunched cleanly. Result: 22 caught, 8 unviable, 7 missed. **Investigated all 7, not
   hand-waved**: 1 was a genuine, security-critical gap in the `create_result.is_ok() && valid`
   ready/failed decision (ADR-072 Decision B's own authoritative signal) — no test forced the
   asymmetric state that would distinguish `&&` from `||`. Fixed by extracting a pure
   `build_succeeded()` function with a direct 4-case unit test, rather than attempting to reliably
   simulate a real mid-build Postgres connection drop. The other 6 (aws_secret/gcp_secret DSN
   resolution) split into a scoping artifact (gcp_secret — real coverage exists in the transaction
   sweeper's own test suite, outside this run's scope) and a genuine but pre-existing,
   already-disclosed, out-of-scope gap (aws_secret — extracted logic never covered anywhere in the
   codebase, matching ADR-072's own explicit direct_pg-only end-to-end limitation).

## Lessons Learned

1. **A mutation-testing miss that survives because two conditions happen to co-vary in every
   existing test (both true, or both false, together) is not automatically a scoping artifact —
   verify whether the underlying logic genuinely needs them independently testable.** The
   `&&`-vs-`||` gap here specifically matched ADR-072's own stated reason for the check (a
   mid-build connection drop producing an ASYMMETRIC create-ok/index-invalid state) — the fact
   that no test forced that asymmetry was the actual, real gap, not an artifact of test scope.
2. **When an integration test can't reliably force a narrow, timing-dependent real-system race,
   extracting the decision logic into a pure function with a direct unit test can give complete,
   deterministic coverage the integration test never could** — a smaller, more robust fix than
   trying to simulate the race itself.
3. **A subagent dispatch can occasionally run stale/duplicated against work another dispatch under
   the same session already completed** (observed here as a leftover DISTILL agent). The
   subagent's own self-detection and clean backoff (reverting its own edits, not touching
   production code, flagging the conflict honestly) worked correctly — worth noting as a positive
   confirmation that this failure mode is recoverable without orchestrator intervention, not a
   new operational hazard requiring a new discipline.
4. **A full-workspace regression or mutation-testing baseline can fail due to real, independently-
   observable system memory pressure** (confirmed via the harness's own low-memory kill of an
   unrelated wait-loop shell moments before) — always check actual system state (memory, load,
   Docker container count) before treating a sudden `ConnectionReset`/`PoolTimedOut` cluster as a
   code regression, and confirm via isolated rerun after cleanup.

## Key Files

- `crates/embyr-server/src/adapters/composite_index_ddl.rs` (new) — pure DDL builders.
- `crates/embyr-server/src/adapters/customer_db_connect.rs` (new) — extracted DSN resolution.
- `crates/embyr-server/src/adapters/composite_index_builder.rs` (new) — async build task,
  `build_succeeded()` pure decision function (QUALITY_GATE fix).
- `crates/embyr-server/src/admin/handlers/composite_indexes.rs` — validates pre-INSERT, spawns
  build, idempotent retry, delete drops the real index.
- `crates/embyr-server/src/sweepers/transaction_sweeper.rs` — now imports the extracted module.
- `migrations/0036_composite_indexes_status_lifecycle.sql` (new).
- `tests/composite_index_real_creation/acceptance/{cxr01,cxr02,cxr03}*.rs` (new, 12 scenarios).
- `docs/feature/composite-index-real-creation/deliver/mutation/mutation-report.md` — full account.

## Follow-Up Work

- `resolve_aws_secret_dsn` has zero test coverage anywhere in the codebase (pre-existing, not
  introduced by this feature) — a real candidate for a future aws_secret-mode-specific feature.
- ADR-072 Decision D's own two named uncovered query shapes (2+ orderBy no filter; IN/range
  landing last in the composite key) remain real Postgres indexes that exist and pass
  CreateIndex/DeleteIndex's own ACs, but may not be planner-selected for those two shapes.
- Findings #6-#8 from the same audit remain: missing 168h soft-delete purge sweeper;
  hardcoded-None aws/gcp secret fetchers (finding #7, likely related to the aws_secret gap above);
  no build/release path for embyr-agent.
