//! Outbound HTTP for admin SSO (ADR-086 decision 3): discovery, JWKS and the token endpoint, plus
//! the per-instance discovery/JWKS caches.
//!
//! Every request goes through `outbound_policy` (https only, no userinfo, every resolved address
//! vetted) and connects to the vetted address (reqwest `resolve_to_addrs`, so DNS rebinding cannot
//! swap in a private one). No redirects, no proxy, a 5 s total timeout and a hard cap on the body
//! (streamed, so an endless response is cut at the cap). The JWKS URL is only ever taken from the
//! provider's own discovery document.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use embyr_core::admin::sso::claims::{Jwks, SsoReject};
use embyr_core::admin::sso::discovery::{
    discovery_url, parse_discovery, percent_encode, DiscoveryDoc, DiscoveryError,
};
use embyr_core::admin::sso::url_policy::OutboundRejection;
use uuid::Uuid;

pub const DISCOVERY_CAP: usize = 64 * 1024;
pub const TOKEN_CAP: usize = 64 * 1024;
pub const JWKS_CAP: usize = 256 * 1024;
const TIMEOUT: Duration = Duration::from_secs(5);
const DISCOVERY_TTL: Duration = Duration::from_secs(3600);
const JWKS_TTL: Duration = Duration::from_secs(3600);
/// At most one forced JWKS refetch (unknown `kid`) per provider per minute.
const JWKS_FORCED_MIN_AGE: Duration = Duration::from_secs(60);
/// Bounds both caches; see `insert_bounded`.
const MAX_CACHE_ENTRIES: usize = 1024;

#[derive(Debug)]
pub enum FetchError {
    Policy(OutboundRejection),
    /// Connect/TLS/timeout/read failure.
    Transport,
    /// Non-2xx, including a redirect (never followed).
    Status(u16),
    TooLarge,
}

impl FetchError {
    pub fn reason(&self) -> &'static str {
        match self {
            Self::Policy(r) => r.reason(),
            Self::Transport => "fetch_failed",
            Self::Status(_) => "fetch_status",
            Self::TooLarge => "fetch_too_large",
        }
    }
}

/// What can go wrong obtaining a provider's discovery document or keys.
#[derive(Debug)]
pub enum SsoFetchError {
    Fetch(FetchError),
    Discovery(DiscoveryError),
    JwksMalformed,
}

impl SsoFetchError {
    pub fn reason(&self) -> &'static str {
        match self {
            Self::Fetch(e) => e.reason(),
            Self::Discovery(e) => e.reason(),
            Self::JwksMalformed => "jwks_malformed",
        }
    }
}

impl From<FetchError> for SsoFetchError {
    fn from(e: FetchError) -> Self {
        Self::Fetch(e)
    }
}

pub enum Body<'a> {
    Get,
    /// Form POST; `basic` = `client_secret_basic` credentials (id, secret).
    Form { basic: Option<(&'a str, &'a str)>, fields: &'a [(&'a str, &'a str)] },
}

pub struct Limits {
    pub timeout: Duration,
    pub max_bytes: usize,
}

impl Limits {
    pub fn capped(max_bytes: usize) -> Self {
        Self { timeout: TIMEOUT, max_bytes }
    }
}

/// Fetch `url` under the process-wide outbound policy.
pub async fn fetch_capped(url: &str, body: Body<'_>, limits: Limits) -> Result<Vec<u8>, FetchError> {
    fetch_capped_with(url, body, limits, super::dsn_policy::allow_private()).await
}

pub async fn fetch_capped_with(
    url: &str,
    body: Body<'_>,
    limits: Limits,
    allow_private: bool,
) -> Result<Vec<u8>, FetchError> {
    let target = super::outbound_policy::resolve_checked_with(url, allow_private)
        .await
        .map_err(FetchError::Policy)?;
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(limits.timeout)
        .no_proxy();
    if target.url.ip_literal.is_none() {
        builder = builder.resolve_to_addrs(&target.url.host, &target.addrs);
    }
    let client = builder.build().map_err(|_| FetchError::Transport)?;

    let request = match body {
        Body::Get => client.get(url),
        Body::Form { basic, fields } => {
            let mut r = client.post(url).form(fields);
            if let Some((id, secret)) = basic {
                // RFC 6749 section 2.3.1: id and secret are form-urlencoded before the base64.
                use base64::Engine;
                let cred = base64::engine::general_purpose::STANDARD
                    .encode(format!("{}:{}", percent_encode(id), percent_encode(secret)));
                r = r.header(reqwest::header::AUTHORIZATION, format!("Basic {cred}"));
            }
            r
        }
    };
    let mut response = request
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|_| FetchError::Transport)?;
    if !response.status().is_success() {
        return Err(FetchError::Status(response.status().as_u16()));
    }
    if response.content_length().is_some_and(|n| n > limits.max_bytes as u64) {
        return Err(FetchError::TooLarge);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| FetchError::Transport)? {
        if bytes.len() + chunk.len() > limits.max_bytes {
            return Err(FetchError::TooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

struct JwksEntry {
    jwks: Arc<Jwks>,
    fetched: Instant,
}

/// Per-instance caches (ADR-086 decision 3): discovery 1 h, JWKS 1 h with one forced refetch per
/// minute per provider on an unknown `kid`. Keyed by provider id (issuer and client_id are immutable).
pub struct SsoClient {
    discovery: Mutex<HashMap<Uuid, (Arc<DiscoveryDoc>, Instant)>>,
    jwks: Mutex<HashMap<Uuid, JwksEntry>>,
    /// Minimum age of the cached JWKS before an unknown `kid` may force a refetch.
    /// `JWKS_FORCED_MIN_AGE` in production; the test harness injects a short one.
    forced_refetch_min_age: Duration,
}

impl Default for SsoClient {
    fn default() -> Self {
        Self::with_forced_refetch_min_age(JWKS_FORCED_MIN_AGE)
    }
}

/// ponytail: a full map is cleared rather than evicted LRU; 1024 live providers per instance is far
/// beyond expected use and a clear only costs one refetch each.
fn insert_bounded<V>(map: &mut HashMap<Uuid, V>, key: Uuid, value: V) {
    if map.len() >= MAX_CACHE_ENTRIES && !map.contains_key(&key) {
        map.clear();
    }
    map.insert(key, value);
}

impl SsoClient {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Same caches with another forced-refetch throttle (acceptance tests use a short one; the
    /// composition root never calls this, so production keeps 60 s).
    pub fn with_forced_refetch_min_age(min_age: Duration) -> Self {
        Self {
            discovery: Mutex::default(),
            jwks: Mutex::default(),
            forced_refetch_min_age: min_age,
        }
    }

    /// Fetch and vet the discovery document for `issuer`, bypassing the cache. Used when an Owner
    /// creates or enables a provider (AC-SSO-07).
    pub async fn discover_fresh(&self, issuer: &str) -> Result<Arc<DiscoveryDoc>, SsoFetchError> {
        let bytes =
            fetch_capped(&discovery_url(issuer), Body::Get, Limits::capped(DISCOVERY_CAP)).await?;
        parse_discovery(&bytes, issuer, super::dsn_policy::allow_private())
            .map(Arc::new)
            .map_err(SsoFetchError::Discovery)
    }

    /// Cached discovery for a provider; refetched when older than an hour.
    pub async fn discovery(
        &self,
        provider_id: Uuid,
        issuer: &str,
    ) -> Result<Arc<DiscoveryDoc>, SsoFetchError> {
        if let Some((doc, at)) = self.discovery.lock().unwrap().get(&provider_id) {
            if at.elapsed() < DISCOVERY_TTL && doc.issuer == issuer {
                return Ok(doc.clone());
            }
        }
        self.refresh_discovery(provider_id, issuer).await
    }

    /// Fetch discovery now and (re)fill the cache for `provider_id`.
    pub async fn refresh_discovery(
        &self,
        provider_id: Uuid,
        issuer: &str,
    ) -> Result<Arc<DiscoveryDoc>, SsoFetchError> {
        let doc = self.discover_fresh(issuer).await?;
        insert_bounded(&mut self.discovery.lock().unwrap(), provider_id, (doc.clone(), Instant::now()));
        Ok(doc)
    }

    /// Cached JWKS from the provider's discovery document. `unknown_kid` allows one refetch per
    /// throttle window (a minute in production) even while the cached copy is fresh (key rotation); callers compare `Arc::ptr_eq` to
    /// see whether anything new arrived.
    pub async fn jwks(
        &self,
        provider_id: Uuid,
        doc: &DiscoveryDoc,
        unknown_kid: bool,
    ) -> Result<Arc<Jwks>, SsoFetchError> {
        let cached = self.jwks.lock().unwrap().get(&provider_id).map(|e| (e.jwks.clone(), e.fetched.elapsed()));
        if let Some((jwks, age)) = &cached {
            let stale = *age >= JWKS_TTL || (unknown_kid && *age >= self.forced_refetch_min_age);
            if !stale {
                return Ok(jwks.clone());
            }
        }
        let bytes = fetch_capped(&doc.jwks_uri, Body::Get, Limits::capped(JWKS_CAP)).await?;
        let jwks = Arc::new(Jwks::parse(&bytes).map_err(|_: SsoReject| SsoFetchError::JwksMalformed)?);
        insert_bounded(
            &mut self.jwks.lock().unwrap(),
            provider_id,
            JwksEntry { jwks: jwks.clone(), fetched: Instant::now() },
        );
        Ok(jwks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{http::StatusCode, response::Redirect, routing::get, Router};

    async fn serve() -> String {
        let app = Router::new()
            .route("/ok", get(|| async { "hello" }))
            .route("/big", get(|| async { "x".repeat(2000) }))
            .route("/err", get(|| async { StatusCode::INTERNAL_SERVER_ERROR }))
            .route("/redir", get(|| async { Redirect::temporary("/ok") }))
            .route(
                "/slow",
                get(|| async {
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    "late"
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        base
    }

    fn limits(max: usize) -> Limits {
        Limits { timeout: Duration::from_millis(300), max_bytes: max }
    }

    #[tokio::test]
    async fn fetches_within_the_cap() {
        let base = serve().await;
        let body = fetch_capped_with(&format!("{base}/ok"), Body::Get, limits(100), true).await.unwrap();
        assert_eq!(body, b"hello");
    }

    #[tokio::test]
    async fn oversized_body_is_refused() {
        let base = serve().await;
        let e = fetch_capped_with(&format!("{base}/big"), Body::Get, limits(1000), true).await.unwrap_err();
        assert!(matches!(e, FetchError::TooLarge), "{e:?}");
    }

    #[tokio::test]
    async fn redirects_and_errors_are_not_followed() {
        let base = serve().await;
        let e = fetch_capped_with(&format!("{base}/redir"), Body::Get, limits(100), true).await.unwrap_err();
        assert!(matches!(e, FetchError::Status(307)), "{e:?}");
        let e = fetch_capped_with(&format!("{base}/err"), Body::Get, limits(100), true).await.unwrap_err();
        assert!(matches!(e, FetchError::Status(500)), "{e:?}");
    }

    #[tokio::test]
    async fn slow_responses_time_out() {
        let base = serve().await;
        let e = fetch_capped_with(&format!("{base}/slow"), Body::Get, limits(100), true).await.unwrap_err();
        assert!(matches!(e, FetchError::Transport), "{e:?}");
    }

    #[tokio::test]
    async fn policy_applies_before_any_connection() {
        let base = serve().await;
        let e = fetch_capped_with(&format!("{base}/ok"), Body::Get, limits(100), false).await.unwrap_err();
        assert!(matches!(e, FetchError::Policy(OutboundRejection::NotHttps)), "{e:?}");
        let e = fetch_capped_with("https://127.0.0.1:1/x", Body::Get, limits(100), false).await.unwrap_err();
        assert!(matches!(e, FetchError::Policy(OutboundRejection::ForbiddenAddress)), "{e:?}");
        let e = fetch_capped_with("https://169.254.169.254/latest", Body::Get, limits(100), false)
            .await
            .unwrap_err();
        assert!(matches!(e, FetchError::Policy(OutboundRejection::ForbiddenAddress)), "{e:?}");
    }

    #[test]
    fn cache_is_bounded() {
        let mut m: HashMap<Uuid, u8> = HashMap::new();
        for _ in 0..MAX_CACHE_ENTRIES {
            insert_bounded(&mut m, Uuid::new_v4(), 0);
        }
        assert_eq!(m.len(), MAX_CACHE_ENTRIES);
        let keep = *m.keys().next().unwrap();
        insert_bounded(&mut m, keep, 1); // existing key never triggers a clear
        assert_eq!(m.len(), MAX_CACHE_ENTRIES);
        insert_bounded(&mut m, Uuid::new_v4(), 0);
        assert_eq!(m.len(), 1);
    }

    #[test]
    fn reasons_are_labels() {
        use embyr_core::admin::audit::is_label;
        for r in [
            FetchError::Policy(OutboundRejection::Malformed).reason(),
            FetchError::Transport.reason(),
            FetchError::Status(500).reason(),
            FetchError::TooLarge.reason(),
            SsoFetchError::JwksMalformed.reason(),
            SsoFetchError::Discovery(DiscoveryError::IssuerMismatch).reason(),
        ] {
            assert!(is_label(r), "{r}");
        }
    }
}
