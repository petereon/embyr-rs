//! JWKS fetch/cache adapter — `oauth-providers` (ADR-037 Decision 7).
//!
//! Fetches a JWKS document over HTTP (production default
//! `https://www.googleapis.com/oauth2/v3/certs`, but the URL is a
//! constructor parameter — never hardcoded into the fetch call itself — so
//! tests can point this at a local mock JWKS server). Caches for a fixed
//! 6-hour TTL; bounded 5-second request timeout (unlike
//! `admin/handlers/auth.rs::oidc_callback`'s existing untimed
//! `reqwest::get`, a real pre-existing gap this feature does not inherit).
//!
//! Earned Trust answer for this adapter (ADR-037 Decision 7): every fault
//! class this adapter can hit — DNS failure, TCP connect timeout/refusal,
//! TLS handshake failure, HTTP non-2xx, HTTP 200 with a malformed body, a
//! response slower than the 5s bound — folds into the SAME distinguishable
//! [`JwksUnreachable`] error. Never panics, never silently serves a
//! stale/empty JWKS as if it were valid.

use std::sync::Arc;
use std::time::{Duration, Instant};

use jsonwebtoken::jwk::JwkSet;
use tokio::sync::RwLock;

/// Production default — Google's own live JWKS endpoint.
pub const GOOGLE_JWKS_PRODUCTION_URL: &str = "https://www.googleapis.com/oauth2/v3/certs";

const CACHE_TTL: Duration = Duration::from_secs(6 * 60 * 60);
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);

/// Every fetch-failure fault class folds into this one distinguishable
/// error (ADR-037 Decision 7) — the caller (REST handler) maps it to
/// `503 {"reason": "GOOGLE_JWKS_UNREACHABLE"}` (AC-19-11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Google JWKS endpoint unreachable or returned an invalid response")]
pub struct JwksUnreachable;

pub struct GoogleJwksCache {
    url: String,
    client: reqwest::Client,
    cached: RwLock<Option<(Arc<JwkSet>, Instant)>>,
}

impl GoogleJwksCache {
    /// `url` is the JWKS document endpoint — production callers pass
    /// [`GOOGLE_JWKS_PRODUCTION_URL`]; tests pass a local mock server's URL
    /// (or a closed/unresponsive address, to simulate AC-19-11).
    pub fn new(url: impl Into<String>) -> Self {
        GoogleJwksCache {
            url: url.into(),
            client: reqwest::Client::builder()
                .timeout(FETCH_TIMEOUT)
                .build()
                .expect("reqwest client with fixed timeout always builds"),
            cached: RwLock::new(None),
        }
    }

    pub fn production() -> Self {
        Self::new(GOOGLE_JWKS_PRODUCTION_URL)
    }

    /// Return the cached JWKS if it is younger than the 6-hour TTL;
    /// otherwise fetch a fresh copy and cache it. `Err(JwksUnreachable)` on
    /// any fetch failure — the stale cache entry, if any, is left
    /// untouched (never served as if it were fresh, never silently
    /// discarded either).
    pub async fn get(&self) -> Result<Arc<JwkSet>, JwksUnreachable> {
        if let Some(jwks) = self.fresh_cached().await {
            return Ok(jwks);
        }

        let jwks = Arc::new(self.fetch().await?);
        *self.cached.write().await = Some((Arc::clone(&jwks), Instant::now()));
        Ok(jwks)
    }

    async fn fresh_cached(&self) -> Option<Arc<JwkSet>> {
        let guard = self.cached.read().await;
        let (jwks, fetched_at) = guard.as_ref()?;
        if fetched_at.elapsed() < CACHE_TTL {
            Some(Arc::clone(jwks))
        } else {
            None
        }
    }

    async fn fetch(&self) -> Result<JwkSet, JwksUnreachable> {
        let response = self
            .client
            .get(&self.url)
            .send()
            .await
            .map_err(|_| JwksUnreachable)?;
        if !response.status().is_success() {
            return Err(JwksUnreachable);
        }
        response.json::<JwkSet>().await.map_err(|_| JwksUnreachable)
    }
}
