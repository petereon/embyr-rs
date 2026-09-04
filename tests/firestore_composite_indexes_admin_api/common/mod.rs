// SCAFFOLD: true
//! Common test infrastructure — firestore-composite-indexes-admin-api
//! acceptance tests.
//!
//! Reused via `#[path]` import from `security_rules`'s own common module
//! directly — this feature needs no access-rule-specific scaffolding (its
//! own domain example uses an unrestricted collection with no rule
//! defined), just the full composition root (`SecurityRulesFullContext`)
//! for real `RunQuery` + admin HTTP enforcement proof.

#![allow(dead_code, unused_imports)]

#[path = "../../security_rules/common/mod.rs"]
pub mod security_rules_common;
pub use security_rules_common::SecurityRulesFullContext;
