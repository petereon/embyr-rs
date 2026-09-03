// SCAFFOLD: true
//! Common test infrastructure — security-rules-cel-path-matching acceptance
//! tests.
//!
//! Slice 01 (US-01, ADR-063) needed only the admin-only server context
//! (`SecurityRulesAdminContext`) — the import-and-decompose story, identical
//! shape to `security-rules-cel-parity`'s own cp01. Slice 02 (US-02, ADR-063)
//! needs the FULL production composition root (`SecurityRulesFullContext`)
//! too — real gRPC `GetDocument`, mirroring `security-rules-cel-parity`'s own
//! cp02 exactly (the routed pattern's captured ancestor variable must be
//! evaluated by the REAL `handle_get_document` seam). Both reused via
//! `#[path]` import from that feature's own common module rather than
//! duplicated, mirroring the SAME precedent `security-rules-cel-parity`'s own
//! common/mod.rs established one feature ago.

#![allow(dead_code, unused_imports)]

#[path = "../../security_rules_cel_parity/common/mod.rs"]
pub mod security_rules_cel_parity_common;
pub use security_rules_cel_parity_common::{
    mint_client_identity_token, now_unix, SecurityRulesAdminContext, SecurityRulesFullContext,
};
