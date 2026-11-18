//! In-process mock OpenID Connect provider (admin-oidc-sso, ADR-086; feature-delta section 7).
//!
//! A real axum server on 127.0.0.1 with generated RSA and EC signing keys, serving
//! discovery, JWKS, an authorize endpoint (302 back to the caller's `redirect_uri` with `code` and
//! `state`) and a token endpoint that checks client credentials, `redirect_uri` and the PKCE S256
//! verifier, and mints a real signed ID token. Everything a test might want to break is a field of
//! [`Behaviour`], changed between steps with [`MockIdp::set`].
//!
//! The server under test must be started with `EMBYR_ALLOW_PRIVATE_DSN_HOSTS=true` to reach it
//! (it listens on loopback and speaks plain http); the SSRF refusal itself is tested separately.
//!
//! Include with `#[path = "../../common/mock_oidc_idp.rs"] mod mock_oidc_idp;` (see
//! `tests/admin_oidc_sso/acceptance/mock_idp_selftest.rs`). Needs only dev-dependencies of
//! `embyr-server` (axum, reqwest, rsa, rcgen, jsonwebtoken, serde_json, base64, sha2, rand_core).

#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::{
    extract::{Form, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use base64::{engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD}, Engine};
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use rsa::pkcs1::EncodeRsaPrivateKey;
use rsa::traits::PublicKeyParts;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

// ── Keys ──────────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct SigningKey {
    kid: String,
    alg: Algorithm,
    encoding: Arc<EncodingKey>,
    jwk: Value,
}

fn new_rsa_key(kid: &str) -> SigningKey {
    let key = rsa::RsaPrivateKey::new(&mut rand_core::OsRng, 2048).expect("generate RSA key");
    let der = key.to_pkcs1_der().expect("RSA to DER");
    SigningKey {
        kid: kid.into(),
        alg: Algorithm::RS256,
        encoding: Arc::new(EncodingKey::from_rsa_der(der.as_bytes())),
        jwk: json!({
            "kty": "RSA", "use": "sig", "alg": "RS256", "kid": kid,
            "n": URL_SAFE_NO_PAD.encode(key.n().to_bytes_be()),
            "e": URL_SAFE_NO_PAD.encode(key.e().to_bytes_be()),
        }),
    }
}

fn new_ec_key(kid: &str) -> SigningKey {
    let pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("generate EC key");
    let point = pair.public_key_raw(); // 0x04 || X || Y
    assert_eq!(point.len(), 65);
    SigningKey {
        kid: kid.into(),
        alg: Algorithm::ES256,
        encoding: Arc::new(EncodingKey::from_ec_der(&pair.serialize_der())),
        jwk: json!({
            "kty": "EC", "crv": "P-256", "use": "sig", "alg": "ES256", "kid": kid,
            "x": URL_SAFE_NO_PAD.encode(&point[1..33]),
            "y": URL_SAFE_NO_PAD.encode(&point[33..]),
        }),
    }
}

/// RSA keygen is slow in debug builds; the initial pair is made once per test process.
fn initial_keys() -> Vec<SigningKey> {
    static KEYS: OnceLock<Vec<SigningKey>> = OnceLock::new();
    KEYS.get_or_init(|| vec![new_rsa_key("rsa-1"), new_ec_key("ec-1")]).clone()
}

// ── Behaviour (what a test can break) ─────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sig {
    Rs256,
    Es256,
    /// HS256 keyed with the RSA public modulus (the classic key-confusion forgery).
    Hs256Confusion,
}

#[derive(Clone, Debug)]
pub enum NonceMode {
    /// Echo the nonce from the authorize request (the honest IdP).
    Echo,
    Wrong(String),
    Omit,
}

#[derive(Clone)]
pub struct Behaviour {
    pub sub: String,
    pub email: Option<String>,
    pub email_verified: Value,
    pub sig: Sig,
    pub kid: Option<String>,
    /// Claim overrides, merged over the generated claims; `Value::Null` removes the claim.
    pub claim_overrides: HashMap<String, Value>,
    pub exp_in_secs: i64,
    pub iat_offset_secs: i64,
    pub nonce: NonceMode,
    // discovery / jwks / token faults
    pub discovery_issuer: Option<String>,
    pub discovery_redirect_to: Option<String>,
    pub jwks_status: u16,
    pub jwks_delay: Duration,
    pub jwks_oversize: bool,
    pub token_status: u16,
    pub token_delay: Duration,
    pub omit_id_token: bool,
    /// Authorize answers `?error=<value>&state=..` instead of a code.
    pub authorize_error: Option<String>,
}

impl Default for Behaviour {
    fn default() -> Self {
        Self {
            sub: "idp-user-1".into(),
            email: Some("user@example.com".into()),
            email_verified: json!(true),
            sig: Sig::Rs256,
            kid: None,
            claim_overrides: HashMap::new(),
            exp_in_secs: 300,
            iat_offset_secs: 0,
            nonce: NonceMode::Echo,
            discovery_issuer: None,
            discovery_redirect_to: None,
            jwks_status: 200,
            jwks_delay: Duration::ZERO,
            jwks_oversize: false,
            token_status: 200,
            token_delay: Duration::ZERO,
            omit_id_token: false,
            authorize_error: None,
        }
    }
}

struct CodeRecord {
    redirect_uri: String,
    code_challenge: String,
    nonce: String,
}

struct Inner {
    issuer: String,
    client_id: String,
    client_secret: String,
    keys: Vec<SigningKey>,
    behaviour: Behaviour,
    codes: HashMap<String, CodeRecord>,
    hits: HashMap<String, usize>,
    /// Every authorize request's query, for assertions on what the server sent.
    authorize_requests: Vec<HashMap<String, String>>,
    next_code: u64,
}

type Shared = Arc<Mutex<Inner>>;

// ── Public handle ─────────────────────────────────────────────────────────────

pub struct MockIdp {
    /// `http://127.0.0.1:<port>`; also the default `issuer`.
    pub base_url: String,
    pub client_id: String,
    pub client_secret: String,
    inner: Shared,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for MockIdp {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl MockIdp {
    pub async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind mock IdP");
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let (client_id, client_secret) = ("mock-client".to_string(), "mock-secret".to_string());
        let inner: Shared = Arc::new(Mutex::new(Inner {
            issuer: base_url.clone(),
            client_id: client_id.clone(),
            client_secret: client_secret.clone(),
            keys: initial_keys(),
            behaviour: Behaviour::default(),
            codes: HashMap::new(),
            hits: HashMap::new(),
            authorize_requests: Vec::new(),
            next_code: 0,
        }));
        let app = Router::new()
            .route("/.well-known/openid-configuration", get(discovery))
            .route("/jwks", get(jwks))
            .route("/authorize", get(authorize))
            .route("/token", post(token))
            .route("/elsewhere", get(|| async { "redirect target" }))
            .with_state(inner.clone());
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self { base_url, client_id, client_secret, inner, task }
    }

    pub fn issuer(&self) -> String {
        self.inner.lock().unwrap().issuer.clone()
    }

    pub fn redirect_target(&self) -> String {
        format!("{}/elsewhere", self.base_url)
    }

    /// Change what the IdP does from now on (affects the next request it serves).
    pub fn set(&self, f: impl FnOnce(&mut Behaviour)) {
        f(&mut self.inner.lock().unwrap().behaviour);
    }

    pub fn reset(&self) {
        self.inner.lock().unwrap().behaviour = Behaviour::default();
    }

    /// Requests served so far for `path` ("discovery", "jwks", "authorize", "token").
    pub fn hits(&self, path: &str) -> usize {
        self.inner.lock().unwrap().hits.get(path).copied().unwrap_or(0)
    }

    pub fn authorize_requests(&self) -> Vec<HashMap<String, String>> {
        self.inner.lock().unwrap().authorize_requests.clone()
    }

    /// Key rotation: from now on tokens are signed with a new RSA key (`kid`) and the JWKS no
    /// longer lists the old RSA key. Slow in debug builds (RSA keygen).
    pub fn rotate_rsa_key(&self, kid: &str) {
        let key = new_rsa_key(kid);
        let mut i = self.inner.lock().unwrap();
        i.keys.retain(|k| k.alg != Algorithm::RS256);
        i.keys.push(key);
        i.behaviour.kid = Some(kid.into());
    }

    /// What `GET <authorize>` does for a browser: returns the `Location` the IdP redirects to
    /// (the server's callback URL with `code`+`state`, or `error`+`state`).
    pub async fn follow_authorize(&self, authorization_url: &str) -> String {
        let client = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
        let resp = client.get(authorization_url).send().await.expect("authorize request");
        assert!(resp.status().is_redirection(), "authorize answered {}", resp.status());
        resp.headers()[header::LOCATION].to_str().unwrap().to_string()
    }
}

// ── Handlers ──────────────────────────────────────────────────────────────────

fn hit(s: &Shared, path: &str) {
    *s.lock().unwrap().hits.entry(path.into()).or_default() += 1;
}

async fn discovery(State(s): State<Shared>) -> Response {
    hit(&s, "discovery");
    let i = s.lock().unwrap();
    if let Some(to) = &i.behaviour.discovery_redirect_to {
        return Redirect::temporary(to).into_response();
    }
    let issuer = i.behaviour.discovery_issuer.clone().unwrap_or_else(|| i.issuer.clone());
    Json(json!({
        "issuer": issuer,
        "authorization_endpoint": format!("{}/authorize", i.issuer),
        "token_endpoint": format!("{}/token", i.issuer),
        "jwks_uri": format!("{}/jwks", i.issuer),
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["RS256", "ES256"],
        "token_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post"],
        "code_challenge_methods_supported": ["S256"],
    }))
    .into_response()
}

async fn jwks(State(s): State<Shared>) -> Response {
    hit(&s, "jwks");
    let (status, delay, oversize, keys) = {
        let i = s.lock().unwrap();
        let b = &i.behaviour;
        (b.jwks_status, b.jwks_delay, b.jwks_oversize, i.keys.iter().map(|k| k.jwk.clone()).collect::<Vec<_>>())
    };
    tokio::time::sleep(delay).await;
    if status != 200 {
        return StatusCode::from_u16(status).unwrap().into_response();
    }
    let mut doc = json!({ "keys": keys });
    if oversize {
        doc["padding"] = json!("x".repeat(512 * 1024));
    }
    Json(doc).into_response()
}

async fn authorize(State(s): State<Shared>, Query(q): Query<HashMap<String, String>>) -> Response {
    hit(&s, "authorize");
    let mut i = s.lock().unwrap();
    i.authorize_requests.push(q.clone());
    let get = |k: &str| q.get(k).cloned().unwrap_or_default();
    let (redirect_uri, state) = (get("redirect_uri"), get("state"));
    let bad = get("response_type") != "code"
        || get("client_id") != i.client_id
        || get("code_challenge_method") != "S256"
        || get("code_challenge").is_empty()
        || get("nonce").is_empty()
        || state.is_empty()
        || redirect_uri.is_empty();
    let sep = if redirect_uri.contains('?') { '&' } else { '?' };
    let enc = |v: &str| form_urlencoded::byte_serialize(v.as_bytes()).collect::<String>();
    if bad {
        return (StatusCode::BAD_REQUEST, "mock IdP: malformed authorize request").into_response();
    }
    if let Some(err) = i.behaviour.authorize_error.clone() {
        return Redirect::temporary(&format!("{redirect_uri}{sep}error={}&state={}", enc(&err), enc(&state)))
            .into_response();
    }
    i.next_code += 1;
    let code = format!("code-{}-{}", i.next_code, URL_SAFE_NO_PAD.encode(rand_bytes()));
    i.codes.insert(
        code.clone(),
        CodeRecord { redirect_uri: redirect_uri.clone(), code_challenge: get("code_challenge"), nonce: get("nonce") },
    );
    Redirect::temporary(&format!("{redirect_uri}{sep}code={}&state={}", enc(&code), enc(&state))).into_response()
}

fn rand_bytes() -> [u8; 12] {
    let mut b = [0u8; 12];
    rand_core::RngCore::fill_bytes(&mut rand_core::OsRng, &mut b);
    b
}

fn oauth_error(status: StatusCode, error: &str) -> Response {
    (status, Json(json!({ "error": error }))).into_response()
}

async fn token(State(s): State<Shared>, headers: HeaderMap, Form(f): Form<HashMap<String, String>>) -> Response {
    hit(&s, "token");
    let delay = s.lock().unwrap().behaviour.token_delay;
    tokio::time::sleep(delay).await;
    let mut i = s.lock().unwrap();
    if i.behaviour.token_status != 200 {
        return StatusCode::from_u16(i.behaviour.token_status).unwrap().into_response();
    }
    // client authentication: Basic (preferred) or client_secret_post
    let basic = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Basic "))
        .and_then(|v| STANDARD.decode(v).ok())
        .and_then(|v| String::from_utf8(v).ok())
        .and_then(|v| v.split_once(':').map(|(a, b)| (a.to_string(), b.to_string())));
    let creds = basic.unwrap_or_else(|| {
        (f.get("client_id").cloned().unwrap_or_default(), f.get("client_secret").cloned().unwrap_or_default())
    });
    if creds != (i.client_id.clone(), i.client_secret.clone()) {
        return oauth_error(StatusCode::UNAUTHORIZED, "invalid_client");
    }
    let get = |k: &str| f.get(k).cloned().unwrap_or_default();
    if get("grant_type") != "authorization_code" {
        return oauth_error(StatusCode::BAD_REQUEST, "unsupported_grant_type");
    }
    // codes are single use, and the verifier must match the challenge sent to /authorize
    let Some(rec) = i.codes.remove(&get("code")) else {
        return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant");
    };
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(get("code_verifier").as_bytes()));
    if rec.redirect_uri != get("redirect_uri") || rec.code_challenge != challenge {
        return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant");
    }
    let mut body = json!({ "access_token": "mock-access-token", "token_type": "Bearer", "expires_in": 3600 });
    if !i.behaviour.omit_id_token {
        body["id_token"] = json!(sign_id_token(&i, &rec.nonce));
    }
    Json(body).into_response()
}

fn sign_id_token(i: &Inner, request_nonce: &str) -> String {
    let b = &i.behaviour;
    let now = chrono::Utc::now().timestamp();
    let mut claims = json!({
        "iss": i.issuer, "aud": i.client_id, "sub": b.sub,
        "exp": now + b.exp_in_secs, "iat": now + b.iat_offset_secs,
        "email_verified": b.email_verified,
    });
    if let Some(email) = &b.email {
        claims["email"] = json!(email);
    }
    match &b.nonce {
        NonceMode::Echo => claims["nonce"] = json!(request_nonce),
        NonceMode::Wrong(n) => claims["nonce"] = json!(n),
        NonceMode::Omit => {}
    }
    for (k, v) in &b.claim_overrides {
        if v.is_null() {
            claims.as_object_mut().unwrap().remove(k);
        } else {
            claims[k] = v.clone();
        }
    }
    let wanted = match b.sig {
        Sig::Es256 => Algorithm::ES256,
        _ => Algorithm::RS256,
    };
    let key = i.keys.iter().find(|k| k.alg == wanted).expect("signing key for algorithm");
    let kid = b.kid.clone().or_else(|| Some(key.kid.clone()));
    if b.sig == Sig::Hs256Confusion {
        let n = URL_SAFE_NO_PAD.decode(key.jwk["n"].as_str().unwrap()).unwrap();
        let mut h = Header::new(Algorithm::HS256);
        h.kid = kid;
        return jsonwebtoken::encode(&h, &claims, &EncodingKey::from_secret(&n)).unwrap();
    }
    let mut h = Header::new(key.alg);
    h.kid = kid;
    jsonwebtoken::encode(&h, &claims, &key.encoding).expect("sign id token")
}
