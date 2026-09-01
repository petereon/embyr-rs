// SCAFFOLD: true
//! Common test infrastructure — security-rules-cel-parity acceptance tests.
//!
//! Slice 01 (US-01, ADR-062) needs only the admin-only server context
//! (`SecurityRulesAdminContext`) — Activity A / admin-port-only story,
//! identical shape to `security-rules`'s own sr01 (rule define/redefine).
//! Reused via path import rather than duplicated — mirrors
//! `security_rules`'s own common/mod.rs reusing `client_auth`'s fixture
//! the same way.

#![allow(dead_code, unused_imports)]

#[path = "../../security_rules/common/mod.rs"]
pub mod security_rules_common;
pub use security_rules_common::{assert_state_delta, set_to, universe, SecurityRulesAdminContext};
