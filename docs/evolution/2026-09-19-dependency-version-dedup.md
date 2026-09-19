# Evolution: dependency-version-dedup

**Date:** 2026-09-19
**Commit:** `afa97f7`

## This closes finding #27 from `docs/product/production-readiness-audit-2026-09-08.md`

## Business Context

The audit flagged 4 duplicate dependency-version pairs in the workspace lockfile (testcontainers, tower, base64, thiserror) as binary-bloat/patch-fragmentation risk. This closes the finding by fixing the one pair that was actually fixable, and documenting the rest.

## Key Decisions

| Decision | Verdict |
|---|---|
| base64 0.21.7/0.22.1 duplicate | Fixed by **deleting** the entirely-unused `config` (config-rs) crate from `embyr-server`/`embyr-agent`/`embyr-admin`, not by bumping any version — zero call sites used its API, it only existed to drag in `ron` → `base64 0.21.7` |
| Side effect | 26 packages dropped from `Cargo.lock` (config, ron, json5, rust-ini, yaml-rust2, convert_case, pathdiff, pest*, toml*, winnow, etc.) |
| thiserror 1/2, tower 0.4/0.5 | Documented as genuinely transitive/unfixable — pulled in by third-party crates' (tonic, testcontainers, bollard) own internal pins; closing tower requires a tonic 0.12→0.13+ major bump, out of scope |
| testcontainers 0.21/0.23 | Flagged for a dedicated future feature — `testcontainers-modules` bump has real breaking builder-API changes (`RunnableImage` → `ContainerRequest`/`ImageExt`) across many call sites |
| ADR | None — dependency housekeeping, no architectural decision |

## Lessons

- The real fix here wasn't a version bump, it was noticing the dependency causing the duplicate was entirely unused — worth checking "is this dependency even used" before reaching for a version-unification fix.

## Key Files

- `Cargo.toml`, `crates/embyr-server/Cargo.toml`, `crates/embyr-agent/Cargo.toml`, `crates/embyr-admin/Cargo.toml` — removed `config` dep
- `Cargo.lock` — 26 packages dropped
- `docs/feature/dependency-version-dedup/feature-delta.md` — full investigation of all 4 pairs

## Follow-Up Work

- `testcontainers-modules` bump as its own dedicated future feature (call-site audit for breaking builder API)
- Finding #28+ next in audit order
