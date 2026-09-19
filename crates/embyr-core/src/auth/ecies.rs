use aes_gcm::{
    aead::{Aead, AeadCore},
    Aes256Gcm, KeyInit,
};
use hkdf::Hkdf;
use rand_core::OsRng;
use sha2::Sha256;
use x25519_dalek::{EphemeralSecret, PublicKey, StaticSecret};

use crate::error::CoreError;

fn derive_static_secret(api_key: &[u8]) -> StaticSecret {
    let hk = Hkdf::<Sha256>::new(Some(b"embyr-ecies-v1-static"), api_key);
    let mut okm = [0u8; 32];
    hk.expand(&[], &mut okm).expect("32 bytes always valid for HKDF");
    StaticSecret::from(okm)
}

/// Derives the AES key, binding both the ephemeral and recipient public keys
/// into the HKDF `info` parameter (standard ECIES domain separation against
/// key-substitution attacks — see docs/evolution/2026-09-19-ecies-kdf-domain-separation.md).
fn derive_aes_key(shared_secret: &[u8], eph_pub: &[u8; 32], recipient_pub: &[u8; 32]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(Some(b"embyr-ecies-v1-enc"), shared_secret);
    let mut info = [0u8; 64];
    info[..32].copy_from_slice(eph_pub);
    info[32..].copy_from_slice(recipient_pub);
    let mut okm = [0u8; 32];
    hk.expand(&info, &mut okm).expect("32 bytes valid");
    okm
}

pub fn derive_public_key(api_key: &[u8]) -> [u8; 32] {
    let secret = derive_static_secret(api_key);
    PublicKey::from(&secret).to_bytes()
}

pub fn encrypt(recipient_pubkey: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>, CoreError> {
    let eph_secret = EphemeralSecret::random_from_rng(OsRng);
    let eph_pub = PublicKey::from(&eph_secret);
    let shared = eph_secret.diffie_hellman(&PublicKey::from(*recipient_pubkey));
    let aes_key = derive_aes_key(shared.as_bytes(), eph_pub.as_bytes(), recipient_pubkey);
    let cipher = Aes256Gcm::new(&aes_key.into());
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ct = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|e| CoreError::InvalidArgument(e.to_string()))?;
    let mut out = Vec::with_capacity(32 + 12 + ct.len());
    out.extend_from_slice(eph_pub.as_bytes());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

pub fn decrypt(api_key: &[u8], ciphertext: &[u8]) -> Result<Vec<u8>, CoreError> {
    if ciphertext.len() < 32 + 12 + 16 {
        return Err(CoreError::InvalidArgument("ciphertext too short".into()));
    }
    let eph_pub_bytes: [u8; 32] = ciphertext[..32].try_into().unwrap();
    let nonce_bytes: [u8; 12] = ciphertext[32..44].try_into().unwrap();
    let ct = &ciphertext[44..];
    let static_secret = derive_static_secret(api_key);
    let recipient_pub = PublicKey::from(&static_secret);
    let shared = static_secret.diffie_hellman(&PublicKey::from(eph_pub_bytes));
    let aes_key = derive_aes_key(shared.as_bytes(), &eph_pub_bytes, recipient_pub.as_bytes());
    let cipher = Aes256Gcm::new(&aes_key.into());
    cipher
        .decrypt(&nonce_bytes.into(), ct)
        .map_err(|_| CoreError::InvalidArgument("decryption failed".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ecies_encrypt_decrypt_roundtrip() {
        let api_key = b"customer-api-token-xyz";
        let pub_key = derive_public_key(api_key);
        let dsn = b"postgres://user:pass@host:5432/db";
        let ct = encrypt(&pub_key, dsn).unwrap();
        let pt = decrypt(api_key, &ct).unwrap();
        assert_eq!(pt, dsn);
    }

    #[test]
    fn ecies_wrong_api_key_fails_to_decrypt() {
        let api_key = b"correct-api-key";
        let pub_key = derive_public_key(api_key);
        let ct = encrypt(&pub_key, b"secret").unwrap();
        let result = decrypt(b"wrong-key", &ct);
        assert!(result.is_err());
    }

    #[test]
    fn ecies_derive_public_key_is_deterministic() {
        let k = b"some-api-key";
        assert_eq!(derive_public_key(k), derive_public_key(k));
    }

    // Finding #33: HKDF info must bind ephemeral + recipient pubkeys, so the
    // same raw DH output yields a different AES key under a different
    // eph/recipient pairing (domain separation against key-substitution).
    #[test]
    fn ecies_derive_aes_key_binds_ephemeral_and_recipient_pubkeys() {
        let shared_secret = [7u8; 32];
        let eph_a = [1u8; 32];
        let eph_b = [2u8; 32];
        let recipient_a = [3u8; 32];
        let recipient_b = [4u8; 32];

        let base = derive_aes_key(&shared_secret, &eph_a, &recipient_a);
        assert_ne!(
            base,
            derive_aes_key(&shared_secret, &eph_b, &recipient_a),
            "changing the ephemeral pubkey must change the derived key"
        );
        assert_ne!(
            base,
            derive_aes_key(&shared_secret, &eph_a, &recipient_b),
            "changing the recipient pubkey must change the derived key"
        );
    }

    // Kills: replace < with ==, replace < with <=, replace + with -
    // Ciphertext structure: 32 (eph_pub) + 12 (nonce) + 16 (GCM tag) = 60 bytes minimum
    #[test]
    fn ecies_encrypt_empty_plaintext_decrypts_successfully() {
        // Kills: replace < with <= in length guard
        // Empty plaintext → exactly 60 bytes (32 eph_pub + 12 nonce + 16 GCM tag).
        // With <= mutation: 60 <= 60 is true → "ciphertext too short" → Err.
        // With correct <: 60 < 60 is false → proceeds to decrypt → Ok.
        let api_key = b"test-key-for-empty-plaintext";
        let pub_key = derive_public_key(api_key);
        let ct = encrypt(&pub_key, b"").unwrap();
        assert_eq!(ct.len(), 60, "empty plaintext must produce 60-byte ciphertext");
        let pt = decrypt(api_key, &ct).unwrap();
        assert_eq!(pt, b"", "empty plaintext must round-trip");
    }

    #[test]
    fn ecies_empty_ciphertext_rejected() {
        let result = decrypt(b"any-key", &[]);
        assert!(result.is_err(), "empty ciphertext must be rejected");
    }

    #[test]
    fn ecies_ciphertext_59_bytes_rejected() {
        // 59 < 60 (32 + 12 + 16) — must be rejected
        let result = decrypt(b"any-key", &[0u8; 59]);
        assert!(result.is_err(), "59-byte ciphertext must be rejected");
    }

    #[test]
    fn ecies_ciphertext_exactly_at_minimum_length_attempts_decrypt() {
        // 60 bytes = minimum valid length; GCM tag will fail but length check passes
        let result = decrypt(b"any-key", &[0u8; 60]);
        // Must err (bad tag), but NOT due to length — kills < vs == mutation
        assert!(
            result.is_err(),
            "garbage 60-byte ciphertext must fail decryption (GCM tag mismatch)"
        );
    }
}
