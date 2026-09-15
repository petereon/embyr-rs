//! tls-startup-warning — acceptance test module root.
//!
//! Single test binary declared via `[[test]] name = "tls_startup_warning"`
//! in `crates/embyr-server/Cargo.toml`.
//!
//! Closes finding #22 (Medium, Security/Ops) from
//! `docs/product/production-readiness-audit-2026-09-08.md`. See
//! `docs/feature/tls-startup-warning/feature-delta.md`.

pub mod common;

mod acceptance {
    mod ts01_startup_warning;
}
