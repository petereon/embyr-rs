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
