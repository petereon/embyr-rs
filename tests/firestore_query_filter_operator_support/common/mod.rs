// SCAFFOLD: true
//! Common test infrastructure — firestore-query-filter-operator-support
//! acceptance tests.
//!
//! Reused via `#[path]` import from `security_rules`'s own common module
//! directly — this feature needs no access-rule-specific scaffolding (an
//! unrestricted collection, no rule defined), just the full composition
//! root (`SecurityRulesFullContext`) for real `RunQuery` proof.

#![allow(dead_code, unused_imports)]

#[path = "../../security_rules/common/mod.rs"]
pub mod security_rules_common;
pub use security_rules_common::SecurityRulesFullContext;
