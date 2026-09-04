// SCAFFOLD: true
//! Common test infrastructure — composite-index-requirement-rules
//! acceptance tests.
//!
//! Reused via `#[path]` import from `firestore_composite_indexes_admin_api`'s
//! own common module directly — this feature needs no access-rule-specific
//! scaffolding either (its own domain example uses an unrestricted
//! collection with no rule defined), just the full composition root
//! (`SecurityRulesFullContext`) for real `RunQuery`/`CreateIndex`
//! enforcement proof.

#![allow(dead_code, unused_imports)]

#[path = "../../firestore_composite_indexes_admin_api/common/mod.rs"]
pub mod firestore_composite_indexes_admin_api_common;
pub use firestore_composite_indexes_admin_api_common::SecurityRulesFullContext;
