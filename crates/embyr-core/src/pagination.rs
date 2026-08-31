//! Pagination `page_token` encode/decode (firestore-list-rpcs, ADR-050/051).
//!
//! Hex-encoded-offset scheme, ported IN SHAPE from the already-proven
//! `crates/embyr-agent/src/server.rs::encode_page_token`/`decode_page_token`
//! (left untouched there — this is a NEW copy, not a refactor of the agent
//! binary's own dead-after-this-feature private functions, per DESIGN's own
//! Escalation Resolutions). Shared by BOTH `handle_list_documents` and
//! `handle_list_collection_ids` so KPI #4 ("zero pagination divergence
//! between the two RPCs") holds by construction — one function, two callers.
//!
//! Pure, no IO — `embyr-core`'s NO-IO constraint (enforced by `deny.toml`).

use crate::error::CoreError;

/// Encode an offset as a hex string page token.
pub fn encode_page_token(offset: u32) -> String {
    format!("{offset:08x}")
}

/// Decode a hex page token back to an offset. Empty token = offset 0.
pub fn decode_page_token(token: &str) -> Result<u32, CoreError> {
    if token.is_empty() {
        return Ok(0);
    }
    u32::from_str_radix(token, 16)
        .map_err(|_| CoreError::InvalidArgument(format!("invalid page_token: {token}")))
}

// Test Budget: 2 behaviors (roundtrip encode/decode holds for any offset;
// a malformed non-hex token is rejected with InvalidArgument, empty token
// decodes to offset 0) x 2 = 4 unit tests max. 3 written — the AT itself
// (multi-page listing) already exercises the roundtrip through the real
// gRPC handler, so this module's own tests stay minimal (Mandate 3, "no
// code without a requiring test" — these exist to pin the pure-function
// contract this feature extracts, not to re-prove what the AT already
// covers end-to-end).
#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// Behavior 1 (Symmetric property, Hebert ch.3): encode then decode
        /// always returns the original offset, for any u32.
        #[test]
        fn roundtrips_for_any_offset(offset in any::<u32>()) {
            let token = encode_page_token(offset);
            prop_assert_eq!(decode_page_token(&token).expect("valid token"), offset);
        }

        /// Behavior 2: a token containing any non-hex-digit character is
        /// rejected with InvalidArgument, never panics.
        #[test]
        fn non_hex_token_rejected(token in "[g-zG-Z!@#$ ]{1,8}") {
            prop_assert!(matches!(decode_page_token(&token), Err(CoreError::InvalidArgument(_))));
        }
    }

    /// Behavior 1's boundary case: the empty token (first page, no prior
    /// call) decodes to offset 0 — not an error.
    #[test]
    fn empty_token_decodes_to_zero() {
        assert_eq!(decode_page_token("").expect("empty token is valid"), 0);
    }
}
