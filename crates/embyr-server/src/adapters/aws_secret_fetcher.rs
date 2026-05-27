use std::{
    collections::HashMap,
    sync::Arc,
    time::Instant,
};

use aws_sdk_secretsmanager::{
    error::{ProvideErrorMetadata, SdkError},
    operation::get_secret_value::GetSecretValueError,
};
use tokio::sync::Mutex;

/// Error variants for AWS Secrets Manager fetch operations.
#[derive(Debug, thiserror::Error)]
pub enum AwsSecretError {
    #[error("IAM access denied")]
    AccessDenied,
    #[error("Secret format invalid: {0}")]
    FormatInvalid(String),
    #[error("Secret not found")]
    NotFound,
    #[error("AWS error: {0}")]
    Sdk(String),
}

/// Driven adapter that resolves DSN strings from AWS Secrets Manager.
///
/// Caches DSN per ARN with a configurable TTL (default 300s).
/// Override via `EMBYR_AWS_SECRET_CACHE_SECONDS` env var (used in tests for short TTL).
///
/// Security invariant: the raw DSN is held only in-process memory and in the
/// customer Postgres connection. It is NEVER written to the system DB.
pub struct AwsSecretFetcher {
    client: aws_sdk_secretsmanager::Client,
    cache: Arc<Mutex<HashMap<String, (Instant, String)>>>,
    ttl_secs: u64,
}

impl AwsSecretFetcher {
    /// Construct the fetcher with a given AWS SDK config and TTL.
    pub async fn new(config: &aws_config::SdkConfig, ttl_secs: u64) -> Self {
        let client = aws_sdk_secretsmanager::Client::new(config);
        Self {
            client,
            cache: Arc::new(Mutex::new(HashMap::new())),
            ttl_secs,
        }
    }

    /// Return the DSN for the given ARN, using the cache when fresh.
    pub async fn get_dsn(&self, arn: &str) -> Result<String, AwsSecretError> {
        {
            let guard = self.cache.lock().await;
            if let Some((fetched_at, dsn)) = guard.get(arn) {
                if fetched_at.elapsed().as_secs() < self.ttl_secs {
                    return Ok(dsn.clone());
                }
            }
        }
        let dsn = self.fetch_raw(arn).await?;
        self.cache
            .lock()
            .await
            .insert(arn.to_string(), (Instant::now(), dsn.clone()));
        Ok(dsn)
    }

    /// Fetch DSN bypassing the cache — used at provision time to validate access.
    pub async fn fetch_fresh(&self, arn: &str) -> Result<String, AwsSecretError> {
        let dsn = self.fetch_raw(arn).await?;
        self.cache
            .lock()
            .await
            .insert(arn.to_string(), (Instant::now(), dsn.clone()));
        Ok(dsn)
    }

    async fn fetch_raw(&self, arn: &str) -> Result<String, AwsSecretError> {
        let resp = self
            .client
            .get_secret_value()
            .secret_id(arn)
            .send()
            .await
            .map_err(Self::map_sdk_error)?;

        let secret_str = resp
            .secret_string()
            .ok_or_else(|| AwsSecretError::FormatInvalid("secret has no string value".into()))?;

        Self::parse_dsn(secret_str)
    }

    fn map_sdk_error(
        err: SdkError<GetSecretValueError>,
    ) -> AwsSecretError {
        match &err {
            SdkError::ServiceError(svc) => {
                match svc.err() {
                    GetSecretValueError::ResourceNotFoundException(_) => AwsSecretError::NotFound,
                    other => {
                        // AccessDeniedException is not a typed variant in SDK v1;
                        // check the error code string.
                        let code = other.code().unwrap_or("");
                        if code == "AccessDeniedException" {
                            AwsSecretError::AccessDenied
                        } else {
                            AwsSecretError::Sdk(err.to_string())
                        }
                    }
                }
            }
            _ => AwsSecretError::Sdk(err.to_string()),
        }
    }

    fn parse_dsn(secret_str: &str) -> Result<String, AwsSecretError> {
        let val: serde_json::Value = serde_json::from_str(secret_str)
            .map_err(|e| AwsSecretError::FormatInvalid(format!("JSON parse error: {e}")))?;
        val["dsn"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| AwsSecretError::FormatInvalid("missing 'dsn' key in secret JSON".into()))
    }
}
