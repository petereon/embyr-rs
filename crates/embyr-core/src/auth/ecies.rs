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

fn derive_aes_key(shared_secret: &[u8]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(Some(b"embyr-ecies-v1-enc"), shared_secret);
    let mut okm = [0u8; 32];
    hk.expand(&[], &mut okm).expect("32 bytes valid");
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
    let aes_key = derive_aes_key(shared.as_bytes());
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
    let shared = static_secret.diffie_hellman(&PublicKey::from(eph_pub_bytes));
    let aes_key = derive_aes_key(shared.as_bytes());
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
}
