# Feature: dependency-version-dedup

Closes finding #27 (Medium, Dependencies) — `docs/product/production-readiness-audit-2026-09-08.md`.
Combined DISCUSS+DESIGN, token-budget mode. No implementation performed — architect role only;
recommendations below are handed to DELIVER.

## Method

Audit is from 2026-09-08; several features since then touched `Cargo.toml`/`Cargo.lock`
(pool-sizing-and-limits, cors-origin-policy → tower-http, structured-json-logging →
tracing-subscriber json feature). Rather than trust the audit's stale list, the CURRENT
`Cargo.lock` (ground truth `cargo tree --duplicates`/`cargo tree -i` read from) was inspected
directly — same data, no build required, safe under the 8GB RAM / jobs=2 constraint.

## Current duplicate list (2026-09-19) — all 4 audit pairs still present

| package | old | new | pulled by (old) | fixable? |
|---|---|---|---|---|
| `base64` | 0.21.7 | 0.22.1 | `ron` ← `config` (config-rs) default features | **YES — remove unused `config` dep** |
| `testcontainers` | 0.21.1 | 0.23.1 | `testcontainers-modules` 0.9.0's own internal pin | Follow-up feature (breaking-API risk) |
| `thiserror` | 1.0.69 | 2.0.18 | `asn1-rs`, `bollard`, `gloo-net`, `metrics-exporter-prometheus` 0.15.3, `redox_users`, `testcontainers` (both) | NO — pure transitive |
| `tower` | 0.4.13 | 0.5.3 | `tonic` 0.12.3's own internal pin | NO (today) — needs tonic 0.13+ major bump |

Also present in the lock file but **out of scope**, same "pure transitive" category, no self-inflicted
workspace pin on either side, and universal to any Rust workspace this size: `bitflags` 1/2,
`getrandom` 0.2/0.3/0.4, `http` 0.2/1, `indexmap` 1/2, `rand_core` 0.6/0.9/0.10,
`rustls-native-certs` 0.7/0.8, `socket2` 0.5/0.6, `windows-sys` 0.48/0.52/0.61. Right-sizing per
task guidance: documenting each of these individually would be noise, not signal — none has an
actionable owner on our side.

## Investigation detail

### 1. base64 0.21.7 vs 0.22.1 — FIXABLE, recommend REMOVAL (not a version bump)

Root cause chain: workspace `config = "0.14"` (config-rs, `Cargo.toml` line 66) → default
features include `ron` format support → `ron 0.8.1` → `base64 0.21.7`. The workspace's own
`base64 = "0.22"` pin is already the newer version (tonic 0.12.3 and
metrics-exporter-prometheus 0.15.3 both already resolve to 0.22.1 too).

Grepped every `.rs` file in the repo for `use config::`, `config::Environment`, `config::File`,
`config::Config::` (config-rs's actual API surface) — **zero matches**. Every `config::`
reference in the codebase is either a crate's own local `crate::config` module (`ServerConfig`,
`AgentConfig`, `DbPrepConfig`, `TlsMaterial`) or `aws_sdk_secretsmanager::config`. The `config`
crate is declared but **entirely unused** in `embyr-server`, `embyr-agent`, and `embyr-admin`.

**Recommendation:** delete `config = "0.14"` from `[workspace.dependencies]` and the 3
`config.workspace = true` lines (`crates/embyr-server/Cargo.toml`,
`crates/embyr-agent/Cargo.toml`, `crates/embyr-admin/Cargo.toml`). This removes `config`, and
with it `ron` → `base64 0.21.7`, `json5`, `rust-ini`, `yaml-rust2`, `convert_case`, `pathdiff`
from the graph entirely — closing the duplicate completely rather than narrowing it, and trimming
5+ unused transitive crates as a bonus.

**Compile-check plan for DELIVER:** after removal, `cargo check -p embyr-server -p embyr-agent
-p embyr-admin` (scoped, not `--workspace`), then confirm `base64 0.21.7` no longer appears in
`cargo tree --duplicates`.

### 2. testcontainers 0.21.1 vs 0.23.1 — fixable in principle, NOT recommended this pass

Root cause: workspace `testcontainers-modules = "0.9"` resolves to `testcontainers-modules
0.9.0`, whose own `Cargo.toml` pins `testcontainers` to `^0.21` internally. The workspace's
direct `testcontainers = "0.23"` pin is unrelated and already the newer version.

A newer `testcontainers-modules` release may depend on `testcontainers ^0.23+`, which would
close this — but confirming that needs a live crates.io lookup not performed in this pass, AND
`testcontainers-modules` has had real breaking API changes across its 0.9→0.11+ line (the
`RunnableImage` builder was replaced by `ContainerRequest`/`ImageExt`). This workspace's test
suite constructs Postgres test containers across dozens of files (per prior mutation-testing
Docker-contention incident on record) — auditing every call site for a new builder API is real
work, not a one-line bump.

**Recommendation:** document only. A future dedicated feature should (a) check the latest
`testcontainers-modules` release and its `testcontainers` pin, (b) grep every
`testcontainers_modules::postgres::Postgres` call site, (c) scope the bump + call-site migration
as its own DISCUSS/DESIGN/DELIVER cycle.

### 3. thiserror 1.0.69 vs 2.0.18 — transitive, unfixable today

Pulled by `asn1-rs`, `bollard`/`bollard-stubs` (docker SDK, testcontainers's own dep),
`gloo-net` (wasm/leptos side, `embyr-admin-ui` only), `metrics-exporter-prometheus 0.15.3`,
`redox_users`, and `testcontainers` itself (both 0.21.1 and 0.23.1 — testcontainers hasn't
moved off thiserror 1 even in its own newer release). All third-party internal pins; every
crate in this workspace already correctly uses the `thiserror = "2"` pin. Nothing to bump on
our side. Monitor `metrics-exporter-prometheus` — worth a glance next time that pin is
routinely touched, not worth a dedicated investigation now.

### 4. tower 0.4.13 vs 0.5.3 — transitive, unfixable today

`tonic 0.12.3` (workspace `tonic = "0.12"`) depends on `tower 0.4` internally — tonic's own
choice. The workspace's own `tower = "0.5"` pin (used directly for axum/tower-http middleware
in `embyr-server` and `embyr-admin-ui` dev-deps) already resolves correctly to 0.5.3. Closing
this means bumping tonic to a release that adopted tower 0.5 (tonic 0.13+) — a major-version
bump touching every gRPC service definition, interceptor, and streaming handler across
`embyr-server` AND `embyr-agent` (the core protocol-translation layer per this repo's
`CLAUDE.md`). Out of scope for a dependency-hygiene pass.

**Recommendation:** document only. Flag tonic 0.12→0.13+ as its own future migration feature.

## ADR decision

**None.** This is dependency housekeeping (one dead-weight removal + documentation of
currently-unfixable transitive duplicates), not an architectural decision — no component
boundary, integration pattern, or quality-attribute trade-off changes.

## Self-review

- The one change recommended for execution (delete unused `config` dependency) has a concrete,
  narrow compile-check plan: `cargo check -p embyr-server -p embyr-agent -p embyr-admin`, not a
  workspace-wide rebuild — respects the 8GB RAM / jobs=2 constraint.
- `testcontainers-modules` and `tonic` bumps are explicitly **not** recommended for execution in
  this pass — flagged as separate future features requiring their own call-site audits. No
  premature version bump risked.
- No `cargo update` run, no broad dependency-tree changes attempted.
- Legitimate outcome: 1 real fix (a deletion, cleaner than a version bump) + 2 documented
  transitive-unfixable pairs + 1 follow-up candidate flagged for its own future feature.
