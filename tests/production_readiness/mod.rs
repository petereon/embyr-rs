// SCAFFOLD: true
//! production-readiness — acceptance test module root.
//!
//! Single test binary declared via `[[test]] name = "production_readiness"` in
//! `crates/embyr-server/Cargo.toml`. All acceptance modules are included here.
//!
//! Test placement: `tests/production_readiness/acceptance/`
//! Precedent: mirrors the `tests/distributed_rate_limiting/` layout.
//!
//! Walking skeleton: `pr01_config_from_env::server_starts_with_all_required_env_vars_set`
//!   — the ONLY test NOT marked #[ignore]. Proves the full startup path end-to-end.
//!   All other tests are #[ignore] — DELIVER unskips them one at a time.
//!
//! Implementation order:
//!   1. pr01 — US-PR-01: env-var config + main() startup (D-PR-1 through D-PR-6)
//!   2. pr02 — US-PR-02: Dockerfile multi-stage build (D-PR-3)
//!   3. pr03 — US-PR-03: CI pipeline (.github/workflows/ci.yml) (D-PR-4)
//!   4. pr04 — US-PR-01: graceful shutdown on SIGTERM (D-PR-5)
//!   5. pr05 — firestore-tls-support US-01: in-process TLS on all 3
//!      listeners via EMBYR_TLS_CERT_PATH/EMBYR_TLS_KEY_PATH (AC-TLS-01..07)
//!
//! Pre-requisites before running any test:
//!   - none — `cargo test` builds `embyr-server` automatically via `CARGO_BIN_EXE_embyr-server`
//!   - Docker daemon available for PR-02 tests
//!   - `CARGO_MANIFEST_DIR` set by cargo (automatic for `[[test]]` entries)

pub mod common;

mod acceptance {
    mod pr01_config_from_env;
    mod pr02_dockerfile;
    mod pr03_ci;
    mod pr04_graceful_shutdown;
    mod pr05_tls_support;
}
