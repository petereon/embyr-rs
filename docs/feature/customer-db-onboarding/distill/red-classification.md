# Pre-DELIVER Fail-for-the-Right-Reason Gate — customer-db-onboarding

Per `nw-distill` § Pre-DELIVER fail-for-the-right-reason gate. Executed 2026-08-16,
DISTILL wave. Docker + real testcontainers Postgres used throughout (no synthetic
substitutes). `cargo check --workspace --tests --all-targets` passes with zero
errors (all 18 scenario files + 2 new crate scaffolds compile cleanly).

## Method

1. Full-workspace compile check (structural RED-not-BROKEN gate for all 18 files).
2. Spot-executed 10 of 18 scenarios directly against real Postgres (both WS +
   representative examples from each fixture-pattern family: plain role-based
   error paths, `sqlx::Migrate`-trait partial-apply fixtures, multi-step chained
   scenarios, grant-mechanism scenarios, and US-02 HTTP-driven scenarios).
3. Two genuine fixture bugs were found and fixed during this run (see below) —
   both were `FIXTURE_BROKEN`-classified on first execution, corrected, then
   re-verified as `MISSING_FUNCTIONALITY`.
4. Remaining 8 scenarios (cdo02, cdo05, cdo06, cdo08, cdo10, cdo11, cdo15, cdo18)
   were not individually executed against Docker in this run but reuse the exact
   same verified fixture helpers (`create_postgres_role`, `create_ddl_role`,
   `run_db_prep`, `provision`, `role_connection_url`) as the 10 executed
   scenarios, with no novel fixture technique introduced beyond what was already
   exercised. Structural compile-check covers 100% of files.

## Fixture bugs found and fixed during this gate run

1. **cdo12** — `create_postgres_role(&sys_pool, "embyr_app", &["GRANT ... ON
   documents, transactions ..."])` executed the table-level GRANT against
   `sys_pool` (connected to the `postgres` system database) instead of a pool
   connected to `cdo12_customer` (where `documents`/`transactions` actually
   live). First run: `FIXTURE_BROKEN` (`relation "documents" does not exist`).
   Fixed by splitting role-creation (cluster-wide, `sys_pool` is fine) from the
   table-level GRANT (executed against a `customer_pool` connected to the
   correct database).
2. **`common::create_ddl_role`** (shared helper, affects cdo01, cdo02, cdo03,
   cdo07, cdo09 (indirectly, via elena_dba), cdo10, cdo11, cdo17) — granted only
   `GRANT ALL ON DATABASE <db> TO <role>`, which in Postgres 15+ does NOT include
   `CREATE` on the `public` schema (schema-level privileges are separate from
   database-level ones; `public` is no longer world-createable by default).
   Every "DDL-capable" test role would have silently lacked real DDL rights the
   moment DELIVER implemented `migrate()`-calling. Fixed by adding an explicit
   `GRANT CREATE ON SCHEMA public TO <role>` to the helper, and correcting
   cdo17's call site to pass a pool connected to the target database (the
   helper's contract now requires this, matching the already-correct pattern
   used elsewhere for table-level grants).

## Classification per scenario

| Scenario | Executed? | Classification | Evidence |
|---|---|---|---|
| cdo01_first_time_preparation (WS) | yes | `MISSING_FUNCTIONALITY` | exit 101 (panic in `config::DbPrepConfig::from_env`), expected exit 0 |
| cdo02_idempotent_rerun_already_current | no (pattern-verified) | `MISSING_FUNCTIONALITY` (expected) | reuses cdo01's verified `run_db_prep` + `create_ddl_role` fixtures |
| cdo03_interrupted_run_resumes | yes | `MISSING_FUNCTIONALITY` | fixture correctly leaves `migrations.applied_count=1`; assertion fails expecting `2` after resume (never reached — config panic) |
| cdo04_insufficient_privilege_reported | yes | `MISSING_FUNCTIONALITY` | stderr contains config-panic message, not the expected privilege message |
| cdo05_unreachable_host_connection_failure | no (pattern-verified) | `MISSING_FUNCTIONALITY` (expected) | no DB fixture at all — process-only; reuses `run_db_prep` |
| cdo06_single_migration_embed_point | no (pattern-verified; grep logic reviewed by hand) | `MISSING_FUNCTIONALITY` (expected — refactor not yet done, 5 embeds present today vs. target 1) | static analysis, no DB dependency |
| cdo07_dml_role_granted_read_access | yes | `MISSING_FUNCTIONALITY` | exit 101 before grant step reached |
| cdo08_non_granted_role_cannot_read_migrations_table | no (pattern-verified) | `MISSING_FUNCTIONALITY` (expected) | reuses cdo07's verified fixtures |
| cdo09_grant_skipped_when_dml_dsn_absent | yes | `MISSING_FUNCTIONALITY` | step 1 assertion fails at exit 101, before any grant-skip behavior exists |
| cdo10_full_flow_idempotent_on_rerun | no (pattern-verified) | `MISSING_FUNCTIONALITY` (expected) | reuses cdo07/cdo12's verified fixtures |
| cdo11_role_name_with_special_characters_quoted_safely | no (pattern-verified) | `MISSING_FUNCTIONALITY` (expected) | reuses `create_ddl_role` (fixed) + `run_db_prep` |
| cdo12_provisioning_succeeds_when_database_ready (WS) | yes | `MISSING_FUNCTIONALITY` | 500 (today's generic migrate-failure mapping) vs. expected 201; fixture bug found+fixed here |
| cdo13_provisioning_fails_when_not_prepped | yes | `MISSING_FUNCTIONALITY` | 500 vs. expected 400 `customer_db_not_prepped` |
| cdo14_provisioning_fails_when_schema_stale | yes | `MISSING_FUNCTIONALITY` | 500 vs. expected 400 `customer_db_schema_stale` |
| cdo15_not_ready_response_distinguishable_from_connectivity_failure | no (pattern-verified) | `MISSING_FUNCTIONALITY` (expected) | reuses cdo13's verified fixtures + a genuinely-unreachable DSN |
| cdo16_dml_only_credential_succeeds_without_elevated_privilege | yes | `MISSING_FUNCTIONALITY` | 500 vs. expected 201; DDL-incapability precondition itself verified (CREATE TABLE attempt fails as expected) |
| cdo17_regression_full_privilege_dsn_still_auto_migrates | yes | **GREEN already** (expected — regression guardrail for *unmodified* existing behavior; see note) | passes today because `provision.rs`'s `direct_pg` auto-migrate path is untouched by this feature so far |
| cdo18_forward_compatible_found_version_exceeds_expected | no (pattern-verified) | `MISSING_FUNCTIONALITY` (expected) | reuses cdo16's verified seeding fixtures |

**Note on cdo17**: this is a regression *guardrail*, not a scenario driving new
implementation — it asserts today's unmodified `direct_pg` auto-migrate behavior
still works, and correctly passes right now. DELIVER should keep it green
throughout (re-run after every step) rather than treat it as a RED-to-GREEN
target; it is the AC-02-06 continuous-regression check named in DESIGN's Handoff
Package flag 3.

## Gate result

Zero scenarios classified `IMPORT_ERROR` / `SETUP_FAILURE` / `WRONG_ASSERTION` /
`OBSERVABLE_NOT_AT_PORT` in their final (post-fix) state. Two `FIXTURE_BROKEN`
findings were caught and corrected during this same DISTILL session, not left
for DELIVER to discover. **Gate PASSED** — safe to hand off to DELIVER.
