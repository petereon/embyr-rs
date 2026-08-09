// SCAFFOLD: true
//! secrets-management — acceptance test module root.
//!
//! Single test binary declared via `[[test]] name = "secrets_management"` in
//! `crates/embyr-server/Cargo.toml`. All acceptance modules are included here.
//!
//! Test placement: `tests/secrets_management/acceptance/`
//! Precedent: mirrors the `tests/production_readiness/` layout (single `mod.rs`
//! root + `common/` + `acceptance/` submodules, one `[[test]]` entry).
//!
//! Walking skeleton:
//!   `sm01_admin_key_secrets_manager::server_starts_with_admin_key_from_aws_secrets_manager`
//!   — the ONLY test NOT marked `#[ignore]`. Proves the AWS-Secrets-Manager-sourced
//!   admin-key startup path end-to-end (US-SM-01), through the production
//!   composition root (real `embyr-server` binary subprocess + real LocalStack).
//!   All other tests are `#[ignore]` — DELIVER unskips them one at a time.
//!
//! Implementation order (mirrors story-map.md release slicing):
//!   1. sm01 — US-SM-01: admin key sourced from AWS/GCP Secrets Manager (walking skeleton)
//!   2. sm02 — US-SM-02: encryption key sourced from AWS/GCP Secrets Manager
//!   3. sm03 — US-SM-03: EMBYR_ENCRYPTION_KEY dual-key rotation window
//!   4. sm04 — US-SM-04: EMBYR_ADMIN_KEY dual-token rotation window
//!
//! Pre-requisites before running any test:
//!   - `cargo build --bin embyr-server` (produces `target/debug/embyr-server`)
//!   - Docker daemon available (Postgres + LocalStack testcontainers)
//!   - `CARGO_MANIFEST_DIR` set by cargo (automatic for `[[test]]` entries)
//!
//! Scaffold classification target: RED for all `#[ignore]` tests once
//! unskipped by DELIVER (`ConfigError`/`decrypt_with_rotation` scaffolds
//! documented in `docs/feature/secrets-management/feature-delta.md`
//! `## Wave: DISTILL` sections).

pub mod common;

mod acceptance {
    mod sm01_admin_key_secrets_manager;
    mod sm02_encryption_key_secrets_manager;
    mod sm03_encryption_key_rotation;
    mod sm04_admin_key_rotation;
}
