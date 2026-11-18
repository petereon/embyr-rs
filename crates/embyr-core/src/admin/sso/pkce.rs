//! PKCE (RFC 7636, S256 only) and the random values of the authorization-code flow.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};

/// `code_challenge_method` sent to the IdP. `plain` is never offered.
pub const CODE_CHALLENGE_METHOD: &str = "S256";

/// 32 random bytes from the OS CSPRNG, base64url without padding (43 chars,
/// 256 bits). Used for `state`, `nonce`, the PKCE `code_verifier` (RFC 7636
/// wants 43-128 unreserved characters) and the `embyr_sso` browser-binding cookie.
pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// `BASE64URL(SHA256(ASCII(code_verifier)))`.
pub fn code_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// RFC 7636 section 4.1 shape: 43-128 characters of `[A-Za-z0-9-._~]`.
pub fn is_valid_verifier(verifier: &str) -> bool {
    (43..=128).contains(&verifier.len())
        && verifier
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~'))
}

/// What `sso_auth_requests.state_hash` and `browser_binding` store: BLAKE3 of
/// the value, so a database read never yields a usable `state` or cookie.
pub fn hash_value(value: &str) -> Vec<u8> {
    blake3::hash(value.as_bytes()).as_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn rfc7636_appendix_b_vector() {
        assert_eq!(
            code_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn random_tokens_are_43_urlsafe_chars_and_valid_verifiers() {
        for _ in 0..50 {
            let t = random_token();
            assert_eq!(t.len(), 43);
            assert!(!t.contains('=') && !t.contains('+') && !t.contains('/'));
            assert!(is_valid_verifier(&t));
        }
    }

    #[test]
    fn random_tokens_do_not_repeat() {
        let all: HashSet<String> = (0..2000).map(|_| random_token()).collect();
        assert_eq!(all.len(), 2000);
    }

    #[test]
    fn challenge_is_deterministic_and_input_sensitive() {
        let v = random_token();
        assert_eq!(code_challenge(&v), code_challenge(&v));
        assert_ne!(code_challenge(&v), code_challenge(&random_token()));
        assert_eq!(code_challenge(&v).len(), 43);
    }

    #[test]
    fn verifier_shape_boundaries() {
        assert!(is_valid_verifier(&"a".repeat(43)));
        assert!(is_valid_verifier(&"a".repeat(128)));
        assert!(!is_valid_verifier(&"a".repeat(42)));
        assert!(!is_valid_verifier(&"a".repeat(129)));
        assert!(is_valid_verifier(&format!("{}-._~", "a".repeat(40))));
        for bad in ['+', '/', '=', ' ', '\u{e9}'] {
            assert!(!is_valid_verifier(&format!("{}{bad}", "a".repeat(43))), "{bad:?}");
        }
    }

    #[test]
    fn hash_value_is_blake3_and_distinct() {
        assert_eq!(hash_value("x").len(), 32);
        assert_eq!(hash_value("x"), blake3::hash(b"x").as_bytes().to_vec());
        assert_ne!(hash_value("x"), hash_value("y"));
    }
}
