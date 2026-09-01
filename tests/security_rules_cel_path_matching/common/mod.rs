// SCAFFOLD: true
//! Common test infrastructure — security-rules-cel-path-matching acceptance
//! tests.
//!
//! Slice 01 (US-01, ADR-063) needs only the admin-only server context
//! (`SecurityRulesAdminContext`) — the import-and-decompose story, identical
//! shape to `security-rules-cel-parity`'s own cp01. Reused via `#[path]`
//! import from that feature's own common module rather than duplicated,
//! mirroring the SAME precedent `security-rules-cel-parity`'s own
//! common/mod.rs established one feature ago (which itself path-imports
//! `security_rules`'s common/mod.rs).

#![allow(dead_code, unused_imports)]

#[path = "../../security_rules_cel_parity/common/mod.rs"]
pub mod security_rules_cel_parity_common;
pub use security_rules_cel_parity_common::SecurityRulesAdminContext;
