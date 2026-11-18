# Release Process

## Overview

embyr-server ships as a single workspace-versioned binary (`version.workspace = true` across
every crate). This document defines what "a release" means: a version bump, a `CHANGELOG.md`
entry, and a `vMAJOR.MINOR.PATCH` git tag on the merge commit — so an operator can always answer
"what version is running" and "what changed since the last deploy" without reading raw git
history. See `docs/product/gaps.md` finding #19.

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
git push origin vX.Y.Z
```

Pushing the tag is what publishes the release: `.github/workflows/ci.yml` runs on `v*` tags,
**fails if the tag does not equal `v` + `[workspace.package] version`**, and otherwise pushes the
`:vX.Y.Z` images. Nothing else ever pushes `:vX.Y.Z`, so a release tag is never overwritten by a
later `master` merge. Never move or re-push an existing git tag — cut a new PATCH instead.

## Container Registry

Both images are published to [GitHub Container Registry](https://ghcr.io) (never from pull
requests, to avoid registry pollution) — see finding #42/#43 in
`docs/product/gaps.md`.

- `ghcr.io/petereon/embyr-server`
- `ghcr.io/petereon/embyr-agent`

| Tag | Pushed by | Mutable? | Use for |
|-----|-----------|----------|---------|
| `:sha-<7-char commit sha>` | every `master` merge and every `v*` tag run | no — one commit, one tag | **deploying and rolling back** |
| `:vX.Y.Z` | a `vX.Y.Z` git tag run only | no, as long as git tags are not moved | naming a release |
| `:master` | every `master` merge | yes — moving pointer | dev/evaluation only, never production |

```bash
docker pull ghcr.io/petereon/embyr-server:sha-f27ff49
docker pull ghcr.io/petereon/embyr-server:v0.1.1      # exists only once the v0.1.1 git tag is pushed
```

The tag run rebuilds from the tagged commit (same source and Dockerfile as that commit's
`:sha-` image, layers reused from the build cache); it is not a registry-side retag.

Before each deploy, record the image you are replacing (`docker inspect -f '{{.Config.Image}}'
embyr-server`, or the tag in the systemd unit) — that is your rollback target.

ghcr.io packages default to **private** on first push regardless of repo visibility — a repo
admin must set each package's visibility to Public once (Package settings → Change visibility)
before an external customer can `docker pull` without authenticating. Until that one-time step is
done, pulling requires `docker login ghcr.io` with a PAT that has `read:packages` on this repo.

## Rollback Procedure

Rollback is **redeploying a previous immutable image** — no checkout, no rebuild.

1. Pick the last known-good image: the `:sha-<short sha>` (or `:vX.Y.Z`) you recorded before the
   deploy, or look it up — `git log --oneline master` gives the short shas in merge order, and
   the package page on ghcr.io lists every pushed tag.
2. Point the deployment at it and restart, e.g. on the single-host setup edit the image tag in
   `/etc/systemd/system/embyr-server.service`, then
   `systemctl daemon-reload && systemctl restart embyr-server`.
3. Confirm: the startup log line (below) shows the expected version and `GET :8081/healthz`
   returns 200.
4. Migrations are not rolled back by this procedure. If the regressed release shipped a
   migration, check that the older binary tolerates the newer schema before rolling back
   (the migrators ignore applied-but-unknown migrations, so it boots; the expand/contract rule
   is what keeps it working); otherwise follow `backup-disaster-recovery.md`. See
   `migration-policy.md`.
5. Read the regressed version's `CHANGELOG.md` entry to narrow root-cause analysis before
   re-attempting the release.

Rebuilding from source (`git checkout <sha> && docker build .`) is the fallback only if the
image is gone from the registry.

## Runtime Correlation

`embyr-server`'s startup log names its own version (sourced from `CARGO_PKG_VERSION`), e.g.:

```text
embyr-server v0.1.1 ready grpc=0.0.0.0:8080 rest=0.0.0.0:8081 admin=0.0.0.0:9090
```

Match this against `git tag -l` and `CHANGELOG.md` to confirm exactly what is running. The
version alone does not identify a `master` build between releases — for that, the deployed
image's `:sha-` tag is authoritative.
