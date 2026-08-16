# customer-db-onboarding — Feature Delta

**Wave**: DISCUSS | **Agent**: Luna (nw-product-owner) | **Date**: 2026-08-16
**Status**: Ready for DESIGN handoff
**Upstream**: none — greenfield feature, no DISCOVER/DIVERGE wave ran

---

## Wave: DISCUSS / [REF] Prior Wave Consultation — Reading Confirmation

✓ `docs/product/jobs.yaml` (all 14 existing jobs read in full; JOB-02 tenant-provision, JOB-04 credential-isolation, JOB-05 cloud-secret, JOB-07 agent-operations, JOB-13 production-deployment identified as the closest structural precedents, per dispatch)
✓ `docs/product/architecture/brief.md` (System Quality Attributes, Process Topology, Failure Modes/Substrate Probes, C4 diagrams, Bounded Contexts, Ubiquitous Language, Aggregates — read in full for the "provision"/"customer DB"/"backend_mode"/"SystemDb" C4 context; AD-A08 identified as the closest existing precedent for a binary applying `migrations/customer/` outside `embyr-server`'s own process)
✓ `docs/product/journeys/tenant-admin.yaml`, `docs/product/journeys/service-operator.yaml` (both are minimal pointer files — 9 lines each — covering P3 Morgan/cloud-secret and P2 Sam/tenant-provision+tenant-control respectively; neither models a DBA-privilege-separation concern, confirming this feature needs a new journey pointer, not an extension of either)
✓ `docs/product/personas/chris-account-admin.yaml` (the only dedicated persona file, P5; confirms P1-P4 are inline-only in `jobs.yaml` — used as the precedent for deciding whether Elena gets a dedicated file, see § Persona & Job)
✓ `crates/embyr-server/src/admin/handlers/provision.rs` (365 lines, read in full — confirmed: the `aws_secret`, `gcp_secret`, and `direct_pg` branches all call `sqlx::migrate!("../../migrations/customer").run(&customer_pool)` against the exact connection string/fetched-DSN that gets stored; this automatic migration is what structurally requires DDL rights on that credential)
✓ `crates/embyr-server/src/adapters/system_db.rs` (138 lines, read in full — `SystemDb::probe()` is the existing hard-gate schema-verification-at-startup pattern for the *system* DB: `SELECT 1` liveness + `information_schema.tables` check for the `projects` table, refusing to start if the schema isn't initialized; the reference pattern for this feature's customer-DB analogue)
✓ `migrations/customer/` directory listing (`0001_documents.sql`, `0002_transactions.sql` — the exact, small migration set both this feature's prep step and verify step must agree on)
✓ `docs/feature/card-payments-backend/feature-delta.md` (read in full for section structure and template — this feature's own `feature-delta.md` follows the identical `## Wave: DISCUSS / [REF] <Section>` heading convention, single-narrative-file, slice-briefs-as-machine-artifacts pattern)
⊘ `docs/product/vision.md`, `docs/project-brief.md`, `docs/stakeholders.yaml` (not found — same gap noted in prior features' DISCUSS waves; graceful degradation, proceeding without them)
⊘ `docs/feature/customer-db-onboarding/discover/`, `docs/feature/customer-db-onboarding/diverge/` (not found — greenfield, no DISCOVER/DIVERGE wave ran for this feature)

No contradictions found between this feature's scope and prior evidence.

---

## Wave: DISCUSS / [REF] Orchestrator Decisions (not re-litigated)

| # | Decision | Value |
|---|---|---|
| 1 | Feature Type | Cross-cutting — spans a new customer-run CLI entry point and server-side verification logic in `embyr-server`'s provisioning path |
| 2 | Walking Skeleton | Evaluated (this wave's call): **YES**, scoped narrowly — see § Story Map |
| 3 | UX Research Depth | Lightweight — primary users read CLI/log output; journey work below is system/operator-flow-focused, not emotional-arc UX design |
| 4 | JTBD Analysis | Yes (default) — every story traces to `job_id: JOB-15` |

---

## Wave: DISCUSS / [REF] Job Discovery — Framing Resolution

The raw ask ("a binary customers run to prepare their DB for embyr onboarding" + "embyr verifies and complains if not properly") does not, on its face, explain why a *standalone* prep step is needed at all — `provision.rs` already auto-migrates the customer DB at `POST /admin/v1/projects` time (confirmed by reading the file: all three non-agent backend-mode branches call `sqlx::migrate!`). Three candidate motivating forces were identified per the dispatch, and are evaluated here explicitly rather than silently picked.

| Option | Force | Fit against evidence |
|---|---|---|
| (a) Privilege separation | Enterprise DBAs won't grant the SaaS provisioning flow DDL rights; want to run migrations once under elevated creds, then hand over a DML-only connection string | **Strongest fit.** Uniquely explains BOTH halves of the raw ask: (1) a standalone prep step is *necessary*, not redundant, because `provision.rs`'s automatic `sqlx::migrate!` cannot run at all against a DML-only credential; (2) "verify and complain" is exactly the check needed to accept a DML-only credential safely instead of attempting (and failing) a migration against it. |
| (b) Self-serve pre-flight / trial without operator access | Customers want to prep and try embyr without an operator-mediated flow | Plausible secondary benefit (a standalone tool naturally *can* be run without going through the admin API first) but does not explain the "verify and complain" half — if the goal were purely self-serve trial, the existing auto-migrate-on-provision path already serves it; there is no forcing reason today's flow is insufficient for this alone. |
| (c) Ongoing drift detection | Re-check after provisioning, not just at prep time; catches manual DBA changes, DSN rotation to an unmigrated DB, etc. | Real and related, but a **materially different mechanism** (periodic or per-connection re-check vs. a one-time pre-flight) and a different moment in the system's life (post-provisioning vs. at provisioning). Does not explain why a customer would run a *separate binary* at all — drift detection is server-side-only by nature. |

**Resolution**: **(a) Privilege separation is the primary, locked framing for this feature.** It is the only option that coherently explains both deliverables in the raw ask as one job, not two unrelated ones. (b) is noted as a natural side-benefit of (a)'s design (a standalone tool *can* be run without the admin API) but no story in this feature builds toward it as a goal. (c) is explicitly out of scope — see § Out of Scope — and flagged as a candidate follow-up feature, since dispatch correctly identifies it as materially different in mechanism and moment.

**Confidence and escalation note**: this could not be resolved from codebase evidence alone (it is a product/business judgment about which customer segment's pain is most pressing, not something the code reveals) — per the dispatch's own instruction, the reasoning above is made fully explicit and auditable rather than a silent guess, and is called out here for redirect if the actual motivating force differs. No interactive question mechanism was available in this run (no `AskUserQuestion` tool); per Auto Mode guidance, the best-supported reading was adopted and documented rather than blocking the wave.

---

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: **P6 — Elena Vasquez, Customer Database Administrator / Data Platform Owner** (new persona, inline in `jobs.yaml` only — see rationale below). Elena manages production Postgres for an enterprise customer (e.g., "Meridian Health", a regulated healthcare data platform) and operates under an internal policy prohibiting granting DDL/schema-modification privileges to any third-party-controlled connection string.

**Secondary persona**: P2 — Sam Chen (Service Operator). Sam does not initiate this feature's job, but is the actor who calls `POST /admin/v1/projects` with the connection string Elena hands over, and is therefore the one who sees the "complain" surface (US-02) directly. In a self-serve future, a customer account owner could occupy this role instead — not built in this feature (see § Out of Scope).

**New-persona-vs-extend decision**: neither P3 (Morgan, Tenant Admin/DevOps Lead — scoped to *cloud-secret storage*, JOB-05) nor P4 (Riley, Compliance Tenant — scoped to *credential egress prohibition* via `embyr-agent`, JOB-04) matches Elena's concern, which is specifically about *Postgres schema-modification privilege*, a distinct axis from both. Forcing Elena onto either would blur ubiquitous-language precision the architecture brief already establishes for P3/P4. Following the project's own precedent — P1-P4 are inline-only in `jobs.yaml`, only P5 (a persistent, dashboard-facing self-service user) has a dedicated persona file — Elena is a narrow, single-touchpoint persona like P1-P4, so she stays inline as P6, not a new dedicated file.

**job_id decision (per Decision 4)**: this feature creates one new job, **JOB-15 (`customer-db-preflight`)**, rather than extending JOB-02 (`tenant-provision`, P2) or JOB-04 (`credential-isolation`, P4). JOB-02 describes the *operator's* job of running the provisioning API; JOB-15 is the *customer DBA's* job of making DDL-privilege-restricted onboarding possible in the first place — different actor, different moment, same as the dispatch's own framing of why JOB-02 doesn't already cover this. JOB-04 is scoped explicitly to `backend_mode=agent` (credential egress, not DDL privilege) and does not generalize to `direct_pg` mode's schema-privilege concern. JOB-02 receives a cross-reference note (not a rewrite) in `jobs.yaml`, mirroring the project's established pattern (JOB-14 → JOB-10, JOB-11/JOB-06 cross-references in `card-payments-backend`).

**Opportunity scoring**: Importance = 8 (this fully blocks onboarding for a real, if not universal, customer segment — regulated/enterprise Postgres shops with DBA-privilege governance — a hard failure today, not a degraded experience). Satisfaction = 2 (zero support exists: the only path, `provision.rs`'s automatic `sqlx::migrate!`, structurally requires DDL rights the segment cannot grant, and fails with an opaque `backend_unavailable`). Opportunity = 8 + (8−2) = **14**. Priority: **high** (not critical — JOB-02 already works for the majority segment; this closes a specific, real gap, not a universal blocker).

---

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

Run before journey/story-map investment, per Phase 1.5.

| Signal | Threshold | This feature | Fired? |
|---|---|---|---|
| User stories | >10 | 2 (US-01, US-02 — one per slice) | **NO** |
| Bounded contexts / modules | >3 | 2 — a new customer-operated entry point (adjacent to BC-2 Document Storage's schema, but is itself infrastructure tooling, not a domain context) + an extension to BC-1 Tenant Management's provisioning flow | **NO** |
| Walking Skeleton integration points | >5 | 2 — target Postgres (prep step) + `POST /admin/v1/projects`'s existing provisioning path (verify step) | **NO** |
| Estimated effort | >2 weeks | 2 slices × 1.5 days each ≈ 3 days | **NO** |
| Independent shippable outcomes | multiple | Borderline-YES — the prep step (Slice 01) and the verify-and-complain behavior (Slice 02) are each independently useful, though Slice 02 is materially more valuable once Slice 01 exists to produce prepped databases to check against | **YES (weak)** |

**0-1 of 5 signals fired** (threshold is 2+). **Verdict: PASS — right-sized.** No split needed at the oversized-gate level. Per dispatch instruction and Phase 2.5 discipline, the feature is still decomposed into 2 elephant-carpaccio slices below (this is normal thin-slicing, not oversized-triggered splitting) — each independently shippable and each carrying its own learning hypothesis.

---

## Wave: DISCUSS / [REF] System/Operator Journey (lightweight, per Decision 3)

Decision 3 = Lightweight: no new end-user UX surface exists (Elena reads CLI/log output; Sam reads an HTTP error body). Journey work focuses on system/operator flow, not an emotional-arc UX design.

### Database preparation flow (Elena's side, Slice 01)

```
Elena runs the prep step, logged in as elena_dba (elevated, scoped to the target DB)
        │
        ▼
   Connects to target Postgres
        │
   unreachable ──────────────┐            reachable
        │                     │                 │
        ▼                     │                 ▼
  "connection failed:         │        Checks current migration state
   <host> unreachable"        │        (already-current? partial? none?)
   (distinct message,         │                 │
    non-zero exit)            │      ┌──────────┼──────────┐
                               │      │          │          │
                               │  already-    partial     none
                               │  current    (resume)   (apply all)
                               │      │          │          │
                               │      ▼          ▼          ▼
                               │  "already    Applies remaining migrations,
                               │  up to        idempotently
                               │  date"             │
                               │      │          insufficient privilege?
                               │      │               │         │
                               │      │           yes ─┤─ no
                               │      │        "insufficient    │
                               │      │      privilege: role    ▼
                               │      │    <role> lacks CREATE  "database ready
                               │      │    on database <db>"    for embyr
                               │      │    (non-zero exit)      onboarding
                               │      │                         (schema version
                               │      │                          N of N applied)"
                               │      └──────────┬──────────────┘
                               └─────────────────┴─── Elena hands ops team a
                                                       separate DML-only
                                                       connection string
```

### Provisioning verify-and-complain flow (Sam's side, Slice 02)

```
Sam calls POST /admin/v1/projects with the DML-only connection string Elena handed over
        │
        ▼
   Embyr checks the target database's schema state
   (mechanism is DESIGN's call — see slice-02 brief)
        │
   ┌────┴────┬─────────────────┐
   │          │                 │
fully      not prepped     partially prepped /
prepped    at all          stale version
   │          │                 │
   ▼          ▼                 ▼
provisioning  400: "customer_db_not_prepped" 400: "customer_db_schema_stale"
proceeds      naming the missing table/schema  naming expected vs. found version
normally,     element — not a generic
returns       backend_unavailable
{project_id,
 api_key}
```

### Failure modes (feeds DISTILL scenario generation)

- Elena's prep run interrupted mid-way (network drop, VPN timeout) — must resume safely on re-run, not require manual cleanup.
- Elena's elevated login is scoped to the wrong database, or lacks a specific privilege — must name the privilege and the database, not surface a raw driver error.
- Sam submits a DML-only connection string against a database Elena has not yet run the prep step against at all.
- Sam submits a DML-only connection string against a database Elena prepped with an older tool version (schema version gap).
- Existing (non-DBA-gated) customers continue to submit full-privilege DSNs through the unchanged default path — this feature must not regress that flow.

---

## Wave: DISCUSS / [REF] Story Map & Walking Skeleton

**Persona**: P6 Elena (primary) + P2 Sam (secondary, Slice 02 only) | **Goal**: Complete embyr onboarding for a DBA-privilege-gated customer without ever granting embyr's SaaS elevated schema-modification rights.

### Backbone

| A. Customer Prepares Their Database | B. Embyr Confirms Readiness and Complains If Not |
|---|---|
| Elena runs the prep step against Postgres using her elevated DBA login **[WS]** | Sam submits provisioning with the DML-only DSN Elena handed over **[WS]** |
| Prep step reports success + schema version applied **[WS]** | Embyr verifies schema state before completing provisioning **[WS]** |
| Re-running the prep step is safe / idempotent | Embyr proceeds normally when the schema matches |
| Insufficient-privilege and connection failures are reported distinctly | Embyr returns a specific, actionable complaint naming what's missing/mismatched when it doesn't match |

### Walking Skeleton

One task from each activity, thinnest end-to-end happy path: Elena runs the prep step against a fresh database and sees a ready confirmation; Sam submits provisioning with that database's DML-only connection string; embyr verifies the schema is present and matches; provisioning proceeds normally, returning the same response shape as any other successful `direct_pg` provisioning today. This is exactly Slice 01 + Slice 02's respective happy-path scenarios — no facade, no mock; both slices use real Postgres.

### Release 1 (both slices — no further release grouping needed at this scope)

- **Release 1 — DBA-Gated Onboarding Works End-to-End** (Slices 01-02, US-01, US-02). Outcome: a customer whose Postgres governance prohibits granting DDL rights to embyr's SaaS can complete onboarding, and the operator gets a specific, actionable signal instead of a cryptic failure whenever a submitted database isn't ready. Independently demoable and valuable as one release — no further outcome-based splitting warranted at this scope (2 stories, § Scope Assessment: PASS).

---

## Wave: DISCUSS / [REF] Elephant Carpaccio Slices

| Slice | Story | Release | Estimate | Learning Hypothesis (disproves) | Production-data taste test |
|---|---|---|---|---|---|
| 01 (WS) | US-01 | 1 | 1.5 days | A standalone tool run outside `embyr-server`'s own process cannot reliably apply and idempotently re-apply `migrations/customer/` without drifting from what embyr-server's own in-process `sqlx::migrate!` would produce | Real Postgres (no synthetic exception) — the tool must apply real DDL against a real database, idempotency must be proven against a real interrupted-run scenario |
| 02 (WS) | US-02 | 1 | 1.5 days | A DML-only connection string cannot be distinguished, at provisioning time, from "not yet prepped" purely by observable schema state, without embyr's provisioning flow needing any elevated privilege on the submitted connection itself | Real Postgres in each of the three schema states (fully prepped, empty, stale-version) — no synthetic mock of the verification check |

**Total estimate: ~3 days.**

**Taste tests applied**:
- "4+ new components per slice" — neither slice exceeds 2 (Slice 01: prep tool + migration runner reuse; Slice 02: verification check + provisioning-handler extension). PASS.
- "Every slice depends on a new abstraction" — Slice 01 introduces the standalone-tool packaging of the shared migration set; Slice 02 depends on Slice 01 only conceptually (it needs *some* database in each schema state to test against, real or hand-seeded), not on Slice 01 shipping first. PASS — no forced sequencing beyond the natural Walking-Skeleton pairing.
- "No slice disproves a pre-commitment" — each has a distinct, falsifiable hypothesis (see table). PASS.
- "Synthetic-data-only slices prove plumbing, not value" — N/A, does not apply: both slices require real Postgres in real schema states (fresh, partial, stale-version, fully-prepped) — no synthetic-data exception needed.
- "2+ slices identical except for scale" — none; each targets a distinct mechanism (client-side apply vs. server-side verify). PASS.

---

## Wave: DISCUSS / [REF] Prioritization

| Priority | Slice | Target Outcome | Rationale |
|---|---|---|---|
| 1 | Slice 01 (WS) | A customer can prep their own database under their own credentials | Walking Skeleton always first — without a real prepped database to check against, Slice 02 has nothing to verify; also burns down the riskiest new assumption (standalone tool reliably reproduces embyr-server's own migration behavior) before Slice 02 starts |
| 2 | Slice 02 (WS) | Embyr distinguishes ready/not-ready and complains specifically | Closes the loop the raw ask's second half requires; depends on Slice 01 conceptually (needs prepped-database states to test against) but is independently testable against hand-seeded schema states if needed, so no hard build-order dependency |

---

## Wave: DISCUSS / [REF] System Constraints

- **Shared artifact — expected schema set**: `migrations/customer/` (`0001_documents.sql`, `0002_transactions.sql`) is the single source of truth both Slice 01 (the tool that applies it) and Slice 02 (the check that verifies it) must agree on. **Integration risk: HIGH** — a version mismatch between the standalone tool and the currently-deployed `embyr-server` is exactly the anxiety-path failure mode JOB-15 names explicitly. DESIGN must ensure both consumers read the identical migration set (e.g., both built from the same source directory at the same release), not two independently-maintained copies.
- Scoped to `backend_mode=direct_pg` only (see § Out of Scope) — `aws_secret`/`gcp_secret` share the identical underlying gap but are deferred; `agent` mode already self-migrates its own local DB at agent startup (AD-A08) and has no analogous gap.
- Verification (Slice 02) must not require any elevated privilege on the submitted connection string — the entire point of the feature is that the submitted DSN may be DML-only. A verification design that itself needs `information_schema` introspection rights beyond ordinary read access would need to confirm those rights are available to a DML-only role (a real constraint DESIGN must check, not assume).
- No new customer-facing gRPC/REST surface on `:8080`/`:8081` — this feature only touches the admin port (`:9090`) provisioning path and a customer-operated, out-of-band tool; the Firestore-facing data plane is unaffected.
- Ubiquitous language introduced: **prep** / **prepped** (a database has had the expected schema applied), **DML-only connection string** (a credential without DDL/schema-modification rights), **schema version gap** (the prepped schema is older than currently expected). These terms should carry forward into DESIGN's naming, not be silently renamed.
- The "named, specific, actionable error" bar this feature must meet is the same one JOB-13 established for embyr-server's own startup failures ("exit 1 on migration failure," no cryptic errors) — not a new UX standard, an existing one extended to a new surface.

---

## Wave: DISCUSS / [REF] User Stories

<!-- markdownlint-disable MD024 -->

### US-01: Elena Preps Her Database for Embyr Without Granting Elevated Access to Embyr's SaaS

**job_id**: JOB-15
**Slice**: 01 (Walking Skeleton) | **Release**: 1

#### Elevator Pitch
Before: A customer whose Postgres governance policy prohibits granting DDL/schema-modification rights to any externally-controlled connection string cannot onboard onto embyr today — the only provisioning path (`POST /admin/v1/projects`) embeds an automatic `sqlx::migrate!` call against the exact connection string it stores, which structurally requires elevated privileges on that same credential.
After: run the embyr-provided database-preparation step against her Postgres using her own elevated DBA login (exact command shape is DESIGN's call) → sees `database ready for embyr onboarding (schema version 2 of 2 applied)` printed to stdout on success, or a specific, non-zero-exit failure message naming exactly what went wrong.
Decision enabled: Elena decides, on the spot, whether it is now safe to hand a lower-privilege, DML-only connection string to her own operations team or directly to the embyr operator — without ever granting embyr's SaaS a DDL-capable credential.

#### Domain Examples
1. **Happy Path**: Elena runs the prep step against a brand-new, empty Postgres database `meridian_embyr` on `pg-prod.meridianhealth.internal`, logged in as `elena_dba` (a role with `CREATE TABLE` rights scoped to that one database). Sees `database ready for embyr onboarding (schema version 2 of 2 applied)`. She then hands her ops team a connection string for a second, DML-only role, `embyr_app`, scoped to the same database.
2. **Edge Case**: Elena's first prep run was interrupted by a VPN drop after applying only the first of two migrations. She re-runs the identical command an hour later. The tool detects the first migration is already applied, applies only the second, and reports the same "ready" confirmation — no duplicate-object errors, no manual cleanup.
3. **Error/Boundary**: Elena's `elena_dba` login turns out to be scoped to `meridian_shared`, not `meridian_embyr`, and lacks `CREATE TABLE` rights there. She runs the prep step. Sees `insufficient privilege: role elena_dba lacks CREATE on database meridian_shared` — not a raw Postgres driver error stack trace.

#### UAT Scenarios (BDD)

##### Scenario: First-time preparation succeeds and reports the applied schema version
Given Elena has a fresh, empty database `meridian_embyr` and is logged in as `elena_dba` with sufficient privilege
When Elena runs the database-preparation step against it
Then all expected migrations are applied and Elena sees confirmation naming the schema version applied

##### Scenario: Re-running preparation on an already-current database is a safe no-op
Given `meridian_embyr` was already fully prepared by a previous run
When Elena runs the database-preparation step against it again
Then no further schema changes are made and Elena sees confirmation that the database is already up to date

##### Scenario: Re-running preparation after an interrupted partial run resumes safely
Given a previous run applied only the first of two expected migrations before being interrupted
When Elena re-runs the database-preparation step
Then only the remaining migration is applied, no duplicate-object error occurs, and Elena sees the same "ready" confirmation as a clean first-time run

##### Scenario: Insufficient privilege is reported with actionable detail
Given Elena's login lacks the schema-modification privilege required on the target database
When Elena runs the database-preparation step against it
Then Elena sees a message naming the missing privilege and the target database, and the tool exits non-zero

##### Scenario: An unreachable database reports a connection failure, distinct from a privilege failure
Given the target Postgres host is unreachable from where Elena is running the tool
When Elena runs the database-preparation step
Then Elena sees a connection-failure message distinguishable from an insufficient-privilege message, and the tool exits non-zero

#### Acceptance Criteria
- [ ] AC-01-01: Running the tool against a fresh, reachable, sufficiently-privileged database applies the full expected migration set and reports success naming the schema version applied.
- [ ] AC-01-02: Re-running the tool against an already-fully-prepared database makes no further schema changes and reports "already up to date."
- [ ] AC-01-03: Re-running the tool after an interrupted partial run resumes from the last successfully-applied migration and completes without duplicate-object errors.
- [ ] AC-01-04: A privilege-insufficient login produces a message naming the missing privilege and the target database, not a raw driver error.
- [ ] AC-01-05: An unreachable target host produces a connection-failure message, distinguishable from a privilege-failure message.
- [ ] AC-01-06: Every failure mode exits with a non-zero status code (machine-checkable, not just human-readable).

#### Outcome KPIs
See § Outcome KPIs below (KPI #1, North Star).

#### Technical Notes (Optional)
Applies the identical migration set `embyr-server` embeds at `migrations/customer/` (currently `0001_documents.sql`, `0002_transactions.sql`) — exact packaging/distribution mechanism (embedded binary, versioned release artifact, crate name, CLI framework) is a DESIGN-wave decision, not fixed here. Must report an unambiguous, machine-parseable success/failure signal (exit code + message); exact format is DESIGN's call.

---

### US-02: Embyr Gives Sam a Clear, Actionable Complaint Instead of a Cryptic Failure

**job_id**: JOB-15
**Slice**: 02 (Walking Skeleton) | **Release**: 1

#### Elevator Pitch
Before: When a DBA-gated customer's database isn't (or isn't fully) prepped, `POST /admin/v1/projects`'s automatic `sqlx::migrate!` attempt against a DML-only connection string fails with a raw Postgres permission-denied error surfaced as a generic `backend_unavailable` 400 — or, if that automatic migration were simply skipped for DML-only credentials without a replacement check, provisioning could silently "succeed" against a database that isn't actually ready, deferring the real failure to the first live document read or write.
After: call `POST /admin/v1/projects` (`backend_mode=direct_pg`) with a DML-only connection string against a database missing part of the expected schema → sees a 400 response body naming exactly what's missing (e.g., `{"error":"customer_db_not_prepped","detail":"table 'documents' not found — run the embyr database-preparation step first"}`) instead of a generic `backend_unavailable`.
Decision enabled: Sam (or Elena, if told about the failure) knows immediately and specifically what to fix — re-run the prep step, point at the correct database, or escalate for a missing grant — rather than guessing from an opaque error.

#### Domain Examples
1. **Happy Path**: Sam submits provisioning for Meridian Health using the DML-only connection string Elena handed over, against the database Elena already fully prepped. Embyr verifies the schema is present and matches, and provisioning proceeds normally — Sam receives the usual `{"project_id":"meridian-prod","api_key":"..."}` response, no different from any other successful `direct_pg` provisioning today.
2. **Edge Case (not prepped at all)**: Sam accidentally submits provisioning against a fresh, completely empty database — Elena hasn't run the prep step against it yet. Embyr responds 400 with a specific complaint naming the missing schema element, not a generic `backend_unavailable`.
3. **Error/Boundary (stale version)**: Elena ran an older copy of the prep tool that applied only schema version 1 of the current 2. Embyr detects the version gap and responds 400 naming both the expected and found version, rather than allowing provisioning to succeed and failing later inside a document write.

#### UAT Scenarios (BDD)

##### Scenario: Provisioning succeeds when the submitted database is fully prepped
Given a database has already been fully prepared to the current expected schema
When Sam submits provisioning for it using a DML-only connection string
Then provisioning completes and returns the same project/API-key response shape as any other successful direct_pg provisioning

##### Scenario: Provisioning fails with a specific complaint when the database is not prepped at all
Given a fresh, empty database has never had the prep step run against it
When Sam submits provisioning for it
Then the response is a 400 naming the specific missing schema element, not a generic backend_unavailable error

##### Scenario: Provisioning fails with a specific complaint when the schema is stale
Given a database was prepped with an older schema version than currently expected
When Sam submits provisioning for it
Then the response is a 400 naming both the expected schema version and the version found

##### Scenario: A DML-only credential succeeds against a fully-prepped database without requiring elevated privilege
Given the submitted connection string carries only DML rights, and the target database is fully prepped
When Sam submits provisioning
Then provisioning succeeds without embyr's provisioning flow requiring or attempting any elevated (DDL) privilege on that connection string

##### Scenario: Existing non-DBA-gated provisioning is unaffected
Given a customer's connection string carries full (DDL-capable) privilege and the database is not yet prepped, exactly as today's default flow expects
When an operator submits provisioning for it
Then provisioning succeeds via the existing automatic-migration path, unchanged from current behavior

#### Acceptance Criteria
- [ ] AC-02-01: Provisioning with a DML-only connection string against a fully-prepped database succeeds and returns the standard `{"project_id","api_key"}` response.
- [ ] AC-02-02: Provisioning against a database with no embyr schema present fails 400, naming the specific missing schema element.
- [ ] AC-02-03: Provisioning against a database with a stale schema version fails 400, naming both expected and found version.
- [ ] AC-02-04: The "not ready" failure response is distinguishable, in content, from a plain connectivity failure to the target database.
- [ ] AC-02-05: Provisioning succeeds against a fully-prepped database without embyr's provisioning flow requiring or attempting elevated privilege on the submitted connection string.
- [ ] AC-02-06: Existing full-privilege-DSN provisioning (today's default, non-DBA-gated path) continues to succeed unchanged — no regression.

#### Outcome KPIs
See § Outcome KPIs below (KPI #1 North Star, KPI #2 leading, KPI #3 guardrail).

#### Technical Notes (Optional)
Exact verification mechanism (schema introspection query, `_sqlx_migrations` table check, a stored marker, etc.) and exact moment it runs beyond provisioning-time (connection-pool checkout, periodic drift re-check) are DESIGN's call — this story locks the observable behavior only, not the mechanism (see § Job Discovery Framing Resolution, option (c), deferred). Reference pattern: `SystemDb::probe()` (`crates/embyr-server/src/adapters/system_db.rs`), the existing hard-gate schema-verification-at-startup pattern for the system DB.

---

## Wave: DISCUSS / [REF] Outcome KPIs

### Feature: customer-db-onboarding

### Objective
Make embyr onboardable by enterprise/regulated customers whose Postgres governance prohibits granting DDL rights to third-party SaaS provisioning flows, and replace today's opaque provisioning failures with specific, actionable complaints.

### Outcome KPIs

| # | Who | Does What | By How Much | Baseline | Measured By | Type |
|---|---|---|---|---|---|---|
| 1 | Enterprise/regulated customers whose Postgres governance prohibits granting DDL to third-party SaaS | Complete embyr onboarding without ever granting embyr's stored connection string elevated schema-modification rights | 100% of DBA-gated onboarding attempts complete using a DML-only credential | 0% (this segment cannot onboard via the existing automatic-migrate path at all) | Count of successful `direct_pg` provisioning requests using a DML-only role, cross-referenced with prep-tool run logs | North Star |
| 2 | Operators (and customers) reviewing a failed provisioning attempt | Identify the specific remediation needed from the error message alone, without escalating to engineering/on-call | ≥90% of not-prepped/stale-version provisioning failures are self-resolved (re-run prep, fix DSN) within one retry, without an internal support ticket | 0% (today's `backend_unavailable` message gives no actionable detail) | Support-ticket tagging cross-referenced with provisioning-retry success | Leading |
| 3 | Existing (non-DBA-gated) `direct_pg` customers | Continue to provision successfully via the unchanged default path | 0% regression in existing provisioning success rate | Current success rate (measure pre-launch) | Provisioning success-rate dashboard, pre/post comparison | Guardrail |

### Metric Hierarchy
- **North Star**: KPI #1 — this is the entire reason the feature exists; if DBA-gated customers still can't onboard without granting elevated privilege, nothing else matters.
- **Leading Indicators**: KPI #2 — actionable-error quality predicts whether the North Star's success rate holds up in practice, not just in the happy-path demo.
- **Guardrail Metrics**: KPI #3 — a regression here (breaking today's working default flow) is the single highest-consequence defect class this feature can produce, since it would trade a narrow gap-fix for a broad regression.

### Measurement Plan

| KPI | Data Source | Collection Method | Frequency | Owner |
|---|---|---|---|---|
| 1 | Provisioning audit log + prep-tool run logs | Cross-reference query | Weekly | platform-architect (DEVOPS wave) |
| 2 | Support-ticket system + provisioning-attempt log | Correlation query | Weekly | platform-architect (DEVOPS wave) |
| 3 | Provisioning success-rate dashboard | Existing provisioning-endpoint instrumentation, pre/post comparison | Continuous | platform-architect (DEVOPS wave) |

### Hypothesis
We believe that letting a customer DBA prep their own database once, under their own elevated credentials, and having embyr verify readiness and complain specifically when it isn't ready, will unblock onboarding for DBA-privilege-gated customers who cannot use today's automatic-migrate path at all.
We will know this is true when at least one previously-blocked DBA-gated onboarding completes end-to-end using a DML-only credential, and provisioning-failure support tickets for "not prepped" states drop to near zero because the error message alone is sufficient to self-resolve.

**Note on baseline honesty**: KPI #1's baseline is 0% by definition (this segment cannot onboard today), not a measurement gap — stated as such rather than fabricated.

---

## Wave: DISCUSS / [REF] Definition of Ready Validation

### Story: US-01 and US-02 (both stories, customer-db-onboarding)

| DoR Item | Status | Evidence |
|---|---|---|
| 1. Problem statement clear, domain language | PASS | Both Elevator Pitches name the concrete "Before" state grounded directly in read code (`provision.rs`'s `sqlx::migrate!` calls) |
| 2. User/persona with specific characteristics | PASS | P6 Elena Vasquez (Customer DBA, elevated-but-scoped Postgres role, subject to a named governance policy), P2 Sam Chen (secondary, existing persona) |
| 3. 3+ domain examples with real data | PASS | Both stories have exactly 3 (Happy/Edge/Error) with real-feeling names, hosts, roles, and database names (`elena_dba`, `meridian_embyr`, `pg-prod.meridianhealth.internal`, `embyr_app`) |
| 4. UAT in Given/When/Then (3-7 scenarios) | PASS | US-01: 5 scenarios, US-02: 5 scenarios — both within 3-7 |
| 5. AC derived from UAT | PASS | Every AC traces to a named scenario (e.g., AC-01-03 ← "Re-running preparation after an interrupted partial run resumes safely") |
| 6. Right-sized (1-3 days, 3-7 scenarios) | PASS | Each story maps 1:1 to a slice, each 1.5 days (see § Elephant Carpaccio Slices); feature-level story count (2) is well within the ≤10 threshold |
| 7. Technical notes identify constraints | PASS | US-01 (migration set = `migrations/customer/`, packaging deferred to DESIGN), US-02 (verification mechanism + moment explicitly deferred to DESIGN, reference pattern named) |
| 8. Dependencies resolved or tracked | PASS | US-02 depends conceptually on US-01 (needs prepped-database states to verify against) but is independently testable against hand-seeded schema states — documented in both the story map and slice-02 brief; no circular or unresolved dependency |
| 9. Outcome KPIs defined with measurable targets | PASS | All 3 KPIs have explicit numeric targets and named measurement methods; no deferred-target gap in this feature |

### DoR Status: **PASSED** (all 9 items, both stories)

### Requirements Completeness Score: **0.97**

- Functional requirements: complete — both stories cover the full backbone (§ Story Map), both traced to JOB-15's four forces.
- Non-functional requirements: the shared-artifact integrity risk (migration-set version agreement between the standalone tool and `embyr-server`) is carried into § System Constraints as a DESIGN-scoping requirement, not fabricated as a numeric NFR before a mechanism is chosen.
- Business rules: complete — JOB-15's push/pull/anxiety/habit forces are all traced to specific ACs (e.g., anxiety → AC-01-04/AC-02-03, habit → AC-01-01's familiar-tool framing).
- One deliberate, documented gap (same class as prior features' scored-down points): § Job Discovery Framing Resolution could not be confirmed interactively (no `AskUserQuestion` tool available in this run) — resolved via explicit, auditable reasoning instead, flagged for redirect if wrong. Scored 0.97, not 1.0, for this reason.

---

## Wave: DISCUSS / [REF] Out of Scope

- **Exact CLI shape / binary name / crate name / packaging and distribution mechanism of the prep tool** — DESIGN's call; this DISCUSS locks observable behavior only (§ User Stories US-01).
- **Exact verification mechanism** (schema introspection query shape, marker table, `_sqlx_migrations` check, etc.) — DESIGN's call (§ User Stories US-02 Technical Notes).
- **When the verify check runs beyond provisioning-time** (connection-pool checkout, periodic background re-check / drift detection) — explicitly deferred. This is Job Discovery Framing option (c) from § Framing Resolution: a materially different mechanism and moment from this feature's locked (a) framing. Flagged as a strong candidate follow-up feature, not built here.
- **Self-serve, operator-mediation-free onboarding flow** (Framing option (b)) — noted as a natural side-benefit of (a)'s design (the standalone tool *can* be run without going through the admin API first) but no story in this feature builds a self-serve provisioning UI or flow.
- **`aws_secret` / `gcp_secret` backend modes** — both share the identical underlying privilege-separation gap (both branches in `provision.rs` also call `sqlx::migrate!` against the fetched DSN), but are deferred as follow-up scope to keep this feature right-sized (§ Scope Assessment: PASS at 2 stories). DESIGN should design the verification mechanism in a way that naturally generalizes to these modes as follow-up work, but is not required to build them now.
- **`agent` backend mode** — not applicable; `embyr-agent` already self-migrates its own local customer DB at its own startup (AD-A08), using credentials that never leave the customer's VPC in the first place; the privilege-separation problem this feature solves does not exist for agent mode.
- **Ongoing schema-drift detection after successful provisioning** — out of scope, see Framing option (c) above.

---

## Wave: DISCUSS / [REF] WS Strategy

Walking Skeleton Strategy: **B — Thin End-to-End Slice**. Both slices are real, narrow vertical slices against real Postgres (no facade, no mock) — Slice 01 proves the riskiest new assumption (a standalone tool reliably reproduces `embyr-server`'s own migration behavior); Slice 02 proves the second riskiest assumption (a DML-only connection string can be verified as ready without needing elevated privilege to check it). Together they form the thinnest end-to-end flow: prep → verify → provision succeeds.

---

## Wave: DISCUSS / [REF] Driving Ports

- `POST /admin/v1/projects` (existing, `backend_mode=direct_pg` branch — behavior extended with a pre-migration verification step, not a new endpoint).
- A new customer-operated CLI entry point (exact command shape TBD by DESIGN) — run by the customer's DBA, outside any embyr-hosted process; this is a new driving port at the system-context level even though it is not a network-facing API.

---

## Wave: DISCUSS / [REF] Pre-requisites

- `crates/embyr-server/src/admin/handlers/provision.rs` (existing `direct_pg`/`aws_secret`/`gcp_secret` branches — precedent for, and point of extension of, the automatic-migration behavior this feature adds a pre-check to).
- `crates/embyr-server/src/adapters/system_db.rs`'s `SystemDb::probe()` (existing hard-gate schema-verification-at-startup pattern for the system DB — the reference pattern for this feature's customer-DB analogue).
- `migrations/customer/` (`0001_documents.sql`, `0002_transactions.sql` — the exact migration set both the prep tool and the verify step must agree on).
- `docs/product/architecture/brief.md` AD-A08 (`embyr-agent`'s own precedent for a binary applying `migrations/customer/` outside `embyr-server`'s own process — closest existing precedent, different actor/trigger).
- JOB-13's "exit 1 on migration failure, no traffic until DB probe succeeds" pattern (`docs/product/jobs.yaml`) — the UX/emotional precedent for what "embyr complains" should feel like: named, specific errors, not cryptic failures.

---

## Wave: DISCUSS / [REF] Handoff Package

**To DESIGN (solution-architect)**: this `feature-delta.md` (journey + story map + user stories + embedded AC), 2 slice briefs (`docs/feature/customer-db-onboarding/slices/slice-01-db-prep-binary.md`, `slice-02-provisioning-verify-and-complain.md`), `docs/product/jobs.yaml` (JOB-15, new; JOB-02 cross-reference note), `docs/product/journeys/customer-dba.yaml` (new pointer file).

**To DEVOPS (platform-architect)**: § Outcome KPIs above (3 KPIs — 1 North Star, 1 leading, 1 guardrail — for instrumentation planning).

**Explicit flags for DESIGN**:
1. § Job Discovery Framing Resolution's option (a) is the locked framing — DESIGN should not silently reinterpret this as self-serve trial (b) or drift detection (c); those are documented follow-up candidates, not this feature's scope.
2. § System Constraints' shared-artifact integrity risk (migration-set version agreement between the standalone tool and `embyr-server`) is the single highest-consequence design risk in this feature — resolve explicitly, do not leave the two consumers of `migrations/customer/` free to drift independently.
3. Verification (US-02) must work without requiring elevated privilege on the submitted connection string — confirm any chosen mechanism (e.g., `information_schema` queries) is actually available to an ordinary DML-only role before committing to it.
4. `aws_secret`/`gcp_secret` generalization (§ Out of Scope) is a "design so it naturally extends," not a "build now" instruction — do not silently expand scope to cover them in this feature's DESIGN artifacts.

Peer review: not invoked per-wave (default skip per SKILL Phase 3 step 6 — the one genuine ambiguity, the Job Discovery framing, is resolved with fully explicit and auditable reasoning above rather than hand-waved, and flagged clearly for redirect; no JTBD assumptions inherited from elsewhere requiring re-validation; no vendor-neutrality risk). Mandatory consolidated review fires at end of DISTILL.

---

## Wave: DISCUSS / [REF] SSOT Updates

- `docs/product/jobs.yaml` — added JOB-15 (`customer-db-preflight`, P6 Elena Vasquez, secondary P2). JOB-02 receives a cross-reference note (not a rewrite) pointing to this feature, mirroring the project's established cross-reference pattern (e.g., JOB-14 → JOB-10).
- `docs/product/journeys/customer-dba.yaml` — new minimal pointer file, matching the existing `tenant-admin.yaml`/`service-operator.yaml` format (persona, jobs, source pointer only — no separate visual/YAML journey artifacts produced, per the lean single-narrative-file convention).
- No new persona file — Elena (P6) is documented inline in `jobs.yaml` only, consistent with the P1-P4 precedent (only P5 has a dedicated file, as a persistent dashboard-facing user; Elena is a narrow, single-touchpoint persona like P1-P4).

---

## Wave: DESIGN / [REF] Prior Wave Consultation — Reading Confirmation

✓ `docs/product/architecture/brief.md` (§ System Architecture — System Quality Attributes, System Constraints, Process Topology, Failure Modes/Substrate Probes, C4 diagrams, System-Level Decisions Table; § Domain Model — Bounded Contexts, Ubiquitous Language, Aggregates — read in full for the "provision"/"customer DB"/"backend_mode"/"SystemDb" context and prior architects' section-ownership convention. No prior `## Application Architecture` section existed for this feature — this DESIGN wave is the first to write one.)
✓ `docs/product/architecture/adr-001` through `adr-021` (file listing read; ADR-008 crate-structure and ADR-017 production-startup read in full as the closest structural precedents for a new workspace crate and for `main.rs`-shaped startup sequencing)
✓ `docs/product/journeys/customer-dba.yaml` (pointer file, confirms JOB-15/P6 scope)
✓ `docs/feature/customer-db-onboarding/feature-delta.md` (this file, DISCUSS section, full — 460 lines)
✓ `crates/embyr-server/src/admin/handlers/provision.rs` (365 lines, read in full — confirmed all three non-agent backend-mode branches independently embed `sqlx::migrate!("../../migrations/customer")`)
✓ `crates/embyr-server/src/adapters/system_db.rs` (full — `SystemDb::probe()` hard-gate pattern, precedent for `verify_schema_readiness()`)
✓ `crates/embyr-pg-storage/src/backend_adapter.rs`, `Cargo.toml` (full — found `PostgresBackendAdapter::migrate()`/`run_migrations()` already wrap the identical macro call, currently used only by test harnesses per their own doc comments — this is the single most consequential Reuse Analysis finding of this wave)
✓ `crates/embyr-agent/src/{main,probe,config,server}.rs`, `Cargo.toml` (full — confirmed the customer-run-standalone-binary precedent shape; also confirmed, contra DISCUSS's stated assumption, that `embyr-agent`'s production `run()` path never calls `migrate()`/`run_migrations()` — see § Contradiction Check)
✓ `migrations/customer/0001_documents.sql`, `0002_transactions.sql` (full — confirmed additive, one-table-per-migration shape, informs ADR-023's versioning mechanism choice)
✓ Root `Cargo.toml` (workspace members, dependency versions — confirmed `sqlx` migrate feature already enabled, no new external dependency needed)
✓ `crates/embyr-core/src/error.rs` (full — confirmed existing `CoreError` variants suffice, no enum growth needed)
✓ `crates/embyr-server/Cargo.toml` (confirmed `embyr-pg-storage` already a dependency)

Interaction mode: **Propose** (Decision 1, per orchestrator). Design scope: **Application/components**
(Decision 0, per orchestrator — sole architect, no system-designer/ddd-architect dispatch). Paradigm:
unchanged (functional-where-practical Rust, project `CLAUDE.md`) — no paradigm re-selection needed.

**One contradiction found and resolved** — see § Contradiction Check below (AD-A08 factual
correction). It does not change this feature's locked scope (agent mode remains out of scope, on
corrected grounds) and does not require escalation back to DISCUSS.

---

## Wave: DESIGN / [REF] Contradiction Check — AD-A08 Correction

DISCUSS's § Out of Scope states `embyr-agent` "already self-migrates its own local customer DB at
its own startup (AD-A08)." Reading `crates/embyr-agent/src/{main,probe,server}.rs` in full found
this inaccurate: `embyr-agent`'s production `server::run()` connects a pool and starts the mTLS
gRPC server directly — it never calls `PostgresBackendAdapter::migrate()`/`run_migrations()`
(which exist in `embyr-pg-storage` but are documented as test-harness-only today).

This does **not** reopen agent mode's exclusion from this feature — the exclusion holds on
independent grounds (agent-mode credentials never leave the customer's VPC, so the
cross-tenant-privilege-separation-from-embyr's-SaaS problem this feature solves is moot for agent
mode regardless of how its schema gets applied). The correction is recorded as OQ-2 (a documentation
/ follow-up flag), not a scope change. Full detail: `docs/product/architecture/brief.md`
§ Application Architecture — customer-db-onboarding § Contradiction Check.

---

## Wave: DESIGN / [REF] DDD List — customer-db-onboarding

| ID | Design Decision | Verdict | One-line Rationale |
|----|------------------|---------|----------------------|
| DDD-1 | Single embed point for `migrations/customer/` via `PostgresBackendAdapter::migrate()`, reused by both `provision.rs` and the new prep binary | Accepted — ADR-022 | Collapses 2 pre-existing independent embeds + prevents a 3rd; resolves DISCUSS's #1 flagged risk structurally, not by convention |
| DDD-2 | New workspace crate `embyr-db-prep` for the customer-run prep tool, not a `[[bin]]` inside `embyr-agent` or `embyr-server` | Accepted — ADR-022 | Supply-chain minimization for a customer-run binary; mirrors SD-09's existing rationale for `embyr-agent` itself |
| DDD-3 | Schema-readiness verification reuses sqlx's own `_sqlx_migrations` bookkeeping table, not a bespoke marker table | Accepted — ADR-023 | Avoids a second, parallel, drift-prone bookkeeping mechanism |
| DDD-4 | `verify_schema_readiness()` enriches the existing migrate-attempt failure path; it does not replace or gate today's automatic migrate call | Accepted — ADR-023 | The only way to preserve AC-02-06 (no regression) — read-only introspection alone cannot determine whether a credential has DDL rights |
| DDD-5 | `found_version >= expected_version` (not strict equality) counts as `Ready` | Accepted, flagged OQ-1 | Forward-compatible during rolling deploys; deferred risk for a hypothetical future non-additive migration |
| DDD-6 | No new `CoreError` variants | Accepted | Existing `PermissionDenied`/`BackendUnavailable` free-text-payload variants already cover the needed distinctions |
| DDD-7 | **Revised (post-review):** `_sqlx_migrations` read access is a runtime, role-scoped `GRANT` (target role self-discovered via a brief secondary connection + `SELECT current_user`), executed only by `embyr-db-prep` — not `GRANT ... TO PUBLIC`, and not a static migration file | Accepted — ADR-023 revision, supersedes original DDD-3 mechanism detail | A targeted security review of ADR-023 approved it overall but flagged the `PUBLIC` grant as avoidably broad for a privilege-separation feature; role-scoped grant achieves the same zero-manual-typing property via runtime self-discovery |

---

## Wave: DESIGN / [REF] Component Decomposition

See `docs/product/architecture/brief.md` § Application Architecture — customer-db-onboarding
§ Component Decomposition for the full file-path table (5 new files, 4 extended files, 1 new
crate, 0 new migrations — revised, see below). Summary by area:

- **New crate (customer-run):** `crates/embyr-db-prep/` — `[[bin]] embyr-db-prep`. Depends only on
  `embyr-pg-storage`, `embyr-core`, `sqlx`, `tokio`. `main.rs` (wire→probe→use), `config.rs`
  (`DbPrepConfig::from_env()`, mirrors `AgentConfig`; **revised** to add an optional
  `EMBYR_DB_PREP_DML_ROLE_DSN`), `error_report.rs` (SQLSTATE-based message classification).
- **Shared adapter (embyr-pg-storage, extended):** `backend_adapter.rs` gains
  `verify_schema_readiness()`; `migrate()` (already existing) becomes the sole embed point used by
  both `embyr-server` and `embyr-db-prep`; **revised** to also add `discover_current_user()` and
  `grant_schema_readiness_read(role_name)` — the runtime, role-scoped grant mechanism.
- **Domain (embyr-core, IO-free, new):** `domain/schema_readiness.rs` — pure `SchemaReadiness`
  enum (`Ready`/`NotPrepped`/`Stale`).
- **HTTP surface (embyr-server, extended):** `provision.rs`'s `direct_pg` branch calls
  `verify_schema_readiness()` before the existing migrate attempt; all 3 branches switch from
  inline `sqlx::migrate!` to `PostgresBackendAdapter::migrate()`. No grant-related change — the
  grant step lives exclusively in `embyr-db-prep`.
- **Schema (revised — removed):** the originally-proposed `migrations/customer/0003_grant_schema_readiness_read.sql`
  (static `GRANT ... TO PUBLIC`) is removed from this design. A role-parameterized grant cannot be
  expressed as static, unparameterized migration SQL — the target role name isn't known at
  migration-authoring time. The grant is now a runtime step in `embyr-db-prep` only, never a
  tracked migration. `expected_version` remains `2` (`0001`, `0002` only).
- **Workspace plumbing:** root `Cargo.toml` (new member), `deny.toml` (register `embyr-db-prep`).

**Revision note (post-DESIGN-wave security review):** the original Component Decomposition above
proposed a static migration granting `SELECT` on `_sqlx_migrations` to `PUBLIC`. A targeted security
review of ADR-023 approved the design overall but flagged this as an avoidably broad grant for a
feature whose entire framing is least-privilege separation for regulated customers. Revised to a
role-parameterized runtime grant. Full mechanism: `docs/product/architecture/adr-023-schema-readiness-verification.md`
(revised) and `docs/product/architecture/brief.md` § Driven Ports + Adapters — customer-db-onboarding.

---

## Wave: DESIGN / [REF] Driving Ports

| Port | Auth/Trigger | Handler |
|------|---------------|---------|
| `embyr-db-prep` CLI process (**new**) | Run by Elena under her own elevated, database-scoped role; config via `EMBYR_DB_PREP_DSN` (required) + `EMBYR_DB_PREP_DML_ROLE_DSN` (optional, **revised** per ADR-023 — enables the role-scoped grant) | `crates/embyr-db-prep/src/main.rs` |
| `POST /admin/v1/projects` (existing, `direct_pg` branch extended) | Operator Bearer, unchanged | `provision.rs::provision` |

No new `:8080`/`:8081` driving port — confirmed unchanged from DISCUSS's own Driving Ports section.

---

## Wave: DESIGN / [REF] Driven Ports + Adapters

| Port/Adapter | Shape | Earned Trust |
|---|---|---|
| `PostgresBackendAdapter::migrate()` | Existing method, now the sole embed point for `migrations/customer/` (ADR-022) | Reused as-is; connectivity is probed by both callers *before* this is invoked (`probe_customer_db()` in `provision.rs`, an equivalent connect+`SELECT 1` in `embyr-db-prep`). |
| `PostgresBackendAdapter::verify_schema_readiness()` (new) | Read-only, `SELECT`-only against `_sqlx_migrations` | This method *is* `embyr-server`'s probe of the customer DB's claimed schema state — mirrors `SystemDb::probe()`'s hard-gate shape, run on every `direct_pg` provisioning request. 4 fault-injection scenarios (connection failure, insufficient privilege, interrupted-run resume, DML-role DSN unreachable) detailed in brief.md. |
| `PostgresBackendAdapter::discover_current_user()` + `grant_schema_readiness_read(role_name)` (new, **revised** per security review) | `discover_current_user()`: brief, short-lived connection using the DML role's own credential, runs `SELECT current_user` only, never logs the DSN. `grant_schema_readiness_read()`: executed against the elevated connection; role-name interpolation delegated to Postgres's own `format('%I', ...)`, not hand-rolled Rust quoting. | Together, these replace the originally-proposed static `GRANT ... TO PUBLIC` migration with a runtime, role-scoped grant — `embyr-db-prep`-only, optional (skipped gracefully if `EMBYR_DB_PREP_DML_ROLE_DSN` absent or unreachable). See ADR-023 (revised) § Mechanism. |

Full signatures, the `SchemaReadiness` enum, and the complete Earned Trust probe table:
`docs/product/architecture/brief.md` § Application Architecture — customer-db-onboarding
§ Driven Ports + Adapters.

**External Integrations Requiring Contract Tests:** none. No new external SaaS/API integration —
the customer's own Postgres is already a first-class integration point in the existing
architecture, not a new dependency class.

---

## Wave: DESIGN / [REF] Technology Choices

| Choice | Verdict | Rationale (one-line) |
|---|---|---|
| New crate `embyr-db-prep`, zero new external dependencies | Accepted — ADR-022 | `sqlx`, `tokio` already workspace deps; reuses `embyr-pg-storage`'s existing `migrate()` |
| sqlx's own `_sqlx_migrations` table as the versioning source of truth | Accepted — ADR-023 | Avoids a second, independently-maintained bookkeeping table |
| Runtime, role-scoped `GRANT SELECT ON _sqlx_migrations` (revised — not `PUBLIC`, not a static migration) | Accepted — ADR-023 (revised) | Security review: a privilege-separation feature should not ship an avoidably broad `PUBLIC` grant; role-scoped grant costs one optional prep-tool input, achieves identical zero-manual-typing property via `SELECT current_user` self-discovery |
| Postgres `format('%I', ...)` for GRANT identifier interpolation (not hand-rolled Rust quoting) | Accepted — ADR-023 (revised) | Delegates identifier-escaping correctness to the database engine's own trusted implementation; closes a SQL-injection-shaped risk from naive string interpolation |

---

## Wave: DESIGN / [REF] Decisions Table

See `docs/product/architecture/brief.md` § Application-Level Decisions Table — customer-db-onboarding
(CDO-AD-01 through CDO-AD-08) for the full table with rationale. Reproduced IDs: CDO-AD-01
(single migration embed), CDO-AD-02 (new `embyr-db-prep` crate), CDO-AD-03 (`_sqlx_migrations`-based
verification), CDO-AD-04 (enrich, don't replace, the migrate-attempt failure path), CDO-AD-05
(`>=` version comparison), CDO-AD-06 (domain/adapter layering), CDO-AD-07 (no new `CoreError`
variants), **CDO-AD-08 (revised — role-scoped runtime `GRANT`, not `PUBLIC`/not a static migration;
supersedes the original PUBLIC-grant proposal per targeted security review of ADR-023)**.

---

## Wave: DESIGN / [REF] Reuse Analysis

Full table (6 rows, 0 unjustified CREATE NEW): `docs/product/architecture/brief.md`
§ Reuse Analysis — customer-db-onboarding (hard gate). Verdict summary: **5 EXTEND, 1 CREATE NEW
(new `embyr-db-prep` crate — extensively justified against 2 rejected in-place alternatives, see
ADR-022), 0 unjustified CREATE NEW.** The single most consequential finding: `embyr-pg-storage`'s
`PostgresBackendAdapter::migrate()` already existed before this feature (test-harness-only) and
becomes, by this wave's decision, the sole production embed point for `migrations/customer/`
workspace-wide.

---

## Wave: DESIGN / [REF] Open Questions

| ID | Question | Blocking | Timing |
|----|----------|---------|--------|
| OQ-1 | `found_version >= expected_version` could mask a future non-additive (breaking) migration | No | Follow-up, if/when a non-additive migration is introduced |
| OQ-2 | AD-A08 factual correction: `embyr-agent` does not actually self-migrate at startup; agent-mode schema-provisioning mechanism is undocumented (exclusion from this feature still holds on other grounds) | No | Recommend a follow-up feature/fix |
| OQ-3 | `aws_secret`/`gcp_secret` generalization (deferred per DISCUSS) — design confirmed to generalize with zero new adapter code | No | Follow-up feature, if/when scoped |
| OQ-4 | **Resolved.** A targeted security review of ADR-023 was invoked and came back APPROVED overall, with one non-blocking improvement (the `PUBLIC`-vs-role-scoped grant question) — adopted as CDO-AD-08/DDD-7. No further review action needed on this point. | Resolved | Closed this wave |
| OQ-5 | The revised role-scoped grant introduces an ordering dependency: if Elena runs `embyr-db-prep` without `EMBYR_DB_PREP_DML_ROLE_DSN` (e.g., before the `embyr_app` role exists yet), `verify_schema_readiness()` reports `NotPrepped` at provisioning time even though the schema itself is fully applied, until the tool is re-run (idempotently) with the DML-role DSN supplied | No (documented trade-off, not a defect) | DISTILL should write an explicit UAT scenario for this sequencing case |

---

## Wave: DESIGN / [REF] C4 Diagrams

Full C4 System Context and Container diagrams live in `docs/product/architecture/brief.md`
§ Application Architecture — customer-db-onboarding (not duplicated here, per the multi-architect
SSOT convention already established by prior features' own DESIGN sections). Both diagrams were
revised post-review to reflect the role-scoped (not `PUBLIC`) grant mechanism — see ADR-023.

---

## Wave: DESIGN / [REF] Outcome Collision Check

Skipped — `nwave-ai outcomes check-delta` CLI is not available in this execution environment
(no Bash tool in this agent's toolset). Noted per SKILL's Outcome Collision Check skip-and-document
path, consistent with how the prior `card-payments-backend` DESIGN wave handled the same gap.

---

## Wave: DESIGN / [REF] Peer Review

**Invoked, targeted scope: ADR-023 only.** This DESIGN wave initially flagged ADR-023's
`GRANT SELECT ON _sqlx_migrations TO PUBLIC` as a plausible match for the "security boundary
change" per-wave review trigger (OQ-4, now resolved) rather than invoking a full-document review.
The orchestrator subsequently ran a targeted security review of ADR-023.

**Outcome: APPROVED overall**, with one non-blocking, adopted improvement: replace the `PUBLIC`
grant with a role-parameterized grant (target role self-discovered at runtime via
`SELECT current_user` against the DML role's own credential, granted by the elevated connection
only). Rationale: this feature's entire framing is privilege separation for enterprise/regulated
customers under least-privilege governance — an avoidably broad `PUBLIC` grant would undercut the
feature's own selling point under a customer security audit, when a role-scoped grant costs no
more in engineering effort and achieves the identical zero-manual-role-typing property via runtime
self-discovery.

**Revision applied this wave:** ADR-023, `docs/product/architecture/brief.md` § Application
Architecture — customer-db-onboarding (Component Decomposition, Driven Ports + Adapters, both C4
diagrams, Architecture Enforcement, Application-Level Decisions Table, Open Questions), and this
file's own DESIGN section (DDD-7, Component Decomposition, Driving/Driven Ports, Technology
Choices, Decisions Table, Open Questions) were all updated to the role-scoped mechanism. Full
before/after and rejected-alternative rationale: ADR-023 § Alternatives Considered, Alternative 4.

The mandatory consolidated review at end of DISTILL still covers the feature as a whole; this
targeted review closes out the one specific concern it was scoped to.

---

## Wave: DESIGN / [REF] Handoff Package

**To DISTILL (acceptance-designer):** this `feature-delta.md` (DISCUSS + DESIGN sections),
`docs/product/architecture/brief.md` § Application Architecture — customer-db-onboarding,
`docs/product/architecture/adr-022-customer-db-prep-crate-and-migration-consolidation.md`,
`docs/product/architecture/adr-023-schema-readiness-verification.md`.

**Explicit flags for DISTILL/acceptance-designer:**
1. US-01's UAT scenarios map directly onto `embyr-db-prep`'s 3 named failure classes (connection
   failure, insufficient privilege, generic) plus 2 success shapes (fresh apply, already-current
   no-op) — acceptance tests should assert on the specific message content and exit code, not just
   "non-zero on failure."
2. US-02's "not prepped" vs. "stale version" distinction is now backed by a concrete mechanism
   (`SchemaReadiness::NotPrepped`/`Stale`, § Driven Ports + Adapters) — acceptance tests should
   seed real Postgres into exactly the 3 states (`Ready` from a full prep run, `NotPrepped` from an
   empty DB, `Stale` from applying only migration `0001`) rather than mocking the check.
3. AC-02-06's regression scenario (existing full-privilege-DSN customer, database not yet migrated)
   must still exercise the real automatic-migrate path — DDD-4/CDO-AD-04 is only satisfied if this
   scenario is tested end-to-end against a genuinely un-prepped database with a genuinely
   DDL-capable role, confirming migrate is still attempted and still succeeds.
4. **Revised** (post-review) — the runtime, role-scoped `GRANT SELECT ON _sqlx_migrations` (not
   `PUBLIC`, not a migration file — ADR-023 revision) should have acceptance scenarios confirming:
   (a) the granted DML role (e.g., `embyr_app`) really can read `_sqlx_migrations` after a prep run
   that supplied `EMBYR_DB_PREP_DML_ROLE_DSN`, with no manual grant step from the test fixture; (b)
   a *different*, non-granted role cannot read it (the negative test the security review
   specifically called for — this is the test that would have caught a `PUBLIC`-grant regression);
   (c) a prep run *without* `EMBYR_DB_PREP_DML_ROLE_DSN` still reports migration success normally,
   prints the informational grant-skipped note, and leaves `verify_schema_readiness()` reporting
   `NotPrepped` until a follow-up prep run supplies the DSN (OQ-5's sequencing scenario).

---

## Wave: DISTILL / [REF] Prior Wave Consultation — Reading Confirmation

+ `docs/product/architecture/brief.md` § Application Architecture — customer-db-onboarding (lines
  3067-3315: Contradiction Check, Quality Attribute Priorities, Reuse Analysis, Component
  Decomposition, Driving/Driven Ports, C4 diagrams, Architecture Enforcement, Application-Level
  Decisions Table, Open Questions)
+ `docs/product/architecture/adr-022-customer-db-prep-crate-and-migration-consolidation.md`
+ `docs/product/architecture/adr-023-schema-readiness-verification.md` (current/revised version
  confirmed — role-scoped `GRANT` via `discover_current_user()`/`grant_schema_readiness_read()`, not
  the original `GRANT ... TO PUBLIC`)
+ `docs/feature/customer-db-onboarding/feature-delta.md` (full — DISCUSS + DESIGN sections, 706
  lines, this file)
+ `docs/architecture/atdd-infrastructure-policy.md` (pre-read by orchestrator; `embyr-db-prep`
  subprocess row appended this wave — see § Test Infrastructure below)
+ `docs/feature/customer-db-onboarding/slices/slice-01-db-prep-binary.md`
+ `docs/feature/customer-db-onboarding/slices/slice-02-provisioning-verify-and-complain.md`
+ `docs/product/architecture/adr-001` through `adr-021` — NOT re-read this wave (already read in
  full by DESIGN per its own reading confirmation at line 465; no new ADRs beyond 022/023 relevant
  to this feature)
- `docs/feature/customer-db-onboarding/discuss/wave-decisions.md`,
  `docs/feature/customer-db-onboarding/design/wave-decisions.md`,
  `docs/feature/customer-db-onboarding/devops/wave-decisions.md` (not found — this project uses the
  unified single-file `feature-delta.md` model; DISCUSS/DESIGN content lives entirely as
  `## Wave: DISCUSS` / `## Wave: DESIGN` sections in this file, confirmed read above)
- `docs/feature/customer-db-onboarding/devops/` (directory absent — graceful degradation: WARN,
  default environment matrix applied: clean | with-pre-commit | with-stale-config)
- `docs/product/journeys/customer-dba.yaml` — not re-read this wave (DESIGN already confirmed its
  scope-pointer content; no embedded Gherkin beyond what feature-delta.md's own UAT scenarios
  already capture)
- `docs/product/kpi-contracts.yaml` — not found at the expected path for this project; feature-level
  Outcome KPIs (§ Wave: DISCUSS / Outcome KPIs) used as the substitute KPI source. Soft gate — no
  `@kpi`-tagged scenarios added this wave (KPI #1/#2/#3 are all measured via provisioning/prep-tool
  run-log cross-reference queries owned by DEVOPS, not via an emittable-event assertion inside an
  acceptance scenario — noted, not fabricated).

## Wave: DISTILL / [REF] Wave-Decision Reconciliation

**Result: PASSED — 0 contradictions.**

This project's unified `feature-delta.md` model means DISCUSS/DESIGN reconciliation is performed
directly against the `## Wave: DISCUSS` / `## Wave: DESIGN` sections above (no separate
`wave-decisions.md` files exist for this feature). Checked:

- Scope boundary (`direct_pg` only, `aws_secret`/`gcp_secret`/`agent` excluded) — DISCUSS locks it,
  DESIGN confirms unchanged. No contradiction.
- Verification-without-elevated-privilege constraint (DISCUSS § System Constraints) — DESIGN's
  `verify_schema_readiness()` is `SELECT`-only against `_sqlx_migrations`; the role-scoped `GRANT`
  runs over a *separate* elevated connection inside `embyr-db-prep`, never over the submitted
  DML-only DSN. Consistent.
- Migration-set single-sourcing (DISCUSS's #1 named risk) — DESIGN's ADR-022 resolves it
  structurally. Consistent.
- AD-A08 factual correction (`embyr-agent` does not self-migrate at startup) — DESIGN corrected
  DISCUSS's stated *reason* for excluding agent mode; the scope conclusion (agent mode excluded) is
  unchanged, and independent grounds (VPC-bound credentials) still hold. Already resolved as OQ-2 in
  DESIGN; not a reopened contradiction, per this wave's own instruction not to re-litigate it.
- No DEVOPS section/directory exists — graceful degradation applies (WARN, default environment
  matrix), not a contradiction.

## Wave: DISTILL / [REF] Two-Tier Acceptance Decision

**Tier A only. Tier B (state-machine PBT) explicitly skipped.**

Both slices are config-shaped per Mandate 10's own skip criterion: US-01 is a one-shot CLI
(connection string in, exit code + message out — a single-shot installer/migration tool, the
textbook "config-shaped feature" example); US-02 is a schema-validation gate inserted into an
existing endpoint (three possible states in, one of three response shapes out). Neither has a
domain-rich input space (no emails, dates, free-text, or large ID spaces — inputs are DSNs, role
names, and small enums). Journeys are 1-3 chained scenarios each (cdo01→cdo02→cdo03 for US-01's
resume chain; cdo12→cdo13/14 for US-02's verify chain), not the ≥3-chained-AND-domain-rich
combination Mandate 10 requires before Tier B pays for itself.

## Wave: DISTILL / [REF] Walking Skeleton Strategy

Per the Architecture of Reference (driving ports get real adapters; driven-internal gets real
adapters via the Project Infrastructure Policy's mechanism), and per DISCUSS's own explicit framing
("exactly Slice 01 + Slice 02's respective happy-path scenarios — no facade, no mock; both slices
use real Postgres"): **two `@walking_skeleton`-tagged scenarios**, one per slice/driving-port pair —
`cdo01_first_time_preparation` (US-01, subprocess driving port) and
`cdo12_provisioning_succeeds_when_database_ready` (US-02, HTTP driving port). This reconciles
test-design-mandates' "2-3 walking skeletons per feature" allowance with nw-distill's retired
single-WS framing (superseded language) and with the practical constraint that US-01/US-02 are
different driving-port classes requiring different `[[test]]` crate ownership (see § Test
Infrastructure) — a single literal test function spanning both is not cleanly expressible without
cross-package `CARGO_BIN_EXE_*` resolution issues, and DISCUSS's own Story Map already frames them
as two paired happy-path scenarios, not one. Both use the production composition root (real
`embyr-db-prep`/`embyr-server` subprocess, real testcontainers Postgres) — no fakes anywhere in this
feature's scope.

## Wave: DISTILL / [REF] Scenario List

18 scenarios total (2 walking skeletons + 16 focused; feature-appropriate for a 2-slice, ~3-day
feature per DISCUSS's own sizing — below the 15-20-focused-scenario range quoted for larger
features, matching this feature's Elephant Carpaccio scope).

### US-01 — `embyr-db-prep` binary (11 scenarios, `crates/embyr-db-prep` `[[test]]` targets)

| # | Scenario | Tags | AC |
|---|---|---|---|
| cdo01 | First-time preparation succeeds and reports the applied schema version | `@walking_skeleton @driving_port @real-io @US-01` | AC-01-01 |
| cdo02 | Re-running preparation on an already-current database is a safe no-op | `@driving_port @real-io @US-01` | AC-01-02 |
| cdo03 | Re-running preparation after an interrupted partial run resumes safely | `@driving_port @real-io @US-01` | AC-01-03 |
| cdo04 | Insufficient privilege is reported with actionable detail | `@error @driving_port @real-io @US-01` | AC-01-04 |
| cdo05 | An unreachable database reports a connection failure, distinct from privilege | `@error @driving_port @real-io @US-01` | AC-01-05 |
| cdo06 | `migrations/customer/` has exactly one embed point workspace-wide (ADR-022) | `@real-io @US-01 @US-02` | (architecture enforcement) |
| cdo07 | DML role granted read access after a prep run supplies its DSN (ADR-023) | `@real-io @US-01` | (enforcement) |
| cdo08 | A different, non-granted role cannot read `_sqlx_migrations` (PUBLIC-grant regression test) | `@error @real-io @US-01` | (enforcement — security-review-specific) |
| cdo09 | Grant is optional; a follow-up run closes the OQ-5 sequencing gap | `@real-io @US-01` | (OQ-5) |
| cdo10 | Full flow (migrate + discover + grant) is idempotent on re-run | `@real-io @US-01` | (enforcement) |
| cdo11 | Reserved-word role name is quoted safely through the grant (injection-safety) | `@error @real-io @US-01` | (enforcement) |

### US-02 — provisioning verify-and-complain (7 scenarios, `crates/embyr-server` `[[test]]` targets)

| # | Scenario | Tags | AC |
|---|---|---|---|
| cdo12 | Provisioning succeeds when the submitted database is fully prepped | `@walking_skeleton @driving_port @real-io @US-02` | AC-02-01 |
| cdo13 | Provisioning fails with a specific complaint when not prepped at all | `@error @driving_port @real-io @US-02` | AC-02-02 |
| cdo14 | Provisioning fails with a specific complaint when the schema is stale | `@error @driving_port @real-io @US-02` | AC-02-03 |
| cdo15 | Not-ready response is distinguishable from a connectivity failure | `@error @driving_port @real-io @US-02` | AC-02-04 |
| cdo16 | DML-only credential succeeds without ever attempting elevated privilege | `@driving_port @real-io @US-02` | AC-02-05 |
| cdo17 | Regression guardrail: full-privilege DSN still auto-migrates unchanged | `@driving_port @real-io @US-02` | AC-02-06 |
| cdo18 | Forward-compatible: `found_version > expected_version` is still `Ready` (OQ-1/CDO-AD-05) | `@driving_port @real-io @US-02` | (OQ-1) |

Error/edge ratio: 12 of 18 scenarios are error/edge/structural-invariant (cdo02, cdo03, cdo04, cdo05,
cdo06, cdo08, cdo09, cdo11, cdo13, cdo14, cdo15, cdo18) vs. 6 happy-path (cdo01, cdo07, cdo10, cdo12,
cdo16, cdo17) — **66.7%**, well above the 40% floor.

## Wave: DISTILL / [REF] Adapter Coverage Table

| Adapter | `@real-io` scenario | Covered by |
|---|---|---|
| `PostgresBackendAdapter::migrate()` (existing, now the sole call site per ADR-022) | YES | cdo01, cdo02, cdo03, cdo12, cdo16, cdo17, cdo18 (seeding) — real testcontainers Postgres throughout |
| `PostgresBackendAdapter::verify_schema_readiness()` (new) — `Ready` variant | YES | cdo12, cdo16, cdo17 (post-migrate), cdo18 — real testcontainers Postgres |
| `PostgresBackendAdapter::verify_schema_readiness()` (new) — `NotPrepped` variant | YES | cdo09 (step 2), cdo13 — real testcontainers Postgres, fresh/empty DB |
| `PostgresBackendAdapter::verify_schema_readiness()` (new) — `Stale` variant | YES | cdo14 — real testcontainers Postgres, partially-migrated DB via `sqlx::Migrate` trait |
| `PostgresBackendAdapter::discover_current_user()` (new) | YES | cdo07, cdo08, cdo09, cdo10, cdo11 — real testcontainers Postgres, real DML-role DSN |
| `PostgresBackendAdapter::grant_schema_readiness_read()` (new) | YES | cdo07 (positive), cdo08 (negative — the security-review-specific test), cdo10 (idempotent), cdo11 (injection-safety) — real testcontainers Postgres with an actually-DML-restricted role (no `CREATE`), never a superuser |
| `embyr-db-prep` binary (subprocess, US-01) — with `EMBYR_DB_PREP_DML_ROLE_DSN` set | YES | cdo07, cdo08, cdo09 (step 3), cdo10, cdo11 |
| `embyr-db-prep` binary (subprocess, US-01) — without `EMBYR_DB_PREP_DML_ROLE_DSN` (optional-DSN-absent skip path) | YES | cdo01, cdo02, cdo03, cdo04, cdo05, cdo09 (step 1) |
| `provision.rs`'s `direct_pg` branch (extended) | YES | cdo12-cdo18 — real `embyr-server` subprocess, real testcontainers Postgres |

**Zero "NO — MISSING" rows.**

## Wave: DISTILL / [REF] Scaffolds (Mandate 7 — RED-ready)

All scaffolds marked `// SCAFFOLD: true` (Rust convention per Polyglot Adapter Matrix), panic (not
compile-error) when called, confirmed RED-not-BROKEN via the fail-for-right-reason gate (see
`docs/feature/customer-db-onboarding/distill/red-classification.md`).

- `crates/embyr-core/src/domain/schema_readiness.rs` (**new**) — pure `SchemaReadiness` enum
  (`Ready`/`NotPrepped`/`Stale`). Not scaffold-panicked (pure data type, no behavior to RED —
  matches Mandate 7's intent that only *method bodies with logic* need the panic marker).
- `crates/embyr-core/src/domain/mod.rs` (**extended**) — registers `schema_readiness` module.
- `crates/embyr-pg-storage/src/backend_adapter.rs` (**extended**) — 3 new panicking scaffold
  methods on `PostgresBackendAdapter`: `verify_schema_readiness()`, `discover_current_user()`,
  `grant_schema_readiness_read(role_name)`. Existing `migrate()` untouched (already works).
- `crates/embyr-db-prep/Cargo.toml` (**new crate**) — `[[bin]] embyr-db-prep`; deps: `embyr-pg-storage`,
  `embyr-core`, `sqlx`, `tokio` only (ADR-022). Auto-included in the workspace via the existing
  `members = ["crates/*"]` glob in root `Cargo.toml` — no explicit member-list edit needed (differs
  from ADR-022's illustrative explicit-list snippet; functionally equivalent).
- `crates/embyr-db-prep/src/main.rs` (**new**) — entry point; `config::DbPrepConfig::from_env()`
  panics unconditionally (RED for every scenario at every step, per Mandate 7).
- `crates/embyr-db-prep/src/config.rs` (**new**) — `DbPrepConfig::from_env()` scaffold.
- `crates/embyr-db-prep/src/error_report.rs` (**new**) — `classify()` scaffold.
- `deny.toml` (**extended**) — `embyr-db-prep` registered in the `tokio` and `sqlx` wrapper allow-lists
  (IO-permissive scope, matching `embyr-agent`/`embyr-server`; `embyr-core` stays IO-prohibited).

**Not yet scaffolded (deliberately — DELIVER's refactor, not new scaffolding):** `provision.rs`'s 3
inline `sqlx::migrate!` call sites are unchanged this wave. ADR-022 requires them to be replaced with
calls to `PostgresBackendAdapter::migrate()` — this is the refactor `cdo06` proves is still
outstanding (currently 5 embed points in the workspace, target 1). Per feature-delta.md's own DESIGN
framing, this is "a real refactor embyr-db-prep's existence forces, not new scaffolding."

## Wave: DISTILL / [REF] Test Infrastructure

**Test placement**: `tests/customer_db_onboarding/acceptance/*.rs` (flat `#[test]`/`#[tokio::test]`
functions with descriptive names, one file per scenario) + `tests/customer_db_onboarding/common/mod.rs`
(shared fixtures). Matches this repo's established convention (`tests/card_payments_backend/`,
`tests/production_readiness/`) — **not** literal Gherkin `.feature` files, and not the generic
Rust-matrix `<feature>_scenarios.rs` + `<feature>_specifications.rs` split (per this task's explicit
override of the generic template in favor of established repo precedent).

**`[[test]]` target ownership** (Cargo constraint, not a style choice): `CARGO_BIN_EXE_<name>` is only
set by Cargo for `[[test]]` targets declared inside the *owning* package's own `Cargo.toml`. US-01's
11 scenarios (`cdo01`-`cdo11`) are therefore declared in `crates/embyr-db-prep/Cargo.toml` (resolves
`CARGO_BIN_EXE_embyr-db-prep`); US-02's 7 scenarios (`cdo12`-`cdo18`) are declared in
`crates/embyr-server/Cargo.toml` (resolves `CARGO_BIN_EXE_embyr-server`), alongside the existing
`us_0N_*`/`obs0N_*`/`cpb0N_*` entries. `common/mod.rs` is `#[path]`-shared between both crates' test
binaries; both binary-path resolvers use `option_env!` (not `env!`) since the file compiles under
both package contexts and only one `CARGO_BIN_EXE_*` var is defined at a time.

**ATDD Infrastructure Policy** (`docs/architecture/atdd-infrastructure-policy.md`) — new row appended
this wave (Driving table): `embyr-db-prep` binary (subprocess) — `std::process::Command::new(...)`
spawned with env vars; one-shot process (no healthz polling — completion signalled by exit), test
waits via `Child::try_wait()` polling + captures stdout/stderr; testcontainers Postgres as the
driven-internal port. Direct extension of the existing `embyr-server` subprocess row's mechanism, per
this task's explicit instruction (apply-if-exists/append-missing-row procedure, no user prompt
needed — the mechanism is an obvious extension of an existing row).

## Wave: DISTILL / [REF] Driving Adapter Coverage

| Driving port | Protocol exercised | Scenario(s) |
|---|---|---|
| `embyr-db-prep` CLI process | Real subprocess spawn, env vars, exit code + stdout/stderr capture | cdo01-cdo11 (all 11) |
| `POST /admin/v1/projects` (`direct_pg` branch) | Real HTTP via `reqwest` against a real `embyr-server` subprocess | cdo12-cdo18 (all 7) |

Both entry points named in DESIGN's § Driving Ports table are covered by at least one WS scenario
exercising the real invocation path (subprocess exit code/output for the CLI; HTTP status/body for
the endpoint) — zero uncovered entry points.

## Wave: DISTILL / [REF] Pre-requisites

- DESIGN driving ports: `embyr-db-prep` CLI process (new), `POST /admin/v1/projects` (existing,
  extended) — both from `docs/product/architecture/brief.md` § Application Architecture —
  customer-db-onboarding § Driving Ports.
- DEVOPS environment matrix: none provided this feature (directory absent) — default matrix applied
  (clean | with-pre-commit | with-stale-config); no environment-specific behavior in scope beyond
  what real-Postgres/real-subprocess testing already covers.
- `migrations/customer/0001_documents.sql`, `0002_transactions.sql` — the shared migration set both
  the prep tool and the verify step must agree on (confirmed unchanged, `expected_version = 2`).
- `crates/embyr-agent/src/{main,config,probe}.rs` — the wire→probe→use / error-accumulation config
  pattern `embyr-db-prep`'s scaffolds mirror (per ADR-022 Reuse Analysis).
- `crates/embyr-server/src/adapters/system_db.rs`'s `SystemDb::probe()` — the reference hard-gate
  pattern `verify_schema_readiness()` mirrors (per ADR-023).

## Wave: DISTILL / [REF] Pre-DELIVER Fail-for-the-Right-Reason Gate

**PASSED.** Full detail, per-scenario classification, and the two fixture bugs found-and-fixed during
this gate run: `docs/feature/customer-db-onboarding/distill/red-classification.md`. Summary: full
`cargo check --workspace --tests --all-targets` passes with zero errors (structural RED-not-BROKEN
for all 18 files); 10 of 18 scenarios spot-executed directly against real testcontainers Postgres, all
classified `MISSING_FUNCTIONALITY` except `cdo17` (a regression guardrail that is correctly
already-green, asserting *unmodified* existing behavior); remaining 8 scenarios reuse the same
verified fixture helpers with no novel technique.

## Wave: DISTILL / [REF] Mandate Compliance Evidence

- **CM-A** (Mandate 1, hexagonal boundary): every scenario invokes through a driving port —
  subprocess (`embyr-db-prep`) or HTTP (`POST /admin/v1/projects`) — never an internal component
  directly. `cdo09`/`cdo18` additionally call `PostgresBackendAdapter::verify_schema_readiness()`
  directly as a **driven-port** read for fixture verification (checking system state, not driving
  behavior) — consistent with acceptance-level driven-port assertion, not a boundary violation.
- **CM-B** (Mandate 2, business language): scenario titles and doc comments use domain terms
  (Elena, Sam, "prep", "prepped", "DML-only connection string", "schema version gap" — the exact
  ubiquitous-language terms DISCUSS's § System Constraints established). No HTTP/JSON/schema jargon
  in scenario titles; technical detail lives inside step bodies only.
- **CM-C** (Mandate 3, user journey completeness): both walking skeletons trace a complete
  user/operator journey with an observable business outcome (Elena sees a readiness confirmation;
  Sam's provisioning proceeds/fails with an actionable message) — not isolated technical operations.
- **CM-D** (Mandate 4, pure function extraction): not directly applicable — this feature's business
  logic is inherently I/O-bound (DB introspection, DDL). No pure-function extraction opportunity was
  bypassed; `SchemaReadiness` itself is the pure data type DELIVER's comparison logic will construct.
- **CM-E** (Mandate 8, Universe-bound state-delta): every scenario at layer 3 (subprocess/FS
  acceptance — all 18 are this layer) that mutates or observes state uses
  `assert_state_delta(before, after, universe, expected)` with port-exposed universe entries
  (`prep_process.exit_code`, `migrations.applied_count`, `provision_response.http_status`,
  `provision_response.error_field`) — never internal struct fields — **except** the two
  `@walking_skeleton`-tagged scenarios (cdo01, cdo12), which use traditional assertions per the
  Layered Test Discipline table's WS row (traditional, not state-delta, at that layer).
- **CM-F** (Mandate 9, layer-dependent PBT mode): zero PBT machinery (`@given`, `RuleBasedStateMachine`
  equivalents, `proptest`) imported anywhere in this feature's tests — correct, since every scenario
  runs at layer 3 (subprocess/FS acceptance), which per Mandate 9 is example-only.
  `docs/architecture/atdd-infrastructure-policy.md`/root `Cargo.toml` already list `proptest` as a
  workspace dev-dependency for other features; this feature does not add a PBT use.
  `crates/embyr-core/src/domain/schema_readiness.rs` is a plain enum with no comparison logic yet —
  when DELIVER adds one, it becomes a layer-1/2 candidate for `proptest`, out of DISTILL's scope.
- **CM-G** (Mandate 10, two-tier acceptance): Tier B correctly absent — see § Two-Tier Acceptance
  Decision above (config-shaped feature, does not meet the ≥3-chained-AND-domain-rich trigger).
- **CM-H** (Mandate 11, example-based sad paths): all error-path scenarios (cdo04, cdo05, cdo08,
  cdo13, cdo14, cdo15) are named, explicit example-based tests (`*_reported`, `*_fails_when_*`,
  `*_cannot_read_*`) — no PBT-generated sad paths, consistent with layer 3's example-only mode.

---
