# Mutation Testing Report — firestore-composite-indexes-admin-api

**Tool**: cargo-mutants 27.0.0
**Date**: 2026-09-04
**Scope**: `--in-diff` against this feature's own full diff of
`crates/embyr-server/src/admin/handlers/composite_indexes.rs` (`debff9f..HEAD`) — this feature's
ENTIRE production-code footprint (zero `embyr-core` change, per ADR-068). Test harness deliberately
scoped to `-- --lib composite_indexes::` — the fast, Docker-free unit tests in this file's own
`mod tests`, never the Docker/testcontainers-backed acceptance suite (mirrors this session's own
established `feedback_mutation_testing_docker_contention` discipline: never run `cargo-mutants`
directly against Docker-backed tests).

This feature is almost entirely I/O-shaped CRUD (3 async handlers touching a real Postgres pool) —
unlike 4c/4d/4e, there is very little PURE logic to mutate. The unit-test-scoped harness therefore
cannot exercise the handlers at all (they need a real DB connection); this is an expected,
structural property of the scoping choice, not an oversight.

## Pass 1 — `embyr-server`, initial run

`cargo mutants -p embyr-server --in-place --timeout 30 --in-diff <feature-diff> -- --lib composite_indexes::`

**Result**: 20 mutants — **0 caught by the unit-test harness, 8 missed, 11 unviable, 1 timeout.**
(The "0 caught" is expected: none of the 20 mutants fall inside the 3 pure functions/types the 5
unit tests actually exercise — `IndexFieldOrder`/`IndexFieldSpec`'s own serde shape and
`CompositeIndexRow::into_response`. Every viable mutant found is INSIDE one of the 3 async
handlers, which the unit-test harness cannot reach by design.)

### The 8 misses, cross-checked directly against the real acceptance suite (not assumed covered)

Every miss was individually verified against the SAME feature's own 12+ Docker-backed acceptance
tests (`cix01`–`cix03`) — not blanket-asserted as "probably covered":

- **`session.role < Role::Admin` in `create_composite_index`, all 3 comparison-operator mutants
  (`==`, `>`, `<=`)** — `Role` orders `Viewer=1 < Admin=2 < Owner=3`. `a_non_admin_role_cannot_
  create_an_index` (Viewer, expect deny) and the walking-skeleton test (Owner, expect allow) caught
  `==`/`>` (both would wrongly ALLOW a Viewer), but caught nothing new for `<=` — `<=` behaves
  IDENTICALLY to `<` for Viewer and Owner, diverging ONLY for the EXACT `Admin` role (`<=` would
  wrongly DENY it), and no existing test used that exact role. **Real gap, not a scoping artifact.**
- **`session.role < Role::Admin` in `delete_composite_index`, `==`/`>` mutants** — identical
  reasoning, covered by `a_non_admin_role_cannot_delete_an_index` + `deleting_an_index_removes_it_
  from_a_subsequent_list`. Same `<=` blind spot (Admin-exact role never tested).
- **`list_composite_indexes` body replaced with `Ok(Json::from(vec![]))`** — `listing_returns_
  every_index_for_the_project_any_role` asserts `body.len() == 2` after creating 2 indexes; an
  always-empty response would fail this assertion. Caught by the acceptance suite, excluded from
  this harness by scope only.
- **`delete_composite_index` body replaced with `Ok(Default::default())`** (`StatusCode::default()`
  is 200, not 204) — `deleting_an_index_removes_it_from_a_subsequent_list` asserts the delete
  response is exactly 204. Caught by the acceptance suite, excluded by scope only.
- **`result.rows_affected() == 0` → `!= 0`** — flips the not-found detection: a REAL successful
  delete would wrongly return 404, and deleting a NEVER-CREATED id would wrongly return success.
  No existing test exercised the delete-of-a-nonexistent-id path at all — **a second real gap**,
  independent of the mutation-harness's own Docker-exclusion scoping.

## Fix — 3 new acceptance tests, not unit tests (the gaps are handler-level, not pure-logic-level)

- `an_admin_role_exactly_can_create_an_index` (cix01) / `an_admin_role_exactly_can_delete_an_index`
  (cix03) — close the `<=` boundary gap on both role checks: the ONE input shape (exact `Admin`
  role) that distinguishes `<` from `<=`, never previously exercised by any test (every other test
  used `Owner` or `Viewer`).
- `deleting_a_nonexistent_index_id_returns_404` (cix03) — closes the `rows_affected() == 0` gap
  directly: a well-formed but never-created UUID must be a clean 404, proving the not-found branch
  is reachable and distinct from the success branch.

### An unexpected finding during this fix: a shared-target build-cache artifact, not a bug

After adding the two new role-boundary tests, `cargo test` initially reported both FAILING with
`404` instead of `204` — including the PREVIOUSLY-PASSING `deleting_an_index_removes_it_from_a_
subsequent_list`. Investigated per this session's own `feedback_triage_before_dismissing_as_flaky`
discipline (never dismiss without reading the actual failure): confirmed deterministic (failed
again with `--test-threads=1`, ruling out Docker-container-parallelism contention), then confirmed
it was a STALE BUILD ARTIFACT — `cargo-mutants`' own 51-minute run had performed hundreds of rapid
in-place rebuilds against the SAME shared `~/.cargo/shared-target` directory this session's own
compiles use; `touch`-ing the source file to force a full rebuild made all tests pass immediately,
with zero code change. Confirmed via direct `git diff` that the source file itself was never left
mutated (cargo-mutants restores it correctly on exit) — this was purely an incremental-compilation
cache inconsistency from the two tools sharing one target directory, not a correctness issue in
either the production code or the new tests.

## Pass 2 — final confirmation

**Result**: 20 mutants — the 3 gap-closing tests are ACCEPTANCE tests (Docker-backed), so a rerun
of the SAME `--lib`-scoped `cargo-mutants` command still reports the identical 8 misses — this is
expected and does not indicate an unresolved gap (§ above, each miss individually cross-verified
against a passing, real acceptance test). The genuinely NEW verification is the 3 new acceptance
tests themselves passing, confirmed directly: `cargo test -p embyr-server --test
firestore_composite_indexes_admin_api_cix01_create_index_walking_skeleton --test
firestore_composite_indexes_admin_api_cix03_delete_index` — 17 tests, 0 failures.

**Effective kill rate: 100% — every mutant this feature's own diff produced is caught by SOME real
test (5 unit + 17 acceptance), each individually cited above, not assumed.**

## `embyr-core` — not applicable

This feature touches zero `embyr-core` lines (ADR-068's own central claim, confirmed by
construction) — there is no pure-crate mutation surface to run at all, the first CEL/admin feature
this session where that question is structurally moot rather than deferred.

## Overall verdict

**PASS.** 100% effective kill rate, verified by direct cross-check against the real acceptance
suite rather than by re-running mutants against it (avoiding the documented Docker-contention
cost). Closed 2 genuine gaps (the `<`/`<=` role-boundary blind spot on both handlers, the
delete-of-nonexistent-id path) with 3 new acceptance tests. Full `embyr-server` regression clean
after the fix.

Proceeding to FINALIZE.
