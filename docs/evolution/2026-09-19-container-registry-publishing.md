# Evolution: container-registry-publishing

**Date:** 2026-09-19
**Feature:** CI now pushes `embyr-server` and `embyr-agent` container images to GitHub Container
Registry (ghcr.io) on every merge to `master`, tagged `:master` and `:vX.Y.Z`.
**Closes:** findings #42 and #43 (both Blocker) from
`docs/product/production-readiness-audit-2026-09-08.md`'s Follow-Up Scan — 2026-09-19.

## Business Context

Both `docker` and `agent` CI jobs already built and tagged images (`embyr-server:ci`,
`embyr-agent:ci`) but discarded them at job end — no registry push existed anywhere in the repo.
Nothing downstream of "merge to master" was pullable by anything: not a SaaS deploy, not a
customer's Kubernetes cluster. `embyr-agent`'s own release pipeline (closed 2026-09-09 as the
prior Blocker #8) only produced a GitHub Actions artifact (repo-access-gated, ~90-day retention)
plus the same build-and-discard image — never a real distribution path a customer could use
directly, which is finding #43's own restatement of the same root cause.

## Key Decisions

| Decision | Verdict |
|---|---|
| Registry | `ghcr.io` (GitHub Container Registry) — zero new secrets (uses the workflow's own `GITHUB_TOKEN` with `packages: write`), free for this repo, no external account setup. Confirmed no existing registry preference anywhere in the repo (`grep -rn` for ghcr.io/ECR/Docker Hub across `.md`/`.yml`/`.toml` returned zero real hits) |
| Tag scheme | Two tags per push: `:master` (latest master build) + `:vX.Y.Z` (version-pinned, read from root `Cargo.toml`'s `[workspace.package] version` at build time) — matches `docs/operations/release-process.md`'s existing `vMAJOR.MINOR.PATCH` git-tag convention, doesn't invent a new one |
| Push gating | `docker/login-action` + `push: true` only when `github.event_name == 'push' && github.ref == 'refs/heads/master'`. PRs still build (validates the Dockerfile) but never push — avoids registry pollution from every PR |
| Implementation mechanism | Swapped the raw `docker build` shell command for `docker/build-push-action@v6` in both jobs (keeps identical `cache-from`/`cache-to type=gha` semantics, including the `agent` job's `scope=agent` cache isolation) rather than hand-rolling `docker tag`/`docker push` steps |
| Permissions | Added explicit `permissions: contents: read, packages: write` to both `docker` and `agent` jobs (required for `GITHUB_TOKEN` to push to ghcr.io) |
| Image names | `ghcr.io/petereon/embyr-server`, `ghcr.io/petereon/embyr-agent` — no rename of the existing `embyr-server`/`embyr-agent` naming |

## Key Files

- `.github/workflows/ci.yml` — `docker` job: version-extraction step, `docker/login-action`, swapped `docker build` for `docker/build-push-action@v6` with `push`/`tags` gated on master. `agent` job: same pattern for `crates/embyr-agent/Dockerfile`.
- `docs/operations/release-process.md` — new "Container Registry" section documenting the ghcr.io location, tag scheme, and pull command.
- `docs/product/production-readiness-audit-2026-09-08.md` — rows #42/#43 marked CLOSED.

No Dockerfile changes — this is CI-wrapper-only, per findings #42/#43's own scope.

## Verification

- YAML syntax validated (`python3 -c "import yaml; yaml.safe_load(...)"`) — passes.
- `docker/login-action`/`docker/build-push-action` inputs reviewed against their documented
  syntax (`registry`/`username`/`password`; `context`/`file`/`push`/`tags`/`cache-from`/
  `cache-to`) — no typos, correct types.
- Local `docker build .` of the root Dockerfile (unmodified by this change) was attempted to
  confirm nothing else broke, but hit this machine's local Docker Desktop VM memory ceiling
  (2 GiB) partway through compiling `async-stripe*` dependencies — a pre-existing local-environment
  constraint unrelated to this change (no Dockerfile edit was made; a prior `embyr-server-test:ci`
  image already present in the local image cache confirms this Dockerfile has built successfully
  here before). Not re-attempted given the cost of a ~20-minute partial compile. GitHub-hosted
  `ubuntu-latest` runners have materially more memory than this local VM and are unaffected.
- `embyr-agent`'s Dockerfile was not build-tested locally — by design it only `COPY`s a pre-built
  musl release binary that doesn't exist outside the CI `agent` job's own musl build step; a
  standalone local build isn't meaningful here.

## Follow-Up

- **First real push only provable once this lands on `master` and CI actually runs it** — a
  `docker push` to ghcr.io cannot be safely dry-run locally or in a PR by design (that's the whole
  point of the gating). Confirm the first post-merge `docker`/`agent` job run actually shows
  `Pushed` in its log, not just `Built`.
- ghcr.io packages default to **private** on first push regardless of repo visibility. A repo
  admin must flip each package (`embyr-server`, `embyr-agent`) to Public once, in GitHub's Package
  settings, before an external customer can `docker pull` unauthenticated. Documented in
  `docs/operations/release-process.md`; not automatable from within the CI workflow itself.
- `docs/product/journeys/agent-deployment.yaml` was checked and left unmodified — it's a narrative
  journey doc (`kubectl apply -f agent-deployment.yaml` referencing an external manifest file that
  doesn't exist in-repo), not a real Kubernetes manifest with an `image:` field to update. No
  actual K8s manifest with an image reference exists anywhere in the repo.
