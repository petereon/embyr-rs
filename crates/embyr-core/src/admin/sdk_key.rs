use crate::auth::{argon2, blake3, ecies};
use crate::error::CoreError;

/// Derived material produced when a new SDK API key is issued.
///
/// All three fields are derived from the raw key bytes:
/// - `argon2id_hash`  — stored in sdk_api_keys.api_key_hash; never the plaintext key
/// - `ecies_pubkey`   — stored in projects.api_key_ecies_pubkey for DSN encryption
/// - `blake3_hash`    — used as the CredentialCache eviction key
pub struct SdkKeyMaterial {
    pub argon2id_hash: String,
    pub ecies_pubkey: [u8; 32],
    pub blake3_hash: [u8; 32],
}

/// Derive all three key-material values from a raw API key in a single pure call.
///
/// This function has no IO: no filesystem, no network, no database access.
/// It is safe to call in unit tests without any environment setup.
pub fn new_sdk_key_material(raw_key: &[u8]) -> Result<SdkKeyMaterial, CoreError> {
    Ok(SdkKeyMaterial {
        argon2id_hash: argon2::hash_api_key(raw_key)?,
        ecies_pubkey: ecies::derive_public_key(raw_key),
        blake3_hash: blake3::derive_cache_key(raw_key),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::argon2::verify_api_key;

    // Behavior 1: returned argon2id_hash is a valid PHC string for the same key
    #[test]
    fn new_sdk_key_material_returns_valid_argon2id_hash() {
        let raw = b"sk-live-abc123";
        let material = new_sdk_key_material(raw).expect("must succeed for valid key");
        assert!(
            verify_api_key(raw, &material.argon2id_hash).unwrap(),
            "argon2id_hash must verify against the original raw key"
        );
    }

    // Behavior 2: argon2id hashes differ across calls (random salt per invocation)
    #[test]
    fn argon2id_hashes_are_non_deterministic() {
        let raw = b"sk-live-same-key";
        let m1 = new_sdk_key_material(raw).unwrap();
        let m2 = new_sdk_key_material(raw).unwrap();
        assert_ne!(
            m1.argon2id_hash, m2.argon2id_hash,
            "random Argon2id salts must produce distinct PHC strings"
        );
    }

    // Behavior 3: ECIES public key is deterministic — same raw key yields the same pubkey
    #[test]
    fn ecies_pubkey_is_deterministic() {
        let raw = b"sk-live-deterministic";
        let m1 = new_sdk_key_material(raw).unwrap();
        let m2 = new_sdk_key_material(raw).unwrap();
        assert_eq!(
            m1.ecies_pubkey, m2.ecies_pubkey,
            "ECIES public key must be deterministic for the same raw key"
        );
    }

    // Behavior 4: BLAKE3 cache key is deterministic — same raw key yields the same hash
    #[test]
    fn blake3_hash_is_deterministic() {
        let raw = b"sk-live-cache-key";
        let m1 = new_sdk_key_material(raw).unwrap();
        let m2 = new_sdk_key_material(raw).unwrap();
        assert_eq!(
            m1.blake3_hash, m2.blake3_hash,
            "BLAKE3 cache key must be deterministic for the same raw key"
        );
    }

    // Behavior 5: different raw keys produce different BLAKE3 cache keys
    #[test]
    fn different_keys_produce_different_blake3_hashes() {
        let m1 = new_sdk_key_material(b"sk-live-key-alpha").unwrap();
        let m2 = new_sdk_key_material(b"sk-live-key-beta").unwrap();
        assert_ne!(
            m1.blake3_hash, m2.blake3_hash,
            "distinct raw keys must produce distinct BLAKE3 cache keys"
        );
    }
}
