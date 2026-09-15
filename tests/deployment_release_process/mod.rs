//! deployment-release-process — acceptance test module root.
//!
//! Single test binary declared via `[[test]] name = "deployment_release_process"`
//! in `crates/embyr-server/Cargo.toml`. All acceptance modules are included here.
//!
//! This feature closes finding #19 (High, DevOps) from
//! `docs/product/production-readiness-audit-2026-09-08.md`, split into two
//! independently-shippable stories:
//!   - US-DRP-01: version / git tag / CHANGELOG / startup log convention
//!   - US-DRP-02: docker-compose.yml for local/single-host evaluation
//!
//! Walking skeletons (one per story — the feature bundles two independently
//! shippable slices, per Mandate 5's "2-5 per feature" allowance):
//!   - `drp01_startup_version_log::startup_log_names_the_running_version`
//!     — NOT #[ignore]. Real subprocess + real Postgres testcontainer.
//!   - `drp02_compose_lifecycle::*` — #[ignore] (real Docker Compose I/O,
//!     2 containers; run explicitly, never in the default `cargo test` sweep
//!     on this machine's constrained resources).
//!
//! All other tests are #[ignore] — DELIVER unskips them one at a time.
//!
//! Test placement precedent: mirrors `tests/production_readiness/` layout
//! (mod.rs + common/ + acceptance/).
//!
//! Implementation order:
//!   1. drp01_startup_version_log — US-DRP-01 walking skeleton: main.rs
//!      Step 12 log line gains a `version` field (DESIGN's exact diff).
//!   2. drp01_changelog_structure — US-DRP-01: CHANGELOG.md exists, is in
//!      Keep a Changelog format, gains a dated `[0.1.1]` entry this feature's
//!      own merge produces.
//!   3. drp01_release_process_doc — US-DRP-01: docs/operations/release-process.md
//!      states the bump/tag/exemption/rollback convention.
//!   4. drp02_compose_structure — US-DRP-02: docker-compose.yml exists, is
//!      valid, builds from the root Dockerfile only, defines both services
//!      with the right ports/env/volume, labels dev-only credentials.
//!   5. drp02_compose_lifecycle — US-DRP-02 walking skeleton: a real
//!      `docker compose up` / restart / `down -v` round trip.
//!
//! Pre-requisites before running any test:
//!   - drp01_*: none beyond `cargo test -p embyr-server` (builds the binary
//!     automatically via `CARGO_BIN_EXE_embyr-server`); Docker daemon
//!     available for the Postgres testcontainer.
//!   - drp02_compose_structure: Docker daemon available (for `docker compose
//!     config`); gracefully skips if absent.
//!   - drp02_compose_lifecycle: Docker daemon available; NEVER run alongside
//!     another active testcontainers-based test run on this 8GB-RAM machine.

pub mod common;

mod acceptance {
    mod drp01_changelog_structure;
    mod drp01_release_process_doc;
    mod drp01_startup_version_log;
    mod drp02_compose_lifecycle;
    mod drp02_compose_structure;
}
