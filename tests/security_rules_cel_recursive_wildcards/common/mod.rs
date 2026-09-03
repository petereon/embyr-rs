// SCAFFOLD: true
//! Common test infrastructure — security-rules-cel-recursive-wildcards
//! acceptance tests.
//!
//! Slice 01 (US-01, ADR-064) needs only the admin-only server context
//! (`SecurityRulesAdminContext`) — the import-and-decompose story, identical
//! shape to `security-rules-cel-path-matching`'s own pm01. Reused via
//! `#[path]` import from that feature's own common module rather than
//! duplicated, mirroring the SAME precedent established one feature ago.

#![allow(dead_code, unused_imports)]

#[path = "../../security_rules_cel_path_matching/common/mod.rs"]
pub mod security_rules_cel_path_matching_common;
pub use security_rules_cel_path_matching_common::{
    mint_client_identity_token, now_unix, SecurityRulesAdminContext, SecurityRulesFullContext,
};
