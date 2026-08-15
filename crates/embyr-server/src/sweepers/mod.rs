//! Background interval tasks (Tokio-interval, advisory-lock-guarded where the
//! task must not run redundantly across instances).
//!
//! `CapUsageRefresher` (ADR-020) is the first module here — no prior sweeper
//! module existed to extend; the only earlier comparable background task is
//! the inline 15-second pool-gauge loop in `lib.rs` (`OBS-05`), which uses no
//! `pg_try_advisory_lock`. `CapUsageRefresher` follows the interval-loop +
//! advisory-lock shape ADR-020 describes.

pub mod cap_usage_refresher;
