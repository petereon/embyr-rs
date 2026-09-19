# Env-Var Documentation

**Status:** FINALIZED 2026-09-19
**Closes:** Medium finding #28 (DevOps), `docs/product/production-readiness-audit-2026-09-08.md`

## Business Context

Env-var documentation was scattered across 3 config structs' own rustdoc
(`ServerConfig`, `AgentConfig`, `DbPrepConfig`) with no root-level index —
an operator standing up embyr-rs had to read 3 separate Rust source files to
find every variable. Pure documentation/config-aggregation fix: no Rust code
changed, no tests, no mutation testing applies.

## What Was Created

- `.env.example` (repo root) — every env var across `embyr-server` (35),
  `embyr-agent` (10), and `embyr-db-prep` (4), grouped by binary, each with a
  one-line description, default (if any), and required/optional status.
  Derived directly from the 3 config structs' rustdoc and `from_env()` parsing
  logic — no values invented.
- `README.md` (repo root, did not previously exist) — 3-binary workspace
  overview plus a "Configuration" section pointing to `.env.example` as the
  single cross-binary source of truth, rather than duplicating per-field
  detail in a second place.

The 3 config structs' own rustdoc was left untouched — it remains the
authoritative per-field detail; `.env.example`/README are a discovery layer
on top.

## Key Files

- `.env.example`
- `README.md`
- `crates/embyr-server/src/config.rs` (`ServerConfig`, read-only)
- `crates/embyr-agent/src/config.rs` (`AgentConfig`, read-only)
- `crates/embyr-db-prep/src/config.rs` (`DbPrepConfig`, read-only)

## Follow-Up

Keep `.env.example` in sync manually when a config struct gains/loses a
variable — no CI check enforces this yet. A future feature could add a
pre-commit or CI grep diffing `std::env::var(...)` call sites in the 3
config.rs files against `.env.example` entries to catch drift automatically.
