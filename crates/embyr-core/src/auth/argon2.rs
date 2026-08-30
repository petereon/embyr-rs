use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Algorithm, Argon2, Params, Version,
};
use rand_core::OsRng;

use crate::error::CoreError;

fn argon2_instance() -> Argon2<'static> {
    Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        Params::new(65536, 3, 4, Some(32)).expect("valid argon2 params"),
    )
}

pub fn hash_api_key(api_key: &[u8]) -> Result<String, CoreError> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = argon2_instance()
        .hash_password(api_key, &salt)
        .map_err(|e| CoreError::InvalidArgument(e.to_string()))?;
    Ok(hash.to_string())
}

pub fn verify_api_key(api_key: &[u8], phc_hash: &str) -> Result<bool, CoreError> {
    let parsed = PasswordHash::new(phc_hash)
        .map_err(|e| CoreError::InvalidArgument(e.to_string()))?;
    Ok(argon2_instance().verify_password(api_key, &parsed).is_ok())
}

/// client-auth-hosted-identity (ADR-036 Decision 8): end-user password
/// hashing — a conceptually distinct credential class from an API key, even
/// though the Argon2id tuning is identical. Body is byte-for-byte the same
/// as `hash_api_key` — both share the single `argon2_instance()` source of
/// truth so the two parameter sets can never silently drift apart. A thin
/// named wrapper (not a raw `hash_api_key(password.as_bytes())` call site)
/// so a maintainer reading a hosted-identity call site never has to reason
/// about why a password is being passed to something named "api_key".
pub fn hash_password(password: &[u8]) -> Result<String, CoreError> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = argon2_instance()
        .hash_password(password, &salt)
        .map_err(|e| CoreError::InvalidArgument(e.to_string()))?;
    Ok(hash.to_string())
}

/// client-auth-hosted-identity (ADR-036 Decision 8): counterpart to
/// `hash_password` — same `argon2_instance()`, same discipline as
/// `verify_api_key`.
pub fn verify_password(password: &[u8], phc_hash: &str) -> Result<bool, CoreError> {
    let parsed = PasswordHash::new(phc_hash)
        .map_err(|e| CoreError::InvalidArgument(e.to_string()))?;
    Ok(argon2_instance().verify_password(password, &parsed).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argon2id_hash_and_verify_roundtrip() {
        let key = b"test-api-key-1234";
        let hash = hash_api_key(key).unwrap();
        assert!(verify_api_key(key, &hash).unwrap());
    }

    #[test]
    fn argon2id_wrong_key_does_not_verify() {
        let hash = hash_api_key(b"correct-key").unwrap();
        assert!(!verify_api_key(b"wrong-key", &hash).unwrap());
    }

    #[test]
    fn argon2id_two_hashes_of_same_key_are_different_but_both_verify() {
        let key = b"same-key";
        let h1 = hash_api_key(key).unwrap();
        let h2 = hash_api_key(key).unwrap();
        assert_ne!(h1, h2, "random salts must differ");
        assert!(verify_api_key(key, &h1).unwrap());
        assert!(verify_api_key(key, &h2).unwrap());
    }

    // ── hash_password / verify_password (client-auth-hosted-identity, ADR-036 Decision 8) ──

    #[test]
    fn hash_password_and_verify_password_roundtrip() {
        let password = b"maria-secret-password-1";
        let hash = hash_password(password).unwrap();
        assert!(verify_password(password, &hash).unwrap());
    }

    #[test]
    fn verify_password_rejects_wrong_password() {
        let hash = hash_password(b"correct-password").unwrap();
        assert!(!verify_password(b"wrong-password", &hash).unwrap());
    }

    #[test]
    fn hash_password_uses_identical_argon2id_parameters_to_hash_api_key() {
        // Kills: a divergent parameter set for end-user vs. admin/API-key
        // passwords (ADR-036 Decision 8's own named risk) — both must share
        // argon2_instance() byte-for-byte.
        let hash = hash_password(b"probe-password").unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert!(hash.contains("m=65536"));
        assert!(hash.contains("t=3"));
        assert!(hash.contains("p=4"));
    }

    #[test]
    fn argon2id_phc_string_encodes_required_params() {
        // Kills: replace argon2_instance -> Argon2<'static> with Default::default()
        // Default Argon2 uses Argon2i with m=19456 — both differ from our spec.
        let hash = hash_api_key(b"probe-key").unwrap();
        assert!(
            hash.starts_with("$argon2id$"),
            "must use Argon2id algorithm, got: {hash}"
        );
        assert!(
            hash.contains("m=65536"),
            "must use memory=65536 KiB, got: {hash}"
        );
        assert!(hash.contains("t=3"), "must use iterations=3, got: {hash}");
        assert!(
            hash.contains("p=4"),
            "must use parallelism=4, got: {hash}"
        );
    }
}
