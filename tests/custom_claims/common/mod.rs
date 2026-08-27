//! Common test infrastructure — custom-claims acceptance tests (Slice 02,
//! ADR-034).
//!
//! Reuses `SecurityRulesFullContext`/`mint_client_identity_token`/`now_unix`/
//! `create_document`/`string_field` from `security-rules-write-path`'s own
//! fixture module via a path import (this feature's own driving port is
//! real gRPC `GetDocument`, gated by the SAME `access_rules` table +
//! `evaluate()` routine every prior authorization epic already uses;
//! `create_document` is needed to seed the NEW `flagged_content`/
//! `support_tickets` domain-example documents as real GetDocument-callable
//! preconditions, not fixture-injected end-state — `write_access_rules` has
//! no row for either collection, so creation succeeds via the existing
//! structural no-rule-defined guardrail, unaffected by this feature). No
//! new driving-port context class needed, mirroring
//! `security-rules-query-path`'s own identical reuse-chain precedent.
//! `tests/security_rules/` and `tests/security_rules_write_path/` files are
//! never touched — only imported, read-only, from here.

#![allow(dead_code, unused_imports)]

#[path = "../../security_rules_write_path/common/mod.rs"]
mod security_rules_write_path_common;
pub use security_rules_write_path_common::{
    create_document, mint_client_identity_token, now_unix, string_field, SecurityRulesFullContext,
};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use ed25519_dalek::{Signer, SigningKey};

/// Mint a client-identity token carrying arbitrary extra claims
/// (custom-claims US-01/US-02, ADR-034) alongside the required
/// `sub`/`aud`/`exp` — plays the role of Trailmark's own backend embedding
/// e.g. `is_moderator: true` at mint time. Mirrors
/// `security_rules_common::mint_client_identity_token`'s exact shape (real
/// Ed25519 signing, not a mock of any embyr-owned port), extended with one
/// additional parameter this slice's own domain examples need — kept local
/// to this feature's own fixture module rather than editing
/// `tests/client_auth/common/mod.rs` (never touched, per the established
/// reuse-chain read-only discipline).
pub fn mint_client_identity_token_with_claims(
    signing_key: &SigningKey,
    sub: &str,
    aud: &str,
    exp_unix: i64,
    extra_claims: serde_json::Value,
) -> String {
    let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"EdDSA","typ":"JWT"}"#);
    let mut payload_obj = serde_json::json!({"sub": sub, "aud": aud, "exp": exp_unix});
    if let (Some(payload_map), Some(extra_map)) =
        (payload_obj.as_object_mut(), extra_claims.as_object())
    {
        for (k, v) in extra_map {
            payload_map.insert(k.clone(), v.clone());
        }
    }
    let payload = URL_SAFE_NO_PAD.encode(payload_obj.to_string());
    let signing_input = format!("{header}.{payload}");
    let signature = signing_key.sign(signing_input.as_bytes());
    let sig_b64 = URL_SAFE_NO_PAD.encode(signature.to_bytes());
    format!("{signing_input}.{sig_b64}")
}
