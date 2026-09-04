// SCAFFOLD: true
//! Common test infrastructure — security-rules-cel-expression-grammar
//! acceptance tests.
//!
//! Slice 01 (US-01, ADR-065) needs only the admin-only server context
//! (`SecurityRulesAdminContext`) plus the full composition root
//! (`SecurityRulesFullContext`) for real `GetDocument` enforcement proof —
//! identical shape to every prior JOB-17 CEL epic's own Slice 01. Reused via
//! `#[path]` import from `security-rules-cel-parity`'s own common module
//! directly (this feature's own domain examples are single-collection,
//! flat, no multi-segment/recursive-wildcard routing needed) rather than
//! duplicated.

#![allow(dead_code, unused_imports)]

#[path = "../../security_rules_cel_parity/common/mod.rs"]
pub mod security_rules_cel_parity_common;
pub use security_rules_cel_parity_common::{
    mint_client_identity_token, now_unix, SecurityRulesAdminContext, SecurityRulesFullContext,
};
