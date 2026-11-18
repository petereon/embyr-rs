//! admin-oidc-sso slice 1: the mock IdP harness works end to end against the pure core
//! (PKCE, state/nonce, claim validator) and the outbound policy. No Docker, no database.
//!
//! This is the contract later slices rely on: whatever the harness can fault, the validator must
//! name. If a harness behaviour changes, this file changes with it.

#[path = "../../common/mock_oidc_idp.rs"]
mod mock_oidc_idp;

use std::collections::HashMap;

use embyr_core::admin::sso::claims::{validate_id_token, ExpectedClaims, Jwks, SsoReject, VerifiedIdToken};
use embyr_core::admin::sso::pkce::{code_challenge, random_token};
use embyr_server::adapters::outbound_policy::resolve_checked_with;
use mock_oidc_idp::{Behaviour, MockIdp, NonceMode, Sig};
use serde_json::{json, Value};

const REDIRECT_URI: &str = "https://console.example.test/admin/v1/auth/oidc/callback";

/// What `start` stores and the callback later needs.
struct Flow {
    state: String,
    nonce: String,
    verifier: String,
}

fn http() -> reqwest::Client {
    reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap()
}

async fn get_json(url: &str) -> Value {
    http().get(url).send().await.unwrap().json().await.unwrap()
}

fn query(location: &str) -> HashMap<String, String> {
    let (_, q) = location.split_once('?').expect("redirect has a query");
    form_urlencoded::parse(q.as_bytes()).into_owned().collect()
}

/// `start` + the browser's trip to the IdP. Returns the stored values and the callback's query.
async fn authorize(idp: &MockIdp) -> (Flow, HashMap<String, String>) {
    let flow = Flow { state: random_token(), nonce: random_token(), verifier: random_token() };
    let url = format!(
        "{}/authorize?{}",
        idp.base_url,
        form_urlencoded::Serializer::new(String::new())
            .append_pair("response_type", "code")
            .append_pair("client_id", &idp.client_id)
            .append_pair("redirect_uri", REDIRECT_URI)
            .append_pair("scope", "openid email")
            .append_pair("state", &flow.state)
            .append_pair("nonce", &flow.nonce)
            .append_pair("code_challenge", &code_challenge(&flow.verifier))
            .append_pair("code_challenge_method", "S256")
            .finish()
    );
    let location = idp.follow_authorize(&url).await;
    assert!(location.starts_with(REDIRECT_URI), "{location}");
    (flow, query(&location))
}

async fn exchange(idp: &MockIdp, code: &str, verifier: &str) -> reqwest::Response {
    http()
        .post(format!("{}/token", idp.base_url))
        .basic_auth(&idp.client_id, Some(&idp.client_secret))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", REDIRECT_URI),
            ("code_verifier", verifier),
        ])
        .send()
        .await
        .unwrap()
}

async fn fetch_jwks(idp: &MockIdp) -> Jwks {
    let bytes = http().get(format!("{}/jwks", idp.base_url)).send().await.unwrap().bytes().await.unwrap();
    Jwks::parse(&bytes).unwrap()
}

/// Full happy path up to the raw ID token.
async fn id_token_for(idp: &MockIdp) -> (Flow, String) {
    let (flow, cb) = authorize(idp).await;
    assert_eq!(cb["state"], flow.state);
    let resp = exchange(idp, &cb["code"], &flow.verifier).await;
    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.unwrap();
    let token = body["id_token"].as_str().expect("id_token").to_string();
    (flow, token)
}

fn verify(idp_issuer: &str, client_id: &str, flow: &Flow, jwks: &Jwks, token: &str) -> Result<VerifiedIdToken, SsoReject> {
    validate_id_token(
        token,
        jwks,
        &ExpectedClaims {
            issuer: idp_issuer,
            client_id,
            nonce: &flow.nonce,
            now_unix: chrono::Utc::now().timestamp(),
        },
    )
}

#[tokio::test]
async fn discovery_is_fetchable_through_the_outbound_policy_only_with_the_dev_flag() {
    let idp = MockIdp::start().await;
    let url = format!("{}/.well-known/openid-configuration", idp.base_url);
    assert!(resolve_checked_with(&url, false).await.is_err(), "loopback http must be refused by default");
    let target = resolve_checked_with(&url, true).await.expect("allowed with the dev flag");
    let doc = get_json(&url).await;
    assert_eq!(doc["issuer"], idp.issuer());
    assert_eq!(target.addrs.len(), 1);
    for key in ["authorization_endpoint", "token_endpoint", "jwks_uri"] {
        assert!(doc[key].as_str().unwrap().starts_with(&idp.base_url), "{key}");
    }
}

#[tokio::test]
async fn honest_flow_yields_a_token_the_validator_accepts_for_rsa_and_ec() {
    let idp = MockIdp::start().await;
    idp.set(|b| b.sub = "alice".into());
    let (flow, token) = id_token_for(&idp).await;
    let jwks = fetch_jwks(&idp).await;
    let v = verify(&idp.issuer(), &idp.client_id, &flow, &jwks, &token).ok().expect("accepted");
    assert_eq!((v.subject.as_str(), v.email.as_deref(), v.email_verified), ("alice", Some("user@example.com"), true));

    idp.set(|b| b.sig = Sig::Es256);
    let (flow, token) = id_token_for(&idp).await;
    assert!(verify(&idp.issuer(), &idp.client_id, &flow, &jwks, &token).is_ok());

    // what start sent is what a real IdP would insist on
    let sent = idp.authorize_requests();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0]["code_challenge_method"], "S256");
    assert_eq!(sent[0]["scope"], "openid email");
}

#[tokio::test]
async fn token_endpoint_enforces_single_use_codes_pkce_and_client_auth() {
    let idp = MockIdp::start().await;
    let (flow, cb) = authorize(&idp).await;
    // wrong verifier
    assert_eq!(exchange(&idp, &cb["code"], &random_token()).await.status(), 400);
    // the failed attempt consumed the code (as a real IdP does)
    assert_eq!(exchange(&idp, &cb["code"], &flow.verifier).await.status(), 400);

    let (flow, cb) = authorize(&idp).await;
    assert_eq!(exchange(&idp, &cb["code"], &flow.verifier).await.status(), 200);
    assert_eq!(exchange(&idp, &cb["code"], &flow.verifier).await.status(), 400, "replayed code");

    let (flow, cb) = authorize(&idp).await;
    let bad_auth = http()
        .post(format!("{}/token", idp.base_url))
        .basic_auth(&idp.client_id, Some("wrong"))
        .form(&[("grant_type", "authorization_code"), ("code", &cb["code"]), ("redirect_uri", REDIRECT_URI), ("code_verifier", &flow.verifier)])
        .send()
        .await
        .unwrap();
    assert_eq!(bad_auth.status(), 401);
}

#[tokio::test]
async fn every_token_fault_is_named_by_the_validator() {
    let idp = MockIdp::start().await;
    let jwks = fetch_jwks(&idp).await;
    let faults: Vec<(&str, Box<dyn Fn(&mut Behaviour)>, SsoReject)> = vec![
        ("wrong nonce", Box::new(|b| b.nonce = NonceMode::Wrong("replayed".into())), SsoReject::NonceMismatch),
        ("no nonce", Box::new(|b| b.nonce = NonceMode::Omit), SsoReject::NonceMismatch),
        ("wrong aud", Box::new(|b| { b.claim_overrides.insert("aud".into(), json!("someone-else")); }), SsoReject::AudienceMismatch),
        ("wrong iss", Box::new(|b| { b.claim_overrides.insert("iss".into(), json!("https://evil.example")); }), SsoReject::IssuerMismatch),
        ("aud list without azp", Box::new(|b| { b.claim_overrides.insert("aud".into(), json!(["mock-client", "other"])); }), SsoReject::AzpMismatch),
        ("expired", Box::new(|b| b.exp_in_secs = -3600), SsoReject::Expired),
        ("issued in the future", Box::new(|b| b.iat_offset_secs = 3600), SsoReject::IssuedInFuture),
        ("empty sub", Box::new(|b| b.sub = String::new()), SsoReject::BadSubject),
        ("oversized sub", Box::new(|b| b.sub = "s".repeat(256)), SsoReject::BadSubject),
        ("unknown kid", Box::new(|b| b.kid = Some("not-in-jwks".into())), SsoReject::UnknownKey),
        ("HS256 key confusion", Box::new(|b| b.sig = Sig::Hs256Confusion), SsoReject::AlgNotAllowed),
        ("RS256 header on the EC key", Box::new(|b| b.kid = Some("ec-1".into())), SsoReject::KeyTypeMismatch),
    ];
    for (name, fault, expected) in faults {
        idp.reset();
        idp.set(|b| fault(b));
        let (flow, token) = id_token_for(&idp).await;
        let got = verify(&idp.issuer(), &idp.client_id, &flow, &jwks, &token).err();
        assert_eq!(got, Some(expected), "fault: {name}");
    }
}

#[tokio::test]
async fn unverified_email_and_foreign_email_are_reported_not_hidden() {
    let idp = MockIdp::start().await;
    let jwks = fetch_jwks(&idp).await;
    idp.set(|b| {
        b.email = Some("another-member@example.com".into());
        b.email_verified = json!("true");
    });
    let (flow, token) = id_token_for(&idp).await;
    let v = verify(&idp.issuer(), &idp.client_id, &flow, &jwks, &token).ok().unwrap();
    assert_eq!(v.email.as_deref(), Some("another-member@example.com"));
    assert!(!v.email_verified, "string \"true\" is not a verified e-mail");
}

#[tokio::test]
async fn key_rotation_needs_a_jwks_refetch() {
    let idp = MockIdp::start().await;
    let old_jwks = fetch_jwks(&idp).await;
    idp.rotate_rsa_key("rsa-2");
    let (flow, token) = id_token_for(&idp).await;
    let stale = verify(&idp.issuer(), &idp.client_id, &flow, &old_jwks, &token).err();
    assert_eq!(stale, Some(SsoReject::UnknownKey));
    assert!(!old_jwks.contains_kid("rsa-2"));
    let fresh = fetch_jwks(&idp).await;
    assert!(fresh.contains_kid("rsa-2") && !fresh.contains_kid("rsa-1"));
    assert!(verify(&idp.issuer(), &idp.client_id, &flow, &fresh, &token).is_ok());
    assert_eq!(idp.hits("jwks"), 2);
}

#[tokio::test]
async fn discovery_jwks_and_token_faults_are_observable() {
    let idp = MockIdp::start().await;
    let disco = format!("{}/.well-known/openid-configuration", idp.base_url);

    idp.set(|b| b.discovery_issuer = Some("https://other-issuer.example".into()));
    assert_eq!(get_json(&disco).await["issuer"], "https://other-issuer.example");

    idp.reset();
    let target = idp.redirect_target();
    idp.set(|b| b.discovery_redirect_to = Some(target));
    let r = http().get(&disco).send().await.unwrap();
    assert!(r.status().is_redirection(), "a client that does not follow redirects sees the 3xx");

    idp.reset();
    idp.set(|b| b.jwks_status = 500);
    assert_eq!(http().get(format!("{}/jwks", idp.base_url)).send().await.unwrap().status(), 500);

    idp.reset();
    idp.set(|b| b.jwks_oversize = true);
    let big = http().get(format!("{}/jwks", idp.base_url)).send().await.unwrap().bytes().await.unwrap();
    assert!(big.len() > 256 * 1024, "larger than the 256 KiB JWKS cap");

    idp.reset();
    idp.set(|b| b.token_status = 500);
    let (flow, cb) = authorize(&idp).await;
    assert_eq!(exchange(&idp, &cb["code"], &flow.verifier).await.status(), 500);

    idp.reset();
    idp.set(|b| b.omit_id_token = true);
    let (flow, cb) = authorize(&idp).await;
    let body: Value = exchange(&idp, &cb["code"], &flow.verifier).await.json().await.unwrap();
    assert!(body.get("id_token").is_none());
}

#[tokio::test]
async fn idp_error_redirect_carries_error_and_state() {
    let idp = MockIdp::start().await;
    idp.set(|b| b.authorize_error = Some("access_denied".into()));
    let (flow, cb) = authorize(&idp).await;
    assert_eq!(cb["error"], "access_denied");
    assert_eq!(cb["state"], flow.state);
    assert!(!cb.contains_key("code"));
}
