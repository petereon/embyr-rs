//! OIDC discovery document handling (ADR-086 decision 3, AC-SSO-06), pure half.
//!
//! The server fetches `{issuer}/.well-known/openid-configuration` (size- and time-capped, under the
//! outbound policy) and hands the bytes here. Nothing is trusted until `issuer` equals the stored
//! issuer byte for byte and every endpoint URL passes the outbound URL policy; the JWKS URL comes
//! only from this document, never from a token.

use serde::Deserialize;

use super::url_policy::{parse_outbound_url, OutboundRejection};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoveryError {
    /// Not JSON, or a required field is missing/not a string.
    Malformed,
    /// Document `issuer` differs from the provider's stored issuer.
    IssuerMismatch,
    /// An endpoint URL fails the outbound policy (non-https, userinfo, private literal, ...).
    BadEndpoint(OutboundRejection),
}

impl DiscoveryError {
    /// Stable label for audit rows and metrics.
    pub fn reason(&self) -> &'static str {
        match self {
            Self::Malformed => "discovery_malformed",
            Self::IssuerMismatch => "discovery_issuer_mismatch",
            Self::BadEndpoint(_) => "discovery_bad_endpoint",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryDoc {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub jwks_uri: String,
    /// `token_endpoint_auth_methods_supported`, when advertised.
    pub token_auth_methods: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct Raw {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
    #[serde(default)]
    token_endpoint_auth_methods_supported: Option<Vec<String>>,
}

/// URL of the discovery document for `issuer` (a trailing `/` on the issuer is not doubled).
pub fn discovery_url(issuer: &str) -> String {
    format!("{}/.well-known/openid-configuration", issuer.trim_end_matches('/'))
}

/// Parse and vet a discovery document for the provider whose stored issuer is `expected_issuer`.
pub fn parse_discovery(
    bytes: &[u8],
    expected_issuer: &str,
    allow_private: bool,
) -> Result<DiscoveryDoc, DiscoveryError> {
    let raw: Raw = serde_json::from_slice(bytes).map_err(|_| DiscoveryError::Malformed)?;
    if raw.issuer != expected_issuer {
        return Err(DiscoveryError::IssuerMismatch);
    }
    for url in [&raw.authorization_endpoint, &raw.token_endpoint, &raw.jwks_uri] {
        parse_outbound_url(url, allow_private).map_err(DiscoveryError::BadEndpoint)?;
    }
    Ok(DiscoveryDoc {
        issuer: raw.issuer,
        authorization_endpoint: raw.authorization_endpoint,
        token_endpoint: raw.token_endpoint,
        jwks_uri: raw.jwks_uri,
        token_auth_methods: raw.token_endpoint_auth_methods_supported,
    })
}

impl DiscoveryDoc {
    /// `client_secret_basic` unless the IdP advertises methods without it but with `client_secret_post`.
    pub fn uses_basic_auth(&self) -> bool {
        match &self.token_auth_methods {
            Some(m) if !m.iter().any(|x| x == "client_secret_basic") => {
                !m.iter().any(|x| x == "client_secret_post")
            }
            _ => true,
        }
    }
}

/// Everything the authorization request carries (ADR-086 decision 2). `redirect_uri` is built by
/// the server from `EMBYR_PUBLIC_BASE_URL`, never from a request.
pub struct AuthorizationRequest<'a> {
    pub client_id: &'a str,
    pub redirect_uri: &'a str,
    pub state: &'a str,
    pub nonce: &'a str,
    pub code_challenge: &'a str,
}

/// RFC 3986 unreserved characters pass; everything else is percent-encoded.
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `authorization_endpoint` plus the query of the code + PKCE (S256) request. An existing query on
/// the endpoint is preserved.
pub fn authorization_url(endpoint: &str, req: &AuthorizationRequest<'_>) -> String {
    let sep = if endpoint.contains('?') { '&' } else { '?' };
    format!(
        "{endpoint}{sep}response_type=code&client_id={}&redirect_uri={}&scope={}&state={}&nonce={}\
         &code_challenge={}&code_challenge_method={}",
        percent_encode(req.client_id),
        percent_encode(req.redirect_uri),
        percent_encode("openid email"),
        percent_encode(req.state),
        percent_encode(req.nonce),
        percent_encode(req.code_challenge),
        super::pkce::CODE_CHALLENGE_METHOD,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ISS: &str = "https://idp.example.com";

    fn doc(over: serde_json::Value) -> Vec<u8> {
        let mut d = json!({
            "issuer": ISS,
            "authorization_endpoint": "https://idp.example.com/authorize",
            "token_endpoint": "https://idp.example.com/token",
            "jwks_uri": "https://idp.example.com/jwks",
        });
        for (k, v) in over.as_object().unwrap() {
            if v.is_null() {
                d.as_object_mut().unwrap().remove(k);
            } else {
                d[k] = v.clone();
            }
        }
        serde_json::to_vec(&d).unwrap()
    }

    #[test]
    fn accepts_a_matching_document() {
        let d = parse_discovery(&doc(json!({})), ISS, false).unwrap();
        assert_eq!(d.jwks_uri, "https://idp.example.com/jwks");
        assert!(d.uses_basic_auth());
    }

    #[test]
    fn issuer_must_match_byte_for_byte() {
        for other in ["https://idp.example.com/", "https://IDP.example.com", "https://evil.example.com", ""] {
            assert_eq!(
                parse_discovery(&doc(json!({"issuer": other})), ISS, false),
                Err(DiscoveryError::IssuerMismatch),
                "{other:?}"
            );
        }
    }

    #[test]
    fn malformed_documents_are_refused() {
        assert_eq!(parse_discovery(b"not json", ISS, false), Err(DiscoveryError::Malformed));
        assert_eq!(parse_discovery(b"[]", ISS, false), Err(DiscoveryError::Malformed));
        for missing in ["issuer", "authorization_endpoint", "token_endpoint", "jwks_uri"] {
            assert_eq!(
                parse_discovery(&doc(json!({ missing: null })), ISS, false),
                Err(DiscoveryError::Malformed),
                "{missing}"
            );
        }
        assert_eq!(
            parse_discovery(&doc(json!({"jwks_uri": 7})), ISS, false),
            Err(DiscoveryError::Malformed)
        );
    }

    #[test]
    fn every_endpoint_goes_through_the_outbound_policy() {
        for field in ["authorization_endpoint", "token_endpoint", "jwks_uri"] {
            for (url, why) in [
                ("http://idp.example.com/x", OutboundRejection::NotHttps),
                ("https://127.0.0.1/x", OutboundRejection::ForbiddenAddress),
                ("https://169.254.169.254/x", OutboundRejection::ForbiddenAddress),
                ("https://u:p@idp.example.com/x", OutboundRejection::Userinfo),
            ] {
                assert_eq!(
                    parse_discovery(&doc(json!({ field: url })), ISS, false),
                    Err(DiscoveryError::BadEndpoint(why)),
                    "{field} {url}"
                );
            }
        }
        // the dev flag lifts scheme/address rules only
        let d = parse_discovery(&doc(json!({"jwks_uri": "http://127.0.0.1:9/jwks"})), ISS, true);
        assert!(d.is_ok());
    }

    #[test]
    fn token_auth_method_choice() {
        let m = |v: serde_json::Value| {
            parse_discovery(&doc(json!({"token_endpoint_auth_methods_supported": v})), ISS, false)
                .unwrap()
                .uses_basic_auth()
        };
        assert!(m(json!(["client_secret_basic", "client_secret_post"])));
        assert!(m(json!(["client_secret_basic"])));
        assert!(!m(json!(["client_secret_post"])));
        assert!(m(json!(["private_key_jwt"]))); // nothing we support: default to basic
        assert!(m(json!([])));
    }

    #[test]
    fn discovery_url_handles_trailing_slash() {
        assert_eq!(discovery_url("https://a.example"), "https://a.example/.well-known/openid-configuration");
        assert_eq!(discovery_url("https://a.example/"), "https://a.example/.well-known/openid-configuration");
        assert_eq!(
            discovery_url("https://login.example/tenant/v2.0"),
            "https://login.example/tenant/v2.0/.well-known/openid-configuration"
        );
    }

    #[test]
    fn authorization_url_carries_every_parameter_encoded() {
        let u = authorization_url(
            "https://idp.example.com/authorize",
            &AuthorizationRequest {
                client_id: "cid & co",
                redirect_uri: "https://console.example.com/admin/v1/auth/oidc/callback",
                state: "st-ate_1",
                nonce: "no.nce~",
                code_challenge: "chal",
            },
        );
        assert_eq!(
            u,
            "https://idp.example.com/authorize?response_type=code&client_id=cid%20%26%20co\
             &redirect_uri=https%3A%2F%2Fconsole.example.com%2Fadmin%2Fv1%2Fauth%2Foidc%2Fcallback\
             &scope=openid%20email&state=st-ate_1&nonce=no.nce~&code_challenge=chal&code_challenge_method=S256"
        );
        let with_query = authorization_url(
            "https://idp.example.com/authorize?tenant=a",
            &AuthorizationRequest { client_id: "c", redirect_uri: "r", state: "s", nonce: "n", code_challenge: "x" },
        );
        assert!(with_query.starts_with("https://idp.example.com/authorize?tenant=a&response_type=code&"));
    }
}
