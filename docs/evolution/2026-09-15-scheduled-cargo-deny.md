# Scheduled cargo-deny — 2026-09-15

Closes finding #23 (Medium, Dependencies) from `docs/product/production-readiness-audit-2026-09-08.md`.

`cargo deny check` moved out of `ci.yml`'s `lint` job into its own workflow, `.github/workflows/cargo-deny.yml`, with `on: [push, pull_request, schedule]` (daily cron `0 6 * * *`). Split into a standalone workflow — rather than just adding `schedule:` to `ci.yml` directly — because that trigger applies workflow-wide: bolted onto `ci.yml` it would run the full test/lint/docker/agent job graph (Postgres service, full workspace compile) every night instead of just the advisory check. A newly published RUSTSEC advisory against an unchanged dependency is now caught within 24h instead of waiting for the next push/PR.

No ADR — pure CI/YAML config, no architectural decision.
