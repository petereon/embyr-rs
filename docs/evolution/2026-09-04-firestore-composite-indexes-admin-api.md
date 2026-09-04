# Evolution: firestore-composite-indexes-admin-api

**Date:** 2026-09-04
**Feature:** Composite-index admin CRUD — `POST`/`GET /admin/v1/projects/:project_id/indexes`,
`DELETE /admin/v1/projects/:project_id/indexes/:index_id`. Closes the last-mile gap in the
composite-index gate `RunQuery` has correctly enforced since the original walking skeleton: a
`FAILED_PRECONDITION` rejection with no way to ever satisfy it.
**Job:** JOB-01 (`sdk-compat`) — realization closing a documented-but-unbuilt gap, mirroring the
`aggregation-queries`/`batch-get-documents` "make it real" pattern.
**ADRs:** ADR-068 (`docs/product/architecture/adr-068-composite-index-admin-crud.md`)

## Business Context

`crates/embyr-server/src/grpc/handler.rs`'s own `RunQuery` has correctly rejected any filter+
`orderBy`-on-a-different-field query with `FAILED_PRECONDITION` since the original walking skeleton
(`docs/evolution/2026-05-27-embyr-rs.md`, slice 04-02) — real-Firestore-accurate behavior, gated by
`IndexManager::is_index_ready` against the `composite_indexes` table (migration 0003). But no
production code path has ever written a row into that table: it starts empty for every project and
stays empty forever, so the rejection is permanent. Unlike real Firestore (which gives the
developer a console link to create the missing index), Alex had no way out. This feature closes
that gap with a narrow, 3-verb admin CRUD surface.

## Key Decisions

| Decision | Verdict |
|---|---|
| Reuse JOB-01 (mirrors `aggregation-queries`/`batch-get-documents`'s own "make it real" pattern) — a documented-but-unbuilt gap in Alex's own job, not a new job | ADR-068 |
| Synchronous `status: 'ready'` on create — no simulation of real Firestore's own async `CREATING` window, since this backend has no real secondary-index build latency to model | ADR-068 |
| Metadata-only — `CreateIndex` never issues a real Postgres `CREATE INDEX` DDL; query correctness never depended on one, only performance would (unevidenced, deferred) | ADR-068 |
| No dependency-safety check on `DeleteIndex` — matches this codebase's own existing admin-mutation precedent (write-rule redefinition can equally "break" a previously-passing write) | ADR-068 |
| `CreateIndex` on an exact-duplicate `(collection_path, fields)` spec is idempotent (`ON CONFLICT ... DO UPDATE`, a deliberate no-op self-update) — 200 with the existing row, never a 409 | ADR-068 |
| Zero change to the query path (`handler.rs`) — this feature is purely upstream, populating a table the existing gate already reads unmodified | ADR-068 |

## Steps Completed

1. **Slice 01** (US-01, Walking Skeleton) — `create_composite_index` + the `fields` JSONB shape
   (typed `IndexFieldSpec`/`IndexFieldOrder`, reused verbatim from the one place this table was
   ever written to before this feature: `tests/acceptance/us_04_query_collection.rs`'s own
   fixture). Real, previously-permanently-stuck `RunQuery` proven to succeed immediately after one
   real `CreateIndex` call.
2. **Slice 02** (US-02) — `list_composite_indexes`, any role, read-only (already implemented
   alongside create in Slice 01's own handler file; this slice is the acceptance-test proof).
3. **Slice 03** (US-03, LAST slice) — `delete_composite_index`, no dependency-safety check (proven
   directly: a query dependent on the sole ready index fails `FAILED_PRECONDITION` again
   immediately after that index is deleted).

**QUALITY_GATE** — `cargo-mutants --in-diff` scoped to this feature's own entire production-code
footprint (`composite_indexes.rs`, zero `embyr-core` change), test harness deliberately limited to
the fast, Docker-free unit tests (`-- --lib composite_indexes::`) per this session's own
Docker-contention discipline. Found 8 misses, ALL inside the 3 async handlers the unit-test harness
structurally cannot reach — individually cross-checked (not assumed) against the real acceptance
suite: 6 were already covered, but 2 were genuine, previously-untested gaps — the `<`/`<=` boundary
on both role checks (every prior test used `Owner`/`Viewer`, never the exact `Admin` role) and the
delete-of-a-never-created-id 404 path. Closed with 3 new acceptance tests. Full details:
`docs/feature/firestore-composite-indexes-admin-api/deliver/mutation/mutation-report.md`.

**Full regression**: `cargo test -p embyr-server`, 410 tests passed, 0 failures attributable to
this feature (one pre-existing, unrelated `distributed_rate_limiting` test — a completely separate,
already-FINALIZED feature with zero file overlap — was observed failing intermittently during this
work; investigated and confirmed NOT connected to this feature's own changes, but not resolved
here, since it is outside this feature's own scope; see § Lessons Learned).

## Lessons Learned

1. **A mutation-testing harness scoped to avoid Docker-backed tests will report EVERY handler-level
   mutant as "missed," even when the real acceptance suite already catches it — cross-checking each
   miss individually against the actual acceptance tests (not blanket-asserting "the acceptance
   suite covers this") is what actually finds the genuine gaps hiding among the expected
   scope-exclusion noise.** Of 8 reported misses, only 2 were real; the other 6 were exactly the
   kind of false signal a lazier "the Docker-backed layer isn't run, trust the acceptance suite"
   write-up (this session's own precedent from 4c/4d/4e) could have missed without the individual
   verification this feature's own QUALITY_GATE did.
2. **A hand-rolled role-comparison check (`role < Role::Admin`) needs a test at the EXACT boundary
   value, not just above and below it.** Every prior test in this feature used `Owner` (above) or
   `Viewer` (below) — the ONE role that distinguishes `<` from `<=` (`Admin` itself) was never
   exercised until the mutation pass named it directly.
3. **Two tools that both perform rapid, repeated in-place rebuilds against the SAME shared
   `~/.cargo/shared-target` directory can produce a stale, deterministic-LOOKING test failure with
   zero actual code defect.** A 51-minute `cargo-mutants` run left an incremental-compilation
   artifact that made two acceptance tests fail reproducibly (including one that had passed cleanly
   minutes earlier) until a `touch`+rebuild cleared it — confirmed via direct `git diff` that the
   source file itself was never actually left mutated. Worth checking for BEFORE assuming a newly
   -added test exposed a real regression, especially right after a long mutation-testing run.
4. **An unrelated, currently-flaky pre-existing test should be reported honestly, not silently
   absorbed into "confirmed transient" without enough evidence.** `distributed_rate_limiting`'s own
   `drl_b12_postgres_rate_limit` failed 3 of 4 isolated reruns during this work (a genuinely
   inconsistent pattern, not the clean "always passes alone" signature of the OTHER transient
   failures found earlier this session) — flagged explicitly rather than either chasing a fix
   outside this feature's own scope or falsely characterizing it as resolved.

## Key Files

- `crates/embyr-server/src/admin/handlers/composite_indexes.rs` — `IndexFieldSpec`/
  `IndexFieldOrder`/`CreateCompositeIndexBody`/`CompositeIndexResponse`, `create_composite_index`/
  `list_composite_indexes`/`delete_composite_index`, 5 unit tests.
- `crates/embyr-server/src/admin/router.rs` — 2 new route entries, zero changes to any existing
  route.
- `crates/embyr-server/src/grpc/handler.rs` — **untouched**, confirmed by construction.
- `docs/product/architecture/adr-068-composite-index-admin-crud.md`
- `tests/firestore_composite_indexes_admin_api/acceptance/` — cix01 through cix03, 3 acceptance
  targets, 17 tests, shared `common/mod.rs` (re-exports `security_rules`'s own full composition
  -root harness — no access-rule-specific scaffolding needed).
- `docs/feature/firestore-composite-indexes-admin-api/feature-delta.md` — full DISCUSS/DESIGN
  narrative (retained in place, this project's established SSOT convention).
- `docs/feature/firestore-composite-indexes-admin-api/slices/` — 3 elephant-carpaccio slice briefs.
- `docs/feature/firestore-composite-indexes-admin-api/deliver/mutation/mutation-report.md`

## Follow-Up Work

- **Widening `requires_composite_index`'s own simplistic heuristic** (single orderBy-vs-filter
  -field check, vs real Firestore's actual multi-field/array-contains/exemption rules) — explicitly
  out of this feature's own scope from the outset (named at kickoff); a separate, evidenced
  follow-up, no candidate feature id assigned yet.
- **Real Postgres index provisioning for query performance** — this feature is metadata-only; no
  domain example currently shows composite-query latency as a problem. Named, deferred.
- **Real Firestore's own async `CREATING` window / build-latency simulation** — zero domain
  evidence. Named, deferred.
- **Per-field-set granularity in `is_index_ready`** (today: ANY ready index for a collection
  unblocks ANY query needing one for that collection, regardless of field match) — a PRE-EXISTING
  behavior this feature neither builds nor changes; named so it is never mistaken for a regression
  this feature introduced.
- **`distributed_rate_limiting`'s own intermittent test failure** (`drl_b12_postgres_rate_limit::
  distributed_rate_limit_rejects_when_bucket_exhausted`) — observed failing 3 of 4 isolated reruns
  during this feature's own work, zero file overlap, zero connection to this feature. Not
  investigated further here (out of scope); flagged for whoever next touches that feature.
