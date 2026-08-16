// SCAFFOLD: true
//! Error classification for embyr-db-prep.
//!
//! Classifies the underlying `sqlx`/Postgres error (connect-phase vs.
//! `SQLSTATE 42501` insufficient_privilege vs. other) into the 3 named
//! message shapes UAT requires (AC-01-01, AC-01-04, AC-01-05). Role and
//! database are named from the DSN's own components, not from server
//! response content (`docs/product/architecture/brief.md` § Driven Ports +
//! Adapters — customer-db-onboarding, Earned Trust probe design table).
//!
//! RED scaffold (DISTILL wave, feature customer-db-onboarding, 2026-08-16).
//! DELIVER implements classification via Outside-In TDD.

use embyr_core::error::CoreError;

/// Classify an adapter-layer error into one of the 3 named message shapes:
/// connection failure, insufficient privilege, or a generic fallback.
///
/// SCAFFOLD: true — RED scaffold (DISTILL wave, feature customer-db-onboarding).
pub fn classify(err: &CoreError) -> String {
    let _ = err;
    panic!(
        "error_report::classify: not yet implemented \
         -- RED scaffold (DISTILL wave, feature customer-db-onboarding)"
    )
}
