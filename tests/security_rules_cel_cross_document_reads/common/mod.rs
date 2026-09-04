// SCAFFOLD: true
//! Common test infrastructure — security-rules-cel-cross-document-reads
//! acceptance tests.
//!
//! Reused via `#[path]` import from `security-rules-cel-expression-grammar`'s
//! own common module directly (this feature's own domain examples are
//! single-collection, flat, no multi-segment/recursive-wildcard routing
//! needed) rather than duplicated.

#![allow(dead_code, unused_imports)]

#[path = "../../security_rules_cel_expression_grammar/common/mod.rs"]
pub mod security_rules_cel_expression_grammar_common;
pub use security_rules_cel_expression_grammar_common::{
    mint_client_identity_token, now_unix, SecurityRulesAdminContext, SecurityRulesFullContext,
};
