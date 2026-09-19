# Release Process

## Overview

embyr-server ships as a single workspace-versioned binary (`version.workspace = true` across
every crate). This document defines what "a release" means: a version bump, a `CHANGELOG.md`
entry, and a `vMAJOR.MINOR.PATCH` git tag on the merge commit — so an operator can always answer
"what version is running" and "what changed since the last deploy" without reading raw git
history. See `docs/product/production-readiness-audit-2026-09-08.md` finding #19.

## When to Bump

Bump the workspace version (root `Cargo.toml`, `[workspace.package] version`) in the same pull
request that changes any of:

- `crates/` (any crate source)
- `migrations/`
- `Dockerfile`
- `crates/embyr-agent/Dockerfile`

Use [Semantic Versioning](https://semver.org/): MAJOR for a breaking wire/API contract change,
MINOR for a new, backward-compatible capability, PATCH for a fix or internal change.

### Exemption

Pull requests that only modify files under `docs/` are **exempt** — a docs/-only change has zero
effect on the running binary, so no version bump, git tag, or `CHANGELOG.md` entry is required.

## CHANGELOG.md

Location: repository root, [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) format.

In the same PR that bumps the version, add a new `## [X.Y.Z] - YYYY-MM-DD` section above the
previous release (below `## [Unreleased]`) with at least a one-line description naming the change
and the finding/feature that motivated it.

## Git Tag Convention

Tag format: `vMAJOR.MINOR.PATCH` (e.g. `v0.1.1`). The tag is created on the **merge commit** that
performs the corresponding version bump — after merge to `master`, a maintainer runs:

```bash
git tag vX.Y.Z <merge-sha>
git push --tags
```

No CI automation enforces this today (no historical drift to guard against yet, and a
`push: tags:`-triggered workflow does not exist). Revisit trigger: if a future release ships
without a matching tag or CHANGELOG entry, add a `release-tag-check` CI job comparing the pushed
tag against `cargo metadata`'s workspace version.

## Rollback Procedure

1. Identify the last known-good tag from `git tag -l` and `CHANGELOG.md`.
2. Check out that tag: `git checkout vX.Y.Z`.
3. Rebuild the Docker image from that exact tagged commit: `docker build -t embyr-server:vX.Y.Z .`.
4. Deploy the rebuilt image in place of the regressed version.
5. Read the regressed version's own `CHANGELOG.md` entry to narrow root-cause analysis before
   re-attempting the release.

## Container Registry

Both images are published to [GitHub Container Registry](https://ghcr.io) on every push to
`master` (never on pull requests, to avoid registry pollution) — see finding #42/#43 in
`docs/product/production-readiness-audit-2026-09-08.md`.

- `ghcr.io/petereon/embyr-server`
- `ghcr.io/petereon/embyr-agent`

Each push publishes two tags: `:master` (always the latest master build) and a version-pinned
`:vMAJOR.MINOR.PATCH` tag read from `[workspace.package] version` — matching the git tag
convention above. Pull a specific release with:

```bash
docker pull ghcr.io/petereon/embyr-server:v0.1.1
docker pull ghcr.io/petereon/embyr-agent:v0.1.1
```

ghcr.io packages default to **private** on first push regardless of repo visibility — a repo
admin must set each package's visibility to Public once (Package settings → Change visibility)
before an external customer can `docker pull` without authenticating. Until that one-time step is
done, pulling requires `docker login ghcr.io` with a PAT that has `read:packages` on this repo.

## Runtime Correlation

`embyr-server`'s startup log names its own version (sourced from `CARGO_PKG_VERSION`), e.g.:

```text
embyr-server v0.1.1 ready grpc=0.0.0.0:8080 rest=0.0.0.0:8081 admin=0.0.0.0:9090
```

Match this against `git tag -l` and `CHANGELOG.md` to confirm exactly what is running.
