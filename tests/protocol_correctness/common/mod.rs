//! Common test infrastructure — protocol-correctness acceptance tests.
//!
//! Reused via `#[path]` from `composite_index_requirement_rules`' own common
//! module: only the full composition root (`SecurityRulesFullContext`) is needed.

#![allow(dead_code, unused_imports)]

#[path = "../../composite_index_requirement_rules/common/mod.rs"]
pub mod cir_common;
pub use cir_common::firestore_composite_indexes_admin_api_common::SecurityRulesFullContext;
