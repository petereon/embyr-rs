# Feature: deployment-release-process

<!-- markdownlint-disable MD024 -->

## Wave: DISCUSS
## Date: 2026-09-15
## Status: Ready for DESIGN wave

---

## This addresses finding #19 from `docs/product/production-readiness-audit-2026-09-08.md`

> No deployment automation beyond the Dockerfile itself (no K8s manifests, Helm chart, ECS
> task def, or docker-compose), and no release process — crate version frozen at `0.1.0`, no
> CHANGELOG, no git tags. "The release" is whatever commit is on `master` when someone runs
> `docker build`.

---

## Investigation Findings (confirmed, not assumed)

Read the codebase directly rather than trusting the audit's own summary alone — consistent
with this session's convention of re-verifying prior findings before scoping work against them.

| Claim | Verified | Evidence |
|---|---|---|
| Dockerfile exists (production readiness already shipped it) | YES | `Dockerfile` (embyr-server, ADR-017 D-PR-3) and `crates/embyr-agent/Dockerfile` (ADR-074) both exist and are CI-validated |
| No K8s manifests / Helm chart / ECS task def / docker-compose | CONFIRMED — still true | `Glob` for `**/k8s/**`, `**/helm/**`, `**/docker-compose*` — zero hits |
| No CHANGELOG | CONFIRMED — still true | `Glob` for `CHANGELOG*` — zero hits |
| No git tags | CONFIRMED — still true | `.git/packed-refs` contains only the header comment; no loose tag refs under `.git/refs/tags/` |
| Crate version frozen at 0.1.0 | CONFIRMED | root `Cargo.toml`: `[workspace.package] version = "0.1.0"`; every crate (`crates/embyr-server/Cargo.toml` checked directly) uses `version.workspace = true` — one shared version, workspace-wide, never bumped |
| CI does anything release-shaped (tag/publish/version bump) | NO | `.github/workflows/ci.yml` has 4 jobs (`test`, `lint`, `docker`, `agent`) — all build/test/validate, none tags, publishes, or bumps a version |
| `embyr-server` exposes its own running version anywhere | NO | grep of `crates/embyr-server/src/{main,config}.rs` for `CARGO_PKG_VERSION`/`--version` — zero real hits (one false-positive comment). No `--version` flag, no version field on `/healthz`, no version in the startup log line |

---

## Job Discovery Framing Resolution

**Reused: JOB-13 (`production-deployment`), persona P2 Sam Chen (Service Operator / Platform
Engineer)** — same job this session's `production-readiness` and `embyr-agent-release-pipeline`
features used. No new job warranted: this is the same "ship without writing infrastructure from
scratch" job, now extended to "know exactly what shipped and when."

### Prior-art check (per instructions, before reinventing anything)

- **`docs/evolution/2026-08-09-production-readiness.md`** (ADR-017): established the Dockerfile,
  CI, and `ServerConfig::from_env()` conventions this feature builds on. Established NO
  versioning/tagging/CHANGELOG convention — out of its own scope.
- **`docs/evolution/2026-09-09-embyr-agent-release-pipeline.md`** (ADR-074): built a CI
  build/artifact/Dockerfile path for the separate `embyr-agent` binary. Checked closely for a
  reusable versioning scheme — **found none**. Its artifact is named
  `embyr-agent-x86_64-unknown-linux-musl` (target-qualified, not version-qualified); no git tag,
  no CHANGELOG entry, no version bump was part of that feature. Its own Follow-Up Work section
  explicitly says: *"Finding #19 (release-versioning/changelog infra, High) was explicitly named
  as this feature's own out-of-scope follow-up during DISCUSS."* This confirms #19 is genuinely
  unclaimed territory, not a duplicate of work already done — there is no existing convention to
  reuse; this feature establishes the first one.
- **`docs/evolution/2026-09-15-backup-disaster-recovery-docs.md`**: deferred its RTO/RPO targets
  as "pending hosting decision (finding #19, tracked separately)". Read literally, this implies
  #19 is expected to make a **hosting-provider/topology decision** (which cloud, which
  orchestrator). Investigated whether that's actually this finding's job — see Scope Decision
  below. Conclusion: the hosting decision is a separate, larger, currently-unblocked-by-nothing
  architectural question that does NOT need to be resolved to give "the release" a concrete
  meaning, and forcing it into this feature would make it balloon past Elephant Carpaccio limits
  for no corresponding user value gained today.

---

## Scope Decision (Elephant Carpaccio Gate)

The finding bundles two genuinely separable concerns. Splitting them, rather than treating #19 as
one monolithic "deployment automation" epic, is the central DISCUSS finding of this feature.

| Concern | In scope now? | Rationale |
|---|---|---|
| **Release process** (versioning, git tags, CHANGELOG) | **YES — Slice 1** | Actionable today, zero dependency on any undecided architectural question. Directly closes the literal "crate version frozen at 0.1.0, no CHANGELOG, no git tags" language in the finding. |
| **Local/single-host evaluation** (docker-compose) | **YES — Slice 2** | Also actionable today — a compose file for local dev/evaluation needs no hosting-provider decision, no cloud choice, nothing beyond the Dockerfile this repo already has and CI already validates. Directly closes the literal "no docker-compose" language in the finding. |
| **K8s manifests / Helm chart / ECS task def** | **NO — explicitly deferred** | See below. |

### Why K8s manifests / Helm / ECS are deferred, not included

1. **No hosting/cloud-provider decision exists anywhere in this repo.** Grepped
   `docs/product/architecture/` for a hosting/cloud-provider ADR — none exists. The only
   Kubernetes reference in the entire product docs tree
   (`docs/product/journeys/agent-deployment.yaml:39-40`, `kubectl apply -f
   agent-deployment.yaml`) is about a **customer** deploying the `embyr-agent` binary into
   **their own** VPC Kubernetes cluster (ADR-001's privacy boundary) — a wholly different
   concern from embyr operating its own SaaS backend (`embyr-server`) on Kubernetes. There is
   zero evidence anywhere in this codebase that embyr-server's own production hosting target is
   Kubernetes, ECS, or anything more specific than "a container runtime that can run
   `docker run`" (JOB-13's own literal job story).
2. **Building K8s manifests/Helm without a hosting decision is confirmation bias, not
   requirements.** Per `nw-po-review-dimensions` Dimension 1 (Technology Bias): writing
   Kubernetes YAML today would assume a specific orchestrator nothing in this session's evidence
   trail selected. JOB-13's own dimensions and four-forces analysis mention Docker + env vars —
   never Kubernetes, ECS, or any specific orchestrator — for embyr-server itself.
3. **This session's own recent history shows real cost to getting orchestrator-specific naming
   wrong.** `backup-disaster-recovery-docs` (closed today, same session) had to correct a
   fabricated `/readyz` endpoint reference; `healthz-dependency-checks` (ADR-078) only recently
   locked the actual `/healthz` (readiness) vs `/livez` (liveness) split a K8s manifest's own
   probe configuration would need to reference correctly. Writing K8s manifests now, before a
   hosting decision locks which orchestrator (if any) is actually used, risks the same
   drift-from-ground-truth failure mode on a much larger artifact (a full manifest/Helm chart)
   than a single doc paragraph.
4. **The `backup-disaster-recovery-docs` deferral is about RTO/RPO numbers, not K8s YAML
   specifically.** Re-reading that evolution doc's own language: it deferred *SLA numbers*
   pending "a hosting decision" — it did not say #19 must produce Kubernetes manifests. A hosting
   decision (self-hosted VM fleet vs. managed K8s vs. ECS vs. bare Docker on a single host) is
   itself the missing prerequisite, and making that decision is a platform-architecture
   exercise (DESIGN/DEVOPS wave scope, likely its own future feature with a named persona need),
   not a DISCUSS-wave user-story-writing exercise. Recommendation: file a new, separate,
   explicitly-hosting-decision-blocked finding/candidate (e.g. `production-hosting-topology`) for
   whoever picks up the backlog next; do not fold it into this feature.

**This feature deliberately does NOT make a hosting decision.** It closes the two clearly
actionable halves of #19 and leaves the orchestrator-specific automation (K8s/Helm/ECS) as an
explicitly named, hosting-decision-blocked follow-up.

---

## Job Traceability

### JOB-13 (REUSED — see jobs.yaml, no changes)

**Persona:** Sam Chen (P2 — Service Operator / Platform Engineer)
**Job Story:** When I want to deploy embyr to production, I want a single `docker run` command
to start the server, so I can ship without writing infrastructure from scratch.

**Mapping:**
- US-DRP-01 → JOB-13 (versioning/tagging/CHANGELOG gives "the release" Sam ships a concrete,
  repeatable meaning)
- US-DRP-02 → JOB-13 (docker-compose extends "ship without writing infrastructure from scratch"
  to local/single-host evaluation, the same job's own pull force)

---

## System Constraints

- No new Rust crate dependencies for either story: `CARGO_PKG_VERSION` is a compiler-provided
  `env!()` value already available at build time (zero Cargo.toml change needed).
- `docker-compose.yml` MUST build via the existing root `Dockerfile` — no second, divergent
  build definition. CI's existing `docker` job remains the single source of truth for "does the
  image build."
- The version-bump/tag/CHANGELOG convention applies **forward only**, starting from this
  feature's own merge. Retroactively tagging/versioning the ~40 features already merged this
  session is explicitly out of scope — matches this session's own "adapt visibly, do not
  silently expand scope" discipline.
- Documentation-only changes (under `docs/`) are exempt from the bump/tag/CHANGELOG requirement
  — a change with zero effect on the running binary should not force a release.
- `embyr-core` remains untouched (IO-free invariant, `deny.toml`-enforced) — neither story
  touches the domain layer.
- K8s manifests, Helm charts, and ECS task definitions are explicitly OUT of scope for every
  story in this feature (see Scope Decision above) — no story may introduce orchestrator-specific
  deployment YAML.

---

## User Stories

### US-DRP-01: Give "The Release" a Repeatable Meaning

**job_id:** JOB-13

#### Elevator Pitch
**Before:** Every crate in the workspace is frozen at version `0.1.0` forever
(`version.workspace = true`, never bumped). No git tag has ever been created. No CHANGELOG
exists. Sam cannot answer "what version is running in staging right now" or "what changed since
last Tuesday's deploy" without manually diffing `git log`.
**After:** Every change that ships in the running binary bumps the workspace version, is tagged
`vX.Y.Z` on the merge commit, and gets a dated CHANGELOG.md entry. `embyr-server`'s own startup
log line names its version. Sam runs `git tag -l`, reads `CHANGELOG.md`, and matches either
against the server's own startup log to know exactly what is running and what changed.
**Decision enabled by:** Sam decides whether to roll forward or roll back to a specific prior
release, and can tell a customer or auditor exactly what shipped between two dates — decisions
that are currently impossible to make with confidence.

#### Problem
Sam Chen is a service operator who has shipped ~40 features to production this session alone
(per this session's own evolution-doc history) but has no way to answer "what version is
currently running" or "what changed since the last deploy" without manually reading raw git
history — because the crate version has never moved off `0.1.0`, no git tag has ever been
created, and no CHANGELOG exists anywhere in the repository.

#### Who
- Sam Chen (P2) | Service operator responsible for production deployments and incident response
  | Needs to correlate a running instance to a specific, dated set of changes without git
  archaeology.

#### Solution
A SemVer versioning convention (bump `[workspace.package] version` in root `Cargo.toml`) paired
with a `CHANGELOG.md` (Keep a Changelog format) and a `vX.Y.Z` git tag on the commit that bumps
the version — applied to every merge that changes `crates/`, `migrations/`, or either
Dockerfile. `embyr-server`'s existing startup log line (ADR-017 step 12, "embyr-server ready")
is extended with its own version, sourced from `CARGO_PKG_VERSION` (already available, zero new
dependency).

#### Domain Examples

**Example 1 (Happy path — first real release under the new convention):** Sam's team merges the
PR that will close a future finding. Under the new convention, that PR bumps the workspace
version `0.1.0` → `0.1.1`, adds a `## [0.1.1] - 2026-09-16` entry to `CHANGELOG.md` describing
the change, and after merge a maintainer runs `git tag v0.1.1 <merge-sha> && git push --tags`.
`git tag -l` now shows `v0.1.1`.

**Example 2 (Runtime correlation):** Sam deploys the new image and wants to confirm which
version is live. The startup log now reads `embyr-server v0.1.1 ready grpc=0.0.0.0:8080
rest=0.0.0.0:8081 admin=0.0.0.0:9090` — one field added to an existing log line, zero new log
line. Sam matches "v0.1.1" against `CHANGELOG.md`.

**Example 3 (Regression triage):** A regression appears in a server running `v0.1.4`. Sam runs
`git log v0.1.2..v0.1.4 --oneline` and reads the `[0.1.3]` and `[0.1.4]` CHANGELOG sections,
narrowing the search from "all of master's history" to two dated, human-readable entries.

**Example 4 (Docs-only exemption, boundary case):** A contributor opens a PR that only edits
`docs/product/architecture/adr-081-something.md`. No version bump, tag, or CHANGELOG entry is
required or expected — nothing in the running binary changed.

**Example 5 (Rollback):** `v0.1.4` is confirmed to contain a regression. Sam runs
`git checkout v0.1.3` and rebuilds the Docker image from that exact tagged commit. The
`[0.1.4]` CHANGELOG entry documents precisely what changed, narrowing root-cause analysis.

#### UAT Scenarios (BDD)

```gherkin
Scenario: Sam identifies exactly which release is running
  Given embyr-server was built and started from the commit tagged v0.1.1
  When Sam reads the server's startup log
  Then the log line names the version "v0.1.1"
  And CHANGELOG.md contains a "[0.1.1]" section describing what shipped in that release

Scenario: A merged change to server code is captured in the release history
  Given a pull request modifies a file under crates/embyr-server/src
  When the pull request is merged to master following the release convention
  Then CHANGELOG.md gains a new dated entry under an incremented version number
  And a git tag matching that version exists on the merge commit

Scenario: Sam finds what changed between two deployments
  Given embyr-server was running v0.1.3 last week and is running v0.1.5 today
  When Sam reads CHANGELOG.md between the "[0.1.3]" and "[0.1.5]" sections
  Then Sam sees one dated entry per intermediate release ("[0.1.4]", "[0.1.5]")
  And each entry has a one-line-or-longer description naming the change and its motivating
    finding or feature, matching this session's own evolution-doc convention

Scenario: A documentation-only change does not require a new release
  Given a pull request only modifies files under docs/
  When the pull request is merged to master
  Then no version bump, tag, or CHANGELOG entry is required

Scenario: Sam rolls back to a known-good release
  Given v0.1.4 introduced a regression identified in production
  When Sam checks out the v0.1.3 tag and rebuilds the Docker image
  Then the resulting image is built from the exact source tree that was running before v0.1.4
  And the "[0.1.4]" CHANGELOG entry documents exactly what changed, aiding root-cause identification
```

#### Acceptance Criteria
- [ ] `CHANGELOG.md` exists at the repository root in Keep a Changelog format with an
      `[Unreleased]` section
- [ ] A documented convention — at `docs/operations/release-process.md` (matching this
      repository's existing `docs/operations/` convention, e.g.
      `backup-disaster-recovery.md`) — states which changes require a version bump (anything
      touching `crates/`, `migrations/`, `Dockerfile`, or `crates/embyr-agent/Dockerfile`) and
      which are exempt (`docs/`-only changes)
- [ ] The same document states the git tag format (`vMAJOR.MINOR.PATCH`) and that it is created
      on the merge commit that performs the corresponding version bump
- [ ] `embyr-server`'s startup log line includes its own version, sourced from
      `CARGO_PKG_VERSION`
- [ ] This feature's own merge performs the first real version bump (`0.1.0` → `0.1.1`), the
      first real git tag (`v0.1.1`), and the first real CHANGELOG.md entry — proving the
      mechanism end-to-end rather than only documenting it
- [ ] Documentation-only changes are explicitly exempted from the bump/tag/CHANGELOG requirement

#### Outcome KPIs
- **Who:** Sam Chen (service operator)
- **Does what:** Identifies the exact version running in any environment and what changed since
  a prior deployment, without reading raw git history
- **By how much:** Time to answer "what changed between what's running now and last week's
  deploy" drops from unbounded (manual git archaeology) to under 2 minutes (read CHANGELOG.md)
- **Measured by:** Manual timing during the next production incident or rollback; percentage of
  non-docs-only merges to master that include a CHANGELOG entry (target: 100% going forward)
- **Baseline:** 0% — no CHANGELOG exists, no tag has ever been created, version has never moved
  off `0.1.0`

#### Technical Notes
- `env!("CARGO_PKG_VERSION")` is a Rust std-provided compile-time macro; `version.workspace =
  true` (confirmed in `crates/embyr-server/Cargo.toml`) means it already resolves to the
  workspace version with zero new dependency.
- The specific enforcement mechanism (e.g., a CI check that blocks a PR touching `crates/`
  without a corresponding CHANGELOG.md diff) is a DESIGN-wave decision. DISCUSS specifies the
  observable behavior (a release has a version, a tag, and a changelog entry) — not how it is
  enforced.
- No dependency on US-DRP-02; independently shippable.
- Explicitly excludes retroactive tagging/versioning of this session's own already-merged
  history (~40 features) — convention applies forward from this feature's merge only.

---

### US-DRP-02: Run the Full Stack Locally Without Writing Infrastructure

**job_id:** JOB-13

#### Elevator Pitch
**Before:** No `README`, no `Makefile`, no `docker-compose.yml` exist anywhere in the
repository. A new contributor or a prospective self-hosting evaluator must read
`crates/embyr-server/src/config.rs` to discover the three required env vars, hand-craft a
64-hex-char encryption key, separately start a Postgres container, and only then run
`cargo run`/`docker build` — with no single, committed reference for "how do I run this thing."
**After:** `docker compose up` starts `embyr-server` and Postgres together, using the same root
`Dockerfile` CI already validates, with working (clearly dev-only) default credentials.
**Decision enabled by:** A new contributor or evaluator decides whether embyr is worth adopting
based on a real, running local instance reached in minutes — not a decision blocked on reading
Rust source to reverse-engineer a config contract.

#### Problem
Sam Chen (or any new contributor evaluating embyr for the first time) wants to see the full
stack running locally to try it out or develop against it, but finds it needlessly effortful:
Postgres must be started separately, three environment variables (`DATABASE_URL`,
`EMBYR_ADMIN_KEY`, `EMBYR_ENCRYPTION_KEY`) must be discovered by reading source code and
hand-crafted correctly (the encryption key specifically must be exactly 64 hex characters), and
there is no single committed command that ties it together.

#### Who
- Sam Chen (P2) | Service operator evaluating embyr for a new deployment, or a new contributor
  onboarding | Needs a single command that produces a running, healthy local stack without
  reading application source code first.

#### Solution
A `docker-compose.yml` at the repository root that builds `embyr-server` from the existing root
`Dockerfile` (no second build definition) and starts it alongside a Postgres 15 container, with
working dev-only default values for the three required env vars, and a named volume so data
survives `docker compose down`/`up` cycles.

#### Domain Examples

**Example 1 (Happy path):** Sam clones the repository fresh and runs `docker compose up`.
Within roughly two minutes, `embyr-server` is listening on `:8080`/`:8081`/`:9090`, backed by a
Postgres 15 container on the compose network, using pre-filled dev-only default credentials.

**Example 2 (Iteration):** Sam edits a config default, runs `docker compose up --build`. The
image rebuilds via the existing `Dockerfile`; Postgres data persists across the restart via a
named volume — the test project Sam provisioned earlier is still there.

**Example 3 (Clean reset):** Sam runs `docker compose down -v` to discard all local state, then
`docker compose up` again, landing on a freshly migrated, empty database — useful for testing
migrations from zero.

**Example 4 (Port conflict, boundary):** Sam already has a local Postgres bound to `5432`.
Compose exposes Postgres on host port `5433` by default (documented in a comment) so
`docker compose up` does not silently fail to bind on a machine with an existing Postgres.

#### UAT Scenarios (BDD)

```gherkin
Scenario: Sam starts the full stack with one command
  Given a fresh clone of the repository with Docker installed
  When Sam runs "docker compose up"
  Then GET :9090/healthz returns HTTP 200 within 2 minutes
  And Postgres is reachable on the compose network with no manual setup

Scenario: Data persists across restarts
  Given the stack is running and Sam has provisioned a test project via the admin API
  When Sam runs "docker compose down" followed by "docker compose up"
  Then the previously provisioned project still exists
  And migrations do not re-run destructively

Scenario: Sam resets to a clean environment
  Given the stack has accumulated test data
  When Sam runs "docker compose down -v"
  Then the next "docker compose up" starts from an empty, freshly migrated database

Scenario: Compose reuses the production Dockerfile
  Given docker-compose.yml's "build" section points its "dockerfile" key at the repository's own
    root Dockerfile (not a copy or a compose-specific variant)
  When Sam runs "docker compose build"
  Then the build output shows the same stage names (chef, planner, builder, runtime) CI's own
    "docker build" job already produces
  And no second Dockerfile or inline "build.context" Dockerfile-equivalent exists anywhere in
    the repository

Scenario: Default dev credentials are clearly non-production
  Given Sam has not overridden any environment variables
  When Sam inspects docker-compose.yml
  Then EMBYR_ADMIN_KEY and EMBYR_ENCRYPTION_KEY default values are labeled as dev-only placeholders
  And a comment warns against reusing them in any shared or production environment
```

#### Acceptance Criteria
- [ ] `docker-compose.yml` at the repository root starts `embyr-server` + Postgres 15 with a
      single `docker compose up`
- [ ] Compose builds `embyr-server` using the existing root `Dockerfile` — no divergent build
      definition
- [ ] Default env values for `EMBYR_ADMIN_KEY`/`EMBYR_ENCRYPTION_KEY` are present, functional,
      and clearly commented as dev-only, non-production values
- [ ] Postgres data persists across `docker compose down` / `up` via a named volume
- [ ] `docker compose down -v` fully resets to an empty, re-migratable database
- [ ] `GET :9090/healthz` returns HTTP 200 within 2 minutes of `docker compose up` on a clean
      checkout

#### Outcome KPIs
- **Who:** New contributors and prospective self-hosting evaluators
- **Does what:** Reach a running, healthy local `embyr-server` + Postgres stack
- **By how much:** Time-to-first-healthy-response drops from unbounded/undocumented (must read
  source to discover config contract) to under 2 minutes with one command
- **Measured by:** Manual timing on a clean checkout during the next onboarding
- **Baseline:** 0% — no `docker-compose.yml`, `README`, or `Makefile` exist; no contributor can
  start the stack without reading application source first

#### Technical Notes
- Reuses the existing root `Dockerfile` unchanged — compose is a thin orchestration layer, not a
  second build definition.
- Does not touch `crates/embyr-agent/Dockerfile` or any agent orchestration — the agent runs in
  the customer's own VPC/Kubernetes cluster (ADR-001), never on the operator's own compose stack.
  Out of scope for this story.
- No hosting/cloud-provider decision required — strictly local-machine tooling, orthogonal to the
  deferred K8s/Helm/ECS question (see Scope Decision).
- No dependency on US-DRP-01; independently shippable.

---

## Story Map

### User: Sam Chen (P2 — Service Operator / Platform Engineer)
### Goal: Know exactly what "the release" is, and run the full stack anywhere without writing new infrastructure

### Backbone (left-to-right user activities)

| Version | Tag & Document | Run Locally |
|---------|-----------------|-------------|
| Bump workspace version | Create git tag | `docker compose up` |
| See version at server startup | Write CHANGELOG entry | Data persists across restarts |
| | | Reset to clean state |

### Walking Skeleton

The thinnest end-to-end slice that gives "the release" a concrete meaning:
- **US-DRP-01**: this feature's own merge performs the first real bump (0.1.0 → 0.1.1) + tag
  (`v0.1.1`) + CHANGELOG entry + version-in-startup-log, proving the whole mechanism in one pass.

### Release 1: The Release Has a Name (US-DRP-01)

Outcome: Sam can answer "what's running" and "what changed" without git archaeology.

| Story | Effort |
|-------|--------|
| US-DRP-01 | 1 day |

### Release 2: Evaluate Anywhere Without Writing Infrastructure (US-DRP-02)

Outcome: A new contributor or evaluator reaches a healthy local stack in one command.

| Story | Effort |
|-------|--------|
| US-DRP-02 | 1 day |

### Priority Rationale

1. **US-DRP-01** first — it is the walking skeleton and the literal, most-quoted half of the
   finding ("crate version frozen at 0.1.0, no CHANGELOG, no git tags"). Zero dependencies,
   smallest possible slice that gives "the release" a repeatable meaning.
2. **US-DRP-02** second — independently valuable (closes the "no docker-compose" half of the
   finding) but lower urgency than US-DRP-01: local evaluation friction is a DX/adoption
   concern, not an active production-operations gap the way "we can't tell what's running" is.

---

## Scope Assessment

**PASS** — 2 user stories, 1 bounded context touched (composition-root/DevOps infrastructure:
`embyr-server` startup log + root `Cargo.toml`/`CHANGELOG.md`/`docker-compose.yml`), estimated 2
days total. Each story is independently deliverable with no dependency between them. Well under
the oversized thresholds (>10 stories, >3 bounded contexts, >5 integration points, >2 weeks).
K8s/Helm/ECS explicitly split out as a separate, deferred, hosting-decision-blocked
finding/candidate rather than forced into this feature's scope — see Scope Decision above.

---

## DoR Checklist

| Item | US-DRP-01 | US-DRP-02 |
|------|-----------|-----------|
| Problem statement clear, domain language | PASS | PASS |
| User/persona with specific characteristics | PASS (Sam Chen, P2) | PASS (Sam Chen, P2) |
| 3+ domain examples with real data | PASS (5 examples) | PASS (4 examples) |
| UAT scenarios in Given/When/Then (3-7) | PASS (5 scenarios) | PASS (5 scenarios) |
| AC derived from UAT | PASS | PASS |
| Right-sized (1-3 days, 3-7 scenarios) | PASS (1 day) | PASS (1 day) |
| Technical notes: constraints/dependencies | PASS | PASS |
| Dependencies resolved or tracked | PASS (none) | PASS (none) |
| Outcome KPIs defined with measurable targets | PASS | PASS |
| job_id present | PASS (JOB-13) | PASS (JOB-13) |
| Elevator Pitch present | PASS | PASS |

**Overall DoR: PASS — both stories ready for DESIGN wave**

---

## Outcome KPIs (Feature Level)

### Objective
Give "the release" of embyr-server a concrete, repeatable meaning, and let anyone run the full
stack locally in minutes without reading source code — both without making any hosting/cloud
decision this session has no evidence to support yet.

### Metric Table

| # | Who | Does What | By How Much | Baseline | Measured By | Type |
|---|-----|-----------|-------------|----------|-------------|------|
| 1 | Sam Chen | Identifies the running version and what changed since a prior deploy | Under 2 minutes (from unbounded git archaeology) | 0% — no tag/CHANGELOG/version ever existed | Manual timing during next incident/rollback | Leading |
| 2 | Repository maintainers | Merge non-docs-only changes with a CHANGELOG entry + tag | 100% going forward | 0% | Presence of CHANGELOG.md diff + matching git tag per release-worthy merge | Leading |
| 3 | New contributors / evaluators | Reach a healthy local stack | Under 2 minutes with one command | Unbounded/undocumented | Manual timing on clean checkout | Leading |

### Guardrail Metrics
- `embyr-core` IO-free invariant must NOT be violated by either story
- The existing root `Dockerfile` remains the single source of truth for the production image —
  no second, divergent build definition introduced by `docker-compose.yml`
- No orchestrator-specific (K8s/Helm/ECS) artifacts introduced by this feature

### North Star
Any commit on `master` that ships to production carries a version, a git tag, and a CHANGELOG
entry — "the release" is no longer "whatever's on master when someone runs `docker build`."

---

## Wave Decisions Log

**2026-09-15** — Reused JOB-13 (no new job). Confirmed via direct codebase investigation (not
assumption) that the audit's finding #19 claims are all still accurate: no K8s/Helm/ECS/compose,
no CHANGELOG, no git tags, version frozen at 0.1.0, CI does nothing release-shaped. Confirmed
`embyr-agent-release-pipeline` (ADR-074) built no versioning/tagging convention despite building
a full CI/artifact pipeline for a different binary — this feature is genuinely first-of-its-kind,
not a duplicate. Split the finding into two independently-shippable, hosting-decision-independent
slices (release process; local-eval compose) and explicitly deferred K8s/Helm/ECS pending a
hosting/cloud-provider decision this repo has not made — recommended as a separate future
finding/candidate (`production-hosting-topology`) rather than folded into this feature. No
DIVERGE wave run (infrastructure-shaped stories enabling an already-validated job, same pattern
as `production-readiness`); DIVERGE absence noted as risk: none identified — the "give the
release a name" and "run it locally" needs are self-evident from the finding text and JOB-13's
own already-validated job story, no competing design directions to evaluate.

**Scope Assessment:** PASS — 2 user stories, 1 bounded context, 2 days total. No split required
beyond the deferral already applied to K8s/Helm/ECS.

**Follow-Up Work (explicitly out of scope for this feature):**
- `production-hosting-topology` (candidate, not yet filed as a job/feature) — the actual
  hosting/cloud-provider decision (self-hosted VM fleet vs. managed Kubernetes vs. ECS vs.
  something else) for `embyr-server`'s own SaaS backend. Once decided, K8s manifests / Helm
  chart / ECS task definitions become actionable, AND `backup-disaster-recovery-docs`'
  "pending hosting decision" RTO/RPO targets can be revisited and locked.
- Retroactive tagging/versioning of this session's ~40 already-merged features — explicitly not
  attempted; convention applies forward only from this feature's own merge.

---

## Wave: DESIGN
## Date: 2026-09-15
## Status: Ready for DISTILL wave
## Architect: Morgan (nw-solution-architect)

---

## Scope-Sizing Note

Both stories are docs/tooling-shaped: zero new Rust logic beyond one structured-log field, zero
new dependencies, zero schema change, zero new bounded context. DESIGN effort is scoped
accordingly — no C4 diagrams (no new component/container is introduced; both stories extend
existing, already-diagrammed infrastructure from ADR-017), no new ports/adapters, no ADR (see
ADR Decision below).

---

## Existing-System Findings (confirmed before designing)

| Question | Answer | Evidence |
|---|---|---|
| Is `version.workspace = true` already the pattern? | YES — already universal across every crate | Root `Cargo.toml` `[workspace.package] version = "0.1.0"`; `crates/embyr-server/Cargo.toml:3` `version.workspace = true`. One edit (root `Cargo.toml` line 6) bumps every crate at once. |
| Is a version-bump tool (`cargo-release`, `cargo-workspaces`, `cargo-smart-release`) already a dev-dependency? | NO | Grepped root `[workspace.dependencies]` (no `[dev-dependencies]` block exists at workspace level) and `crates/embyr-server/Cargo.toml`'s own `[dev-dependencies]` — neither lists any release-automation crate. |
| Does `embyr-server` already auto-run migrations at startup? | YES | `main.rs` startup sequence (top-of-file doc comment, ADR-017 D-PR-2): Step 5 `SystemDb::migrate()`, Step 6 `SystemDb::probe()` (hard gate). `sqlx::migrate!` tracks applied migrations in its own table — safe to re-run on every `docker compose up`, never destructive. This closes US-DRP-02's "migrations do not re-run destructively" AC with **zero new design** — the existing mechanism already satisfies it. |
| Where does the startup log line live today? | `crates/embyr-server/src/main.rs`, Step 12 (~line 382-388), `tracing::info!(grpc = .., rest = .., admin = .., "embyr-server ready")` | Read directly. |
| Are `DATABASE_URL`/`EMBYR_ADMIN_KEY`/`EMBYR_ENCRYPTION_KEY` the only *required* env vars? | YES | `config.rs::from_env()` — `collect_required("DATABASE_URL", ..)`, `EMBYR_ADMIN_KEY`/`EMBYR_ENCRYPTION_KEY` pushed to `missing` only via their own resolver fns. `STRIPE_SECRET_KEY` is explicitly optional (line 105 doc comment, `.ok()` at line 378) — compose does not need to set it. |
| Does CI already use a known-safe placeholder `EMBYR_ENCRYPTION_KEY`? | YES | `.github/workflows/ci.yml` line 33: `0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20` (64 hex chars). Reused verbatim in compose — avoids inventing a second "safe fake secret" value to maintain. |
| Did `embyr-agent-release-pipeline` (ADR-074) establish any versioning/CHANGELOG convention to reuse? | NO — confirmed absent, and its own Follow-Up Work section names finding #19 as the follow-up | `docs/evolution/2026-09-09-embyr-agent-release-pipeline.md`; artifact name is target-qualified (`embyr-agent-x86_64-unknown-linux-musl`), not version-qualified. This feature is first-of-its-kind for the workspace; nothing to reuse, nothing to reconcile. |

---

## ADR Decision: No New ADR

**Confirmed, not defaulted.** Checked explicitly for a genuinely new architectural pattern in
either story:

- No new component, container, port, or adapter is introduced (C4 Container diagram for
  `embyr-server` is unchanged from ADR-017 — same 3 listeners, same `SystemDb`, same startup
  sequence, one new structured log field added to an existing log statement).
- No new technology is selected. `docker-compose.yml` orchestrates the existing, already-decided
  Dockerfile (ADR-017 D-PR-3) and `postgres:15` (already the workspace's only supported Postgres
  version per CI). SemVer/Keep-a-Changelog/git-tag are process conventions, not components.
- The one technology-shaped micro-decision (manual `Cargo.toml` edit vs. adding `cargo-release`/
  `cargo-workspaces` as a new dev-dependency) is resolved by the existing-tooling-first check
  above: **no automation tool exists yet, and a single `version.workspace = true` field means a
  manual one-line edit is strictly simpler than installing and configuring a new cargo subcommand
  for one field in one file.** This is a build-tooling choice with no quality-attribute trade-off
  (performance/security/reliability/scalability are all unaffected either way) — it does not meet
  the bar for an ADR (no alternatives worth failing forward from; revisit only if/when a second
  workspace member needs an independent version, which `version.workspace = true` explicitly
  precludes today).

Conclusion: this feature ships as a design note in this document, not a new
`docs/product/architecture/adr-08X-*.md` file.

---

## US-DRP-01 Design: Version / Tag / CHANGELOG / Startup Log

### 1. Where the version lives

Unchanged location, first real bump: root `Cargo.toml` line 6, `[workspace.package] version =
"0.1.0"` → `"0.1.1"` for this feature's own merge. Every crate inherits it via
`version.workspace = true` — no per-crate edits required.

### 2. Bump procedure

**Manual edit + PR — no new tooling.** Confirmed above: no `cargo-release`/`cargo-workspaces`
dev-dependency exists, and adding one for a single shared workspace version field is not
justified (OSS-first still applies, but "adopt a new dependency" is not the simplest solution
when "edit one line" already works). Procedure, to be captured verbatim in
`docs/operations/release-process.md`:

1. In the same PR that changes `crates/`, `migrations/`, `Dockerfile`, or
   `crates/embyr-agent/Dockerfile`, bump `version` in root `Cargo.toml` `[workspace.package]`
   (SemVer: MAJOR = breaking wire/API contract, MINOR = new capability/backward-compatible,
   PATCH = fix or internal change).
2. Add a new `## [X.Y.Z] - YYYY-MM-DD` section to `CHANGELOG.md`, above the previous release,
   below `## [Unreleased]`, describing the change and citing the finding/feature that motivated
   it (matches this session's own evolution-doc citation convention).
3. On merge to `master`, a maintainer runs `git tag vX.Y.Z <merge-sha> && git push --tags`.
4. `docs/`-only PRs skip all three steps (explicitly exempted per US-DRP-01 AC).

### 3. CHANGELOG.md — exact format and location

- **Location:** repository root, `CHANGELOG.md` (sibling to root `Cargo.toml`, standard
  convention the tool ecosystem — GitHub, `cargo-release` itself, Keep a Changelog — expects).
- **Format:** [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) 1.1.0 — industry-standard
  format, avoids inventing a bespoke schema Sam Chen (or a future operator) would need to
  relearn. Structure:

```markdown
# Changelog

All notable changes to embyr-server are documented here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versioning follows
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.1] - 2026-09-16

### Added
- Versioning/tagging/CHANGELOG release process (finding #19). `embyr-server`'s startup log now
  reports its own version.
```

- **Update mechanism:** manual, per-PR discipline documented in `release-process.md` — no
  automation. Right-sized for team size (Sam Chen, solo operator per JOB-13) and this session's
  own established pattern (every other finding closure this session used a hand-written evolution
  doc, not generated tooling output).

### 4. Startup log — exact code change

File: `crates/embyr-server/src/main.rs`, Step 12 (current lines ~382-388). Add one field
(`version`) sourced from the compiler-provided `env!("CARGO_PKG_VERSION")` (zero new
dependency — this is a Rust std/cargo built-in), and prefix the human-readable message with
`v{version}` to match the `vX.Y.Z` git tag format exactly (`CARGO_PKG_VERSION` itself resolves to
`"0.1.1"`, without the `v` prefix — the message text adds it):

```rust
// ── Step 12: log ready ────────────────────────────────────────────────
tracing::info!(
    version = env!("CARGO_PKG_VERSION"),
    grpc = %format!("0.0.0.0:{}", cfg.grpc_port),
    rest = %format!("0.0.0.0:{}", cfg.rest_port),
    admin = %format!("0.0.0.0:{}", cfg.admin_port),
    "embyr-server v{} ready",
    env!("CARGO_PKG_VERSION")
);
```

Produces exactly the log line named in US-DRP-01 Example 2: `embyr-server v0.1.1 ready
grpc=0.0.0.0:8080 rest=0.0.0.0:8081 admin=0.0.0.0:9090` (plus a structured `version="0.1.1"`
field for log-pipeline queries). This is the only production-code change in this feature — one
field, one dependency-free macro, no new port/adapter/module. Implementation and its unit/
acceptance test belong to DISTILL/DELIVER, not this DESIGN doc.

### 5. Git tag convention — CI change or documented human procedure?

**Decision: documented human procedure only. No CI change in this feature.**

Justification:
- No git tag has ever existed in this repository — there is no historical drift to guard against
  yet, and a CI gate validating "tag matches `Cargo.toml` version" needs a `on: push: tags:`
  trigger workflow that does not exist today. Building that workflow before a single real tag has
  been pushed is automation ahead of evidence (this session's own recurring lesson: build for a
  demonstrated need, not a hypothetical one).
- JOB-13's persona (Sam Chen) is a single operator, not a multi-committer team where tag/version
  drift is a realistic near-term risk. Team-size-1 does not justify CI enforcement overhead yet
  (mirrors this DESIGN doc's own "team <10, time-to-market" framing).
- US-DRP-01's AC only requires the convention to exist and to be proven once (this feature's own
  merge) — it does not require future non-compliance to be *mechanically* blocked. Over-building
  enforcement beyond the stated AC is scope creep for a docs/tooling-shaped feature.
- **Revisit trigger (documented in `release-process.md`, not built now):** if a future release
  ships without a matching tag or CHANGELOG entry, add a `release-tag-check` CI job (triggered on
  `push: tags: ['v*']`, comparing the tag to `cargo metadata`'s workspace version) at that point —
  the mechanism is cheap to add later and not worth building speculatively today.

---

## US-DRP-02 Design: docker-compose.yml

### Services needed

Exactly two: `embyr-server` (built from the existing root `Dockerfile`, unchanged) and
`postgres` (image `postgres:15`, matching CI's own service container). No second Postgres
instance for "System DB vs. customer DB" — `embyr-server`'s own Walking Skeleton / CI convention
already runs both roles against the same local Postgres instance in different databases when
needed; a local single-Postgres-container eval stack does not need to simulate the
customer's-own-database topology (that's the `direct_pg`/`agent`/cloud-secret backend modes,
irrelevant to "can I see it run locally").

### Exact env vars and ports (mirrored from `config.rs` + CI, not invented)

| Var | Compose value | Source of truth |
|---|---|---|
| `DATABASE_URL` | `postgres://postgres:postgres@postgres:5432/embyr` | `postgres` is the compose service's own DNS name on the compose network |
| `EMBYR_ADMIN_KEY` | `dev-only-admin-key-do-not-use-in-production` | New placeholder, clearly labeled non-production per AC |
| `EMBYR_ENCRYPTION_KEY` | `0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20` | Reused verbatim from `.github/workflows/ci.yml` line 33 — already a known-safe, already-public 64-hex-char fixture value; reusing it avoids maintaining a second fake secret |
| Ports | `8080` (gRPC), `8081` (REST/gRPC-Web), `9090` (Admin/`/healthz`) | `Dockerfile` `EXPOSE 8080 8081 9090`, matches `cfg.grpc_port`/`cfg.rest_port`/`cfg.admin_port` defaults |

### Exact `docker-compose.yml` content (specification — DISTILL/DELIVER creates the file)

```yaml
# Local/single-host evaluation stack (finding #19, US-DRP-02).
# Builds embyr-server from the repository's own root Dockerfile — no second,
# divergent build definition. NOT for production use: EMBYR_ADMIN_KEY and
# EMBYR_ENCRYPTION_KEY below are dev-only placeholders.
services:
  postgres:
    image: postgres:15
    environment:
      POSTGRES_USER: postgres
      POSTGRES_PASSWORD: postgres   # dev-only placeholder — do not reuse in any shared/production environment
      POSTGRES_DB: embyr
    ports:
      - "5433:5432"   # host 5433 (not 5432) avoids clashing with a locally-installed Postgres
    volumes:
      - embyr_pgdata:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U postgres"]
      interval: 5s
      timeout: 5s
      retries: 10

  embyr-server:
    build:
      context: .
      dockerfile: Dockerfile
    depends_on:
      postgres:
        condition: service_healthy
    ports:
      - "8080:8080"
      - "8081:8081"
      - "9090:9090"
    environment:
      DATABASE_URL: postgres://postgres:postgres@postgres:5432/embyr
      EMBYR_ADMIN_KEY: dev-only-admin-key-do-not-use-in-production          # dev-only placeholder — do not reuse in any shared/production environment
      EMBYR_ENCRYPTION_KEY: 0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20   # dev-only placeholder — 64 hex chars, mirrors ci.yml's own fixture value

volumes:
  embyr_pgdata:
```

### How the ACs are satisfied by this design, not new code

- **Data persists across restarts (`down` / `up`):** named volume `embyr_pgdata` — default
  compose behavior, no extra step.
- **`down -v` resets cleanly:** `-v` removes named volumes — default compose behavior.
- **Migrations "do not re-run destructively":** already true today, for free — `main.rs` Step 5
  (`SystemDb::migrate()`) uses `sqlx::migrate!`, which tracks applied migrations in its own table
  and is idempotent on every container start. Zero new design.
- **`healthz` reachable within 2 minutes:** `depends_on: condition: service_healthy` on Postgres
  plus the existing fail-fast startup sequence (ADR-017) — `embyr-server` will not attempt to
  bind its listeners until Postgres is ready, and binds all three ports or none (D-PR-6).
- **Compose reuses the production Dockerfile, no second build definition:** `build.dockerfile:
  Dockerfile` points at the existing root file — enforced by inspection at DISTILL time (`grep`
  for a second `FROM` chain / compose-specific Dockerfile is a 1-line CI or review check, not new
  architecture).

### Explicitly out of scope for this design

- `crates/embyr-agent/Dockerfile` / any agent orchestration in compose (ADR-001: the agent runs
  in the *customer's* VPC, never on the operator's own compose stack).
- Any `.env` file — env vars are declared inline in `docker-compose.yml`'s own `environment:`
  block; a separate `.env` adds a file and an override-precedence rule for zero benefit at this
  scale (ponytail: one file beats two).

---

## External Integration / Contract-Test Annotation

**None.** Neither story touches an external API or third-party service boundary. `postgres` in
compose is a local container, not an external integration. No contract-test annotation applies
to this handoff.

---

## Quality Gate Checklist (self-validated)

- [x] Requirements traced to components — US-DRP-01 → `Cargo.toml`/`CHANGELOG.md`/
      `release-process.md`/`main.rs` Step 12; US-DRP-02 → `docker-compose.yml`.
- [x] Component boundaries — none introduced; both stories operate on existing artifacts (root
      manifest, existing Dockerfile, existing startup sequence).
- [x] Technology choices justified — `postgres:15` (already CI's own version), no new dep for
      versioning (existing-tooling-first check documented above).
- [x] Quality attributes addressed — Maintainability (CHANGELOG/tag traceability), Usability
      (one-command local eval), Portability (compose reuses the one Dockerfile everywhere).
- [x] Dependency-inversion compliance — N/A, no new port/adapter.
- [x] C4 diagrams — N/A per Scope-Sizing Note (no new component/container; ADR-017's existing
      diagrams still describe the system accurately).
- [x] Integration patterns specified — compose service-to-service DNS (`postgres` hostname),
      `depends_on: condition: service_healthy`.
- [x] OSS preference validated — Postgres (PostgreSQL License), Docker Compose (Apache 2.0);
      zero new dependencies added.
- [x] AC behavioral, not implementation-coupled — verified against US-DRP-01/02 AC directly.
- [x] External integrations annotated — none exist (see above).
- [x] Enforcement tooling — N/A for a process convention; CI enforcement explicitly deferred with
      documented revisit trigger (§ Git tag convention above).
- [ ] Peer review completed and approved — pending `solution-architect-reviewer` invocation below.

---

## Handoff to DISTILL (acceptance-designer)

Both stories are ready for BDD/acceptance-test design:

- **US-DRP-01**: acceptance test proves the mechanism end-to-end — bump `Cargo.toml` version,
  add `CHANGELOG.md` entry, create git tag, assert the startup log line (structured `version`
  field + message text) matches `CARGO_PKG_VERSION`. No new port/adapter to stub — this is an
  integration/process proof, closest in shape to `production_readiness`'s own startup-sequence
  tests.
- **US-DRP-02**: acceptance-test shape is `docker compose config` (static validation, safe on
  this machine's resource constraints — does not start containers) plus, if DISTILL/DELIVER
  chooses to exercise it live, a single `docker compose up` + `GET :9090/healthz` + `down -v`
  round trip. Do not run this against `--workspace` cargo targets; it is a docker-only check,
  orthogonal to the Rust test suite.
- No external integrations in either story — no contract-test annotation needed for
  platform-architect.

---

## Wave: DISTILL
## Date: 2026-09-15
## Status: Ready for DELIVER wave
## Acceptance Designer: Quinn (nw-acceptance-designer)

---

## Scope-Sizing Note (right-sized per orchestrator instruction)

This feature has zero new Rust modules and zero new ports/adapters (confirmed in DESIGN).
Mandate 7 scaffolding (stub production modules for not-yet-implemented imports) does not
apply — no acceptance test in this feature imports a new production module. US-DRP-01's test
drives the existing `embyr-server` binary as a subprocess (same driving-port mechanism already
in the Project Infrastructure Policy for `production-readiness`); US-DRP-02's tests read/exec
`docker-compose.yml`/`docs/operations/release-process.md`/`CHANGELOG.md` directly — no Rust
import at all. Effort scoped down accordingly: no Tier B (config/docs-shaped, ≤2 scenarios per
journey — Mandate 10's own "skip Tier B" criteria), no new Project Infrastructure Policy rows
(the `embyr-server` binary subprocess row already covers this feature's one real driving-port
test), plain `assert!`-with-message assertions instead of `assert_state_delta`/Universe (see
Wave-Boundary Decision below for why).

---

## Wave-Boundary Decision: DISTILL writes tests only, not the deliverables themselves

Considered writing `docker-compose.yml`/`CHANGELOG.md`/`docs/operations/release-process.md`
directly during DISTILL (the orchestrator's instructions explicitly allowed this for a
docs/tooling-shaped feature, citing `backup-disaster-recovery-docs` as precedent). Decided
**against** it, for a reason `backup-disaster-recovery-docs` itself doesn't share:

- `backup-disaster-recovery-docs` skipped DISTILL entirely (pipeline adaptation) because its
  deliverable (a Markdown runbook) had no automatable RED→GREEN transition — "nothing to
  mutation-test in a markdown file," per that feature's own evolution doc.
- This feature's three deliverables DO have a genuine, cheap, automatable RED→GREEN transition:
  a real subprocess log-capture assertion (US-DRP-01) and real file-shape/`docker compose config`
  assertions (US-DRP-02) — confirmed by RED-running all 12 tests against the current, pre-DELIVER
  codebase (see RED Verification below): every test fails for a real, correct reason (artifact
  genuinely absent / version field genuinely absent from the log line), none fails from a test
  bug or import error.
- Writing the final, correct file content now would erase that RED→GREEN signal for zero
  benefit — DELIVER would have nothing left to do, and the acceptance suite would never have
  proven anything. Per ADR-025, DISTILL's job is the scaffolded RED, not the GREEN.

Conclusion: normal DISTILL/DELIVER separation holds. DISTILL wrote 5 test files + 1 test-harness
file + 1 `Cargo.toml` `[[test]]` registration; it did NOT write `docker-compose.yml`,
`CHANGELOG.md`, or `docs/operations/release-process.md` — those are DELIVER's GREEN work, using
the exact content DESIGN already specified verbatim.

---

## [REF] Scenario list with tags

| # | Scenario | File | Tags | Status |
|---|---|---|---|---|
| 1 | Sam identifies exactly which release is running (startup log names version) | `drp01_startup_version_log.rs::startup_log_names_the_running_version` | `@walking_skeleton @driving_port @real-io @US-DRP-01` | RED (not `#[ignore]`) |
| 2 | CHANGELOG.md exists in Keep a Changelog format with `[Unreleased]` | `drp01_changelog_structure.rs::changelog_exists_in_keep_a_changelog_format` | `@US-DRP-01` | RED, `#[ignore]` |
| 3 | A merged change to server code is captured in the release history | `drp01_changelog_structure.rs::changelog_gains_a_dated_entry_for_this_features_own_bump` | `@US-DRP-01` | RED, `#[ignore]` |
| 4 | Sam finds what changed between two deployments (entry shape) | `drp01_changelog_structure.rs::every_dated_entry_has_a_valid_date_and_a_description` | `@US-DRP-01` | RED, `#[ignore]` |
| 5 | Doc states bump triggers + docs-only exemption | `drp01_release_process_doc.rs::documents_which_changes_require_a_version_bump` | `@US-DRP-01` | RED, `#[ignore]` |
| 6 | Doc states git tag convention (`vMAJOR.MINOR.PATCH`, merge commit) | `drp01_release_process_doc.rs::documents_the_git_tag_convention` | `@US-DRP-01` | RED, `#[ignore]` |
| 7 | Sam rolls back to a known-good release (doc describes procedure) | `drp01_release_process_doc.rs::documents_the_rollback_procedure` | `@US-DRP-01` | RED, `#[ignore]` |
| 8 | Compose config validates without starting containers | `drp02_compose_structure.rs::compose_config_validates_without_starting_containers` | `@US-DRP-02` | RED, `#[ignore]` |
| 9 | Compose reuses the production Dockerfile (no second Dockerfile) | `drp02_compose_structure.rs::compose_builds_embyr_server_from_the_one_root_dockerfile` | `@US-DRP-02` | RED, `#[ignore]` |
| 10 | Compose defines both services with right ports/env/volume | `drp02_compose_structure.rs::compose_defines_required_services_ports_and_volume` | `@US-DRP-02` | RED, `#[ignore]` |
| 11 | Default dev credentials are clearly non-production | `drp02_compose_structure.rs::dev_credentials_are_labeled_non_production` | `@US-DRP-02` | RED, `#[ignore]` |
| 12 | Sam starts stack / data persists across restart / resets to clean env (chained lifecycle) | `drp02_compose_lifecycle.rs::compose_up_restart_and_reset_round_trip` | `@walking_skeleton @driving_port @real-io @adapter-integration @US-DRP-02` | RED, `#[ignore]` |

12 tests map onto all 10 UAT scenarios from DISCUSS (US-DRP-01's 5 + US-DRP-02's 5); scenario 12
folds 3 of US-DRP-02's UAT scenarios into one chained lifecycle test (Pillar 2) to minimize
Docker container churn on the 8GB-RAM constrained machine, rather than standing up 3 separate
compose stacks for what is genuinely one continuous journey (up → provision → restart → verify
persistence → reset → verify emptiness). Error/edge ratio: this is a docs/tooling feature with
no business-rule branching to speak of — the "40%+ error path" mandate is not a good fit (there
is no error path in "write a CHANGELOG" or "run docker compose up"); the closest analogs
(structural/negative-shape checks: missing sections, missing services, missing dev-only labels)
are exactly the RED state every non-walking-skeleton test already starts from.

---

## [REF] Walking Skeleton strategy

Two walking skeletons (one per independently-shippable story), matching Mandate 5's 2-5-per-feature
allowance:

- **US-DRP-01**: `startup_log_names_the_running_version` — real subprocess + real Postgres
  testcontainer, NOT `#[ignore]`. Cheap (~6s), safe to leave enabled by default.
- **US-DRP-02**: `compose_up_restart_and_reset_round_trip` — real `docker compose` I/O, 2
  containers, `#[ignore]` (resource-constraint override: this one is NOT run by default even
  though it's the walking skeleton, because it needs 2 concurrent containers on an 8GB-RAM
  Docker VM that reserves 4GB fixed — explicitly documented in the test's own module doc comment
  and in `mod.rs`). RED-verified cheaply: with `docker-compose.yml` absent, `docker compose up`
  fails in ~1s before starting any container.

---

## [REF] Adapter coverage table

| Adapter / artifact | `@real-io` scenario | Covered by |
|---|---|---|
| `embyr-server` binary (subprocess) — existing port, reused | YES | `drp01_startup_version_log.rs` walking skeleton |
| `docker-compose.yml` (Docker Compose CLI) | YES | `drp02_compose_structure.rs::compose_config_validates_...` (static) + `drp02_compose_lifecycle.rs` (live, `#[ignore]`) |
| `CHANGELOG.md` (file artifact, no adapter) | N/A — plain file read | `drp01_changelog_structure.rs` |
| `docs/operations/release-process.md` (file artifact, no adapter) | N/A — plain file read | `drp01_release_process_doc.rs` |

No new driven adapters introduced by this feature (confirmed in DESIGN) — the only adapter-shaped
surface is the Docker Compose CLI itself, covered both statically (`config`) and live (`up`/`down`).

---

## [REF] Scaffolds

None created. Mandate 7 scaffolding applies to production modules newly imported by acceptance
tests; this feature's tests import zero new production modules (see Scope-Sizing Note above).
The only "implementation absent" surface is: (a) one field on an existing `tracing::info!` call
in `main.rs` (edited, not created), and (b) three new non-Rust artifacts (`docker-compose.yml`,
`CHANGELOG.md`, `docs/operations/release-process.md`) whose absence is itself the RED signal —
no stub needed, `std::fs::read_to_string` failing IS the RED state.

---

## [REF] Test placement

`tests/deployment_release_process/{mod.rs, common/mod.rs, acceptance/*.rs}`, registered as
`[[test]] name = "deployment_release_process"` in `crates/embyr-server/Cargo.toml`. Precedent:
mirrors `tests/production_readiness/` layout exactly (mod.rs module root + common/ harness +
acceptance/ one-file-per-AC-cluster). `common/mod.rs` is a trimmed duplicate of
`tests/production_readiness/common/mod.rs::ServerProcess` (Rust test binaries don't share code
across `[[test]]` targets without a dedicated support crate — established precedent in this
exact repo, not a new decision).

---

## [REF] Driving Adapter coverage

The one driving adapter DESIGN specifies for this feature (`embyr-server` binary subprocess) is
exercised via its real invocation path (spawn, env vars, stdout/stderr capture, SIGTERM) in
`drp01_startup_version_log.rs` — same mechanism, not re-derived, as the already-covered
`production-readiness` feature. `docker compose` (the CLI a contributor actually types) is
exercised both statically (`config`) and live (`up`/`down`/`down -v`) — not just file-content
assertions — satisfying "invoke via the user's actual invocation path," not just "the YAML
parses."

---

## [REF] Pre-requisites

- DESIGN's exact `docker-compose.yml` content, `main.rs` Step 12 diff, and
  `release-process.md`/`CHANGELOG.md` format (feature-delta.md §§ US-DRP-01/02 Design) — DELIVER
  implements these verbatim; the acceptance tests assert against that exact shape (port numbers,
  volume name, env var names, tag format).
- Docker daemon available locally for `drp02_compose_structure.rs`/`drp02_compose_lifecycle.rs`
  (graceful skip via `docker_available()` for the static check; the live lifecycle test is
  `#[ignore]` and assumed to be run deliberately, not in the default sweep).
- No DEVOPS wave artifacts exist for this feature (none were produced — feature-delta.md has no
  `## Wave: DEVOPS` section); default environment assumptions applied per the Graceful
  Degradation Matrix (warn, not block).
- No `docs/product/architecture/brief.md`/`kpi-contracts.yaml` consulted beyond the existing
  `atdd-infrastructure-policy.md` — this project's established convention (confirmed across ~50
  prior features this session) uses ADR-based `docs/product/architecture/adr-*.md` plus
  `feature-delta.md` as the DESIGN SSOT, not a separate `brief.md`/KPI-contract file; the
  DESIGN section already embedded in this same document supplies driving-port and journey
  context directly. Logged as a soft warning per the Graceful Degradation Matrix, not a block.

---

## RED Verification (pre-DELIVER fail-for-the-right-reason gate)

Ran the full suite twice against the current, pre-DELIVER codebase:

1. **Default run** (`cargo test -p embyr-server --test deployment_release_process --
   --test-threads=1`): 1 test executed (the walking skeleton, not `#[ignore]`), 11 skipped.
   Result: **FAILED** — `startup log must report the running version (0.1.0) as a structured
   'version' field` — MISSING_FUNCTIONALITY (the `version` field genuinely does not exist in
   `main.rs`'s current Step 12 log line). Correct RED.
2. **Ignored run** (`-- --ignored --test-threads=1`): 11 tests executed. Result: **11/11
   FAILED**, every one at a `panic!`/`assert!` inside the test's own file-read or
   `docker compose config` call — `docker-compose.yml`, `CHANGELOG.md`, and
   `docs/operations/release-process.md` are all genuinely absent from the repository today
   (confirmed by `ls` before writing any test). Zero import errors, zero compile errors, zero
   fixture-setup failures. All MISSING_FUNCTIONALITY. Correct RED.
3. Total wall time for both runs: ~7 seconds combined (the live-Docker walking skeleton for
   US-DRP-02 was NOT run in this RED pass — `#[ignore]`d by design; its RED state was confirmed
   separately and cheaply: `docker compose up` against the absent file fails in ~1s with no
   container ever started, per the test's own module doc comment).
4. Zero testcontainers leaked (`docker ps -a --filter label=org.testcontainers=true` empty after
   both runs).

Classification: 12/12 tests are genuine RED (`MISSING_FUNCTIONALITY`), 0/12 are
`IMPORT_ERROR`/`FIXTURE_BROKEN`/`SETUP_FAILURE`/`WRONG_ASSERTION`. Gate passes — safe to hand off
to DELIVER.

---

## Mandate Compliance Evidence

- **CM-A** (Mandate 1, hexagonal boundary): every test invokes through a driving port — the
  `embyr-server` binary subprocess (US-DRP-01) or the `docker compose` CLI (US-DRP-02) — never an
  internal Rust component. Zero `use embyr_server::...` internal-module imports appear in any of
  the 5 acceptance test files (confirmed by inspection — only `crate::common::*` and stdlib/dev-dep
  imports).
- **CM-B** (Mandate 2, business language): scenario/test names use domain terms (`startup log
  names the running version`, `data persists across restarts`, `dev-only credentials`) — no
  `assert response.status_code`-shaped tests; `docker compose config`/HTTP status checks live
  inside step bodies, not scenario names.
- **CM-C** (Mandate 3, journey completeness): each test carries Given/When/Then in its own doc
  comment tracing back to the exact UAT scenario text in DISCUSS; the compose lifecycle test is
  an explicit 3-scenario chained journey (Pillar 2).
- **CM-D** (Mandate 4, pure-function extraction): N/A — no business logic to extract; every test
  is either a subprocess/file assertion or a static text-shape check. No fixture parametrization
  used.
- **CM-E** (Mandate 8, Universe/state-delta): NOT applied — deliberate, documented deviation.
  This exact layer (subprocess/FS acceptance) has an established, previously-reviewed precedent
  in this repo (`tests/production_readiness/acceptance/pr01_config_from_env.rs`) using plain
  `assert!`/descriptive-panic-message assertions, not `assert_state_delta`. Followed that
  precedent for consistency rather than introducing a second style for the identical layer in
  the same test suite family.
- **CM-F** (Mandate 9, PBT layer discipline): no PBT machinery (`proptest`, `#[given]`) used
  anywhere in this feature — correct, since every scenario here is layer 3+ (subprocess/FS
  acceptance) example-only territory, and none of the ACs express a quantifiable property (`for
  any valid X...`) — all are concrete "this exact file/line exists" checks.
- **CM-G** (Mandate 10, two-tier acceptance): Tier B correctly SKIPPED — config/docs-shaped
  feature per Mandate 10's own skip criteria (no journey has ≥3 chained scenarios with a
  domain-rich input space; the one 3-scenario chain that exists, the compose lifecycle test, has
  a narrow input space — one project ID, no free-text/date/payload variation — so state-machine
  PBT would add exploration cost with no corresponding gap-finding value).
- **CM-H** (Mandate 11, example-based sad paths): no sad-path/PBT conflation — every
  `#[ignore]`d test is a named, example-based, single-assertion-cluster test; none imports PBT
  machinery.

---

## Peer Review

**Reviewer:** Sentinel (nw-acceptance-designer-reviewer, Haiku)
**Verdict:** `approved` — 0 blockers, 0 high, 0 low findings.

All 8 mandates (CM-A through CM-H) scored `pass`. All 8 critique dimensions scored 9-10/10
(happy-path bias N/A-exceeded for a docs/tooling feature, GWT format, business language,
coverage completeness, walking-skeleton centricity, priority validation, observable-behavior
assertions, traceability, walking-skeleton boundary proof). Reviewer independently verified the
`pr01_config_from_env.rs` precedent cited for the Mandate 8 deviation is real and applies at the
identical layer. No revision cycle needed — approved on first pass.

Reviewer's two recommendations (both non-blocking, forward-looking): (1) this feature's
subprocess/Docker-layer pattern (plain `assert!` + resource-constraint `#[ignore]` markers) is
now a reusable template for future infra-shaped features; (2) the "N/N genuine RED" RED-
verification statement produced here is recommended as a standard DISTILL handoff artifact going
forward.

---

## Definition of Done — validated

1. [x] All acceptance scenarios written with passing (well-formed) RED step definitions — 12/12
       compile and run; 1 walking skeleton enabled, 11 `#[ignore]`d for one-at-a-time DELIVER.
2. [x] Test pyramid complete for this feature's shape — no unit-test layer needed (zero new pure
       logic beyond one log field); acceptance layer is the whole pyramid here, by design.
3. [x] Peer review approved (0 blockers/high/low).
4. [ ] Tests run in CI/CD pipeline — not yet; the new `[[test]]` target
       (`deployment_release_process`) will run automatically once `cargo test --workspace` (or
       CI's own per-crate invocation) picks it up post-DELIVER. No CI config change needed (new
       `[[test]]` entries are auto-discovered).
5. [ ] Story demonstrable to stakeholders — pending DELIVER (tests are RED by design).
6. [x] Project Infrastructure Policy present — inherited, no new row needed (existing
       `embyr-server` binary subprocess row reused verbatim).
7. [x] Target language detected and logged — `[lang-mode] rust` (via `Cargo.toml` marker).
8. [x] State-delta port present — `tests/common/state_delta.rs` already exists (inherited, no
       bootstrap needed this run).
9. [x] Wave-Decision Reconciliation HARD GATE passed — 0 contradictions (single-file
       feature-delta.md model, no separate discuss/design/devops wave-decision files to
       reconcile; DISCUSS→DESIGN read directly, no drift found).
10. [x] Mandate 8 — deliberately, explicitly NOT applied at this layer, per documented precedent
        (`pr01_config_from_env.rs`), confirmed real and applicable by peer review.
11. [x] Mandate 9 — no PBT machinery anywhere in this feature (correct — no property-shaped ACs).
12. [x] Mandate 10 — Tier B correctly absent (config/docs-shaped feature, narrow input space).
13. [x] Mandate 11 — N/A, no sad-path/PBT conflation risk in this feature (no PBT machinery at
        any layer to conflate).
14. [x] Pillar 1 — zero technical jargon in scenario/test names beyond the domain vocabulary
        appropriate to a DevOps/platform-engineer persona (Docker, CHANGELOG — confirmed
        legitimate domain terms, not implementation leakage, by peer review).
15. [x] Pillar 2 — chained narrative verified: `drp02_compose_lifecycle` chains 3 UAT scenarios
        (start → persist → reset) in one continuous journey.
16. [x] Pillar 3 — Tier A only; both walking skeletons use the real production entry point
        (embyr-server binary subprocess / real docker compose CLI), no in-memory composition
        root needed (no Tier B).

Items 4-5 are expected-incomplete at DISTILL handoff (they complete once DELIVER turns RED to
GREEN) — not gate failures.

## Handoff to DELIVER

Ready. DELIVER implements, in order (per `mod.rs`'s own Implementation order comment):
1. `drp01_startup_version_log` — edit `main.rs` Step 12 per DESIGN's exact diff.
2. `drp01_changelog_structure` — create `CHANGELOG.md` per DESIGN's exact format; bump root
   `Cargo.toml` to `0.1.1`.
3. `drp01_release_process_doc` — create `docs/operations/release-process.md`.
4. `drp02_compose_structure` — create `docker-compose.yml` per DESIGN's exact content.
5. `drp02_compose_lifecycle` — should already pass once `docker-compose.yml` is correct; run
   deliberately and in isolation given the resource constraint (never alongside another
   Docker/testcontainers-heavy run).

After DELIVER, a maintainer performs the human-only step DESIGN scoped out of CI: `git tag
v0.1.1 <merge-sha> && git push --tags`.

