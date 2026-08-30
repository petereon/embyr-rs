//! Google ID-token verification — `oauth-providers` (BC-1 extension, ADR-037
//! Decision 5). Pure, zero-IO domain module, mirroring `client_identity`'s
//! own module-boundary discipline: this function verifies a caller-supplied
//! JWKS — it never fetches one itself (that's `GoogleJwksCache`'s job, an
//! embyr-server adapter concern).
//!
//! Different shape from `client_identity`'s own EdDSA verification
//! deliberately: RS256 (not EdDSA), Google's own live JWKS (not a single
//! registered/embyr-owned key), Google's own `iss`/`aud`/`sub`/`exp` claims
//! (not `ClientIdentityClaims`).
//!
//! Signature validity is always checked before expiry/audience are trusted
//! (mirrors `verify_client_identity_token`'s own signature-first
//! discipline, and `jsonwebtoken`'s own library-level guarantee) — a token
//! with an invalid signature is always `Malformed`, never
//! `Expired`/`AudienceMismatch`, regardless of what the (unverified) claims
//! inside the token happen to say.

use jsonwebtoken::{decode, decode_header, jwk::JwkSet, Algorithm, DecodingKey, Validation};

/// The resolved identity of a Google-authenticated end user, once their
/// Google ID token has verified successfully.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedOAuthIdentity {
    pub sub: String,
    pub email: Option<String>,
    /// Unix timestamp (seconds) the token expires at — the `exp` claim.
    pub expires_at_unix: i64,
}

/// Rejection taxonomy (ADR-037 Decision 5 / Decision 6's own 400 reason
/// table).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthIdentityVerifyError {
    /// No token presented at all.
    MissingToken,
    /// Bad structure, unknown `kid`, or signature fails under every JWKS key.
    Malformed,
    /// Structurally valid, correctly signed, `exp` in the past.
    Expired,
    /// Structurally valid, correctly signed, `aud` != `expected_client_id`.
    AudienceMismatch,
}

/// Verify a Google-issued ID token against a caller-supplied JWKS.
///
/// `id_token` is `None` when the caller presented no token at all (distinct
/// from an empty-string token — see `verify_client_identity_token`'s
/// identical distinction).
///
/// Key selection mirrors `admin/handlers/auth.rs::oidc_callback`'s own
/// pattern (as a PATTERN, not shared code — that flow is per-account,
/// session-shaped, implicit-flow; this function is pure, per-request,
/// project-scoped): match the JWT header's `kid` against the JWKS; fall
/// back to the JWKS's first key when the header carries no `kid`. No
/// matching key -> `Malformed` (never a distinct "unknown key" class).
///
/// Pure computation — no IO, no clock injection beyond what `jsonwebtoken`
/// itself consults for the `exp` check.
pub fn verify_google_id_token(
    id_token: Option<&str>,
    expected_client_id: &str,
    jwks: &JwkSet,
) -> Result<VerifiedOAuthIdentity, OAuthIdentityVerifyError> {
    let token = id_token.ok_or(OAuthIdentityVerifyError::MissingToken)?;

    let header = decode_header(token).map_err(|_| OAuthIdentityVerifyError::Malformed)?;
    let jwk = match header.kid.as_deref() {
        Some(kid) if !kid.is_empty() => jwks
            .keys
            .iter()
            .find(|k| k.common.key_id.as_deref() == Some(kid)),
        _ => jwks.keys.first(),
    }
    .ok_or(OAuthIdentityVerifyError::Malformed)?;

    let decoding_key =
        DecodingKey::from_jwk(jwk).map_err(|_| OAuthIdentityVerifyError::Malformed)?;

    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(&[expected_client_id]);

    match decode::<GoogleIdTokenClaims>(token, &decoding_key, &validation) {
        Ok(data) => Ok(VerifiedOAuthIdentity {
            sub: data.claims.sub,
            email: data.claims.email,
            expires_at_unix: data.claims.exp,
        }),
        Err(err) => Err(map_jwt_error_kind(err.kind())),
    }
}

/// Resolution 3(B): deterministic, stateless `end_user_id` derivation —
/// gives AC-19-06 ("the same Google account signing in again resolves to
/// the identical `end_user_id`") for free, as a pure function of Google's
/// own stable `sub` claim.
pub fn derive_end_user_id(provider: &str, subject: &str) -> String {
    format!("{provider}:{subject}")
}

#[derive(serde::Deserialize)]
struct GoogleIdTokenClaims {
    sub: String,
    #[serde(default)]
    email: Option<String>,
    exp: i64,
}

fn map_jwt_error_kind(kind: &jsonwebtoken::errors::ErrorKind) -> OAuthIdentityVerifyError {
    use jsonwebtoken::errors::ErrorKind;
    match kind {
        ErrorKind::ExpiredSignature => OAuthIdentityVerifyError::Expired,
        ErrorKind::InvalidAudience => OAuthIdentityVerifyError::AudienceMismatch,
        // InvalidToken | Base64(_) | Json(_) | InvalidSignature | InvalidAlgorithm | _
        _ => OAuthIdentityVerifyError::Malformed,
    }
}

#[cfg(test)]
mod tests {
    //! Port-to-port: `verify_google_id_token`/`derive_end_user_id` ARE their
    //! own driving ports (pure domain functions) — calling them directly
    //! here IS port-to-port testing, per `client_identity`'s own established
    //! convention.
    //!
    //! Real RS256 signing throughout — mints test Google-ID-token-shaped
    //! JWTs with a real RSA keypair, exactly as `oidc_callback` verifies a
    //! real IdP-issued token in production. No hand-mocked verification
    //! result.

    use super::*;
    use jsonwebtoken::{Algorithm as JwtAlgorithm, EncodingKey, Header};

    fn now_unix() -> i64 {
        chrono::Utc::now().timestamp()
    }

    /// A fixed, real 2048-bit RSA test keypair (generated once via `openssl
    /// genrsa`, never used outside this test module) — PKCS#1 PEM for
    /// signing (`EncodingKey::from_rsa_pem`) plus its exact JWK
    /// (`n`/`e`) modulus/exponent for verification. Real RS256
    /// sign-then-verify throughout, exactly as production
    /// `verify_google_id_token` exercises against a real Google-published
    /// JWKS — no hand-mocked verification result. Fixed rather than
    /// freshly-generated per test run: this module has no `rsa`-crate
    /// dependency (that crate is scoped to `embyr-server`'s own dev-only
    /// mock-JWKS-server test infrastructure, per ADR-037's "zero new crate
    /// dependency" consequence) — a fixed keypair proves the identical
    /// real-signature-verification property without adding one.
    const TEST_RSA_PRIVATE_KEY_PEM: &str = "-----BEGIN RSA PRIVATE KEY-----\n\
MIIEogIBAAKCAQEAmAB6fsxR8kVSNO+Oqp9QsYvEmWzgVCPOincm8v9Y0RDRKM4K\n\
n0XshhO6K0XyyGGGbY4huxGzvg82GMXqpijlfe4JpGTfJ9D2SxfvqcWk4lFtf+ba\n\
yJremBNaz0Ji9o5rrAQ9ZuSA7hdkwr9q20uR43xdkErgwRxG2y4FwUjN/d4Te2rI\n\
1YFgCbklsvFmJM8B9LzT4U8vHCWhuqdY3tOCaCw7+U+2L+ktbZfTTpy/QqRNQx70\n\
9sK76Bd0s7eAMj02aO6dBrfZi4lvLntF+6JbVGmzjlckgpxFG+2mmw3m6gwkf371\n\
4yGODJOdTZ4LGuliOR3BALJcRBNDVcbRoYQ6VwIDAQABAoIBADoV1Lmnspj2TJfW\n\
A5rIDroKQzKuHHuKw16+ca/iTDp4RrVlJ0H+IoiJ+VaYAFL6kbhD3Df0Z891WKLW\n\
3vhoIGqjECQ2N+CrRbMkjn09/ehOXZ3Gxkxqgi1zF/yjzdWSTysI473yDCLv5Z1W\n\
MKtkkXdxCwAc3Q5uk9UmHSTjgXRFVfWr8IXivqHsFHJAqE4j706OnHkv/GpoS4eU\n\
lj95+Fl3h/3/LSIRcXCsWyu3Bish8xSRWX1Xgc174yRmcMPxSAQOWBHLdv/ACrSA\n\
9hujwWU4g7BvP+WsgwMhIi2dH02H5xq5XHw6WiLdJNeDJgTo2D4ZIe7jde5MOLRz\n\
i5wHaGECgYEA0fjCWprfriYjv4AF9qH7T/EkkYZP2Zm3zCeGpxkKB3d4KpspkbKe\n\
yuN5pp9EHqIRKRPR0klyi3yz1Rz1fFfofNdVVgzdbvyExeA2RvKKcTdVIIQIcSDg\n\
W8/6ch752+VvNiYuAngthxKlilujsMJ699jH3tjJVnRsGjyOeYNQosMCgYEAuVKN\n\
3y8b0nhOeaNEymrdNR3TRUK0iI3e1DoID6agzGUc482JWIb0ZwP8m2i1ysLBhtJm\n\
OD0GRbZHXQUuJukSE+rqO/nGszmH9/f/yXdO+uiZ+5TOWYqTthvITNNsG0OrpO2S\n\
TErJRmwOS/oyUB9x641rwhl2Z7hTJVR23ISN6N0CgYBHjUrFz3McCFk0P+9ITYiz\n\
hExe3vNFBtIQWwPS24CEbNzhQADZapIcN4pUEoAHJFlOszdUPr0u9W7e18B8AAT0\n\
zfiBm162HI0uVSqJ4Mr2q9FxhCzQSFDMWMJSs2WX3saYIEibhNKW7e7FiFbuvEdl\n\
QFVUBnIN6RyRkENI+0u2OwKBgBk041sh+zTLbFEbJfYqRuA8qEnZYtjYAyD2q7YF\n\
QgXLEvzhLfc+k+uXHTq+KUvk5ZHC+GhZ1IqS2m4KkFZ2iSDwaN+zf5VNE/NkZgQv\n\
GC0Eij0v5klDkgfJC5t3flLPB3+tWKLL4UiU1fT7mPBQ9pvFZozGDdbZuWVwcaJY\n\
3Tx5AoGAcZA04y4sylR+XBtUVLfB2+4H7rb9ysZhPdNgyydmDLZm0O+hFkofQQ1k\n\
DLVyRk7/IK2V0I+g1ygnqz+enhs2H7F2Wi0+zQM11vBMK8oJmoZppgbGM/MZWyDH\n\
QbYkFeEf8V+WdWgKKGU/jVeiJAPwZs9aTEKRDXXd445qPelfou8=\n\
-----END RSA PRIVATE KEY-----\n";

    const TEST_RSA_MODULUS_B64URL: &str = "mAB6fsxR8kVSNO-Oqp9QsYvEmWzgVCPOincm8v9Y0RDRKM4Kn0XshhO6K0XyyGGGbY4huxGzvg82GMXqpijlfe4JpGTfJ9D2SxfvqcWk4lFtf-bayJremBNaz0Ji9o5rrAQ9ZuSA7hdkwr9q20uR43xdkErgwRxG2y4FwUjN_d4Te2rI1YFgCbklsvFmJM8B9LzT4U8vHCWhuqdY3tOCaCw7-U-2L-ktbZfTTpy_QqRNQx709sK76Bd0s7eAMj02aO6dBrfZi4lvLntF-6JbVGmzjlckgpxFG-2mmw3m6gwkf3714yGODJOdTZ4LGuliOR3BALJcRBNDVcbRoYQ6Vw";
    const TEST_RSA_EXPONENT_B64URL: &str = "AQAB";

    /// Build the JWKS representation of the fixed test keypair's public
    /// half (a `{"keys": [...]}` document, hand-built per the well-known
    /// JWK field shape) — mirrors the local mock-JWKS-server test
    /// infrastructure this feature's own acceptance tests stand up over
    /// HTTP, at unit-test granularity (no HTTP here — this module never
    /// fetches, per its own zero-IO contract).
    fn test_jwks(kid: &str) -> JwkSet {
        let jwk_json = serde_json::json!({
            "keys": [{
                "kty": "RSA",
                "use": "sig",
                "alg": "RS256",
                "kid": kid,
                "n": TEST_RSA_MODULUS_B64URL,
                "e": TEST_RSA_EXPONENT_B64URL,
            }]
        });
        serde_json::from_value(jwk_json).expect("parse hand-built JWKS")
    }

    fn mint_google_id_token(
        kid: &str,
        sub: &str,
        aud: &str,
        exp: i64,
        email: Option<&str>,
    ) -> String {
        let encoding_key = EncodingKey::from_rsa_pem(TEST_RSA_PRIVATE_KEY_PEM.as_bytes())
            .expect("build jsonwebtoken RSA key from fixed test PEM");
        let mut header = Header::new(JwtAlgorithm::RS256);
        header.kid = Some(kid.to_string());

        let mut claims = serde_json::json!({"sub": sub, "aud": aud, "exp": exp, "iss": "https://accounts.google.com"});
        if let Some(email) = email {
            claims["email"] = serde_json::Value::String(email.to_string());
        }
        jsonwebtoken::encode(&header, &claims, &encoding_key).expect("mint RS256 token")
    }

    #[test]
    fn a_valid_unexpired_token_for_the_registered_client_id_verifies_successfully() {
        let jwks = test_jwks("test-kid-1");
        let exp = now_unix() + 3600;
        let token = mint_google_id_token(
            "test-kid-1",
            "maria-google-sub",
            "expected-client-id",
            exp,
            Some("maria@example.com"),
        );

        let result = verify_google_id_token(Some(&token), "expected-client-id", &jwks);

        assert_eq!(
            result,
            Ok(VerifiedOAuthIdentity {
                sub: "maria-google-sub".to_string(),
                email: Some("maria@example.com".to_string()),
                expires_at_unix: exp,
            })
        );
    }

    #[test]
    fn missing_token_is_rejected_as_missing_token() {
        let jwks = test_jwks("test-kid-1");

        let result = verify_google_id_token(None, "expected-client-id", &jwks);

        assert_eq!(result, Err(OAuthIdentityVerifyError::MissingToken));
    }

    #[test]
    fn structurally_corrupted_token_is_rejected_as_malformed() {
        let jwks = test_jwks("test-kid-1");

        let result = verify_google_id_token(Some("not-a-jwt-at-all"), "expected-client-id", &jwks);

        assert_eq!(result, Err(OAuthIdentityVerifyError::Malformed));
    }

    #[test]
    fn expired_token_is_rejected_as_expired_distinguishable_from_malformed() {
        let jwks = test_jwks("test-kid-1");
        let token = mint_google_id_token(
            "test-kid-1",
            "dana-google-sub",
            "expected-client-id",
            now_unix() - 3600,
            None,
        );

        let result = verify_google_id_token(Some(&token), "expected-client-id", &jwks);

        assert_eq!(result, Err(OAuthIdentityVerifyError::Expired));
    }

    #[test]
    fn audience_mismatch_is_rejected_distinguishable_from_every_other_reason() {
        let jwks = test_jwks("test-kid-1");
        let token = mint_google_id_token(
            "test-kid-1",
            "maria-google-sub",
            "some-other-client-id",
            now_unix() + 3600,
            None,
        );

        let result = verify_google_id_token(Some(&token), "expected-client-id", &jwks);

        assert_eq!(result, Err(OAuthIdentityVerifyError::AudienceMismatch));
    }

    #[test]
    fn signature_validity_is_checked_before_expiry_or_audience_are_trusted() {
        // A tampered token: correct aud, unexpired claims, genuinely signed
        // by the real test key — but the signature segment is corrupted
        // after minting (simulates a forged/tampered signature the way an
        // attacker-signed token would also fail: the bytes under the
        // signature no longer match what the JWKS's public key predicts).
        // Must be Malformed, never AudienceMismatch/Expired based on the
        // (structurally valid, unexpired, correct-audience) claim values
        // alone.
        let jwks = test_jwks("test-kid-1");
        let token = mint_google_id_token(
            "test-kid-1",
            "maria-google-sub",
            "expected-client-id",
            now_unix() + 3600,
            None,
        );
        let mut segments: Vec<&str> = token.split('.').collect();
        let corrupted_sig = format!("{}X", segments[2]);
        segments[2] = &corrupted_sig;
        let tampered = segments.join(".");

        let result = verify_google_id_token(Some(&tampered), "expected-client-id", &jwks);

        assert_eq!(result, Err(OAuthIdentityVerifyError::Malformed));
    }

    #[test]
    fn derive_end_user_id_is_a_pure_deterministic_function_of_provider_and_subject() {
        assert_eq!(
            derive_end_user_id("google", "maria-google-sub"),
            "google:maria-google-sub"
        );
        // AC-19-06: the same (provider, sub) always resolves to the same id.
        assert_eq!(
            derive_end_user_id("google", "maria-google-sub"),
            derive_end_user_id("google", "maria-google-sub")
        );
    }
}
