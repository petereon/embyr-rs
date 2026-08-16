# Evolution: customer-db-onboarding

**Date:** 2026-08-16
**Feature:** Privilege-separated database onboarding — a standalone customer-run CLI
(`embyr-db-prep`) that a customer's own DBA runs under their own elevated Postgres
credentials to prep the database, then hands `embyr-server` a lower-privilege DML-only
connection string; `embyr-server` verifies schema readiness (read-only) before attempting
any migration and returns a named, actionable complaint instead of a generic failure.
**Job:** JOB-15 (`customer-db-preflight`), new job this feature — P6 Elena Vasquez (Customer
DBA, primary), P2 Sam Chen (Service Operator, secondary)
**ADRs:** ADR-022 (`docs/product/architecture/adr-022-customer-db-prep-crate-and-migration-consolidation.md`),
ADR-023 (`docs/product/architecture/adr-023-schema-readiness-verification.md`, revised mid-DESIGN)

## Business Context

Regulated/enterprise customers whose internal Postgres governance prohibits granting
DDL/schema-modification rights to any externally-controlled connection string could not
onboard onto embyr at all: the only provisioning path (`POST /admin/v1/projects`) embedded
an automatic `sqlx::migrate!` call against the exact connection string it stored long-term,
which structurally requires elevated privileges on that same credential.

**Job Discovery framing had to be worked out explicitly, not assumed.** The raw ask ("a
binary customers run to prepare their DB" + "embyr verifies and complains if not properly")
doesn't on its face explain why a standalone tool is needed at all, since `provision.rs`
already auto-migrated the customer DB at provisioning time. DISCUSS evaluated three
candidate motivating forces before locking scope:

| Option | Force | Verdict |
|---|---|---|
| (a) Privilege separation | Enterprise DBAs won't grant DDL rights to a SaaS provisioning flow; want to prep once under elevated creds, then hand over a DML-only string | **Locked.** Only option explaining both halves of the raw ask as one job — a standalone prep step is *necessary* (auto-migrate can't run against a DML-only credential) and "verify and complain" is exactly the check needed to safely accept one |
| (b) Self-serve pre-flight without operator access | Trial without going through the admin API | Plausible side-benefit of (a)'s design, but doesn't explain the "complain" half — deferred |
| (c) Ongoing drift detection | Re-check after provisioning, not just at prep time | Real but materially different mechanism/moment — deferred, flagged as a candidate follow-up feature |

This could not be resolved from codebase evidence alone (a product-segment-priority
judgment, not something the code reveals); DISCUSS made the reasoning fully explicit and
auditable, flagged for redirect if wrong, rather than blocking on an unavailable
`AskUserQuestion` interactive gate. No redirect came back — framing (a) held through DESIGN
and DELIVER.

## Key Decisions

### DISCUSS-wave scope decisions

- **JOB-15, new job** — not an extension of JOB-02 (`tenant-provision`, describes the
  *operator's* job) or JOB-04 (`credential-isolation`, scoped to `embyr-agent`'s VPC-bound
  credentials, a different privilege axis entirely).
- **P6 Elena Vasquez, new inline persona** (not a dedicated file, matching the P1-P4
  precedent — only P5 gets a dedicated file, as a persistent dashboard-facing user).
- **Scoped to `backend_mode=direct_pg` only.** `aws_secret`/`gcp_secret` share the identical
  gap but are deferred (OQ-3); `agent` mode is excluded on independent grounds (VPC-bound
  credentials, no cross-tenant privilege-separation problem to solve).
- Scope-assessment gate: 0-1 of 5 oversized-feature signals fired (threshold 2+) — PASS,
  right-sized as 2 stories / 2 slices, no split needed.

### DESIGN-wave decisions (ADR-022, ADR-023)

- **ADR-022 — Single embed point for `migrations/customer/`.** Collapsed 5 independent
  `sqlx::migrate!("../../migrations/customer")` invocations (3 in `provision.rs`'s branches,
  2 in `backend_adapter.rs`) down to 1, hoisted to a module-level `static MIGRATOR` in
  `PostgresBackendAdapter`. This directly resolves DISCUSS's #1-flagged integration risk
  (the standalone tool and `embyr-server` must never read two independently-maintained
  copies of the expected schema) structurally, not by convention. New workspace crate
  `embyr-db-prep` (not a `[[bin]]` inside `embyr-agent`/`embyr-server`) for supply-chain
  minimization, mirroring the existing rationale for `embyr-agent`'s own separate-binary
  existence.
- **ADR-023 — Schema-readiness verification reuses sqlx's own `_sqlx_migrations`
  bookkeeping table** (no bespoke marker table, avoiding a second drift-prone bookkeeping
  mechanism), read-only, compared via `found_version >= expected_version` (not strict
  equality — forward-compatible during rolling deploys, flagged OQ-1). Verification enriches
  the existing migrate-attempt failure path rather than replacing it — the only way to
  preserve AC-02-06 (no regression to today's full-privilege auto-migrate flow), since
  read-only introspection alone cannot determine whether a credential actually has DDL
  rights.
- Postgres's own `format('%I', ...)` for GRANT identifier interpolation — quoting
  correctness delegated to the database engine, not hand-rolled Rust-side escaping.

### Mid-DESIGN security review and ADR-023 revision (notable finding)

The original Component Decomposition proposed granting `_sqlx_migrations` read access via a
static migration file: `GRANT SELECT ON _sqlx_migrations TO PUBLIC`. A targeted security
review of ADR-023 (invoked because the grant was flagged as a plausible "security boundary
change" per-wave review trigger) came back **APPROVED overall**, but rejected this specific
mechanism as avoidably broad for a feature whose entire framing is least-privilege
separation for regulated customers under governance audit — a `PUBLIC` grant would undercut
the feature's own selling point.

**Revised mechanism (DDD-7 / CDO-AD-08):** the grant is now a runtime, role-scoped
operation performed only by `embyr-db-prep`, never a tracked migration. `embyr-db-prep`
opens a brief secondary connection using the DML role's own credential, runs
`SELECT current_user` to self-discover the exact role name, drops that connection, then
executes `GRANT SELECT ON _sqlx_migrations TO <role>` over the elevated connection, with the
role identifier quoted via Postgres's own `format('%I', ...)`. This achieves the identical
zero-manual-role-typing property as the `PUBLIC` proposal at no extra engineering cost, while
closing the over-broad-grant risk. The revision touched ADR-023, the architecture brief's
Component Decomposition/Driven Ports/both C4 diagrams/Decisions Table, and this feature's own
DESIGN section — and a dedicated negative acceptance scenario (`cdo08`, a distinct
non-granted role cannot read `_sqlx_migrations`) exists specifically to catch a `PUBLIC`-grant
regression, the exact test that would have caught the original proposal.

## Steps Completed

All 6 roadmap steps (`docs/feature/customer-db-onboarding/deliver/execution-log.json`) show
complete `PREPARE → RED_ACCEPTANCE → GREEN → COMMIT` traces, plus a `phase3-refactor` pass.

| Step | Name | Status |
|---|---|---|
| 01-01 | Walking Skeleton (US-01) — `DbPrepConfig::from_env()` + `main.rs` migrate-and-report flow (fresh apply, idempotent no-op, interrupted-run resume) | PASS (see Notable Findings — cdo03 fixture defect) |
| 01-02 | US-01 error classification — `error_report::classify()` + connect/migrate error wiring | PASS |
| 02-01 | Migration Embed Consolidation (ADR-022) — collapse 5 embed points to 1, hard prerequisite for 03-01 | PASS |
| 03-01 | Walking Skeleton (US-02) — `SchemaReadiness::classify()` + `verify_schema_readiness()` + Ready-skip-migrate branch (forward-compatible `>=`) | PASS |
| 04-01 | Provisioning error enrichment — `customer_db_not_prepped`/`customer_db_schema_stale` bodies, regression guardrail (AC-02-06) | PASS |
| 05-01 | DML-role grant mechanism (ADR-023 revised) — `discover_current_user()` + `grant_schema_readiness_read()`, PUBLIC-regression guard, idempotency, injection-safety, OQ-5 sequencing | PASS |

Commits: `6c91eef`..`97b39aa` (feature steps), plus `d11adba` (Phase 3 refactor — deduped
cdo12-cdo18 setup boilerplate into `common/mod.rs` helpers) and `37d59c4` (Phase 5
mutation-testing gap closure). `des-verify-integrity` reports 6/6 steps traced, exit 0.

## Scenarios: 18/18 green, 0 ignored

`cargo test` across both `embyr-db-prep` and `embyr-server` `[[test]]` targets: all 18
`cdo01`-`cdo18` acceptance scenarios pass, zero `#[ignore]` markers remaining, independently
verified by the orchestrator. Error/edge ratio 12/18 = 66.7%, well above the 40% floor.

## Quality Gates

- **Roadmap review (Phase 1):** APPROVED, 0 blockers — all 18 DISTILL scenarios mapped to
  exactly one of 6 steps, dependency DAG verified acyclic (the 02-01→03-01 hard prerequisite,
  driven by cdo06's single-embed-point architecture-enforcement test, was traced before
  dispatch, not discovered afterward).
- **Per-step TDD (Phase 2):** 6/6 steps COMMIT/PASS.
- **Post-merge integration + demo evidence (Phase 3.5):** both stories' real-subprocess,
  real-Postgres acceptance runs stand as demo evidence — all exit 0/201/400 as expected.
- **Refactor L1-L6 (Phase 3):** deduped byte-identical setup boilerplate across
  cdo12-cdo18 into shared `common/mod.rs` helpers; stale RED-scaffold doc comments removed.
- **Adversarial review (Phase 4):** APPROVED, 0 blockers/defects — independently spot-checked
  the role-scoped GRANT mechanism and the AC-02-06 regression path against actual source
  before accepting the verdict.
- **Mutation testing (Phase 5, `per-feature`):** 72 mutants (diff-scoped vs `079d563`, split
  across 4 packages), kill rate 47/49 = 95.9% (excluding 23 structurally-unviable mutants
  against non-`Default` return types), well past the 80% gate. Closed a real gap — see
  Lessons Learned.
- **Deliver integrity verification (Phase 6):** exit 0, 6/6 steps traced.

## Notable Findings

### Test-infrastructure defects — 4 found across the feature's lifecycle, all caught before or during DELIVER

None of the 4 defects found this feature were production-code bugs — all were
test-infrastructure defects, honestly documented and fixed rather than silently patched.
Worth naming as a pattern: **testing infrastructure needs the same scrutiny as production
code**, since a broken fixture can mask real functionality gaps (`FIXTURE_BROKEN` looks
identical to `MISSING_FUNCTIONALITY` from the outside until someone actually reads the
failure).

1. **DISTILL's own fail-for-right-reason gate (2 found, self-caught before DELIVER even
   started):**
   - `cdo12`'s fixture executed a table-level `GRANT ... ON documents, transactions` against
     `sys_pool` (connected to the `postgres` system database) instead of a pool connected to
     the actual customer database — first run failed `FIXTURE_BROKEN` with
     `relation "documents" does not exist`. Fixed by splitting cluster-wide role creation
     from the table-level grant, which must run against the correct database's own pool.
   - The shared `common::create_ddl_role` helper granted only
     `GRANT ALL ON DATABASE <db> TO <role>`, which in **Postgres 15+ does not include
     `CREATE` on the `public` schema** (schema-level privileges are separate from
     database-level ones; `public` is no longer world-createable by default). Every
     "DDL-capable" test role in the suite would have silently lacked real DDL rights the
     moment DELIVER implemented `migrate()`-calling — a version-specific Postgres behavior
     change that would have surfaced as a confusing false-negative deep into DELIVER instead
     of being caught structurally beforehand.
2. **DELIVER step 01-01 (1 found, by the orchestrator):** `cdo03`'s Given-block applied
   migration `0001` via the testcontainer superuser DSN rather than `elena_dsn`, making role
   `postgres` the owner of `_sqlx_migrations`; the actually-scoped `elena_dba` role then got
   `permission denied for table _sqlx_migrations` when `migrate()` ran under her DSN.
   Contradicted ADR-023's stated assumption that `_sqlx_migrations` is owned by Elena's
   elevated role by virtue of having created it — confirmed via direct psql reproduction and
   the real compiled binary before the fix. Outside step 01-01's `files_to_modify` scope
   (test-only fix), tracked as an explicit escalation rather than silently patched
   out-of-scope.
3. **DELIVER step 04-01 (1 found, by the crafter):** a missing `grant_migrations_table_read()`
   helper call in `cdo14`'s fixture.

### Mutation-testing gap-closure on pure helper functions — third occurrence this session

Phase 5 surfaced 4 mutants surviving in `embyr-db-prep`'s pure helper functions
(`host_from_dsn`, `is_insufficient_privilege`, `role_and_database_from_dsn`, `classify()`) —
these functions had **zero direct unit tests**, relying entirely on loose integration-test
message-content checks (`contains("ready")` rather than asserting exact values) to exercise
them indirectly. Closed with 8 direct unit tests (`37d59c4`), confirmed by a scoped mutant
re-run. **This is now the third feature this session** (following `card-payments-backend`
and one earlier) where DELIVER's own mutation-testing phase — not code review, not
acceptance-scenario count — found zero-unit-coverage pure functions hiding behind loose
integration-test assertions. Worth naming as a recurring, structural pattern rather than a
one-off: integration/acceptance tests that assert on substring presence rather than exact
output routinely let individual logic branches inside small pure helpers go completely
unverified, and only mutation testing catches it reliably.

2 mutants remain accepted survivors: `verify_schema_readiness()`'s SQLSTATE match guard
(`42P01`|`42501`) — embedded inside an async I/O function, not cheaply unit-testable in
isolation, and the observable behavior it drives (`NotPrepped` classification) is already
covered end-to-end by cdo01-03/cdo09's real-Postgres scenarios.

## Open Questions (5, all non-blocking)

- **OQ-1** — `found_version >= expected_version` (not strict equality) is forward-compatible
  during rolling deploys but could mask a future non-additive/breaking migration. No action
  needed unless/until a non-additive migration is introduced.
- **OQ-2** — DESIGN found a factual error in DISCUSS's stated reason for excluding
  `embyr-agent` from scope: `embyr-agent`'s production `run()` path does not actually
  self-migrate at startup (contra the original AD-A08 characterization) — its own schema
  provisioning mechanism is undocumented. Does not reopen scope: the exclusion holds on
  independent grounds (agent-mode credentials never leave the customer's VPC, so this
  feature's privilege-separation problem doesn't exist there regardless of how schema gets
  applied). Recommended as a follow-up documentation/fix item.
- **OQ-3** — Design confirmed to generalize to `aws_secret`/`gcp_secret` backend modes with
  zero new adapter code, but was not built for them this feature (deferred per DISCUSS scope).
- **OQ-4** — Resolved this wave: the PUBLIC-vs-role-scoped grant question, closed by the
  mid-DESIGN security review (see Notable Findings above).
- **OQ-5** — The role-scoped grant introduces a documented ordering dependency: if Elena runs
  `embyr-db-prep` without `EMBYR_DB_PREP_DML_ROLE_DSN` set (e.g. before the DML role exists
  yet), `verify_schema_readiness()` reports `NotPrepped` at provisioning time even though the
  schema itself is fully applied, until a follow-up prep run supplies the DSN. Documented
  trade-off, not a defect — has its own acceptance test (`cdo09`, the 3-step chained
  sequencing scenario).

## Lessons Learned

1. **Job Discovery framing sometimes has to be worked out, not assumed from the raw ask.**
   The raw feature request looked redundant against an existing auto-migrate path until
   DISCUSS explicitly enumerated and scored three candidate motivating forces — the
   resolution (privilege separation) was the only one that coherently explained both halves
   of the ask as one job. Worth treating "does the raw ask actually need what it says it
   needs, given what already exists" as a standing DISCUSS-wave question, not just accepting
   scope at face value.
2. **A security review mid-DESIGN caught an avoidably broad grant before it shipped.**
   The original `PUBLIC`-grant proposal would have technically worked and passed every
   positive test — only a targeted security review, invoked because the change touched a
   privilege boundary, caught that it undercut the feature's own least-privilege framing.
   The fix cost no extra engineering effort (role-scoped self-discovery vs. a static grant)
   but required someone to ask "is this grant as narrow as it could be," not just "does this
   grant work."
3. **Test-infrastructure bugs deserve the same rigor as production bugs.** 4 fixture/test
   defects were found and fixed across this feature's lifecycle (2 self-caught in DISTILL's
   own fail-for-right-reason gate, 2 caught during DELIVER) — zero were production-code
   bugs. Each was documented explicitly rather than silently patched, including one
   (Postgres 15+'s schema-privilege-separate-from-database-privilege behavior) that would
   have silently invalidated every "DDL-capable" test role in the suite if left uncaught.
4. **Mutation testing keeps finding the same shape of gap: pure helpers behind loose
   integration assertions.** Third occurrence this session of DELIVER's mutation-testing
   phase surfacing zero-unit-coverage pure functions that acceptance/integration tests only
   exercised via substring-presence checks. This is a structural blind spot worth naming and
   watching for proactively in future features' RED_UNIT phases, not just catching
   reactively in Phase 5 each time.

## Key Files

- `crates/embyr-db-prep/` — new crate, `main.rs`, `config.rs`, `error_report.rs`
- `crates/embyr-core/src/domain/schema_readiness.rs` — pure `SchemaReadiness` enum +
  `classify()` (NO IO, enforced by `deny.toml`)
- `crates/embyr-pg-storage/src/backend_adapter.rs` — `verify_schema_readiness()`,
  `discover_current_user()`, `grant_schema_readiness_read()`; `migrate()`/`run_migrations()`
  refactored to a single module-level `static MIGRATOR` (ADR-022)
- `crates/embyr-server/src/admin/handlers/provision.rs` — `direct_pg` branch: verify-then-
  conditionally-migrate + enriched error bodies; all 3 branches call the adapter's
  `migrate()` instead of an inline macro
- `docs/product/architecture/adr-022-customer-db-prep-crate-and-migration-consolidation.md`
- `docs/product/architecture/adr-023-schema-readiness-verification.md` (revised mid-DESIGN)
- `docs/product/architecture/brief.md` § Application Architecture — customer-db-onboarding
- `docs/product/jobs.yaml` — JOB-15 (new), JOB-02 cross-reference note
- `docs/product/journeys/customer-dba.yaml` — new pointer file
- `tests/customer_db_onboarding/` — 18 acceptance scenarios (`cdo01`-`cdo18`) + shared
  `common/mod.rs` harness
- `docs/feature/customer-db-onboarding/feature-delta.md` — full DISCUSS+DESIGN+DISTILL+DELIVER
  narrative (retained in place, not migrated)
- `docs/feature/customer-db-onboarding/distill/red-classification.md` — the 2 DISTILL-caught
  fixture bugs, full detail

## Follow-Up Work

- OQ-1: revisit the `>=` forward-compatible version comparison if/when a non-additive
  (breaking) migration is introduced.
- OQ-2: document or fix `embyr-agent`'s actual schema-provisioning mechanism (currently
  undocumented; the DISCUSS-stated AD-A08 self-migration characterization was incorrect).
- OQ-3: `aws_secret`/`gcp_secret` backend-mode generalization, if/when scoped — design already
  confirmed to require zero new adapter code.
- OQ-5's sequencing gap is a documented trade-off, not slated for a fix, but worth keeping in
  mind for onboarding documentation aimed at Elena's persona.
- Outcome KPI measurement (KPI #1 North Star: DBA-gated onboarding completion rate; KPI #2
  leading: self-resolved-failure rate; KPI #3 guardrail: no regression to existing
  `direct_pg` provisioning success rate) — DEVOPS-wave scope, owner platform-architect,
  weekly/continuous per the Measurement Plan in `feature-delta.md` § Outcome KPIs.
- Job Discovery option (c), ongoing schema-drift detection after successful provisioning, is
  a candidate follow-up feature — materially different mechanism and moment from this
  feature's locked privilege-separation framing.
