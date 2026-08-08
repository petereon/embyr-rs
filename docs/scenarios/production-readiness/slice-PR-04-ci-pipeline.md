# Slice PR-04: GitHub Actions CI Pipeline

## Slice Metadata

| Field | Value |
|-------|-------|
| Slice ID | PR-04 |
| Parent Story | US-PR-03 (CI Pipeline Gating Every PR) |
| job_id | JOB-13 |
| Estimate | 0.5 day |
| Depends on | PR-01, PR-02 (real main — needed for tests to pass), PR-03 (Dockerfile — needed for docker build job) |
| Blocks | Nothing (final slice) |

## Deliverable

After this slice, every `git push` to master and every PR targeting master triggers three
automated jobs on GitHub Actions. All three must pass before merge is possible. Sam sees
a green badge at the top of every PR.

## Decision (D-PR-4)

GitHub Actions CI with three jobs:
1. `test`: `cargo test --workspace`
2. `lint`: `cargo clippy -- -D warnings` + `cargo deny check`
3. `docker`: `docker build . -t embyr-server`

Triggered on `push` (master) and `pull_request` (targeting master).

## New Files

### `.github/workflows/ci.yml`

```yaml
name: CI

on:
  push:
    branches: [master]
  pull_request:
    branches: [master]

env:
  CARGO_TERM_COLOR: always
  RUST_BACKTRACE: 1

jobs:
  test:
    name: cargo test
    runs-on: ubuntu-latest
    services:
      postgres:
        image: postgres:15-alpine
        env:
          POSTGRES_USER: postgres
          POSTGRES_PASSWORD: postgres
          POSTGRES_DB: embyr_test
        ports:
          - 5432:5432
        options: >-
          --health-cmd pg_isready
          --health-interval 10s
          --health-timeout 5s
          --health-retries 5
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/registry
            ~/.cargo/git
            target/
          key: ${{ runner.os }}-cargo-test-${{ hashFiles('**/Cargo.lock') }}
          restore-keys: |
            ${{ runner.os }}-cargo-test-
      - name: cargo test
        run: cargo test --workspace
        env:
          DATABASE_URL: postgres://postgres:postgres@localhost:5432/embyr_test

  lint:
    name: clippy + cargo deny
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy
      - uses: actions/cache@v4
        with:
          path: |
            ~/.cargo/registry
            ~/.cargo/git
            target/
          key: ${{ runner.os }}-cargo-lint-${{ hashFiles('**/Cargo.lock') }}
          restore-keys: |
            ${{ runner.os }}-cargo-lint-
      - name: clippy
        run: cargo clippy --workspace --all-targets -- -D warnings
      - name: cargo deny
        uses: EmbarkStudios/cargo-deny-action@v1
        with:
          command: check
          arguments: --all-features

  docker:
    name: docker build
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Set up Docker Buildx
        uses: docker/setup-buildx-action@v3
      - name: Build Docker image
        uses: docker/build-push-action@v5
        with:
          context: .
          push: false
          tags: embyr-server:ci
          cache-from: type=gha
          cache-to: type=gha,mode=max
```

## Implementation Notes

### Why three separate jobs (not one)

Separate jobs run in parallel and give precise failure attribution. If `test` fails while
`lint` passes, Sam sees exactly which gate failed without digging through a combined log.
Branch protection rules can be configured to require all three independently.

### `services: postgres` in the `test` job

The testcontainers-based tests in `system_db.rs` start their own Postgres via Docker. The
`services: postgres` block in the `test` job provides a Postgres instance for any tests that
use `DATABASE_URL` from the environment (future process-level integration tests for PR-02).
Both mechanisms work simultaneously on GitHub-hosted runners (Docker-in-Docker is supported).

### `dtolnay/rust-toolchain@stable`

Uses the community-standard Rust toolchain action. Reads `rust-toolchain.toml` if present;
falls back to latest stable if not. No nightly toolchain required.

### `cargo-deny-action` (EmbarkStudios)

The official `cargo-deny` GitHub Action. Uses the existing `deny.toml` at the workspace root.
No additional configuration required.

### `docker/build-push-action@v5` with `cache-from: type=gha`

GitHub Actions cache is used for Docker layer caching. `cargo-chef` layer caching
(Cargo dependency compilation) is preserved across CI runs. A PR that changes only `src/`
will hit the dependency cache and complete the docker build job in < 60 seconds.

### Branch protection

After the workflow is merged, Sam should enable branch protection on `master`:
- Required status checks: `cargo test`, `clippy + cargo deny`, `docker build`
- Require branches to be up to date before merging
- This is an operator action outside the scope of this PR (cannot be automated via code).

### `RUST_BACKTRACE: 1`

Enables full backtraces for any panics in tests. Improves CI failure diagnosis.

## Acceptance Criteria for This Slice

- [ ] `.github/workflows/ci.yml` exists with `on: push/pull_request` triggers
- [ ] `test` job runs `cargo test --workspace` and fails workflow on any test failure
- [ ] `lint` job runs `cargo clippy -- -D warnings` and fails workflow on any warning
- [ ] `lint` job runs `cargo deny check` and fails workflow on any disallowed crate or advisory
- [ ] `docker` job runs `docker build . -t embyr-server` and fails workflow if build fails
- [ ] All three jobs run in parallel (no sequential dependency in the workflow definition)
- [ ] `actions/cache` is used in `test` and `lint` jobs for `~/.cargo/registry` and `target/`
- [ ] Docker layer cache is preserved across runs via `cache-from: type=gha`
- [ ] `test` job runs with a Postgres service container available on port 5432

## Out of Scope for This Slice

- Pushing the Docker image to a registry (requires secrets setup — deferred)
- `cargo audit` separate job (covered by `cargo deny check` with advisories in deny.toml)
- Release workflow (tag-based Docker publish) — deferred
- Mutation testing (per `CLAUDE.md` strategy: `per-feature`, runs after DELIVER wave)

## Test Guidance

Validation is self-referential: open a PR with this workflow file and verify the three jobs
appear in the GitHub Actions tab. Simulate failures:

1. Introduce a failing test locally, push to PR branch → confirm `test` job fails
2. Introduce an unused variable, push → confirm `lint` job fails with clippy output
3. Break the Dockerfile syntax temporarily, push → confirm `docker` job fails

Remove each artificial failure before merging.
