// SCAFFOLD: true
//! Common test infrastructure — security-rules-cel-functions acceptance
//! tests.
//!
//! Reused via `#[path]` import from `security-rules-cel-cross-document-
//! reads`'s own common module directly (this feature's own domain examples
//! are single-collection, flat, no multi-segment/cross-document reads
//! needed) rather than duplicated.

#![allow(dead_code, unused_imports)]

#[path = "../../security_rules_cel_cross_document_reads/common/mod.rs"]
pub mod security_rules_cel_cross_document_reads_common;
pub use security_rules_cel_cross_document_reads_common::{
    mint_client_identity_token, now_unix, SecurityRulesAdminContext, SecurityRulesFullContext,
};
