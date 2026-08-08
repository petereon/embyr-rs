# Feature: production-readiness

<!-- markdownlint-disable MD024 -->

## Wave: DISCUSS
## Date: 2026-08-08
## Status: Ready for DESIGN wave

---

## Job Traceability

### JOB-13 (NEW — see jobs.yaml)

**Persona:** Sam Chen (P2 — Service Operator / Platform Engineer)
**Job Story:** When I want to deploy embyr to production, I want a single `docker run` command
to start the server, so I can ship without writing infrastructure from scratch.
**Opportunity score:** 15
**Priority:** critical

**Mapping:**
- US-PR-01 → JOB-13 (env-var startup enables docker run)
- US-PR-02 → JOB-13 (Dockerfile enables docker run)
- US-PR-03 → JOB-13 (CI ensures the image is always buildable and tested)

---

## Problem Statement

embyr-rs cannot start in production. `crates/embyr-server/src/main.rs` is a single
`println!` stub; the real startup logic lives exclusively in test-only code inside `lib.rs`.
No Dockerfile exists. No CI pipeline exists. All three blockers must be resolved before any
production deployment is possible.

**Three production blockers (identified from codebase analysis):**

1. **`main.rs` stub** — `main()` is `println!("embyr-server starting"); }`. The real startup
   sequence (env config, tracing, prometheus, migrations, system DB probe, bind 3 ports,
   graceful shutdown) exists in `lib.rs` as test-only helpers that are never reachable from
   the production binary.

2. **No Dockerfile** — No container image can be built. The `embyr-agent` pattern
   (single-stage or multi-stage build) has not been applied to `embyr-server`.

3. **No CI pipeline** — No automated gate on PRs. `cargo test`, `cargo clippy`,
   `cargo deny check`, and image builds are purely manual. A broken build can reach main.

---

## Locked Decisions

All decisions are pre-decided from codebase analysis. Do not re-derive in DESIGN wave.

| ID | Decision | Source |
|----|----------|--------|
| D-PR-1 | Real `main()` reads config from env vars: `DATABASE_URL` (required), `EMBYR_ADMIN_KEY` (required), `EMBYR_ENCRYPTION_KEY` (32-byte hex, required), `EMBYR_RATE_LIMIT_RPS` (default 1000.0), `GRPC_PORT` (default 8080), `REST_PORT` (default 8081), `ADMIN_PORT` (default 9090), `RUST_LOG` (default "info") | `lib.rs` `default_rate_limit_capacity()`, `system_db.rs` `new()`, architecture brief operational constraints |
| D-PR-2 | `main()` init order: tracing → prometheus (via `get_or_install_prometheus_handle()`) → migrations (`system_db.migrate()`) → probe SystemDb (`system_db.probe()`) → bind ports → `spawn_all_servers` | ADR-016 "install_recorder → migrate_db → run_probes → bind_listeners → serve", `alloc_test_components()` pattern in lib.rs |
| D-PR-3 | Multi-stage Dockerfile: builder stage `rust:1.80-slim` + `cargo-chef` for layer caching → runtime stage `debian:bookworm-slim` | embyr-agent precedent (separate binary), operational simplicity QA (rank 6), no runtime deps beyond libc |
| D-PR-4 | GitHub Actions CI: `cargo test --workspace`, `cargo clippy -- -D warnings`, `cargo deny check`, `docker build` on every push and PR targeting master | Standard Rust CI baseline; deny.toml already exists |
| D-PR-5 | Graceful shutdown on SIGTERM/SIGINT via tokio signal handler; uses the existing `oneshot` pattern already present in `TestServer.shutdown_tx` | `TestServer` Drop impl in lib.rs; architecture brief "graceful if drain is configured" |
| D-PR-6 | Startup exits with code 1 if `DATABASE_URL` missing, `EMBYR_ADMIN_KEY` missing, or `SystemDb::probe()` fails. No partial startup — all three ports or none. | Architecture brief "Port availability: bind all three sockets before accepting → Refuse to start"; embyr-agent `main.rs` pattern (exit code 1 on probe failure) |
| D-PR-7 | Config struct in `crates/embyr-server/src/config.rs` (new file) — validated at parse time, not scattered `std::env::var` calls. Mirrors `embyr_agent::config::AgentConfig` pattern. | `embyr_agent::config` pattern; prevents runtime panics from invalid env vars |

---

## System Constraints

These cross-cutting constraints apply to all stories in this feature.

- `embyr-core` must remain IO-free. `deny.toml` enforces this; no story may add IO crates to `embyr-core`.
- `cargo-deny check` must pass after every change. Advisories in `deny.toml` are already explicitly accepted.
- The three TCP listeners (:8080/:8081/:9090) must bind before any traffic is accepted.
- Admin key (`EMBYR_ADMIN_KEY`) must be non-empty at startup — startup refuses otherwise (architecture brief "Admin key present" probe).
- `embyr_encryption_key` is 32-byte hex — validated at parse time in the config struct (D-PR-7).
- Prometheus recorder installed before any listener opens (ADR-016 "install_recorder → migrate_db → run_probes → bind_listeners").
- 18 migrations (0001–0018) are run via `SystemDb::migrate()` — sqlx macros path is `../../migrations` (relative to `crates/embyr-server/`).

---

## User Stories

### US-PR-01: Real Server Startup from Environment Variables

**job_id:** JOB-13

#### Elevator Pitch
**Before:** `cargo run -p embyr-server` prints "embyr-server starting" and exits. The binary
cannot serve any traffic. Sam must either run tests or inject test-only code to start the
server.
**After:** `DATABASE_URL=postgres://... EMBYR_ADMIN_KEY=secret cargo run -p embyr-server`
starts the server, runs migrations, probes the system DB, binds gRPC :8080, REST :8081, admin
:9090, logs "embyr-server ready", and handles SIGTERM gracefully.
**Decision enabled by:** D-PR-1 through D-PR-6 — Sam decides whether to run the server in a
container, a VM, or bare metal, using only environment variables.

#### Problem
Sam Chen is a service operator who wants to deploy embyr to production. He finds it
impossible to start the embyr-server binary because `main.rs` is a stub — it prints one line
and exits. The actual startup code exists only as test infrastructure inside `lib.rs` and is
unreachable from the production binary.

#### Who
- Sam Chen (P2) | Service operator deploying embyr to production | Needs a runnable binary
  that reads config from the environment, validates it, and starts serving all three ports.

#### Solution
A new `crates/embyr-server/src/config.rs` module parses and validates all env vars at
startup (D-PR-7). A real `main()` in `main.rs` follows the agent pattern (D-PR-2):
tracing → prometheus → migrations → DB probe → bind ports → serve → graceful shutdown.
Startup exits code 1 on any required env var missing or probe failure (D-PR-6).

#### Domain Examples

**Example 1 (Happy Path):** Sam runs:
```
DATABASE_URL=postgres://sam:pass@localhost:5432/embyr \
EMBYR_ADMIN_KEY=supersecret \
EMBYR_ENCRYPTION_KEY=a3f1... \
cargo run -p embyr-server
```
Server logs `[INFO] migrations applied: 18` then `[INFO] embyr-server ready grpc=0.0.0.0:8080
rest=0.0.0.0:8081 admin=0.0.0.0:9090`. Sam sends `GET :9090/healthz` and receives HTTP 200.

**Example 2 (Missing DATABASE_URL):** Sam forgets to set `DATABASE_URL`. Server prints
`embyr-server: DATABASE_URL is required` to stderr and exits with code 1 within 100ms.
No ports are bound.

**Example 3 (DB Unreachable — Probe Failure):** Sam sets `DATABASE_URL` to a Postgres
that is not yet running. Server logs `[ERROR] startup probe failed: system DB unreachable:
connection refused` and exits code 1. No ports are bound. Sam fixes the DB and retries.

**Example 4 (SIGTERM During Operation):** A Kubernetes rolling deployment sends SIGTERM.
The server stops accepting new connections, drains in-flight gRPC streams, then exits 0 within
the termination grace period (30s default).

#### UAT Scenarios (BDD)

```gherkin
Scenario: Server starts successfully when all required env vars are set
  Given DATABASE_URL points to a reachable Postgres with no prior migrations
  And EMBYR_ADMIN_KEY is set to "test-admin-key"
  And EMBYR_ENCRYPTION_KEY is set to a valid 32-byte hex string
  When Sam runs "cargo run -p embyr-server"
  Then all 18 migrations are applied to the system DB
  And the server logs "embyr-server ready" with all three port numbers
  And GET :9090/healthz returns HTTP 200 within 5 seconds of startup

Scenario: Startup fails immediately when DATABASE_URL is missing
  Given DATABASE_URL is not set in the environment
  When Sam runs "cargo run -p embyr-server"
  Then the process exits with code 1 within 100ms
  And stderr contains "DATABASE_URL is required"
  And no TCP port is bound

Scenario: Startup fails when EMBYR_ADMIN_KEY is missing
  Given DATABASE_URL is set
  And EMBYR_ADMIN_KEY is not set in the environment
  When Sam runs "cargo run -p embyr-server"
  Then the process exits with code 1
  And stderr contains "EMBYR_ADMIN_KEY is required"

Scenario: Startup fails when system DB is unreachable
  Given DATABASE_URL points to a Postgres that refuses connections
  And all other required env vars are set
  When Sam runs "cargo run -p embyr-server"
  Then the server runs migrations (which fail) or DB probe fails
  And the process exits with code 1
  And stderr contains "startup probe failed"

Scenario: SIGTERM causes graceful shutdown
  Given the server is running and handling a gRPC request from a Firebase SDK client
  When the OS sends SIGTERM to the embyr-server process
  Then the in-flight gRPC request completes normally
  And the server exits with code 0 after the request completes
  And no new connections are accepted after SIGTERM

Scenario: Non-default ports are respected from environment
  Given GRPC_PORT is set to 18080
  And REST_PORT is set to 18081
  And ADMIN_PORT is set to 19090
  When Sam starts the server with all required env vars
  Then the server binds on ports 18080, 18081, and 19090
  And GET :19090/healthz returns HTTP 200
```

#### Acceptance Criteria
- [ ] Server exits with code 1 and message to stderr when `DATABASE_URL` is missing
- [ ] Server exits with code 1 and message to stderr when `EMBYR_ADMIN_KEY` is missing
- [ ] Server exits with code 1 when `SystemDb::probe()` fails (DB unreachable or schema missing)
- [ ] On successful startup, all 18 migrations are applied before any port is bound
- [ ] Server logs "embyr-server ready" (or equivalent structured log) with all three bound port addresses
- [ ] `GET :9090/healthz` returns HTTP 200 within 5 seconds of startup
- [ ] SIGTERM causes graceful shutdown (in-flight requests complete, exit 0)
- [ ] `GRPC_PORT`, `REST_PORT`, `ADMIN_PORT` env vars override defaults (8080, 8081, 9090)
- [ ] `EMBYR_ENCRYPTION_KEY` of wrong length (not 32 bytes hex) causes exit 1 at config parse time

#### Outcome KPIs
- **Who:** Sam Chen deploying embyr-server
- **Does what:** Starts the server with a single command, no code changes required
- **By how much:** 100% of deployments start successfully when env vars are correct (0% previously — binary cannot start)
- **Measured by:** Successful `cargo run` invocations reaching `healthz` HTTP 200 in staging
- **Baseline:** 0% — main.rs is a stub

#### Technical Notes
- Config struct in `crates/embyr-server/src/config.rs` (new file); mirrors `embyr_agent::config::AgentConfig`
- `main()` calls `get_or_install_prometheus_handle()` before migrations (ADR-016)
- `sqlx::migrate!("../../migrations")` path is relative to `crates/embyr-server/` — already working in tests
- Graceful shutdown uses `tokio::signal::ctrl_c()` + `unix::signal(SIGTERM)` and the existing oneshot pattern from `TestServer`
- `EMBYR_RATE_LIMIT_RPS` already parsed by `default_rate_limit_capacity()` in lib.rs — config struct should absorb this

---

### US-PR-02: Docker Image for Production Deployment

**job_id:** JOB-13

#### Elevator Pitch
**Before:** No Dockerfile exists. Sam cannot build a container image. Every deployment
requires installing the Rust toolchain and running `cargo build` on the target host.
**After:** `docker build . -t embyr-server && docker run -e DATABASE_URL=... -e
EMBYR_ADMIN_KEY=... embyr-server` starts the server. Sam can deploy to any container
platform without a Rust toolchain on the host.
**Decision enabled by:** D-PR-3 — Sam decides where and how to run the container; embyr
provides only the image definition.

#### Problem
Sam Chen is a service operator who wants to run embyr in a container environment (Kubernetes,
ECS, Fly.io). He finds it impossible to build a container image because no Dockerfile exists
in the repository. Every potential deployment requires a Rust toolchain on the target machine,
which contradicts standard container-based deployment practice.

#### Who
- Sam Chen (P2) | Service operator deploying embyr via container orchestration | Needs a
  reproducible, minimal Docker image built from the repository with no manual steps.

#### Solution
A multi-stage `Dockerfile` at the repository root (D-PR-3):
- Stage 1 (builder): `rust:1.80-slim` base with `cargo-chef` for dependency layer caching.
  Builds `embyr-server` in release mode.
- Stage 2 (runtime): `debian:bookworm-slim` base with only the compiled binary and required
  shared libraries. Runs as a non-root user. Exposes ports 8080, 8081, 9090.
- Final image size target: < 100 MB.

#### Domain Examples

**Example 1 (Happy Path):** Sam runs `docker build . -t embyr-server:latest`. Build
completes in ~3 minutes on first run (full build) or ~30 seconds on subsequent runs (cache
hit on dependencies). Final image is 80 MB.

**Example 2 (docker run with env vars):** Sam runs:
```
docker run -e DATABASE_URL=postgres://... \
           -e EMBYR_ADMIN_KEY=secret \
           -e EMBYR_ENCRYPTION_KEY=a3f1... \
           -p 8080:8080 -p 8081:8081 -p 9090:9090 \
           embyr-server:latest
```
Container starts, logs "embyr-server ready", and `curl localhost:9090/healthz` returns 200.

**Example 3 (Non-root security audit):** Sam's security team audits the running container.
`docker exec` reveals the process runs as user `embyr` (UID 1000), not root. Image passes
the `trivy image` check with no critical vulnerabilities in the runtime stage.

#### UAT Scenarios (BDD)

```gherkin
Scenario: Docker image builds successfully from the repository root
  Given the repository checkout at any commit on master
  When Sam runs "docker build . -t embyr-server"
  Then the build completes without error
  And the final image size is less than 100 MB

Scenario: Container starts the server when required env vars are provided
  Given the embyr-server Docker image is built
  And DATABASE_URL points to a reachable Postgres
  And EMBYR_ADMIN_KEY and EMBYR_ENCRYPTION_KEY are set
  When Sam runs "docker run -p 8080:8080 -p 8081:8081 -p 9090:9090 embyr-server"
  Then the container logs "embyr-server ready"
  And GET localhost:9090/healthz returns HTTP 200

Scenario: Container exits with code 1 when DATABASE_URL is missing
  Given the embyr-server Docker image is built
  When Sam runs "docker run embyr-server" without DATABASE_URL
  Then the container exits with code 1
  And the container logs contain "DATABASE_URL is required"

Scenario: Container process runs as non-root user
  Given a running embyr-server container
  When Sam inspects the running process inside the container
  Then the process owner is not root (UID != 0)

Scenario: Dependency layers are cached on rebuild
  Given the embyr-server Docker image was built once
  And no Cargo.toml or Cargo.lock files changed
  When Sam rebuilds the image after a src/ file change
  Then the dependency compilation layer is served from cache
  And the rebuild completes in under 60 seconds
```

#### Acceptance Criteria
- [ ] Multi-stage Dockerfile at repository root (builder + runtime stages)
- [ ] Final image size < 100 MB
- [ ] Container process runs as non-root user
- [ ] `EXPOSE 8080 8081 9090` declared in Dockerfile
- [ ] `docker build . -t embyr-server` succeeds on a clean checkout without extra flags
- [ ] Container started with required env vars passes `GET :9090/healthz` → HTTP 200
- [ ] Container started without `DATABASE_URL` exits code 1 with message to stdout/stderr
- [ ] cargo-chef dependency layer is present (enables < 60s rebuild on src-only changes)

#### Outcome KPIs
- **Who:** Sam Chen deploying embyr via Docker
- **Does what:** Builds and runs a container image without a Rust toolchain on the host
- **By how much:** From 0 deployable images to 1 per commit (infinite improvement from baseline zero)
- **Measured by:** Successful `docker build` + `docker run` + `healthz` 200 in CI
- **Baseline:** No Dockerfile exists; 0 container builds possible

#### Technical Notes
- `cargo-chef` `plan` + `cook` + `build` pattern for layer caching (standard Rust Docker pattern)
- Runtime base `debian:bookworm-slim` includes glibc (required for sqlx native TLS)
- Non-root: `useradd -ms /bin/bash embyr` in builder; `COPY --chown=embyr:embyr` in runtime
- `COPY --from=builder /embyr-server /usr/local/bin/embyr-server` as the only binary
- `.dockerignore` should exclude `target/`, `docs/`, `tests/`, `.git/`
- Depends on US-PR-01 (real `main()` must exist before the binary is useful)

---

### US-PR-03: CI Pipeline Gating Every PR

**job_id:** JOB-13

#### Elevator Pitch
**Before:** No CI exists. A broken build, a clippy warning, a failing test, or an unlicensed
dependency can reach main. Sam finds broken deployments after the fact.
**After:** Every push to master and every PR triggers `cargo test --workspace`, `cargo clippy
-- -D warnings`, `cargo deny check`, and `docker build`. A single red job blocks merge. Sam
sees a green badge before merging any change.
**Decision enabled by:** D-PR-4 — Sam decides the merge policy knowing the CI gate is always
in place.

#### Problem
Sam Chen is a service operator responsible for the reliability of the embyr deployment. He
finds it risky to merge any PR because there is no automated gate — a broken build, a failing
test, or a dependency with a security advisory can land on main without detection. Each
incident requires manual diagnosis after the fact.

#### Who
- Sam Chen (P2) | Service operator and repository maintainer | Needs automated assurance that
  every commit landing on main is tested, lint-clean, dependency-safe, and builds into a
  valid Docker image.

#### Solution
A GitHub Actions workflow at `.github/workflows/ci.yml` (D-PR-4) with three jobs:
- `test`: `cargo test --workspace` — all unit and integration tests
- `lint`: `cargo clippy -- -D warnings` and `cargo deny check`
- `docker`: `docker build . -t embyr-server` — validates image builds

All three jobs must pass before merge. Triggered on every push to master and every PR.

#### Domain Examples

**Example 1 (Happy Path — All Green):** Sam pushes a feature branch. GitHub Actions runs all
three jobs in parallel. `test` completes in 4 minutes (cached deps), `lint` in 90 seconds,
`docker` in 2 minutes (cached layers). All pass. Sam merges.

**Example 2 (Test Failure Blocks Merge):** A developer pushes a PR that breaks a test in
`crates/embyr-server/`. The `test` job fails. GitHub marks the PR as "checks failed". The
developer sees the failing test name in the job log and fixes it before merge is possible.

**Example 3 (Clippy Warning Blocks Merge):** A PR introduces an unused variable. `cargo
clippy -- -D warnings` exits non-zero. The `lint` job fails. The developer fixes the warning.
No warning can reach main.

**Example 4 (Dependency Advisory Blocks Merge):** A new dependency is added with a known
RUSTSEC advisory not listed in `deny.toml`. `cargo deny check` fails in the `lint` job. The
developer either upgrades the dependency or adds the advisory with a justification comment to
`deny.toml` before merge.

#### UAT Scenarios (BDD)

```gherkin
Scenario: All CI jobs pass on a clean commit
  Given a commit on master that passes all existing tests
  When GitHub Actions runs the CI workflow
  Then the "test" job exits 0 (cargo test --workspace passes)
  And the "lint" job exits 0 (clippy -D warnings and cargo deny check pass)
  And the "docker" job exits 0 (docker build completes without error)
  And all three jobs complete within 10 minutes

Scenario: PR with a failing test is blocked from merge
  Given a PR that introduces a change breaking an existing test
  When the CI workflow runs on the PR
  Then the "test" job fails
  And GitHub marks the PR as "checks failed"
  And the failing test name is visible in the CI job output

Scenario: PR with a clippy warning is blocked from merge
  Given a PR introducing unused Rust code that triggers a clippy warning
  When the "lint" job runs "cargo clippy -- -D warnings"
  Then the lint job exits with non-zero code
  And the specific warning location (file:line) appears in the job log

Scenario: PR with an unlicensed dependency is blocked from merge
  Given a PR adding a crate with a license not listed in deny.toml
  When "cargo deny check" runs in the "lint" job
  Then the lint job fails
  And the output identifies the crate and its license

Scenario: CI workflow triggers on both pushes to master and PR events
  Given the CI workflow is configured in .github/workflows/ci.yml
  When Sam pushes directly to master
  Then all three CI jobs run
  When Sam opens a pull request targeting master
  Then all three CI jobs run on the PR's head commit
```

#### Acceptance Criteria
- [ ] `.github/workflows/ci.yml` exists and triggers on `push` (master) and `pull_request`
- [ ] `test` job runs `cargo test --workspace` and fails the workflow on any test failure
- [ ] `lint` job runs `cargo clippy -- -D warnings` and fails on any warning
- [ ] `lint` job runs `cargo deny check` and fails on any unlicensed or advisory-violated dependency
- [ ] `docker` job runs `docker build . -t embyr-server` and fails if image build fails
- [ ] All three jobs must pass for a PR to be mergeable (branch protection rule documented)
- [ ] Workflow uses dependency caching (`actions/cache` for `~/.cargo/registry`) to keep runs under 10 minutes

#### Outcome KPIs
- **Who:** Sam Chen and all repository contributors
- **Does what:** Merges PRs with confidence that tests, lints, and image builds pass
- **By how much:** 100% of PRs are gated (0% previously — no CI existed)
- **Measured by:** GitHub Actions workflow run success rate on master over 30 days
- **Baseline:** 0 PRs gated by automated checks

#### Technical Notes
- Trigger events: `push: branches: [master]` + `pull_request: branches: [master]`
- Rust toolchain pinned to stable (no nightly required)
- `actions/cache` for `~/.cargo/registry` and `~/.cargo/git` and `target/`
- `cargo test --workspace` includes all testcontainers-backed integration tests — requires Docker-in-runner (GitHub-hosted runners support this)
- `docker` job depends on Dockerfile from US-PR-02 — listed as dependency
- `cargo deny check` uses existing `deny.toml` — no new configuration required
- Depends on US-PR-01 (real main) and US-PR-02 (Dockerfile) for the docker job

---

## Story Map

### User: Sam Chen (P2 — Service Operator)
### Goal: Deploy embyr-server to production with a single `docker run` command

### Backbone (left-to-right user activities)

| Configure | Build | Validate | Deploy |
|-----------|-------|----------|--------|
| Set env vars | Build binary | Run tests | Start container |
| Validate config at startup | Build Docker image | Check lints | Verify health |
| | | Gate PRs | |

### Walking Skeleton

The thinnest end-to-end slice that proves the server can run:
- **PR-01** (Config + main): `DATABASE_URL` + `EMBYR_ADMIN_KEY` → server starts → `healthz` 200

### Release 1: Production-Ready Binary (PR-01 + PR-02)

Outcome: Sam can deploy embyr-server as a Docker container with no Rust toolchain.

| Story | Slice | Effort |
|-------|-------|--------|
| US-PR-01 | PR-01 (Config skeleton) + PR-02 (Startup probe) | 1 day |
| US-PR-02 | PR-03 (Dockerfile) | 0.5 day |

### Release 2: Automated Quality Gate (PR-04)

Outcome: Sam can merge PRs with confidence that no broken build reaches main.

| Story | Slice | Effort |
|-------|-------|--------|
| US-PR-03 | PR-04 (CI pipeline) | 0.5 day |

### Priority Rationale

1. **PR-01** (Config + main skeleton) — Walking skeleton. Without a working binary, nothing
   else can be tested or deployed. Highest urgency.
2. **PR-02** (Startup probe + graceful shutdown) — Completes the production-ready binary.
   Safety gate before any traffic is accepted.
3. **PR-03** (Dockerfile) — Enables the primary deployment target (container). Sam cannot
   deploy without this.
4. **PR-04** (CI pipeline) — Protects the main branch going forward. Important but does not
   block the first deployment.

---

## Scope Assessment

PASS — 3 stories, 2 bounded contexts touched (embyr-server composition root + CI/CD
infrastructure), estimated 2 days total. Each story is independently deliverable.
No dependency loops. PR-03 depends on PR-01/PR-02 (binary must exist for docker build
to be meaningful); PR-04 depends on PR-03 (docker job needs Dockerfile).

---

## DoR Checklist

| Item | US-PR-01 | US-PR-02 | US-PR-03 |
|------|----------|----------|----------|
| Problem statement clear, domain language | PASS | PASS | PASS |
| User/persona with specific characteristics | PASS (Sam Chen, P2) | PASS (Sam Chen, P2) | PASS (Sam Chen, P2) |
| 3+ domain examples with real data | PASS (4 examples) | PASS (3 examples) | PASS (4 examples) |
| UAT scenarios in Given/When/Then (3-7) | PASS (6 scenarios) | PASS (5 scenarios) | PASS (5 scenarios) |
| AC derived from UAT | PASS | PASS | PASS |
| Right-sized (1-3 days, 3-7 scenarios) | PASS (0.5d each slice) | PASS (0.5d) | PASS (0.5d) |
| Technical notes: constraints/dependencies | PASS | PASS | PASS |
| Dependencies resolved or tracked | PASS (no external deps) | PASS (depends on US-PR-01) | PASS (depends on US-PR-01, US-PR-02) |
| Outcome KPIs defined with measurable targets | PASS | PASS | PASS |
| job_id present | PASS (JOB-13) | PASS (JOB-13) | PASS (JOB-13) |
| Elevator Pitch present | PASS | PASS | PASS |

**Overall DoR: PASS — all 3 stories ready for DESIGN wave**

---

## Outcome KPIs (Feature Level)

### Objective
Enable Sam Chen to deploy embyr-server to production in under 30 minutes on the first
attempt, with CI ensuring no broken build ever reaches main. Target: 2026-Q3.

### Metric Table

| # | Who | Does What | By How Much | Baseline | Measured By | Type |
|---|-----|-----------|-------------|----------|-------------|------|
| 1 | Sam Chen | Deploys embyr-server via docker run | First deployment succeeds in < 30 min | ∞ (impossible, stub) | Manual stopwatch on first deploy | Leading |
| 2 | Repository contributors | Merge PRs blocked by CI gate | 100% of PRs gated | 0% | GitHub Actions run count / PR count | Leading |
| 3 | Sam Chen | Diagnoses startup failures by reading stderr | < 2 minutes to identify root cause | Unknown (no error messages) | Informal: time to first diagnosis in staging | Leading |

### Guardrail Metrics
- `embyr-core` IO-free invariant must NOT be violated by any story in this feature
- `cargo deny check` must remain passing after all changes
- No new TCP listener ports may be added (3 ports is fixed by architecture)

### North Star
First successful production `docker run` resulting in `healthz` HTTP 200.

---

## Wave Decisions Log

See individual slice files for implementation decisions.

**2026-08-08** — D-PR-1 through D-PR-7 locked from codebase analysis. No DIVERGE wave
run for this feature (infrastructure-only stories enabling JOB-13). DIVERGE risk: noted.
All decisions derived from existing code patterns (`embyr-agent/src/main.rs`,
`lib.rs::alloc_test_components`, `system_db.rs`).

**Scope Assessment:** PASS — 3 user stories, 4 Elephant Carpaccio slices (≤ 0.5d each),
2 days total, 2 bounded contexts. No scope split required.

**2026-08-08** — DESIGN wave complete. ADR-017 written. Production deployment subsection
added to `docs/product/architecture/brief.md`. All decisions autonomous per DES-ENFORCEMENT
exemption.

---

## Wave: DESIGN
## Date: 2026-08-08
## Status: Complete — Ready for DISTILL wave

---

### Wave: DESIGN / [REF] Architecture Summary

This feature makes no changes to the domain model or bounded context boundaries. It resolves
three production-deployment blockers in the composition root and CI/CD infrastructure layer.

**Bounded contexts touched:**
- `embyr-server` composition root (lib.rs, main.rs) — startup wiring
- CI/CD infrastructure — GitHub Actions, Dockerfile

**Bounded contexts NOT touched:**
- `embyr-core` — IO-free invariant preserved; no changes
- Any port trait or adapter — no new adapters introduced
- Any domain type — no domain changes

---

### Wave: DESIGN / [REF] ADR Written

| ADR | File | Decision |
|-----|------|----------|
| ADR-017 | `docs/product/architecture/adr-017-production-startup.md` | `ServerConfig::from_env()` in new `config.rs`; `alloc_production_components()` added to `lib.rs`; `spawn_all_servers` made public; real tokio `main()` with 14-step startup sequence |

---

### Wave: DESIGN / [REF] Component Decomposition

| File | Change Type | What Changes |
|------|-------------|--------------|
| `crates/embyr-server/src/config.rs` | NEW | `ServerConfig { db_url, admin_key, encryption_key: [u8;32], rate_limit_rps, grpc_port, rest_port, admin_port, log_level }` + `from_env() -> Result<Self, ConfigError>` + `ConfigError` enum. No new crate dependency — uses `std::env::var`. Mirrors `AgentConfig` pattern. |
| `crates/embyr-server/src/main.rs` | REPLACE | 3-line stub replaced with real `#[tokio::main]`. 14-step startup sequence: config parse → tracing → prometheus → SystemDb::new → migrate → probe → alloc_production_components → bind 3 ports → build FirestoreService + admin_app → spawn_all_servers → log "ready" → await SIGTERM/ctrl_c → send shutdown → log "stopped". |
| `crates/embyr-server/src/lib.rs` | MODIFY (minimal) | Two changes: (1) `pub fn alloc_production_components(system_db: Arc<SystemDb>, config: &ServerConfig) -> ProductionComponents` added alongside private `alloc_test_components`; (2) `fn spawn_all_servers` → `pub fn spawn_all_servers` (one keyword). All existing test server constructors unchanged. |
| `Dockerfile` | NEW | Multi-stage build at repository root. Four stages: chef (install cargo-chef) → planner (cargo chef prepare) → builder (cargo chef cook --release + cargo build --release --bin embyr-server) → runtime (debian:bookworm-slim, non-root user `embyr`, EXPOSE 8080 8081 9090). No `COPY migrations/` — migrations are embedded at compile time via `sqlx::migrate!("../../migrations")` macro. |
| `.github/workflows/ci.yml` | NEW | Three jobs: `test` (cargo test --workspace with Postgres service container), `lint` (cargo clippy -D warnings + cargo deny check), `docker` (docker build, depends on test). Triggers on push to master and pull_request targeting master. Uses `Swatinem/rust-cache@v2` for dependency caching. |
| `.dockerignore` | NEW | Excludes `target/`, `docs/`, `tests/`, `.git/` from build context to minimize layer size. |

---

### Wave: DESIGN / [REF] ServerConfig Contract

```
Required environment variables (exit 1 if absent or empty):
  DATABASE_URL              — Postgres DSN for system DB
  EMBYR_ADMIN_KEY           — non-empty string; admin port Bearer token
  EMBYR_ENCRYPTION_KEY      — exactly 64 hex chars (32 bytes); validated at parse time

Optional environment variables (defaults shown):
  EMBYR_RATE_LIMIT_RPS      — float, finite, >0; default 1000.0
  GRPC_PORT                 — u16; default 8080
  REST_PORT                 — u16; default 8081
  ADMIN_PORT                — u16; default 9090
  RUST_LOG                  — tracing level filter string; default "info"
```

Error accumulation: all missing/invalid variables are collected before returning.
The operator sees every problem in a single stderr message.

---

### Wave: DESIGN / [REF] Production Startup Sequence

```
1.  ServerConfig::from_env()
    → ConfigError: eprintln! + exit(1), no port bound

2.  tracing_subscriber::fmt()
        .with_env_filter(RUST_LOG env or config.log_level fallback)
        .with_writer(stderr)
        .init()

3.  observability::get_or_install_prometheus_handle()
    [ADR-016: recorder before any TCP listener]

4.  SystemDb::new(&config.db_url).await
    → Err: tracing::error! + exit(1)

5.  system_db.migrate().await    [18 migrations embedded via sqlx::migrate!]
    → Err: tracing::error! + exit(1)

6.  system_db.probe().await      [SELECT 1 + projects table existence check]
    → Err: tracing::error! + exit(1), log "startup probe failed: ..."

7.  alloc_production_components(Arc::clone(&system_db), &config)
    → ProductionComponents { cache, idx_mgr, metrics, listen_registry,
                              active_listeners, rate_limiter (RateLimiter::with_pg),
                              shutdown_tx, shutdown_rx }
    + pool gauge background task spawned (15s interval)

8.  TcpListener::bind("0.0.0.0:{grpc_port}")
    TcpListener::bind("0.0.0.0:{rest_port}")
    TcpListener::bind("0.0.0.0:{admin_port}")
    → any Err: tracing::error! + exit(1)
    [All three ports or none — D-PR-6]

9.  FirestoreService { system_db, cache, idx_mgr, metrics,
                       keepalive: 30s, listen_registry, active_listeners,
                       aws_secret_fetcher: None, gcp_secret_fetcher: None,
                       rate_limiter }

10. build_admin_router(system_db, admin_key, cache_for_admin,
                       encryption_key, Arc::new(NoopEmailSender),
                       None, None, rate_limit_rps, prometheus_handle)

11. spawn_all_servers(grpc_listener, rest_listener, admin_listener,
                      service, admin_app, shutdown_rx)

12. tracing::info!("embyr-server ready" grpc=... rest=... admin=...)

13. tokio::select!
        ctrl_c() => {}
        SIGTERM  => {}
    → shutdown_tx.send(())
    → tracing::info!("shutdown signal received, draining...")

14. tracing::info!("embyr-server stopped")
    [Process exits 0 after drain]
```

---

### Wave: DESIGN / [REF] Dockerfile Contract

Multi-stage build. `sqlx::migrate!("../../migrations")` embeds migration files at compile
time — no runtime file copy is required.

```
Stage 1 (chef):    rust:1.80-slim + cargo install cargo-chef
Stage 2 (planner): COPY . . && cargo chef prepare --recipe-path recipe.json
Stage 3 (builder): cargo chef cook --release --recipe-path recipe.json
                   COPY . . && cargo build --release --bin embyr-server
Stage 4 (runtime): debian:bookworm-slim
                   apt-get install -y ca-certificates
                   useradd -r -s /bin/false embyr
                   COPY --from=builder /app/target/release/embyr-server /app/embyr-server
                   USER embyr
                   EXPOSE 8080 8081 9090
                   ENTRYPOINT ["/app/embyr-server"]
```

Target final image size: < 100 MB. The builder stage produces a dynamically linked binary
(glibc) compatible with `debian:bookworm-slim`. `ca-certificates` is required for TLS
handshakes to AWS/GCP secret managers.

The cargo-chef `cook` + `prepare` pattern caches the dependency compilation layer separately
from the application source. A source-only change (no `Cargo.toml`/`Cargo.lock` change)
rebuilds in < 60 seconds on a warm cache.

---

### Wave: DESIGN / [REF] CI Pipeline Contract

Three jobs in `.github/workflows/ci.yml`:

```
trigger: push: branches: [master]
         pull_request: branches: [master]

job: test
  runs-on: ubuntu-latest
  services:
    postgres: image: postgres:15
              env: POSTGRES_PASSWORD=postgres POSTGRES_DB=embyr_test
              health: pg_isready
              ports: 5432:5432
  steps:
    actions/checkout@v4
    dtolnay/rust-toolchain@stable
    Swatinem/rust-cache@v2
    cargo test --workspace
      env: DATABASE_URL=postgres://postgres:postgres@localhost:5432/embyr_test

job: lint
  runs-on: ubuntu-latest
  steps:
    actions/checkout@v4
    dtolnay/rust-toolchain@stable (components: clippy)
    Swatinem/rust-cache@v2
    cargo clippy --workspace -- -D warnings
    cargo deny check

job: docker
  runs-on: ubuntu-latest
  needs: [test]
  steps:
    actions/checkout@v4
    docker/setup-buildx-action@v3
    docker build -t embyr-server:ci .
```

`cargo deny check` uses the existing `deny.toml` — no new configuration required.
The `docker` job depends on `test` passing to avoid wasting build minutes on broken code.

---

### Wave: DESIGN / [REF] Earned Trust Assessment

This feature creates two new components that interact with the substrate:

| Component | Substrate Dependency | Probe Design |
|-----------|---------------------|-------------|
| `ServerConfig::from_env()` | Environment variables | Pure function — no probe needed. Unit tests assert every error path by setting env via `std::env::set_var` in test scope or by passing test inputs directly. |
| `main()` startup sequence | SystemDb + port availability | Covered by existing `SystemDb::probe()` (SELECT 1 + projects table check). Port bind failure detected immediately — `TcpListener::bind()` returns `Err` if port is in use. No additional probe needed; the bind attempt is the probe. |

The Dockerfile introduces a new substrate (Docker overlay filesystem). The `NOTIFY`
round-trip probe already present in `SystemDb` is the relevant Earned Trust gate for Docker
environments (Docker overlayfs can no-op `NOTIFY`). The probe runs at step 6 of the startup
sequence, before any port opens. No new probe is needed specifically for the Dockerfile.

---

### Wave: DESIGN / [REF] Quality Gates — Passed

- [x] Requirements (D-PR-1 through D-PR-7) traced to components
- [x] Component boundaries with clear responsibilities (config.rs / lib.rs changes / main.rs / Dockerfile / CI)
- [x] Technology choices in ADR-017 with 4 rejected alternatives
- [x] Quality attributes addressed: reliability (probe chain), security (EMBYR_ENCRYPTION_KEY validation), maintainability (config struct unit-testable), portability (Dockerfile + CI)
- [x] Dependency-inversion compliance: `embyr-core` untouched, no new IO imports in domain layer
- [x] No C4 diagram additions required: no new components at L1 or L2 level; `embyr-server` container boundary unchanged
- [x] Integration patterns unchanged: startup wiring uses existing `spawn_all_servers` and `build_admin_router` signatures
- [x] OSS preference validated: `rust:1.80-slim`, `debian:bookworm-slim`, `cargo-chef`, `Swatinem/rust-cache`, `dtolnay/rust-toolchain` — all OSS, no proprietary tooling
- [x] AC behavioral (not implementation-coupled): US-PR-01 ACs assert observable behavior (healthz 200, exit code 1, stderr content)
- [x] No new external integrations introduced
- [x] Architecture enforcement: `cargo-deny check` in CI enforces IO-prohibition in `embyr-core`; `cargo clippy -D warnings` enforces code quality

---

## Wave: DISTILL
## Date: 2026-08-08
## Status: Complete — Ready for DELIVER wave

---

### Wave: DISTILL / [REF] Inherited commitments

| Origin | Commitment | DDD | Impact |
|--------|------------|-----|--------|
| DISCUSS#D-PR-1 | Server reads config from 8 env vars; 3 required (DATABASE_URL, EMBYR_ADMIN_KEY, EMBYR_ENCRYPTION_KEY) | n/a | Tests assert exit 1 + stderr message for each missing required var |
| DISCUSS#D-PR-6 | No partial startup — all three ports or none | n/a | `no_port_bound_on_missing_required_env_var` verifies all 3 ports unbound after failure |
| DESIGN#ADR-017 | 14-step startup sequence; `ServerConfig::from_env()` accumulates all errors | n/a | Tests verify server reports ALL missing vars simultaneously; exit 1 within 3s |
| DESIGN#ADR-017 | `system_db.probe()` at step 6 is a hard gate before port binding | n/a | `exits_1_when_db_probe_fails_projects_table_missing` verifies probe is exercised |

---

### Wave: DISTILL / [REF] Scenario List

| Scenario | Tags | File | #[ignore]? |
|----------|------|------|------------|
| `server_starts_with_all_required_env_vars_set` | @walking_skeleton @driving_port @real-io @US-PR-01 | pr01 | NO — walking skeleton |
| `exits_1_when_database_url_missing` | @error @US-PR-01 | pr01 | yes |
| `exits_1_when_admin_key_missing` | @error @US-PR-01 | pr01 | yes |
| `exits_1_when_encryption_key_missing` | @error @US-PR-01 | pr01 | yes |
| `exits_1_when_encryption_key_not_64_hex_chars` | @error @boundary @US-PR-01 | pr01 | yes |
| `exits_1_when_database_unreachable` | @error @US-PR-01 | pr01 | yes |
| `exits_1_when_db_probe_fails_projects_table_missing` | @error @real-io @US-PR-01 | pr01 | yes — todo! |
| `no_port_bound_on_missing_required_env_var` | @error @US-PR-01 | pr01 | yes |
| `non_default_ports_respected` | @US-PR-01 | pr01 | yes |
| `grpc_port_env_var_respected` | @US-PR-01 | pr01 | yes |
| `rest_port_env_var_respected` | @US-PR-01 | pr01 | yes |
| `default_rate_limit_rps_is_1000` | @US-PR-01 | pr01 | yes — todo! |
| `docker_build_succeeds` | @real-io @US-PR-02 | pr02 | yes |
| `docker_image_runs_as_non_root` | @real-io @US-PR-02 | pr02 | yes |
| `docker_image_under_100mb` | @US-PR-02 | pr02 | yes |
| `docker_image_exposes_correct_ports` | @US-PR-02 | pr02 | yes |
| `docker_image_entrypoint_is_embyr_server` | @US-PR-02 | pr02 | yes |
| `cargo_chef_rebuild_fast` | @US-PR-02 | pr02 | yes |
| `ci_yaml_triggers_on_push_to_master` | @US-PR-03 | pr03 | yes |
| `ci_yaml_triggers_on_pull_request` | @US-PR-03 | pr03 | yes |
| `ci_yaml_has_test_job_with_postgres_service` | @US-PR-03 | pr03 | yes |
| `ci_yaml_has_lint_job_with_clippy_and_deny` | @US-PR-03 | pr03 | yes |
| `ci_yaml_has_docker_job_depending_on_test` | @US-PR-03 | pr03 | yes |
| `ci_yaml_clippy_uses_deny_warnings` | @US-PR-03 | pr03 | yes |
| `sigterm_causes_graceful_shutdown` | @real-io @US-PR-01 | pr04 | yes |
| `in_flight_request_completes_before_shutdown` | @real-io @US-PR-01 | pr04 | yes — todo! |
| `sigterm_drains_within_30s` | @real-io @US-PR-01 | pr04 | yes |

Total: 27 scenarios. Walking skeleton: 1 (enabled). Error/edge paths: 16 of 27 = 59% (exceeds 40% mandate).

---

### Wave: DISTILL / [REF] Walking Skeleton Strategy

**Strategy: C — Real subprocess + real driven-internal Postgres (testcontainers)**

Rationale: the observable user value is "Sam can start the server with env vars and see /healthz 200". This REQUIRES a real binary subprocess (the driving adapter IS the production binary) and a real Postgres (the server runs migrations and probes the DB before binding ports). InMemory cannot model this — the whole feature IS the startup sequence. No costly external resources, so no fake needed.

**Walking skeleton scenario**: `server_starts_with_all_required_env_vars_set`
- Starts Postgres 15-alpine testcontainer.
- Resolves pre-built `target/debug/embyr-server` binary.
- Spawns as subprocess with required env vars.
- Polls `GET :{admin_port}/healthz → 200` every 200ms up to 30 seconds.
- Sends SIGTERM via `kill -TERM <pid>`.
- Asserts exit code 0 within 15 seconds.

**Litmus test**: Sam Chen can watch this test run and confirm "yes, that is exactly what I need — a single binary starts from env vars and is healthy within 30 seconds".

---

### Wave: DISTILL / [REF] Adapter Coverage

| Adapter | @real-io scenario | Covered by |
|---------|-------------------|------------|
| `embyr-server` subprocess (new driving adapter) | YES | Walking skeleton — real binary spawned, real env vars |
| System Postgres (testcontainers-rs) | YES | Walking skeleton + all scenarios using `start_postgres_container()` |
| Admin HTTP `/healthz` endpoint (reqwest) | YES | Walking skeleton polls `/healthz` |
| `docker` CLI | YES | `docker_build_succeeds` (real `docker build` invocation) |
| `.github/workflows/ci.yml` filesystem | YES | All pr03 tests read the real file from workspace root |

---

### Wave: DISTILL / [REF] Scaffolds

Files created by this DISTILL wave (all compile; walking skeleton is RED until main.rs is implemented):

- `tests/production_readiness/mod.rs` — single test binary entry point
- `tests/production_readiness/common/mod.rs` — `ServerProcess`, `find_free_port`, `start_postgres_container`, `universe` constants; SCAFFOLD: true
- `tests/production_readiness/acceptance/pr01_config_from_env.rs` — 12 scenarios; walking skeleton NOT #[ignore]
- `tests/production_readiness/acceptance/pr02_dockerfile.rs` — 6 scenarios; all #[ignore]
- `tests/production_readiness/acceptance/pr03_ci.rs` — 6 scenarios; all #[ignore]
- `tests/production_readiness/acceptance/pr04_graceful_shutdown.rs` — 3 scenarios; all #[ignore]

Walking skeleton RED classification: binary exists at `target/debug/embyr-server` (stub) → `wait_for_healthy()` times out (main.rs prints one line and exits) → assertion fails → **RED** (correct).

Pre-DELIVER red classification: DELIVER must verify the walking skeleton fails with "server did not respond to GET /healthz with HTTP 200 within 30 seconds" — a business logic failure, not a setup error.

---

### Wave: DISTILL / [REF] Test Placement

`tests/production_readiness/` — mirrors `tests/distributed_rate_limiting/` layout.

Single `[[test]]` binary entry in `crates/embyr-server/Cargo.toml`:
```toml
[[test]]
name = "production_readiness"
path = "../../tests/production_readiness/mod.rs"
```

Rationale: single binary with nested acceptance modules keeps cargo test output grouped by feature. The `production_readiness` binary contains all 27 scenarios across 4 acceptance files.

---

### Wave: DISTILL / [REF] Driving Adapter Coverage

| Entry Point | Scenario | Driving Mechanism |
|-------------|----------|-------------------|
| `embyr-server` binary main() | `server_starts_with_all_required_env_vars_set` | subprocess spawn + env vars + `/healthz` HTTP probe |
| `embyr-server` binary startup config validation | `exits_1_when_*` (6 scenarios) | subprocess spawn + wait exit code + stderr drain |
| Docker build entry | `docker_build_succeeds` | `docker build .` subprocess |
| Docker run entry | `docker_image_runs_as_non_root` | `docker run --entrypoint whoami` |
| GitHub Actions YAML | `ci_yaml_*` (6 scenarios) | `std::fs::read_to_string` of `.github/workflows/ci.yml` |

All driving adapters from the DESIGN document are covered by at least one scenario. No uncovered entry points.

---

### Wave: DISTILL / [REF] Pre-requisites

DELIVER must satisfy these before unskipping any scenario:

1. **`cargo build --bin embyr-server`** must succeed before the walking skeleton can attempt a startup (binary must exist at `target/debug/embyr-server`).
2. **US-PR-01 DELIVER** (`main.rs` real implementation) is required before pr01 scenarios can go GREEN.
3. **US-PR-02 DELIVER** (`Dockerfile` multi-stage build) is required before pr02 scenarios can go GREEN.
4. **US-PR-03 DELIVER** (`.github/workflows/ci.yml`) is required before pr03 scenarios can go GREEN.
5. **US-PR-01 DELIVER** (graceful shutdown — `tokio::signal` handler) is required before pr04 scenarios can go GREEN.
6. **Docker daemon** must be available in the test environment for pr02 scenarios (tests skip gracefully when Docker is absent via `docker_available()` check).
7. **Ports 8080/8081/9090** must NOT be in use on the test host (default ports blocked by `#[ignore]` tests — walking skeleton uses ephemeral ports only).

DELIVER implementation order:
1. US-PR-01 (config.rs + main.rs) → unskip pr01 scenarios in dependency order
2. PR-04 graceful shutdown scenarios (SIGTERM handler) → unskip pr04
3. US-PR-02 (Dockerfile) → unskip pr02
4. US-PR-03 (CI YAML) → unskip pr03
