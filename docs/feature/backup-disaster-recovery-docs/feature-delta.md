# Feature Delta: backup-disaster-recovery-docs

## Wave: DISCUSS / [REF] Reading Confirmation

✓ `docs/product/production-readiness-audit-2026-09-08.md` read in full. Finding #18 (Database /
DevOps, High) confirmed verbatim: *"No backup/disaster-recovery documentation exists anywhere in
the repo — RTO/RPO, PITR guarantees, and backup ownership (embyr's vs. customer's for
BYOC/`direct_pg`) are entirely unstated."* Location cited as `docs/` — "absence confirmed via
full-text search."

✓ Absence re-confirmed independently, not trusted from the audit's own citation alone. Grepped
`docs/` for `backup`, `disaster.recovery`, `RTO`, `RPO`, `PITR` (case-insensitive). Zero
substantive hits — every match is either this audit's own finding row, or an incidental mention
of the word "backup" in three unrelated ADRs (`adr-036`, `adr-024` ×2) describing what a raw
System-DB *breach* (not backup process) would or would not expose. No document anywhere states an
RTO, an RPO, a PITR guarantee, or who owns backup for which database. **Finding confirmed still
accurate as of 2026-09-14.**

✓ `docs/product/architecture/adr-001-process-topology.md` read in full — confirms deployment
topology and the two-database model (System DB vs Customer DB).

✓ `docs/product/architecture/brief.md` read (relevant excerpts: System DB description, failure-mode
table, Postgres connection-pool guidance). Confirms `System DB` is "Operator-managed Postgres
(projects, metrics) — one per deployment."

✓ `crates/embyr-core/src/domain/project.rs`, `crates/embyr-server/src/adapters/project_auth.rs`,
`crates/embyr-server/src/adapters/customer_db_connect.rs`, `crates/embyr-server/src/admin/handlers/provision.rs`
read/grepped for `direct_pg`/`BackendMode`. Confirms the full set of customer-database backend
modes: `direct_pg`, `agent`, `aws_secret`, `gcp_secret`. In **every** mode, the Postgres instance
holding customer documents is the customer's own — embyr never provisions, hosts, or operates it.
The 4 modes differ only in *how embyr obtains a connection string/credential* to the customer's
already-existing database (a literal DSN, a customer-VPC agent binary holding the DSN, or a
customer-owned AWS/GCP secret), never in *who owns the database*.

✓ `CLAUDE.md` (project root) re-read: *"embyr-rs is a Rust reimplementation of the Google Firestore
gRPC protocol server. It is a protocol translation layer only — not a database."* This is the
load-bearing architectural fact for this finding: embyr-rs holds no durable state of its own
outside two Postgres databases (System DB, Customer DB) — confirmed via grep across
`crates/embyr-server/src` for any local filesystem persistence (`sqlite`, `std::fs::File::create`,
`write_all`, `PathBuf::from` used as a data store) — zero hits. No local disk state exists to back
up.

✓ `docs/evolution/2026-08-09-production-readiness.md` (ADR-017, ADR reference) read — production
startup, Dockerfile, CI. No backup/DR content; confirms deployment conventions (env-var config,
`docs/product/architecture/adr-017-production-startup.md` as the authority for startup/operational
docs) that this feature's deliverable should follow, not restate.

✓ `docs/evolution/2026-08-10-secrets-management.md` and `docs/product/architecture/adr-018-secrets-management.md`
read in full. Confirms `EMBYR_ENCRYPTION_KEY` is a single external secret (env var or AWS/GCP
Secrets Manager reference) used to decrypt three encrypted System-DB columns
(`users.totp_secret_enc`, `oidc_providers.client_secret_enc`, `projects.backend_pg_dsn_enc`). The
key itself is **never stored in Postgres** — it is not derivable from a System DB backup. This is a
genuine, previously-undocumented DR dependency (see Investigation 3 below), not restated from any
existing doc.

✓ `docs/product/known-gaps.md` read — confirms this finding is a fresh, independent scan item, no
overlap with the prior 8-item closed list.

✓ `docs/product/jobs.yaml` read in full (JOB-01 through JOB-20), with particular attention to
JOB-13 (`production-deployment`, P2 Sam Chen) — see § Persona & Job.

✓ Searched for any existing backup-adjacent infrastructure (`embyr-db-prep`, adapters, sweepers):
`crates/embyr-db-prep` (schema provisioning/migration only, confirmed via
`docs/feature/customer-db-onboarding/feature-delta.md` and ADR-022/ADR-023 — no backup logic),
`IndexManager` (composite-index metadata, ADR-072/ADR-080 — no backup logic). **Zero backup-related
infrastructure exists anywhere in the codebase** — this is not a case of "the capability exists but
isn't written down."

## Wave: DISCUSS / [REF] Investigation Findings

### Investigation 1 — this is a documentation-only finding: no code or infrastructure gap exists to close

Reading the finding's own text closely: "documentation... unstated." The question DISCUSS must
settle is whether *closing it properly* requires more than writing a document — e.g. a backup
verification script the runbook would reference, or an admin endpoint to check backup status.
Investigation confirms **no such gap exists**:

- Every database embyr-rs touches (System DB, Customer DB in all 4 backend modes) is a *standard,
  externally-managed Postgres instance*. embyr-rs is "a protocol translation layer only — not a
  database" (CLAUDE.md) — it never runs its own storage engine, never owns disk volumes, and has no
  local filesystem state (confirmed by grep, § Reading Confirmation). Backup/PITR capability for a
  standard Postgres instance is provided by the hosting/cloud layer (e.g. RDS automated backups +
  PITR, Cloud SQL automated backups, self-managed `pgbackrest`/WAL-archiving) — a capability that
  **already exists wherever Postgres is deployed**, independent of anything embyr-rs's own code
  does or doesn't do.
- There is no "embyr provisions/manages the customer's Postgres" mode. `direct_pg` names only *how
  embyr connects* (a literal DSN), not who runs the instance — confirmed by reading
  `resolve_direct_pg_dsn` (`crates/embyr-server/src/adapters/customer_db_connect.rs`): it decrypts
  and returns a DSN pointing at a Postgres instance whose existence, backup, and PITR configuration
  are entirely outside embyr-rs's control or knowledge. The audit finding's own phrasing —
  "backup ownership (embyr's vs. customer's for BYOC/`direct_pg`)" — reads as if this were an open
  question; investigation shows it is not: customer-database backup is the **customer's**
  responsibility in all 4 backend modes, with no exception. This determination itself is new
  information this DISCUSS pass contributes (not previously stated anywhere), and is the single
  most important fact the deliverable document must make explicit, since the audit's own wording
  suggested ambiguity that the codebase does not actually contain.
- System DB backup is the **operator's** responsibility (ADR-001: "Operator-managed Postgres...
  one per deployment"). For embyr's own hosted SaaS, embyr *is* the operator, so this is embyr's own
  operational responsibility — but the concrete backup mechanism/RTO/RPO for that specific hosted
  instance is not decided anywhere in this repo today (no IaC/hosting manifests exist at all —
  audit finding #19, still open, separately tracked, explicitly out of scope for this feature).
- No verification script, restore-drill tooling, or admin status endpoint is required to make this
  finding "closed" in the sense the audit means: the finding is entirely about an **unstated
  guarantee and an unstated ownership boundary**, not about a missing capability. A
  well-written document stating "customer owns customer-DB backup in all modes; operator owns
  System-DB backup; here is the recommended mechanism and target" fully closes it. (A future,
  separately-scoped feature could add a `/readyz`-style backup-freshness check or a
  `embyr-db-prep --verify-backup` CLI — flagged as optional future work below, explicitly NOT
  required to close this finding.)

**Conclusion: PURE DOCUMENTATION. Zero code/infrastructure changes required or recommended for this
feature.**

### Investigation 2 — backup ownership boundary, stated precisely

| Database | Who runs it | Who backs it up | Embyr's role |
|---|---|---|---|
| System DB (accounts, sessions, admin, billing, metrics, signing keys) | The operator running embyr-rs (embyr itself, for its hosted SaaS; a self-hoster, if self-hosted) | **Operator** (embyr, for the SaaS; the self-hoster, otherwise) | Documents the requirement and recommended mechanism; does not implement backup tooling itself |
| Customer DB, `backend_mode=direct_pg` | Customer (embyr holds only an encrypted DSN) | **Customer** | None — embyr never sees the instance beyond a connection string |
| Customer DB, `backend_mode=agent` | Customer (credentials never leave the customer's VPC — `embyr-agent` holds them) | **Customer** | None — architecturally cannot back up what it cannot reach (ADR-001 §Alternative C) |
| Customer DB, `backend_mode=aws_secret` / `gcp_secret` | Customer (DSN fetched from the customer's own cloud secret manager at connect time) | **Customer** | None — same boundary as `direct_pg`, different credential-sourcing mechanism |

This table (or its equivalent) is the single most important piece of content the deliverable
document must contain — it directly answers the audit's "embyr's vs. customer's" question with no
remaining ambiguity, for every one of the 4 backend modes, not just the 2 the finding named.

### Investigation 3 — a real DR dependency this investigation surfaced, not previously documented anywhere

Restoring a System DB backup is **not sufficient by itself** to recover service. Three encrypted
columns (`users.totp_secret_enc`, `oidc_providers.client_secret_enc`, `projects.backend_pg_dsn_enc`)
depend on `EMBYR_ENCRYPTION_KEY`, an external secret never stored in Postgres (ADR-018). If that key
(and, during a rotation window, `EMBYR_ENCRYPTION_KEY_PREVIOUS`) is not *also* recoverable at
restore time — independently of the Postgres backup — every restored row encrypted under it becomes
permanently undecryptable: TOTP-protected admin accounts get locked out, OIDC client secrets stop
working, and every `direct_pg`-mode project loses its stored DSN (operators would have to
re-provision those projects' connection strings from scratch). This is a genuine, previously-unstated
operational risk: the document must state that key-material recovery is a **precondition** for a
successful System DB restore, and that responsibility for that key's own durability sits wherever it
is sourced from (AWS/GCP Secrets Manager's own durability guarantees, if sourced that way; the
operator's own secret/config management, if sourced as a plain env var) — not from the Postgres
backup itself. This finding is additive to #18, discovered via this feature's own investigation, not
a restatement of anything already tracked.

### Investigation 4 — RTO/RPO cannot be asserted as a single global number; the document must give operator guidance, not fabricate one SaaS-wide SLA

No infrastructure-as-code, hosting manifest, or deployment platform decision exists anywhere in this
repo (audit finding #19, still open, separately tracked, out of scope here) — `ADR-001` deliberately
describes "operator-managed Postgres... one per deployment" in generic terms, consistent with
embyr-rs being deployable both as embyr's own hosted SaaS and as self-hosted software (ADR-001's own
process-topology reasoning does not distinguish the two). Concrete RTO/RPO numbers are a function of
*which* managed-Postgres backup mechanism an operator actually configures (e.g. AWS RDS automated
backups: ~5 min RPO via continuous WAL streaming, up to 35-day PITR window; Cloud SQL: similar
defaults; self-managed `pgbackrest`: operator-configured). Since no specific hosting platform is
locked in this repo today, this DISCUSS pass **cannot fabricate a single authoritative number**
without inventing an architecture decision outside its authority. The deliverable document instead:
(a) states the recommended minimum guarantees (an explicit, named RPO/RTO **target**, not a
guarantee, pending an actual hosting decision), and (b) gives operators the information needed to
configure and verify their own numbers against that target. This is flagged as an **open item**
requiring either a future DESIGN-wave hosting decision (if/when finding #19 is picked up) or an
explicit operator-facing "fill in your own target" section — not a blocker to shipping the document,
since the ownership-boundary content (the audit's primary ask) does not depend on it.

## Wave: DISCUSS / [REF] Persona & Job

**Persona**: P2 Sam Chen (Service Operator / Platform Engineer) — same persona used by every prior
DevOps/Reliability-flavored finding this session (`production-readiness`, `secrets-management`,
`healthz-dependency-checks`, `pool-sizing-and-limits`).

**Job**: JOB-13 (`production-deployment`) — reused, not a new job. Same persona, and this finding is
a direct extension of JOB-13's existing functional dimension ("ship without writing infrastructure
from scratch," "feel in control") into the one deployment question every one of JOB-13's prior
realizations has left unanswered: *what happens when the database is gone*. This mirrors the
established "make it real" / close-the-remaining-gap extension pattern this session has used
repeatedly for JOB-13 (`firestore-tls-support`, `healthz-dependency-checks`) rather than the
"same persona, different goal ⇒ new job" pattern (reserved for materially different mental models,
e.g. JOB-14 vs JOB-10). No alternative job fits better: JOB-15 (`customer-db-preflight`) is about
*provisioning* a customer database, not backing it up; JOB-11 (`fair-multitenancy`) is an unrelated
dimension (rate limiting/resource fairness). JOB-13 is correct.

**Job story** (JOB-13, unchanged, restated for traceability): *"When I want to deploy embyr to
production, I want a single `docker run` command to start the server, so I can ship without writing
infrastructure from scratch."* This finding extends the **anxiety** force already named in JOB-13:
today Sam has no documented answer to "what if the database is gone — how long until we're back, and
whose job is it to make sure a backup exists at all?" for either database embyr touches.

## Wave: DISCUSS / [REF] Scope Assessment (Elephant Carpaccio Gate)

Oversized-feature signals checked (need 2+ to flag): >10 stories (no — 1 deliverable), >3 bounded
contexts (no — this is a cross-cutting operational document, not a bounded-context change), walking
skeleton needing >5 integration points (no — zero code integration points, it is prose), estimated
effort >2 weeks (no — single document, well under 1-3 days), multiple independent shippable outcomes
(no — one coherent document serves one audience for one purpose).

**Scope Assessment: PASS — 1 deliverable (documentation), 0 bounded contexts touched, estimated
0.5-1 day.** Right-sized. No split needed. This is smaller than a typical code story, not larger —
flagged explicitly because the *shape* differs from every other feature this session (no acceptance
tests, no mutation testing applies), not because the size does.

## Wave: DISCUSS / [REF] Deliverable Spec

**Target artifact**: `docs/operations/backup-disaster-recovery.md` (new file, new `docs/operations/`
directory — no existing directory holds operator-facing runbook-style content; this follows the same
`docs/{category}/{topic}.md` pattern already used by `docs/architecture/`, `docs/scenarios/`,
`docs/adrs/`). Authored during DELIVER; DISCUSS specifies required content, not final prose.

**Audience**: Sam Chen and equivalent operators (embyr's own SRE team operating the SaaS; any
self-hosting operator; secondarily, customers evaluating embyr for a compliance/vendor-security
questionnaire, who need the ownership-boundary content specifically).

**Required sections** (Definition-of-Done for the document itself, not a BDD test suite):

1. **Ownership boundary table** — the exact 4-row table from Investigation 2, unabridged: System DB
   (operator-owned) and all 4 customer-DB backend modes (customer-owned, no exceptions), stated
   plainly enough to answer a customer's security questionnaire without follow-up questions.
2. **System DB backup guidance** — recommended mechanism class (continuous WAL-archiving /
   managed-Postgres PITR), a named RPO/RTO **target** (explicitly labeled as a target pending a
   locked hosting decision, per Investigation 4 — not asserted as a guaranteed SLA), and what
   "restore" concretely means operationally (new Postgres instance, point-in-time or latest,
   `DATABASE_URL` repointed, `embyr-server` restarted — no code changes needed to restore, per
   Investigation 1).
3. **Key-material recovery precondition** — the Investigation 3 finding, stated as an explicit
   restore-runbook step: confirm `EMBYR_ENCRYPTION_KEY` (and `EMBYR_ENCRYPTION_KEY_PREVIOUS` if a
   rotation was in flight) is available *before* declaring a System DB restore complete; name the
   3 dependent encrypted columns and what breaks if the key is unavailable.
4. **Customer DB guidance (all 4 backend modes)** — a short paragraph per mode restating that backup
   is the customer's responsibility, and what embyr's own behavior is if the customer's database is
   unreachable after a restore (points to the existing failure-mode table in
   `docs/product/architecture/brief.md`, cross-referenced rather than duplicated — "Customer DB
   unreachable at request time → `Unavailable` returned to client, SDK retries").
5. **Verification / drill cadence** — a stated recommendation (e.g. periodic restore-drill), flagged
   honestly as "recommended, not currently automated or enforced by embyr-rs" — consistent with
   Investigation 1's conclusion that no verification tooling exists or is required to close this
   finding.
6. **Explicit non-goals** — states plainly that embyr-rs does not provision, monitor, or verify
   backups for any database (System or Customer), matching its role as "a protocol translation
   layer only — not a database" (CLAUDE.md).
7. **Cross-references, not restatement** — links to `adr-001-process-topology.md` (deployment
   topology), `adr-017-production-startup.md` (startup/config conventions),
   `adr-018-secrets-management.md` (key rotation), rather than duplicating their content.

**Verification criteria** (this replaces UAT/AC for a documentation deliverable — see
§ Definition of Ready for how DoR item 4/5 are satisfied by this shape instead of Given/When/Then
executable scenarios):

- [ ] The 4-mode ownership table matches Investigation 2 exactly — reviewable against this document,
      not runnable.
- [ ] The key-material precondition (Investigation 3) is present as an explicit restore-runbook step.
- [ ] No RTO/RPO number is asserted as a guarantee without the "target, pending hosting decision"
      qualifier (Investigation 4) — prevents the document from accidentally over-promising an SLA
      the codebase cannot back up (no pun intended) with real infrastructure.
- [ ] No implementation/code claim is made that investigation didn't confirm (e.g. must not claim a
      verification script or admin endpoint exists — none does).
- [ ] Peer-reviewed by `nw-documentarist` (DIVIO/Diataxis classification — this is primarily a
      **how-to/reference** hybrid: ownership facts are reference material, restore steps are a
      how-to) rather than `nw-acceptance-designer`/mutation testing, which do not apply to prose.

## Wave: DISCUSS / [REF] Elevator Pitch

**Before**: Sam Chen has no documented answer when asked "what's our RTO/RPO?" or "if the customer's
Postgres dies, is that our problem?" — the audit found zero mentions anywhere in the repo. A
customer's security/compliance questionnaire, or a real incident, both currently stall on this gap.

**After**: Sam (or a customer's own security reviewer) opens `docs/operations/backup-disaster-recovery.md`
and reads: a table stating System DB backup is embyr's job and every customer-DB backend mode's
backup is the customer's job (no exceptions across all 4 modes); a stated RPO/RTO target for System
DB; and the key-material precondition that must hold before a System DB restore is considered
complete.

**Decision enabled**: Sam can answer a customer's compliance questionnaire accurately without
escalating; during a real incident, Sam knows immediately whether "wait for the customer to restore
their own database" or "restore embyr's own System DB" is the correct action, and what precondition
(`EMBYR_ENCRYPTION_KEY` availability) must be checked first.

## Wave: DISCUSS / [REF] Definition of Ready Validation

Adapted for a documentation deliverable — DoR items are evaluated against "is the *document's
required content* fully specified and traceable," not against executable test scenarios, per this
feature's own confirmed pure-documentation shape (§ Investigation 1).

| DoR Item | Status | Evidence/Issue |
|---|---|---|
| 1. Problem statement clear, domain language | PASS | § Elevator Pitch "Before" — Sam has no documented answer to a specific, recurring question (compliance questionnaire, incident response), stated in domain language, not "documentation is missing" abstractly |
| 2. User/persona with specific characteristics | PASS | P2 Sam Chen, Service Operator/Platform Engineer, reused from JOB-13 with an established characteristic profile (12-factor habits, anxiety about undocumented failure modes) — § Persona & Job |
| 3. 3+ domain examples with real data | PASS | The 4-row ownership table (Investigation 2) names concrete backend modes (`direct_pg`, `agent`, `aws_secret`, `gcp_secret`) and concrete encrypted columns (`users.totp_secret_enc`, `oidc_providers.client_secret_enc`, `projects.backend_pg_dsn_enc`) — not generic placeholders |
| 4. UAT scenarios in Given/When/Then (3-7) | **ADAPTED** — see below | Traditional executable Given/When/Then does not apply to prose; replaced with the 5-item verification checklist in § Deliverable Spec, each independently reviewable against the document's actual content |
| 5. Acceptance criteria derived from UAT | PASS (adapted) | The verification checklist IS the AC, directly derived from Investigations 1-4, each traceable to a specific investigation finding above |
| 6. Right-sized (1-3 days, 3-7 scenarios) | PASS | § Scope Assessment: 0.5-1 day, 1 document, 5 required sections — smaller than typical, not larger |
| 7. Technical notes: constraints/dependencies | PASS | Must not assert unbuilt tooling (no verification script/endpoint exists); must not assert a fixed SLA number without the "target" qualifier (Investigation 4); must cross-reference rather than duplicate ADR-001/017/018 |
| 8. Dependencies resolved or tracked | PASS | No code dependency. One open item tracked (Investigation 4 — locked hosting decision, tied to audit finding #19, explicitly out of scope here, not blocking) |
| 9. Outcome KPIs defined with measurable targets | PASS (adapted) | See § Outcome KPIs below — behavior-change KPI (questionnaire/incident response time), not a feature-adoption metric, appropriate for a reference document |

### DoR Status: **PASSED**

Item 4 is marked ADAPTED rather than a bare PASS to make the shape difference visible to the
reviewer and to downstream waves, per the orchestrator's explicit instruction that DISTILL/DELIVER/
QUALITY_GATE must not have a code-shaped process forced onto this deliverable. See
§ Pipeline Recommendation.

## Wave: DISCUSS / [REF] Outcome KPIs

| # | Who | Does What | By How Much | Baseline | Measured By | Type |
|---|---|---|---|---|---|---|
| 1 | Sam Chen / operators | Answers a backup-ownership or RTO/RPO question (customer questionnaire or real incident) without escalating or guessing | 100% of such questions answered by pointing to the document, 0 answered by guessing or "let me check with engineering" | 0% today (no document exists) | Manual — next 3 real occurrences (support ticket, security questionnaire, or incident) checked against document use | Leading |
| 2 | A restored System DB | Recovers to a fully functional state (including encrypted-column decryption) on the first restore attempt | 100% of restore drills succeed without an undocumented "the encryption key was missing" surprise | Unknown — no restore drill has ever been documented or run | Restore-drill checklist completion (recommended cadence in the document itself) | Leading |

**North Star**: Zero undocumented-dependency surprises during a real System DB restore (Investigation
3's key-material precondition is the concrete, previously-hidden risk this guards against).
**Guardrail**: The document must never assert a stronger guarantee (a fixed SLA number) than the
underlying infrastructure actually provides (Investigation 4) — a false guarantee is worse than no
document.

## Wave: DISCUSS / [REF] Pipeline Recommendation (how DISTILL/DELIVER/QUALITY_GATE should adapt)

This feature's confirmed shape (pure documentation, zero code/infrastructure change) means the
standard nWave code pipeline does not apply as-is. Recommended adaptation, so downstream waves don't
force a code-shaped process onto a docs-shaped deliverable:

- **DESIGN (solution-architect)**: Skip architecture/component design — there is no component. If
  Investigation 4's open item (a locked hosting decision with real RTO/RPO numbers) is wanted before
  shipping, a short DESIGN pass can resolve it; otherwise DESIGN can be skipped entirely and the
  document ships with the "target, pending hosting decision" qualifier intact.
- **DISTILL (acceptance-designer)**: Skip BDD/Gherkin scenario design and E2E test authoring —
  nothing here is executable. Replace with the § Deliverable Spec verification checklist as the
  equivalent artifact.
- **DELIVER**: Not `nw-software-crafter`/`nw-functional-software-crafter` (no code to write with
  Outside-In TDD). The document is authored directly against § Deliverable Spec's required sections,
  ideally by `nw-documentarist` (already exists in this agent roster, purpose-built for DIVIO/Diataxis
  documentation quality) rather than a code-focused crafter agent.
- **QUALITY_GATE**: No mutation testing — there is no code to mutate. Replace with:
  (a) the § Deliverable Spec verification checklist (human/reviewer-checked against the actual
  document), and (b) `nw-documentarist`'s own DIVIO/Diataxis validation pass (classification
  accuracy, collapse-pattern detection) in place of a code-review pass.
- **Evolution doc**: Still write one (`docs/evolution/{date}-backup-disaster-recovery-docs.md`),
  matching this session's convention — "What Shipped" becomes "what the document now states,"
  "Quality Gates" becomes "verification checklist + documentarist review," not test/mutation counts.

## Wave: DISCUSS / [REF] Out of Scope

- Locking a specific hosting platform / IaC decision for embyr's own SaaS System DB (audit finding
  #19 — separately tracked, not started, explicitly independent of this finding per the audit's own
  notes).
- Building any backup verification tooling, restore-drill automation, or an admin backup-status
  endpoint (Investigation 1 — not required to close this finding; named as optional future work
  only).
- Any change to `embyr-agent`, `embyr-db-prep`, or any adapter — confirmed zero backup-related code
  exists or is needed (§ Reading Confirmation, final bullet).
- Customer-facing contractual/legal backup SLA language (a legal/business decision, not a technical
  documentation one) — the document states the technical ownership boundary and target, not a
  contract.

## Wave: DISCUSS / [REF] Handoff Package

- This `feature-delta.md` (single narrative file, this session's established lean-output convention)
- Required content fully specified in § Deliverable Spec, traceable to Investigations 1-4
- Persona/job traceability: JOB-13, P2 Sam Chen (§ Persona & Job)
- DoR: PASSED, with item 4 explicitly adapted for a documentation shape (§ Definition of Ready
  Validation)
- Outcome KPIs defined (§ Outcome KPIs)
- Recommended next step: either (a) proceed directly to DELIVER (document authoring) skipping
  DESIGN/DISTILL per § Pipeline Recommendation, or (b) a short DESIGN pass first if the orchestrator
  wants Investigation 4's RTO/RPO target locked to a real number before the document ships.
