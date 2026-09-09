# ADR-074: embyr-agent Release Pipeline — musl Target, CI Job Topology, Artifact Shape

## Status

Accepted

## Context

Finding #8 of the 2026-09-08 production-readiness audit (Blocker, LAST of 8): `embyr-agent` has
zero build/release path. ADR-001 already locks the target *architecture* — "a separately compiled,
statically-linked Rust binary," structurally separate from `embyr-server` (Alternative C, merging
the two, is explicitly rejected there on credential-isolation grounds). What ADR-001 does not lock,
and what DISCUSS (`docs/feature/embyr-agent-release-pipeline/feature-delta.md`) explicitly deferred
to DESIGN, is: the exact musl target/toolchain mechanics, the CI job topology, the artifact shape
(bare binary vs. container image vs. both), and how CI proves the built artifact genuinely runs —
not merely compiles.

Confirmed inputs (see feature-delta.md § DISCUSS investigation for full citations):
- `rust-toolchain.toml` pins `channel = "1.95.0"` only — no musl target configured anywhere today.
- `.github/workflows/ci.yml` has `test` (needs nothing), `lint` (needs nothing), `docker` (`needs:
  [test]`, builds only the root `Dockerfile` for `embyr-server`).
- `crates/embyr-agent`'s dependency tree is rustls/ring throughout (confirmed via root
  `Cargo.toml`: `sqlx` uses `runtime-tokio-rustls`, workspace-wide comment states "this workspace
  is rustls-only throughout") — zero `openssl-sys`/`native-tls` anywhere in the agent's dependency
  graph, dev-dependencies included.
- `tests/acceptance/embyr_agent/us_a06_lifecycle.rs` already contains
  `agent_logs_storage_readiness_before_accepting_connections` and
  `storage_credential_never_appears_in_agent_logs` — real subprocess tests that spawn
  `env!("CARGO_BIN_EXE_embyr-agent")`, connect it to a real testcontainers Postgres, and assert the
  exact "connected to Postgres" → "listening on :9191" sequence the deployment journey documents.
  Both currently carry `#[ignore = "requires Docker + embyr-agent binary — unskip in S06A
  delivery"]` — this feature is that delivery.
- `docs/product/journeys/agent-deployment.yaml` step 2 is `kubectl apply -f
  agent-deployment.yaml` — a Kubernetes Deployment/Pod manifest requires a container image
  reference; it cannot reference a bare ELF binary. This materially changes the artifact-shape
  answer: a raw static binary alone does not fulfill Riley's own already-documented deployment step.

## Decision

### 1. Musl target: `x86_64-unknown-linux-musl` only

Add `targets: x86_64-unknown-linux-musl` to the new CI job's `dtolnay/rust-toolchain@stable` step,
pinned to `toolchain: "1.95.0"` — matching `rust-toolchain.toml`'s pin exactly, using the identical
pinning pattern (and identical cautionary comment) already present for the `test`/`lint` jobs and
the root `Dockerfile`'s own stage-1 comment: install the target on the SAME pinned-toolchain
install, not as a separate `rustup target add` afterward (a bare `rustup target add` run after
`rust-toolchain.toml`'s override takes effect lands the target on the wrong toolchain — this
project has already broken CI once from toolchain-version drift; see root `Dockerfile` comments).

`aarch64-unknown-linux-musl` is explicitly NOT added. No persona, journey, or job names an arm64
requirement; GitHub-hosted `ubuntu-latest` runners are x86_64, so an aarch64 build would require
either QEMU emulation or a cross-toolchain, doubling CI cost and complexity for zero validated
demand. YAGNI — add when a customer requests arm64 VPC support.

No `cross`/`cargo-zigbuild` tooling is used. `x86_64-unknown-linux-musl` on an `ubuntu-latest`
(x86_64) runner is a same-architecture, libc-only target swap — a native build, not a cross-arch
cross-compile. `cross`'s Docker-in-Docker toolchain exists to solve cross-architecture emulation,
which this job does not need. The only new system package required is `musl-tools` (provides
`musl-gcc`, needed because the `ring` crate compiles hand-written C/assembly for its target and
needs a C compiler present for that target) — already-available `apt-get install`, zero new
external tooling.

`RUSTFLAGS="-C target-feature=+crt-static"` is set explicitly on the build step. musl targets
statically link by default in current Rust, but this makes the requirement — named in `main.rs`'s
own top comment and in `docs/architecture/embyr-rs/architecture-decisions.md:147` — explicit and
independent of any future default change.

### 2. New CI job `agent`, `runs-on: ubuntu-latest`, no `needs:`

A new job, not a step inside `test`/`lint`/`docker`: independent trigger surface (musl target vs.
`test`'s `wasm32-unknown-unknown`), independent pass/fail semantics, matches this project's
established job-per-concern convention (ADR-008: dedicated build step per artifact type, `cargo
build` vs. `trunk build` already precedent for exactly this shape).

**Deliberately does NOT set `needs: [test]`**, despite that being `docker`'s own shape (which DESIGN
was pointed at as a structural reference). Reason, found during this design's own self-review: GitHub
Actions skips (not fails) a job whose `needs:` dependency failed. `test` already runs `cargo test
--workspace`, which compiles `crates/embyr-agent` as a side effect. A compile error scoped only to
`embyr-agent` would therefore fail `test` FIRST — and a `needs: [test]` `agent` job would show as
**skipped**, not failed, contradicting AC-EARP-05 verbatim ("the new agent build job fails with a
named, visible compiler error" — DISCUSS's own Example 3 domain scenario). `agent` runs fully
parallel to `test`/`lint`/`docker`, with zero shared dependency edge in either direction — the
strongest form of AC-EARP-04's non-interference guarantee (no job blocks or is blocked by this one),
and the only shape that satisfies AC-EARP-05's failure-visibility requirement.

Trade-off accepted: the `agent` job's own artifact can be produced/uploaded even on a PR where the
unrelated `test` job is independently red for a different reason. This is acceptable because the
produced artifact is a transient per-run CI build artifact, not a published release — release
promotion/versioning/publishing is explicitly out of scope (audit finding #19, DISCUSS's own Out of
Scope section). Sam Chen still sees `test` red on the same PR and will not treat the PR as
mergeable regardless of `agent`'s own color.

### 3. Artifact mechanism: BOTH a GitHub Actions build artifact (primary) AND a Dockerfile (secondary)

**Primary: `actions/upload-artifact@v4`** uploading the compiled
`target/x86_64-unknown-linux-musl/release/embyr-agent` binary. Zero new infrastructure — reuses
GitHub's own built-in artifact mechanism, matching ADR-001's literal language ("a separately
compiled, statically-linked Rust binary"). Default retention, no registry, no credentials.

**Secondary: `crates/embyr-agent/Dockerfile`**, wrapping the already-built musl binary
(`FROM scratch`, no compilation inside the image — the binary is copied in from the CI job's
previous step, not rebuilt). Warranted, not scope creep, because
`docs/product/journeys/agent-deployment.yaml` step 2 is `kubectl apply -f agent-deployment.yaml` —
a Kubernetes manifest requires a container image; a bare binary artifact alone leaves Riley's own
already-documented deployment step unfulfillable. `FROM scratch` (not `debian:bookworm-slim`, unlike
the root Dockerfile) is possible specifically because the binary is fully statically linked with zero
`openssl-sys`/glibc runtime dependency (confirmed rustls/ring-only dependency tree, § Context) — no
package manager, no shell, no CA bundle needed at runtime (embyr-agent's only outbound connection is
mTLS to embyr SaaS using customer-supplied cert material, not public-CA-verified HTTPS). Runs as a
numeric non-root UID (`65532`, the common distroless "nonroot" convention — `scratch` has no
`/etc/passwd`, so numeric UID is the only option, same intent as the root Dockerfile's
`useradd --system` non-root pattern).

The Dockerfile build step in the new `agent` job **builds and tags only — no registry push**,
mirroring the existing `docker` job's own exact pattern for `embyr-server` (confirmed by reading
`ci.yml`: it builds+tags `embyr-server:ci` with no push step anywhere). Registry publishing is
audit finding #19's scope, not this feature's.

Cache: `--cache-from type=gha,scope=agent` / `--cache-to type=gha,mode=max,scope=agent` — the
explicit `scope=agent` keeps this job's GitHub Actions cache layer isolated from the existing
`docker` job's own (unscoped) cache, so the two jobs cannot thrash or collide on cache keys
(AC-EARP-04: no shared state between the new job and the existing ones).

### 4. Artifact verification: reuse the existing `us_a06_lifecycle.rs` acceptance tests, run against the real musl release binary

`agent_logs_storage_readiness_before_accepting_connections` and
`storage_credential_never_appears_in_agent_logs` already assert exactly AC-EARP-02's contract (real
Postgres via testcontainers, real TLS cert fixtures via `rcgen`, real subprocess spawn of
`env!("CARGO_BIN_EXE_embyr-agent")`, asserting "connected to Postgres" precedes "listening on
:9191"). DELIVER removes the now-stale `#[ignore = "requires Docker + embyr-agent binary — unskip in
S06A delivery"]` attribute from both — the reason those tests were skipped becomes false once this
job exists. The `agent` job's build step is a single command that both compiles the release musl
binary AND runs these two tests against it:

```
cargo test --release --target x86_64-unknown-linux-musl -p embyr-agent --test embyr_agent -- \
  agent_logs_storage_readiness_before_accepting_connections \
  storage_credential_never_appears_in_agent_logs
```

This verifies the **actual shipped artifact** (musl release build), not a separate glibc debug
build — a musl-target binary can hit substrate differences (assembly codegen in `ring`, static
linking behavior) a glibc-target test run would never surface. This is the Earned Trust check for
this design: CI does not trust "compiles for musl" as proof of "runs correctly on musl" — it runs
the real `StartupProbe` (Postgres connectivity gate + TLS cert validity gate, already implemented in
`crates/embyr-agent/src/probe.rs`) against the actual compiled artifact before calling the pipeline
done.

Scoped to these two tests by name filter — not the full `embyr_agent` acceptance suite (which
already runs against the default target inside the `test` job's `cargo test --workspace`). Running
the full suite a second time against a second target would double CI cost for a walking-skeleton
feature whose locked AC (AC-EARP-02) is specifically about startup/probe behavior, not full RPC
functional parity across targets.

**Known limitation, documented not silently accepted**: the smoke test connects to Postgres via
`127.0.0.1` (testcontainers-assigned port), an IP literal — it does not exercise musl's
`getaddrinfo`/DNS-resolver path, which differs from glibc's (no NSS module support). If a real
customer DSN uses a hostname requiring specific resolver behavior, that path remains unverified by
this pipeline. Not a blocker for this feature (out of DISCUSS's locked scope), flagged as a residual
risk for platform-architect / a future feature to pick up if it becomes a real incident.

## Alternatives Considered

1. **GitHub Actions artifact only, no Dockerfile.** Rejected: leaves
   `agent-deployment.yaml`'s own `kubectl apply` step unfulfillable — a Kubernetes Deployment cannot
   reference a bare binary.

2. **Dockerfile only, drop the raw binary artifact.** Rejected: ADR-001's own language describes "a
   separately compiled, statically-linked Rust binary," and non-Kubernetes VPC operators (e.g.
   systemd unit on a bare VM — not ruled out by any persona document) should not be forced to run a
   container runtime just to obtain the binary. The raw artifact is strictly more flexible and is
   the primary mechanism per DISCUSS's own instruction to pick one primary, zero-new-infra mechanism.

3. **`needs: [test]` (mirrors the `docker` job exactly).** Rejected after self-review: GitHub
   Actions' skip-on-failed-dependency semantics would make a compile break inside `embyr-agent`
   surface as a *skipped* `agent` job (since `test` compiles the whole workspace and would fail
   first), not the *failed* job AC-EARP-05 requires. See Decision § 2.

4. **Multi-arch (x86_64 + aarch64) matrix build.** Rejected: no documented customer requirement;
   adds QEMU/cross-toolchain complexity and CI cost now for unvalidated demand. Reversible — add a
   matrix dimension later if a customer requests arm64.

5. **`cross` or `cargo-zigbuild` for the musl build.** Rejected: unnecessary — `ubuntu-latest`
   runners are already x86_64, so `x86_64-unknown-linux-musl` is a same-arch, libc-only target
   swap achievable with `rustup target add` + `musl-tools`, not a cross-arch build. `cross` solves a
   problem this job doesn't have.

6. **Push the container image to a registry (GHCR/Docker Hub) in this feature.** Rejected: out of
   scope. Audit finding #19 ("no release process — crate version frozen at 0.1.0, no CHANGELOG, no
   git tags") already separately tracks release/publish infrastructure for both binaries; this
   feature closes finding #8 only (a build path exists and is verified), not finding #19 (a
   promotion/publish/versioning path).

## Consequences

**Positive:**
- Closes the last of 8 audit Blockers with zero new infrastructure (no registry, no new secrets, no
  external service) — matches this feature's own narrow, walking-skeleton scope.
- The `agent` job structurally cannot slow down or gate `test`/`lint`/`docker` (no shared `needs:`
  edge in either direction).
- Verification exercises the actual shipped artifact, not a proxy build — closes an Earned Trust
  gap that a "just check it compiles" design would have left open.
- Both artifact shapes needed by the two known deployment paths (bare-VPC install and Kubernetes)
  are produced from a single job, no duplicate compilation.

**Negative / accepted costs:**
- `agent` job can go green even when `test` is independently red for unrelated reasons — mitigated
  by the artifact being per-run/transient, not a published release.
- musl-resolver/DNS-hostname behavior remains unverified by this pipeline (testcontainers uses an IP
  literal) — documented as a residual risk, not silently dropped.
- A second Rust toolchain target now needs installing/caching in CI (marginal CI-minute cost),
  isolated to the new job only.
