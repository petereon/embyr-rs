// SCAFFOLD: true
//! Common test infrastructure — security-rules-cel-parity acceptance tests.
//!
//! Slice 01 (US-01, ADR-062) needed only the admin-only server context
//! (`SecurityRulesAdminContext`) — Activity A / admin-port-only story,
//! identical shape to `security-rules`'s own sr01 (rule define/redefine).
//! Slice 02 (US-02, ADR-062) needs the FULL production composition root
//! (`SecurityRulesFullContext`) too — real gRPC `GetDocument`, mirroring
//! `security_rules`'s own sr02 exactly (Activity B: the imported rule's
//! captured path variable must be evaluated by the REAL
//! `handle_get_document` seam, not a hand-assembled router). Both reused
//! via path import rather than duplicated — mirrors `security_rules`'s own
//! common/mod.rs reusing `client_auth`'s fixture the same way.
//!
//! Slice 03 (US-03, ADR-062) needs real `CreateDocument`/`UpdateDocument`/
//! `DeleteDocument` gRPC calls too — its own acceptance file path-imports
//! `security_rules_write_path`'s fixture module DIRECTLY (not re-exported
//! from here) to avoid a duplicate, nominally-distinct
//! `SecurityRulesFullContext` type: that module ALSO path-imports
//! `security_rules`'s common/mod.rs internally, and re-exporting both
//! chains from this one file would instantiate `security_rules/common`'s
//! source twice within the same test binary, producing two structurally
//! identical but type-incompatible `SecurityRulesFullContext`s.

#![allow(dead_code, unused_imports)]

#[path = "../../security_rules/common/mod.rs"]
pub mod security_rules_common;
pub use security_rules_common::{
    assert_state_delta, mint_client_identity_token, now_unix, set_to, universe,
    SecurityRulesAdminContext, SecurityRulesFullContext,
};
