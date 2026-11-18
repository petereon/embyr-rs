//! ID-token validation for the admin SSO callback (ADR-086 decision 4, AC-SSO-05).
//!
//! Pure: the caller supplies the token, the JWKS fetched from the provider's
//! *own* discovery document, the values it stored at `start` and the current
//! time. Nothing here is derived from the token except the key id used to pick
//! among the caller's keys. Signature first, claims after; every failure is a
//! distinct [`SsoReject`] for the audit row, and the browser sees only one
//! generic message.

use jsonwebtoken::{decode, decode_header, errors::ErrorKind, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use serde_json::Value;

/// Clock skew accepted on `exp` and `iat`.
pub const LEEWAY_SECS: i64 = 60;
/// `admin_sso_identities.subject` is limited to 255 bytes.
pub const MAX_SUBJECT_BYTES: usize = 255;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SsoReject {
    /// Not a JWT, unreadable header/claims, or unusable key material (includes `alg: none`).
    Malformed,
    /// Header `alg` is not RS256 or ES256 (HS*, RS384, PS256, ...).
    AlgNotAllowed,
    /// No key with the header's `kid` (or no unambiguous key when `kid` is absent).
    UnknownKey,
    /// The selected key's type, curve, `use` or `alg` does not fit the header `alg`.
    KeyTypeMismatch,
    BadSignature,
    IssuerMismatch,
    AudienceMismatch,
    AzpMismatch,
    /// A required claim (`exp`, `iat`) is missing or not a number.
    MissingClaim,
    Expired,
    IssuedInFuture,
    NonceMismatch,
    BadSubject,
}

impl SsoReject {
    /// Stable label for `admin_audit_events.reason` and metrics.
    pub fn reason(&self) -> &'static str {
        match self {
            Self::Malformed => "token_malformed",
            Self::AlgNotAllowed => "token_alg_not_allowed",
            Self::UnknownKey => "token_unknown_key",
            Self::KeyTypeMismatch => "token_key_type_mismatch",
            Self::BadSignature => "token_bad_signature",
            Self::IssuerMismatch => "token_issuer_mismatch",
            Self::AudienceMismatch => "token_audience_mismatch",
            Self::AzpMismatch => "token_azp_mismatch",
            Self::MissingClaim => "token_missing_claim",
            Self::Expired => "token_expired",
            Self::IssuedInFuture => "token_iat_in_future",
            Self::NonceMismatch => "token_nonce_mismatch",
            Self::BadSubject => "token_bad_subject",
        }
    }
}

/// What the callback stored when it started the flow, plus the clock.
pub struct ExpectedClaims<'a> {
    /// The provider row's issuer, compared byte for byte (no normalisation).
    pub issuer: &'a str,
    pub client_id: &'a str,
    pub nonce: &'a str,
    pub now_unix: i64,
}

/// Claims the callback may use. Deliberately not `Debug`: `email` must never reach a log (AC-SSO-18).
pub struct VerifiedIdToken {
    pub subject: String,
    pub email: Option<String>,
    /// True only for the JSON boolean `true`; the string `"true"` does not count.
    pub email_verified: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct Jwk {
    kty: String,
    #[serde(default)]
    kid: Option<String>,
    #[serde(default, rename = "use")]
    use_: Option<String>,
    #[serde(default)]
    alg: Option<String>,
    #[serde(default)]
    n: Option<String>,
    #[serde(default)]
    e: Option<String>,
    #[serde(default)]
    crv: Option<String>,
    #[serde(default)]
    x: Option<String>,
    #[serde(default)]
    y: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Jwks {
    keys: Vec<Jwk>,
}

impl Jwks {
    /// Parse a JWKS document. Entries we cannot use (OKP, no `kty`, ...) are skipped, not fatal.
    pub fn parse(bytes: &[u8]) -> Result<Self, SsoReject> {
        let doc: Value = serde_json::from_slice(bytes).map_err(|_| SsoReject::Malformed)?;
        let keys = doc.get("keys").and_then(Value::as_array).ok_or(SsoReject::Malformed)?;
        Ok(Self {
            keys: keys
                .iter()
                .filter_map(|k| serde_json::from_value::<Jwk>(k.clone()).ok())
                .collect(),
        })
    }

    /// True when a token's `kid` is not in this set, so the caller may refetch once (rotation).
    pub fn contains_kid(&self, kid: &str) -> bool {
        self.keys.iter().any(|k| k.kid.as_deref() == Some(kid))
    }
}

fn alg_name(alg: Algorithm) -> &'static str {
    match alg {
        Algorithm::ES256 => "ES256",
        _ => "RS256",
    }
}

fn key_fits(k: &Jwk, alg: Algorithm) -> bool {
    let (kty, crv) = match alg {
        Algorithm::RS256 => ("RSA", None),
        Algorithm::ES256 => ("EC", Some("P-256")),
        _ => return false,
    };
    k.kty == kty
        && k.crv.as_deref() == crv
        && k.use_.as_deref().is_none_or(|u| u == "sig")
        && k.alg.as_deref().is_none_or(|a| a == alg_name(alg))
}

fn select_key<'a>(jwks: &'a Jwks, kid: Option<&str>, alg: Algorithm) -> Result<&'a Jwk, SsoReject> {
    match kid.filter(|k| !k.is_empty()) {
        Some(kid) => {
            let k = jwks
                .keys
                .iter()
                .find(|k| k.kid.as_deref() == Some(kid))
                .ok_or(SsoReject::UnknownKey)?;
            if key_fits(k, alg) {
                Ok(k)
            } else {
                Err(SsoReject::KeyTypeMismatch)
            }
        }
        None => {
            let mut fitting = jwks.keys.iter().filter(|k| key_fits(k, alg));
            match (fitting.next(), fitting.next()) {
                (Some(k), None) => Ok(k),
                _ => Err(SsoReject::UnknownKey),
            }
        }
    }
}

fn decoding_key(k: &Jwk, alg: Algorithm) -> Result<DecodingKey, SsoReject> {
    let key = match alg {
        Algorithm::RS256 => DecodingKey::from_rsa_components(
            k.n.as_deref().ok_or(SsoReject::Malformed)?,
            k.e.as_deref().ok_or(SsoReject::Malformed)?,
        ),
        _ => DecodingKey::from_ec_components(
            k.x.as_deref().ok_or(SsoReject::Malformed)?,
            k.y.as_deref().ok_or(SsoReject::Malformed)?,
        ),
    };
    key.map_err(|_| SsoReject::Malformed)
}

fn number(v: Option<&Value>) -> Option<i64> {
    let v = v?;
    v.as_i64().or_else(|| v.as_f64().map(|f| f.floor() as i64))
}

/// Verify `id_token` against `jwks` and `expected`. See AC-SSO-05.
pub fn validate_id_token(
    id_token: &str,
    jwks: &Jwks,
    expected: &ExpectedClaims<'_>,
) -> Result<VerifiedIdToken, SsoReject> {
    let header = decode_header(id_token).map_err(|_| SsoReject::Malformed)?;
    let alg = header.alg;
    if !matches!(alg, Algorithm::RS256 | Algorithm::ES256) {
        return Err(SsoReject::AlgNotAllowed);
    }
    let key = decoding_key(select_key(jwks, header.kid.as_deref(), alg)?, alg)?;

    // Signature only; every claim is checked below against the injected clock.
    let mut validation = Validation::new(alg);
    validation.validate_exp = false;
    validation.validate_nbf = false;
    validation.validate_aud = false;
    validation.required_spec_claims.clear();
    validation.leeway = 0;
    let claims = decode::<Value>(id_token, &key, &validation)
        .map_err(|e| match e.kind() {
            ErrorKind::InvalidSignature => SsoReject::BadSignature,
            _ => SsoReject::Malformed,
        })?
        .claims;
    let claims = claims.as_object().ok_or(SsoReject::Malformed)?;

    if claims.get("iss").and_then(Value::as_str) != Some(expected.issuer) {
        return Err(SsoReject::IssuerMismatch);
    }

    let audiences: Vec<&str> = match claims.get("aud") {
        Some(Value::String(s)) => vec![s.as_str()],
        Some(Value::Array(a)) => a
            .iter()
            .map(Value::as_str)
            .collect::<Option<_>>()
            .ok_or(SsoReject::AudienceMismatch)?,
        _ => return Err(SsoReject::AudienceMismatch),
    };
    if !audiences.contains(&expected.client_id) {
        return Err(SsoReject::AudienceMismatch);
    }
    // `azp` must be us when present, and is required when several audiences are named.
    match claims.get("azp") {
        None if audiences.len() > 1 => return Err(SsoReject::AzpMismatch),
        None => {}
        Some(Value::String(azp)) if azp == expected.client_id => {}
        Some(_) => return Err(SsoReject::AzpMismatch),
    }

    let exp = number(claims.get("exp")).ok_or(SsoReject::MissingClaim)?;
    let iat = number(claims.get("iat")).ok_or(SsoReject::MissingClaim)?;
    if expected.now_unix > exp.saturating_add(LEEWAY_SECS) {
        return Err(SsoReject::Expired);
    }
    if iat > expected.now_unix.saturating_add(LEEWAY_SECS) {
        return Err(SsoReject::IssuedInFuture);
    }

    if claims.get("nonce").and_then(Value::as_str) != Some(expected.nonce) {
        return Err(SsoReject::NonceMismatch);
    }

    let subject = match claims.get("sub") {
        Some(Value::String(s)) if !s.is_empty() && s.len() <= MAX_SUBJECT_BYTES => s.clone(),
        _ => return Err(SsoReject::BadSubject),
    };

    Ok(VerifiedIdToken {
        subject,
        email: claims.get("email").and_then(Value::as_str).map(str::to_owned),
        email_verified: matches!(claims.get("email_verified"), Some(Value::Bool(true))),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::sso::test_keys::*;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    use jsonwebtoken::{encode, EncodingKey, Header};
    use serde_json::json;

    const NOW: i64 = 1_800_000_000;
    const ISS: &str = "https://idp.example.com";
    const CLIENT: &str = "client-123";
    const NONCE: &str = "nonce-abc";

    fn expected() -> ExpectedClaims<'static> {
        ExpectedClaims { issuer: ISS, client_id: CLIENT, nonce: NONCE, now_unix: NOW }
    }

    fn good() -> Value {
        json!({
            "iss": ISS, "aud": CLIENT, "exp": NOW + 300, "iat": NOW - 10, "nonce": NONCE,
            "sub": "user-1", "email": "a@example.com", "email_verified": true,
        })
    }

    fn with(mut base: Value, k: &str, v: Value) -> Value {
        base[k] = v;
        base
    }

    fn without(mut base: Value, k: &str) -> Value {
        base.as_object_mut().unwrap().remove(k);
        base
    }

    fn rsa_jwk(kid: &str) -> Value {
        json!({"kty":"RSA","use":"sig","alg":"RS256","kid":kid,"n":RSA_N_B64URL,"e":RSA_E_B64URL})
    }

    fn ec_jwk(kid: &str) -> Value {
        json!({"kty":"EC","crv":"P-256","use":"sig","alg":"ES256","kid":kid,"x":EC_X_B64URL,"y":EC_Y_B64URL})
    }

    fn jwks(keys: Vec<Value>) -> Jwks {
        Jwks::parse(json!({ "keys": keys }).to_string().as_bytes()).unwrap()
    }

    fn both() -> Jwks {
        jwks(vec![rsa_jwk("rsa1"), ec_jwk("ec1")])
    }

    fn mint(alg: Algorithm, kid: Option<&str>, claims: &Value) -> String {
        let key = match alg {
            Algorithm::ES256 => EncodingKey::from_ec_pem(EC_PRIVATE_KEY_PEM.as_bytes()).unwrap(),
            _ => EncodingKey::from_rsa_pem(RSA_PRIVATE_KEY_PEM.as_bytes()).unwrap(),
        };
        let mut h = Header::new(alg);
        h.kid = kid.map(str::to_owned);
        encode(&h, claims, &key).unwrap()
    }

    fn rs(claims: &Value) -> String {
        mint(Algorithm::RS256, Some("rsa1"), claims)
    }

    fn check(token: &str) -> Result<VerifiedIdToken, SsoReject> {
        validate_id_token(token, &both(), &expected())
    }

    fn rejected(token: &str) -> Option<SsoReject> {
        check(token).err()
    }

    #[test]
    fn accepts_rs256_and_es256() {
        let v = check(&rs(&good())).unwrap();
        assert_eq!(v.subject, "user-1");
        assert_eq!(v.email.as_deref(), Some("a@example.com"));
        assert!(v.email_verified);
        let v = check(&mint(Algorithm::ES256, Some("ec1"), &good())).unwrap();
        assert_eq!(v.subject, "user-1");
    }

    #[test]
    fn algorithm_allow_list() {
        // HS256 signed with the public modulus as secret (classic confusion attack)
        let n = URL_SAFE_NO_PAD.decode(RSA_N_B64URL).unwrap();
        let mut h = Header::new(Algorithm::HS256);
        h.kid = Some("rsa1".into());
        let hs = encode(&h, &good(), &EncodingKey::from_secret(&n)).unwrap();
        assert_eq!(rejected(&hs), Some(SsoReject::AlgNotAllowed));

        let rs384 = mint(Algorithm::RS384, Some("rsa1"), &good());
        assert_eq!(rejected(&rs384), Some(SsoReject::AlgNotAllowed));

        let seg = |v: Value| URL_SAFE_NO_PAD.encode(v.to_string());
        let none = format!("{}.{}.", seg(json!({"alg":"none","typ":"JWT"})), seg(good()));
        assert_eq!(rejected(&none), Some(SsoReject::Malformed));
        for junk in ["", "abc", "a.b", "a.b.c", "...."] {
            assert_eq!(rejected(junk), Some(SsoReject::Malformed), "{junk:?}");
        }
        // well-formed header, empty signature
        assert_eq!(rejected("eyJhbGciOiJSUzI1NiJ9.e30."), Some(SsoReject::BadSignature));
    }

    #[test]
    fn key_selection_and_type_confusion() {
        // RS256 header pointing at the EC key, and the reverse
        assert_eq!(
            rejected(&mint(Algorithm::RS256, Some("ec1"), &good())),
            Some(SsoReject::KeyTypeMismatch)
        );
        assert_eq!(
            rejected(&mint(Algorithm::ES256, Some("rsa1"), &good())),
            Some(SsoReject::KeyTypeMismatch)
        );
        assert_eq!(
            rejected(&mint(Algorithm::RS256, Some("nope"), &good())),
            Some(SsoReject::UnknownKey)
        );
        // no kid: exactly one fitting key is fine, zero or two are not
        let one = jwks(vec![rsa_jwk("a"), ec_jwk("b")]);
        let t = mint(Algorithm::RS256, None, &good());
        assert!(validate_id_token(&t, &one, &expected()).is_ok());
        let two = jwks(vec![rsa_jwk("a"), rsa_jwk("b")]);
        assert_eq!(validate_id_token(&t, &two, &expected()).err(), Some(SsoReject::UnknownKey));
        let none = jwks(vec![ec_jwk("b")]);
        assert_eq!(validate_id_token(&t, &none, &expected()).err(), Some(SsoReject::UnknownKey));
        // empty kid behaves like absent
        let empty = mint(Algorithm::RS256, Some(""), &good());
        assert!(validate_id_token(&empty, &one, &expected()).is_ok());
    }

    #[test]
    fn jwk_use_alg_and_curve_must_fit() {
        let t = rs(&good());
        let mut enc = rsa_jwk("rsa1");
        enc["use"] = json!("enc");
        assert_eq!(
            validate_id_token(&t, &jwks(vec![enc]), &expected()).err(),
            Some(SsoReject::KeyTypeMismatch)
        );
        let mut wrong_alg = rsa_jwk("rsa1");
        wrong_alg["alg"] = json!("RS512");
        assert_eq!(
            validate_id_token(&t, &jwks(vec![wrong_alg]), &expected()).err(),
            Some(SsoReject::KeyTypeMismatch)
        );
        let mut no_use_no_alg = rsa_jwk("rsa1");
        no_use_no_alg.as_object_mut().unwrap().remove("use");
        no_use_no_alg.as_object_mut().unwrap().remove("alg");
        assert!(validate_id_token(&t, &jwks(vec![no_use_no_alg]), &expected()).is_ok());
        let mut p384 = ec_jwk("ec1");
        p384["crv"] = json!("P-384");
        let es = mint(Algorithm::ES256, Some("ec1"), &good());
        assert_eq!(
            validate_id_token(&es, &jwks(vec![p384]), &expected()).err(),
            Some(SsoReject::KeyTypeMismatch)
        );
        let mut broken = rsa_jwk("rsa1");
        broken.as_object_mut().unwrap().remove("n");
        assert_eq!(
            validate_id_token(&t, &jwks(vec![broken]), &expected()).err(),
            Some(SsoReject::Malformed)
        );
    }

    #[test]
    fn tampered_or_foreign_signature_is_rejected() {
        let t = rs(&good());
        let parts: Vec<&str> = t.split('.').collect();
        let forged = URL_SAFE_NO_PAD.encode(with(good(), "sub", json!("admin")).to_string());
        let tampered = format!("{}.{}.{}", parts[0], forged, parts[2]);
        assert_eq!(rejected(&tampered), Some(SsoReject::BadSignature));
        // signed by the EC key but claiming the RSA kid and alg RS256 is impossible to forge;
        // an ES256 token whose kid names a different EC key fails the signature
        let mut other = ec_jwk("ec1");
        other["x"] = json!("fuYfM4rkMLoGjc7IlYk58SWDZvPVdBO0Khz2XbHGkjQ");
        other["y"] = json!("wEBpRNIDzHZ9wpuh5ynrE8Wj_T7w8VfRTG0SpdzLaPo");
        let es = mint(Algorithm::ES256, Some("ec1"), &good());
        let r = validate_id_token(&es, &jwks(vec![other]), &expected()).err();
        assert!(matches!(r, Some(SsoReject::BadSignature | SsoReject::Malformed)), "{r:?}");
    }

    #[test]
    fn issuer_is_exact() {
        for iss in [
            json!("https://idp.example.com/"),
            json!("https://IDP.example.com"),
            json!("http://idp.example.com"),
            json!(""),
            json!(7),
        ] {
            assert_eq!(rejected(&rs(&with(good(), "iss", iss))), Some(SsoReject::IssuerMismatch));
        }
        assert_eq!(rejected(&rs(&without(good(), "iss"))), Some(SsoReject::IssuerMismatch));
    }

    #[test]
    fn audience_and_azp() {
        let aud = |v: Value| rejected(&rs(&with(good(), "aud", v)));
        assert_eq!(aud(json!("other")), Some(SsoReject::AudienceMismatch));
        assert_eq!(aud(json!(["other"])), Some(SsoReject::AudienceMismatch));
        assert_eq!(aud(json!([])), Some(SsoReject::AudienceMismatch));
        assert_eq!(aud(json!(5)), Some(SsoReject::AudienceMismatch));
        assert_eq!(aud(json!([CLIENT, 5])), Some(SsoReject::AudienceMismatch));
        assert_eq!(rejected(&rs(&without(good(), "aud"))), Some(SsoReject::AudienceMismatch));
        assert!(aud(json!([CLIENT])).is_none());
        // several audiences need azp == client_id
        assert_eq!(aud(json!([CLIENT, "other"])), Some(SsoReject::AzpMismatch));
        let multi = with(good(), "aud", json!(["other", CLIENT]));
        assert_eq!(rejected(&rs(&with(multi.clone(), "azp", json!("other")))), Some(SsoReject::AzpMismatch));
        assert_eq!(rejected(&rs(&with(multi.clone(), "azp", json!(1)))), Some(SsoReject::AzpMismatch));
        assert!(rejected(&rs(&with(multi, "azp", json!(CLIENT)))).is_none());
        // a present azp must be us even with one audience
        assert_eq!(rejected(&rs(&with(good(), "azp", json!("other")))), Some(SsoReject::AzpMismatch));
        assert!(rejected(&rs(&with(good(), "azp", json!(CLIENT)))).is_none());
    }

    #[test]
    fn exp_and_iat_boundaries() {
        let exp = |v: i64| rejected(&rs(&with(good(), "exp", json!(v))));
        assert!(exp(NOW - LEEWAY_SECS).is_none());
        assert_eq!(exp(NOW - LEEWAY_SECS - 1), Some(SsoReject::Expired));
        assert_eq!(exp(0), Some(SsoReject::Expired));
        assert!(exp(NOW + 86_400).is_none());
        let iat = |v: i64| rejected(&rs(&with(good(), "iat", json!(v))));
        assert!(iat(NOW + LEEWAY_SECS).is_none());
        assert_eq!(iat(NOW + LEEWAY_SECS + 1), Some(SsoReject::IssuedInFuture));
        assert!(iat(NOW - 86_400).is_none());
        // fractional seconds are floored, not rejected
        assert!(rejected(&rs(&with(good(), "exp", json!((NOW + 5) as f64 + 0.9)))).is_none());
        assert_eq!(rejected(&rs(&without(good(), "exp"))), Some(SsoReject::MissingClaim));
        assert_eq!(rejected(&rs(&without(good(), "iat"))), Some(SsoReject::MissingClaim));
        assert_eq!(rejected(&rs(&with(good(), "exp", json!("soon")))), Some(SsoReject::MissingClaim));
        // a numeric extreme must not overflow the leeway arithmetic
        assert!(rejected(&rs(&with(good(), "exp", json!(i64::MAX)))).is_none());
        assert_eq!(rejected(&rs(&with(good(), "iat", json!(i64::MAX)))), Some(SsoReject::IssuedInFuture));
    }

    #[test]
    fn nonce_must_match() {
        assert_eq!(rejected(&rs(&with(good(), "nonce", json!("other")))), Some(SsoReject::NonceMismatch));
        assert_eq!(rejected(&rs(&with(good(), "nonce", json!("")))), Some(SsoReject::NonceMismatch));
        assert_eq!(rejected(&rs(&with(good(), "nonce", json!(1)))), Some(SsoReject::NonceMismatch));
        assert_eq!(rejected(&rs(&without(good(), "nonce"))), Some(SsoReject::NonceMismatch));
        // and an empty expected nonce never matches a missing one
        let e = ExpectedClaims { nonce: "", ..expected() };
        assert!(validate_id_token(&rs(&without(good(), "nonce")), &both(), &e).is_err());
    }

    #[test]
    fn subject_bounds() {
        let sub = |v: Value| rejected(&rs(&with(good(), "sub", v)));
        assert!(sub(json!("a".repeat(255))).is_none());
        assert_eq!(sub(json!("a".repeat(256))), Some(SsoReject::BadSubject));
        assert_eq!(sub(json!("")), Some(SsoReject::BadSubject));
        assert_eq!(sub(json!(42)), Some(SsoReject::BadSubject));
        assert_eq!(sub(json!(null)), Some(SsoReject::BadSubject));
        assert_eq!(rejected(&rs(&without(good(), "sub"))), Some(SsoReject::BadSubject));
        // the limit is bytes, not characters
        assert!(sub(json!(format!("{}a", "\u{e9}".repeat(127)))).is_none());
        assert_eq!(sub(json!("\u{e9}".repeat(128))), Some(SsoReject::BadSubject));
    }

    #[test]
    fn email_claims() {
        let v = check(&rs(&with(good(), "email_verified", json!("true")))).unwrap();
        assert!(!v.email_verified);
        let v = check(&rs(&with(good(), "email_verified", json!(false)))).unwrap();
        assert!(!v.email_verified);
        let v = check(&rs(&without(good(), "email_verified"))).unwrap();
        assert!(!v.email_verified);
        let v = check(&rs(&without(without(good(), "email"), "email_verified"))).unwrap();
        assert_eq!((v.email, v.email_verified), (None, false));
        let v = check(&rs(&with(good(), "email", json!(5)))).unwrap();
        assert_eq!(v.email, None);
    }

    #[test]
    fn jwks_parsing() {
        assert_eq!(Jwks::parse(b"nope").err(), Some(SsoReject::Malformed));
        assert_eq!(Jwks::parse(b"{}").err(), Some(SsoReject::Malformed));
        assert_eq!(Jwks::parse(br#"{"keys":"x"}"#).err(), Some(SsoReject::Malformed));
        let set = Jwks::parse(
            json!({"keys":[{"kty":"OKP","crv":"Ed25519","kid":"o","x":"AA"}, {"kid":"nokty"}, rsa_jwk("r")]})
                .to_string()
                .as_bytes(),
        )
        .unwrap();
        assert!(set.contains_kid("r") && set.contains_kid("o") && !set.contains_kid("nokty"));
        assert!(validate_id_token(&rs(&good()), &jwks(vec![]), &expected()).is_err());
    }

    #[test]
    fn reason_labels_are_distinct() {
        use SsoReject::*;
        let all = [
            Malformed, AlgNotAllowed, UnknownKey, KeyTypeMismatch, BadSignature, IssuerMismatch,
            AudienceMismatch, AzpMismatch, MissingClaim, Expired, IssuedInFuture, NonceMismatch,
            BadSubject,
        ];
        let set: std::collections::HashSet<_> = all.iter().map(|r| r.reason()).collect();
        assert_eq!(set.len(), all.len());
    }
}
