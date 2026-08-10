use std::{collections::HashMap, sync::Arc, time::Instant};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use tokio::sync::Mutex;

/// Error variants for GCP Secret Manager fetch operations.
#[derive(Debug, thiserror::Error)]
pub enum GcpSecretError {
    #[error("GCP access denied")]
    AccessDenied,
    #[error("Secret format invalid: {0}")]
    FormatInvalid(String),
    #[error("Secret not found")]
    NotFound,
    #[error("GCP error: {0}")]
    Http(String),
}

/// Driven adapter that resolves DSN strings from GCP Secret Manager REST API.
///
/// Uses `reqwest` + direct GCP Secret Manager REST API instead of a GCP SDK crate.
/// Endpoint:
///   GET {base_url}/v1/{resource_name}/versions/latest:access
///   Authorization: Bearer {token}
///
/// Caches DSN per resource_name with a configurable TTL (default 300s).
///
/// Security invariant: the raw DSN is held only in-process memory and in the
/// customer Postgres connection. It is NEVER written to the system DB.
pub struct GcpSecretFetcher {
    http: reqwest::Client,
    /// Base URL of the GCP Secret Manager API (or in-process mock for tests).
    /// Production: "https://secretmanager.googleapis.com"
    base_url: String,
    /// Bearer token ("test" for emulator/tests).
    token: String,
    cache: Arc<Mutex<HashMap<String, (Instant, String)>>>,
    ttl_secs: u64,
    /// Call log for audit verification in tests.
    call_log: Arc<Mutex<Vec<String>>>,
}

impl GcpSecretFetcher {
    /// Construct the fetcher with a configurable base URL, bearer token, and TTL.
    pub fn new(base_url: &str, token: &str, ttl_secs: u64) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            token: token.to_string(),
            cache: Arc::new(Mutex::new(HashMap::new())),
            ttl_secs,
            call_log: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Return the DSN for the given resource_name, using the cache when fresh.
    pub async fn get_dsn(&self, resource_name: &str) -> Result<String, GcpSecretError> {
        {
            let guard = self.cache.lock().await;
            if let Some((fetched_at, dsn)) = guard.get(resource_name) {
                if fetched_at.elapsed().as_secs() < self.ttl_secs {
                    return Ok(dsn.clone());
                }
            }
        }
        let dsn = self.fetch_raw(resource_name).await?;
        self.cache
            .lock()
            .await
            .insert(resource_name.to_string(), (Instant::now(), dsn.clone()));
        Ok(dsn)
    }

    /// Fetch DSN bypassing the cache — used at provision time to validate access.
    pub async fn fetch_fresh(&self, resource_name: &str) -> Result<String, GcpSecretError> {
        let dsn = self.fetch_raw(resource_name).await?;
        self.cache
            .lock()
            .await
            .insert(resource_name.to_string(), (Instant::now(), dsn.clone()));
        Ok(dsn)
    }

    /// Return the list of AccessSecretVersion call paths (for audit test).
    pub fn call_log_snapshot(&self) -> Vec<String> {
        self.call_log
            .try_lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    /// Fetch the secret's raw string value verbatim — no JSON-DSN parsing,
    /// no cache read/write (D-SM-4: startup-only sourcing, no TTL benefit).
    pub async fn get_raw_secret(&self, resource_name: &str) -> Result<String, GcpSecretError> {
        self.fetch_secret_string(resource_name).await
    }

    async fn fetch_raw(&self, resource_name: &str) -> Result<String, GcpSecretError> {
        let secret_str = self.fetch_secret_string(resource_name).await?;
        Self::parse_dsn(&secret_str)
    }

    /// Shared network-call portion: HTTP GET + base64 decode. Both the
    /// DSN-JSON path (`fetch_raw`) and the raw-string path (`get_raw_secret`)
    /// share this — only DSN parsing differs afterward.
    async fn fetch_secret_string(&self, resource_name: &str) -> Result<String, GcpSecretError> {
        let url = format!(
            "{}/v1/{}/versions/latest:access",
            self.base_url, resource_name
        );

        // Record this call for audit verification.
        self.call_log.lock().await.push(resource_name.to_string());

        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.token))
            .send()
            .await
            .map_err(|e| GcpSecretError::Http(e.to_string()))?;

        let status = resp.status();

        if status == reqwest::StatusCode::FORBIDDEN {
            return Err(GcpSecretError::AccessDenied);
        }
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(GcpSecretError::NotFound);
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(GcpSecretError::Http(format!("{}: {}", status, body)));
        }

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| GcpSecretError::FormatInvalid(format!("JSON parse error: {e}")))?;

        let encoded = body["payload"]["data"]
            .as_str()
            .ok_or_else(|| GcpSecretError::FormatInvalid("missing payload.data field".into()))?;

        let decoded_bytes = STANDARD
            .decode(encoded)
            .map_err(|e| GcpSecretError::FormatInvalid(format!("base64 decode error: {e}")))?;

        String::from_utf8(decoded_bytes)
            .map_err(|_| GcpSecretError::FormatInvalid("decoded value is not valid UTF-8".into()))
    }

    fn parse_dsn(secret_str: &str) -> Result<String, GcpSecretError> {
        let val: serde_json::Value = serde_json::from_str(secret_str)
            .map_err(|e| GcpSecretError::FormatInvalid(format!("JSON parse error: {e}")))?;
        val["dsn"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| GcpSecretError::FormatInvalid("missing 'dsn' key in secret JSON".into()))
    }
}
