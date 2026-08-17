//! Client-identity token verification — BC-1 Tenant Management (feature `client-auth`).
//!
//! Pure, zero-IO domain module (ADR-024). Verifies a JWT-shaped, EdDSA-signed
//! "custom token" that a customer's own backend (e.g. Trailmark) mints per
//! end user, against a project-scoped, directly-registered Ed25519 public
//! key (never a shared secret — see ADR-024 § Decision Drivers, non
//! -impersonation constraint).
//!
//! Algorithm pinning (ADR-024 Enforcement): verification MUST reject any
//! token whose header `alg` is not exactly `EdDSA`. This is the structural
//! defense against JWT algorithm-confusion attacks (e.g. treating the
//! registered *public* key bytes as an HMAC secret under `alg: HS256`).
//!
//! Rejection taxonomy (AC-16-07 / ADR-024 Enforcement table):
//!   MissingToken    — no token presented at all.
//!   Malformed       — structurally invalid JWT, non-EdDSA `alg`, or
//!                     signature fails under both current and previous
//!                     registered keys (ADR-025 dual-generation window).
//!   Expired         — structurally valid, correctly signed, `exp` in the past.
//!   ProjectMismatch — structurally valid, correctly signed, `aud` != project_id.
//!
//! Signature validity is always checked before expiry/audience are trusted —
//! an attacker-forged token with an arbitrary `aud`/`exp` and an invalid
//! signature must never be reported as `ProjectMismatch` or `Expired`; it is
//! `Malformed` (ADR-024).
//!
//! `verify_client_identity_token` verifies via `jsonwebtoken` (aws_lc_rs
//! backend, DDD-CA-1/CA-8); `credential_fingerprint` computes the BLAKE3
//! fingerprint (ADR-025).

/// A project's registered verification credential (ADR-025 dual-generation
/// rotation window). Both keys are raw 32-byte Ed25519 public key material —
/// public data, never hashed or encrypted (ADR-025 § Decision — a public key
/// has no confidentiality property to protect).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientIdentityCredential {
    pub public_key_current: [u8; 32],
    /// `None` when no rotation window is open (ADR-025).
    pub public_key_previous: Option<[u8; 32]>,
}

/// The resolved identity of a customer's end user, once their client-identity
/// token has verified successfully (ADR-024 claims: `sub` -> `end_user_id`,
/// `aud` -> `project_id`, `exp` -> `expires_at_unix`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedEndUserIdentity {
    pub end_user_id: String,
    pub project_id: String,
    /// Unix timestamp (seconds) the token expires at — the `exp` claim.
    pub expires_at_unix: i64,
}

/// Rejection taxonomy — AC-16-07's four distinguishable reasons.
/// `Debug`/`PartialEq` only (no `Display`/`thiserror`): this type crosses the
/// embyr-core -> embyr-server boundary as data; adapters own presentation
/// (HTTP `reason` enum body per ADR-026 § Sign-in action contract).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientIdentityVerifyError {
    /// No token presented at all.
    MissingToken,
    /// Structurally invalid JWT, non-EdDSA `alg` (algorithm-confusion
    /// defense — ADR-024 Enforcement), or signature fails verification
    /// under both `public_key_current` and `public_key_previous`.
    Malformed,
    /// Structurally valid, correctly signed, `exp` in the past.
    Expired,
    /// Structurally valid, correctly signed, `aud` does not match the
    /// project being verified against.
    ProjectMismatch,
}

/// Verify a client-identity token against a project's registered credential.
///
/// `token` is `None` when the caller presented no token at all (distinct
/// from an empty-string token, which is `Malformed` — a caller that goes to
/// the trouble of sending an empty string sent *something*, unlike a caller
/// that omitted the field/header entirely).
///
/// Verification order (ADR-025): try `public_key_current` first; on
/// signature failure, try `public_key_previous` if `Some`. A token that
/// fails under both is `Malformed` — there is no separate "stale credential"
/// error class (ADR-025 § Rotation, US-03 UAT scenario 3).
///
/// Signature validity is checked before expiry/audience are trusted (ADR-024
/// Enforcement) — an invalid signature is always `Malformed`, never
/// `Expired`/`ProjectMismatch`, regardless of what the (unverified) claims
/// inside the token happen to say.
///
/// Pure computation — no IO, no clock injection beyond what the caller
/// supplies implicitly via system time inside the eventual `jsonwebtoken`
/// `exp` check (ADR-024 § Consequences: Ed25519 verify has no partial-trust
/// scenario requiring an Earned Trust probe).
pub fn verify_client_identity_token(
    token: Option<&str>,
    project_id: &str,
    credential: &ClientIdentityCredential,
) -> Result<VerifiedEndUserIdentity, ClientIdentityVerifyError> {
    let token = token.ok_or(ClientIdentityVerifyError::MissingToken)?;

    // ADR-024: strictly single-algorithm — a token whose header `alg` is
    // anything other than EdDSA (including an HS256 algorithm-confusion
    // forgery) fails decode() with InvalidAlgorithm before signature
    // verification is ever attempted.
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::EdDSA);
    validation.set_audience(&[project_id]);

    let current_key = jsonwebtoken::DecodingKey::from_ed_der(&credential.public_key_current);
    match jsonwebtoken::decode::<ClientIdentityClaims>(token, &current_key, &validation) {
        Ok(data) => Ok(data.claims.into()),
        // ADR-025: on signature failure under the current key, retry against
        // the previous key (if a rotation window is open). Any other error
        // kind (bad algorithm, expired, audience mismatch, malformed
        // structure) is trusted as-is — the current key's signature already
        // proved the claims are genuine.
        Err(err) if *err.kind() == jsonwebtoken::errors::ErrorKind::InvalidSignature => {
            match credential.public_key_previous {
                Some(previous) => {
                    let previous_key = jsonwebtoken::DecodingKey::from_ed_der(&previous);
                    match jsonwebtoken::decode::<ClientIdentityClaims>(
                        token,
                        &previous_key,
                        &validation,
                    ) {
                        Ok(data) => Ok(data.claims.into()),
                        // ADR-025: "no separate stale-credential error class"
                        // — a token that fails under both tracked keys is
                        // simply Malformed.
                        Err(_) => Err(ClientIdentityVerifyError::Malformed),
                    }
                }
                None => Err(ClientIdentityVerifyError::Malformed),
            }
        }
        Err(err) => Err(map_jwt_error_kind(err.kind())),
    }
}

/// Claims shape minted by a customer's own backend per ADR-024 § Token
/// format: `sub` -> end user id, `aud` -> project id, `exp` -> unix seconds.
#[derive(serde::Deserialize)]
struct ClientIdentityClaims {
    sub: String,
    aud: String,
    exp: i64,
}

impl From<ClientIdentityClaims> for VerifiedEndUserIdentity {
    fn from(claims: ClientIdentityClaims) -> Self {
        VerifiedEndUserIdentity {
            end_user_id: claims.sub,
            project_id: claims.aud,
            expires_at_unix: claims.exp,
        }
    }
}

/// Map a `jsonwebtoken` error kind to AC-16-07's rejection taxonomy per
/// ADR-024's table. Reached only after signature verification has already
/// succeeded (or the caller has exhausted the current/previous retry, see
/// `verify_client_identity_token`).
fn map_jwt_error_kind(kind: &jsonwebtoken::errors::ErrorKind) -> ClientIdentityVerifyError {
    use jsonwebtoken::errors::ErrorKind;
    match kind {
        ErrorKind::ExpiredSignature => ClientIdentityVerifyError::Expired,
        ErrorKind::InvalidAudience => ClientIdentityVerifyError::ProjectMismatch,
        // InvalidToken | Base64(_) | Json(_) | InvalidSignature | InvalidAlgorithm | _
        _ => ClientIdentityVerifyError::Malformed,
    }
}

/// Compute the non-secret, truncated BLAKE3 fingerprint of a registered
/// public key for the admin registration response (ADR-025 § Registration).
///
/// 16 hex chars — matches the `dc_<16hex>` NOTIFY-channel-naming truncation
/// length precedent. Lets Alex confirm *which* key is active without the raw
/// key material ever appearing in the response body (AC-16-01).
pub fn credential_fingerprint(public_key: &[u8; 32]) -> String {
    let hash = blake3::hash(public_key);
    hash.as_bytes()[..8]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    //! Layer 1 (unit) coverage per Mandate 9 — PBT full where the input
    //! space is quantifiable (proptest, this crate's existing convention —
    //! see `auth::argon2`/`domain::project`), pinned examples for the
    //! specific AC-16-07 rejection reasons and the algorithm-confusion
    //! regression (ADR-024 Enforcement).
    //!
    //! `verify_client_identity_token`/`credential_fingerprint` are fully
    //! implemented (GREEN) — no `#[ignore]` needed here (unlike the
    //! acceptance scenarios in `tests/client_auth/acceptance/`): these are
    //! layer-1 inner-loop unit tests, run under plain `cargo test` as part
    //! of the normal suite. The one-scenario-at-a-time discipline applies to
    //! the outer (acceptance) loop, not this inner loop.

    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use proptest::prelude::*;
    use rand_core::OsRng;

    /// Mint a client-identity token exactly as ADR-024 § Token format
    /// specifies: `header.payload.signature`, base64url (no pad), EdDSA.
    /// Mirrors the "customer's own backend" role — Trailmark, not embyr —
    /// real Ed25519 signing, not a mock of any embyr-owned port.
    fn mint_token(signing_key: &SigningKey, sub: &str, aud: &str, exp_unix: i64) -> String {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"EdDSA","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD.encode(
            serde_json::json!({"sub": sub, "aud": aud, "exp": exp_unix}).to_string(),
        );
        let signing_input = format!("{header}.{payload}");
        let signature = signing_key.sign(signing_input.as_bytes());
        let sig_b64 = URL_SAFE_NO_PAD.encode(signature.to_bytes());
        format!("{signing_input}.{sig_b64}")
    }

    fn mint_hs256_confusion_token(public_key_bytes: &[u8; 32], sub: &str, aud: &str, exp_unix: i64) -> String {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
        use hmac::{Hmac, Mac};
        use sha2::Sha256;

        let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"HS256","typ":"JWT"}"#);
        let payload = URL_SAFE_NO_PAD.encode(
            serde_json::json!({"sub": sub, "aud": aud, "exp": exp_unix}).to_string(),
        );
        let signing_input = format!("{header}.{payload}");
        let mut mac = Hmac::<Sha256>::new_from_slice(public_key_bytes)
            .expect("HMAC accepts any key length");
        mac.update(signing_input.as_bytes());
        let sig_b64 = URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes());
        format!("{signing_input}.{sig_b64}")
    }

    fn credential_for(signing_key: &SigningKey) -> ClientIdentityCredential {
        ClientIdentityCredential {
            public_key_current: signing_key.verifying_key().to_bytes(),
            public_key_previous: None,
        }
    }

    fn now_unix() -> i64 {
        chrono::Utc::now().timestamp()
    }

    // ── Pinned examples (AC-16-07's four rejection reasons + happy path) ──

    #[test]
    fn valid_unexpired_token_for_correct_project_verifies_successfully() {
        let signing_key = SigningKey::generate(&mut OsRng);
        let credential = credential_for(&signing_key);
        let token = mint_token(&signing_key, "maria-santos", "trailmark-prod", now_unix() + 3600);

        let result = verify_client_identity_token(Some(&token), "trailmark-prod", &credential);

        assert_eq!(
            result,
            Ok(VerifiedEndUserIdentity {
                end_user_id: "maria-santos".to_string(),
                project_id: "trailmark-prod".to_string(),
                expires_at_unix: now_unix() + 3600,
            })
        );
    }

    #[test]
    fn missing_token_is_rejected_as_missing_token() {
        let signing_key = SigningKey::generate(&mut OsRng);
        let credential = credential_for(&signing_key);

        let result = verify_client_identity_token(None, "trailmark-prod", &credential);

        assert_eq!(result, Err(ClientIdentityVerifyError::MissingToken));
    }

    #[test]
    fn structurally_corrupted_token_is_rejected_as_malformed() {
        let signing_key = SigningKey::generate(&mut OsRng);
        let credential = credential_for(&signing_key);

        let result = verify_client_identity_token(
            Some("not-a-jwt-at-all"),
            "trailmark-prod",
            &credential,
        );

        assert_eq!(result, Err(ClientIdentityVerifyError::Malformed));
    }

    #[test]
    fn expired_token_is_rejected_as_expired_distinguishable_from_malformed() {
        let signing_key = SigningKey::generate(&mut OsRng);
        let credential = credential_for(&signing_key);
        // Dana Kim's token, minted 2h ago for a 1h validity window (US-02 Domain Example 2).
        let token = mint_token(&signing_key, "dana-kim", "trailmark-prod", now_unix() - 3600);

        let result = verify_client_identity_token(Some(&token), "trailmark-prod", &credential);

        assert_eq!(result, Err(ClientIdentityVerifyError::Expired));
    }

    #[test]
    fn token_minted_for_a_different_project_is_rejected_as_project_mismatch() {
        let signing_key = SigningKey::generate(&mut OsRng);
        let credential = credential_for(&signing_key);
        let token = mint_token(&signing_key, "maria-santos", "trailmark-staging", now_unix() + 3600);

        let result = verify_client_identity_token(Some(&token), "trailmark-prod", &credential);

        assert_eq!(result, Err(ClientIdentityVerifyError::ProjectMismatch));
    }

    #[test]
    fn signature_validity_is_checked_before_expiry_or_audience_are_trusted() {
        // An attacker-forged token: correct project, unexpired claims, but
        // signed with the WRONG key entirely. Must be Malformed, never
        // ProjectMismatch/Expired based on the (untrustworthy) claim values
        // alone (ADR-024 Enforcement — signature checked first).
        let real_key = SigningKey::generate(&mut OsRng);
        let attacker_key = SigningKey::generate(&mut OsRng);
        let credential = credential_for(&real_key);
        let forged = mint_token(&attacker_key, "maria-santos", "trailmark-prod", now_unix() + 3600);

        let result = verify_client_identity_token(Some(&forged), "trailmark-prod", &credential);

        assert_eq!(result, Err(ClientIdentityVerifyError::Malformed));
    }

    // ── Algorithm-confusion regression (ADR-024 Enforcement, security-review-verified) ──

    #[test]
    fn hs256_token_using_the_public_key_bytes_as_hmac_secret_is_rejected_as_malformed() {
        let signing_key = SigningKey::generate(&mut OsRng);
        let credential = credential_for(&signing_key);
        let public_key_bytes = credential.public_key_current;
        // Attacker knows the PUBLIC key (by definition not secret) and
        // crafts an HS256 token using those bytes as the HMAC secret.
        let confusion_token =
            mint_hs256_confusion_token(&public_key_bytes, "maria-santos", "trailmark-prod", now_unix() + 3600);

        let result = verify_client_identity_token(Some(&confusion_token), "trailmark-prod", &credential);

        assert_eq!(
            result,
            Err(ClientIdentityVerifyError::Malformed),
            "algorithm-confusion attack must be rejected as Malformed, never accepted \
             (ADR-024 Enforcement — verifier must be strictly single-algorithm, EdDSA only)"
        );
    }

    // ── ADR-025 dual-generation rotation window (unit-level; acceptance-level in ca03) ──

    #[test]
    fn token_signed_under_the_immediately_previous_key_still_verifies() {
        let current_key = SigningKey::generate(&mut OsRng);
        let previous_key = SigningKey::generate(&mut OsRng);
        let credential = ClientIdentityCredential {
            public_key_current: current_key.verifying_key().to_bytes(),
            public_key_previous: Some(previous_key.verifying_key().to_bytes()),
        };
        let token = mint_token(&previous_key, "maria-santos", "trailmark-prod", now_unix() + 3600);

        let result = verify_client_identity_token(Some(&token), "trailmark-prod", &credential);

        assert!(result.is_ok(), "AC-16-11: immediately-previous-key tokens must still verify");
    }

    #[test]
    fn token_signed_under_a_key_two_rotations_ago_is_rejected_as_malformed_not_a_distinct_stale_class() {
        // ADR-025: "no separate 'stale credential' error class" — a key from
        // 2+ rotations ago is simply not in {current, previous} any more,
        // so it fails signature verification under both and is Malformed.
        let stale_key = SigningKey::generate(&mut OsRng);
        let current_key = SigningKey::generate(&mut OsRng);
        let previous_key = SigningKey::generate(&mut OsRng);
        let credential = ClientIdentityCredential {
            public_key_current: current_key.verifying_key().to_bytes(),
            public_key_previous: Some(previous_key.verifying_key().to_bytes()),
        };
        let token = mint_token(&stale_key, "maria-santos", "trailmark-prod", now_unix() + 3600);

        let result = verify_client_identity_token(Some(&token), "trailmark-prod", &credential);

        assert_eq!(result, Err(ClientIdentityVerifyError::Malformed));
    }

    // ── PBT full (Mandate 9, layer 1) — quantified over the sub/exp input space ──

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// Property: for any non-empty end-user id and any expiry strictly in
        /// the future, a correctly-signed, correct-project token always
        /// verifies and round-trips sub/aud/exp exactly (no truncation, no
        /// silent coercion).
        #[test]
        fn any_valid_token_for_the_correct_project_round_trips_its_claims(
            sub in "[a-z][a-z0-9-]{0,31}",
            seconds_in_future in 1i64..86_400,
        ) {
            let signing_key = SigningKey::generate(&mut OsRng);
            let credential = credential_for(&signing_key);
            let exp = now_unix() + seconds_in_future;
            let token = mint_token(&signing_key, &sub, "trailmark-prod", exp);

            let result = verify_client_identity_token(Some(&token), "trailmark-prod", &credential);

            prop_assert_eq!(
                result,
                Ok(VerifiedEndUserIdentity {
                    end_user_id: sub,
                    project_id: "trailmark-prod".to_string(),
                    expires_at_unix: exp,
                })
            );
        }

        /// Property: for any expiry strictly in the past, verification is
        /// always Expired — never Ok, never any other error class.
        #[test]
        fn any_token_with_a_past_expiry_is_always_rejected_as_expired(
            sub in "[a-z][a-z0-9-]{0,31}",
            seconds_in_past in 1i64..86_400,
        ) {
            let signing_key = SigningKey::generate(&mut OsRng);
            let credential = credential_for(&signing_key);
            let token = mint_token(&signing_key, &sub, "trailmark-prod", now_unix() - seconds_in_past);

            let result = verify_client_identity_token(Some(&token), "trailmark-prod", &credential);

            prop_assert_eq!(result, Err(ClientIdentityVerifyError::Expired));
        }

        /// Property: for any mismatched project id, verification is always
        /// ProjectMismatch, regardless of the specific string values chosen.
        #[test]
        fn any_token_minted_for_a_different_project_is_always_project_mismatch(
            sub in "[a-z][a-z0-9-]{0,31}",
            minted_for in "[a-z][a-z0-9-]{0,20}",
            verified_against in "[a-z][a-z0-9-]{0,20}",
        ) {
            prop_assume!(minted_for != verified_against);
            let signing_key = SigningKey::generate(&mut OsRng);
            let credential = credential_for(&signing_key);
            let token = mint_token(&signing_key, &sub, &minted_for, now_unix() + 3600);

            let result = verify_client_identity_token(Some(&token), &verified_against, &credential);

            prop_assert_eq!(result, Err(ClientIdentityVerifyError::ProjectMismatch));
        }
    }

    // ── credential_fingerprint ──

    #[test]
    fn fingerprint_is_16_hex_chars_and_never_equals_the_raw_key_material() {
        let signing_key = SigningKey::generate(&mut OsRng);
        let public_key = signing_key.verifying_key().to_bytes();

        let fp = credential_fingerprint(&public_key);

        assert_eq!(fp.len(), 16, "fingerprint must be 16 hex chars (dc_<16hex> truncation precedent)");
        assert!(fp.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn fingerprint_is_deterministic_for_the_same_key() {
        let signing_key = SigningKey::generate(&mut OsRng);
        let public_key = signing_key.verifying_key().to_bytes();

        assert_eq!(credential_fingerprint(&public_key), credential_fingerprint(&public_key));
    }
}
