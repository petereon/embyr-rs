//! Background interval tasks (Tokio-interval, advisory-lock-guarded where the
//! task must not run redundantly across instances).
//!
//! `SCAFFOLD: true` — this module directory is created by DISTILL
//! (card-payments-backend). DESIGN's Component Decomposition table described
//! `CapUsageRefresher` as mirroring "the existing `QueryLogSweeper`/
//! `SessionCleaner` advisory-lock sweeper pattern" — DISTILL's own reading of
//! the codebase (`crates/embyr-server/src/lib.rs`, `src/adapters/query_log.rs`)
//! found no such prior sweeper module: today's only comparable background
//! task is an inline 15-second pool-gauge loop in `lib.rs` (`OBS-05`), with no
//! `pg_try_advisory_lock` usage anywhere in the workspace. This is flagged
//! explicitly in `docs/feature/card-payments-backend/distill/upstream-issues.md`
//! as a DESIGN-wave precedent-citation gap, not silently absorbed — DISTILL
//! did not invent a false "extends existing code" narrative to match it;
//! `CapUsageRefresher` below is built fresh, following the *shape* ADR-020
//! describes (interval loop + advisory lock) without literally reusing
//! nonexistent code.

pub mod cap_usage_refresher;
