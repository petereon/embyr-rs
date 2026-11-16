//! Rotation-aware AES-256-GCM decrypt helper.
//!
//! Implements ADR-018 §5 (feature `secrets-management`,
//! `docs/product/architecture/adr-018-secrets-management.md`).
//!
//! `decrypt_with_rotation` tries `current_key` first; on AEAD authentication
//! failure, it retries `previous_key` (when `Some`) before returning
//! `AuthenticationFailed`. The function is generic over ciphertext bytes — the
//! same call must be correct for `users.totp_secret_enc`,
//! `oidc_providers.client_secret_enc`, and `projects.backend_pg_dsn_enc`
//! shapes (US-SM-03 UAT scenario 6), even though only the TOTP call site is
//! live today (`admin/handlers/auth.rs:255`).
//!
//! Grouped under `adapters/` alongside `aws_secret_fetcher.rs` /
//! `gcp_secret_fetcher.rs` for module-organization consistency, even though
//! this helper performs no IO itself (ADR-018 §5, DoR OQ-1 resolution).

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};

/// Error variants for [`decrypt_with_rotation`].
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum RotationDecryptError {
    /// Ciphertext is shorter than the 12-byte nonce prefix.
    #[error("ciphertext malformed: shorter than the 12-byte nonce")]
    Malformed,
    /// AEAD authentication-tag check failed under every configured key.
    #[error("AEAD authentication failed under all configured keys")]
    AuthenticationFailed,
}

/// Decrypt `ciphertext` (12-byte nonce prefix + AES-256-GCM ciphertext+tag)
/// using `current_key`, falling back to `previous_key` on AEAD authentication
/// failure.
pub fn decrypt_with_rotation(
    current_key: &[u8; 32],
    previous_key: Option<&[u8; 32]>,
    ciphertext: &[u8],
) -> Result<Vec<u8>, RotationDecryptError> {
    if ciphertext.len() < 12 {
        return Err(RotationDecryptError::Malformed);
    }

    let (nonce_bytes, encrypted) = ciphertext.split_at(12);

    if let Some(plaintext) = try_decrypt(current_key, nonce_bytes, encrypted) {
        return Ok(plaintext);
    }
    if let Some(previous_key) = previous_key {
        if let Some(plaintext) = try_decrypt(previous_key, nonce_bytes, encrypted) {
            return Ok(plaintext);
        }
    }
    Err(RotationDecryptError::AuthenticationFailed)
}

/// Attempt AES-256-GCM decrypt under `key`; `None` on AEAD authentication
/// failure (never panics on bad ciphertext -- only on a malformed key length,
/// which cannot occur for the fixed `[u8; 32]` key type used here).
fn try_decrypt(key: &[u8; 32], nonce_bytes: &[u8], encrypted: &[u8]) -> Option<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(key).expect("32-byte key is always valid");
    let nonce = Nonce::from_slice(nonce_bytes);
    cipher.decrypt(nonce, encrypted).ok()
}

/// Counterpart of `decrypt_with_rotation`: 12-byte random nonce prefix +
/// AES-256-GCM ciphertext+tag, under the current key only.
pub fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>, aes_gcm::Error> {
    use rand_core::{OsRng, RngCore};
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let cipher = Aes256Gcm::new_from_slice(key).expect("32-byte key is always valid");
    let ciphertext = cipher.encrypt(Nonce::from_slice(&nonce_bytes), plaintext)?;
    let mut out = nonce_bytes.to_vec();
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Hosted-identity signing-seed read order (ADR-085 Decision 5): the
/// server-key column first (`decrypt_with_rotation`), else the legacy
/// ECIES-to-API-key column. A present-but-undecryptable server column is an
/// error, never a silent fall-through to the legacy column.
pub fn decrypt_hosted_signing_seed(
    current_key: &[u8; 32],
    previous_key: Option<&[u8; 32]>,
    api_key: &str,
    srv_enc: Option<&[u8]>,
    legacy_enc: Option<&[u8]>,
) -> Result<Vec<u8>, String> {
    if let Some(ct) = srv_enc {
        return decrypt_with_rotation(current_key, previous_key, ct).map_err(|e| e.to_string());
    }
    let ct = legacy_enc.ok_or("signing key row has no seed column set")?;
    embyr_core::auth::ecies::decrypt(api_key.as_bytes(), ct).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use embyr_core::auth::ecies;

    const SEED: [u8; 32] = [7u8; 32];
    const KEY: [u8; 32] = [1u8; 32];
    const OLD_KEY: [u8; 32] = [2u8; 32];

    fn legacy(api_key: &str) -> Vec<u8> {
        ecies::encrypt(&ecies::derive_public_key(api_key.as_bytes()), &SEED).unwrap()
    }

    #[test]
    fn hosted_seed_server_column_wins_and_survives_server_key_rotation() {
        let srv = encrypt(&OLD_KEY, &SEED).unwrap();
        // Legacy column is garbage: proves it is never consulted when srv is present.
        let got =
            decrypt_hosted_signing_seed(&KEY, Some(&OLD_KEY), "k", Some(&srv), Some(&[0u8; 80]));
        assert_eq!(got.unwrap(), SEED);
    }

    #[test]
    fn hosted_seed_legacy_row_falls_back_to_ecies_under_the_request_api_key() {
        let ct = legacy("k0");
        assert_eq!(
            decrypt_hosted_signing_seed(&KEY, None, "k0", None, Some(&ct)).unwrap(),
            SEED
        );
        assert!(decrypt_hosted_signing_seed(&KEY, None, "wrong", None, Some(&ct)).is_err());
    }

    #[test]
    fn hosted_seed_undecryptable_server_column_does_not_fall_back_to_legacy() {
        let srv = encrypt(&OLD_KEY, &SEED).unwrap();
        let ct = legacy("k0");
        assert!(decrypt_hosted_signing_seed(&KEY, None, "k0", Some(&srv), Some(&ct)).is_err());
        assert!(decrypt_hosted_signing_seed(&KEY, None, "k0", None, None).is_err());
    }
}
