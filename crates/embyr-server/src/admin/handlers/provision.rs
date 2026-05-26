use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use embyr_core::auth::{argon2, ecies};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;

use crate::adapters::{
    aws_secret_fetcher::{AwsSecretError, AwsSecretFetcher},
    credential_cache::CredentialCache,
    gcp_secret_fetcher::{GcpSecretError, GcpSecretFetcher},
    system_db::SystemDb,
};

#[derive(Clone)]
pub struct AdminState {
    pub system_db: Arc<SystemDb>,
    pub admin_key: String,
    pub credential_cache: Arc<CredentialCache>,
    /// Injected AWS Secrets Manager fetcher. None when the server is started
    /// without AWS support (e.g., standard direct_pg/agent tests).
    pub aws_secret_fetcher: Option<Arc<AwsSecretFetcher>>,
    /// Injected GCP Secret Manager fetcher. None when the server is started
    /// without GCP support (e.g., standard direct_pg/aws/agent tests).
    pub gcp_secret_fetcher: Option<Arc<GcpSecretFetcher>>,
}

#[derive(Deserialize)]
pub struct ProvisionRequest {
    pub project_id: String,
    /// Customer DB DSN — required for backend_mode=direct_pg, absent for backend_mode=agent.
    #[serde(default)]
    pub dsn: Option<String>,
    #[serde(default = "default_backend_mode")]
    pub backend_mode: String,
    /// AWS Secrets Manager ARN — required for backend_mode=aws_secret.
    #[serde(default)]
    pub secret_arn: Option<String>,
    /// GCP Secret Manager resource name — required for backend_mode=gcp_secret.
    /// Format: "projects/{project}/secrets/{secret}"
    #[serde(default)]
    pub gcp_resource_name: Option<String>,
    /// Agent gRPC endpoint (host:port) — required for backend_mode=agent.
    #[serde(default)]
    pub agent_endpoint: Option<String>,
    /// PEM-encoded CA cert that signed the agent server cert.
    #[serde(default)]
    pub agent_ca_pem: Option<String>,
    /// PEM-encoded client certificate for SaaS→agent mTLS.
    #[serde(default)]
    pub agent_client_cert_pem: Option<String>,
    /// PEM-encoded client private key for SaaS→agent mTLS.
    #[serde(default)]
    pub agent_client_key_pem: Option<String>,
}

fn default_backend_mode() -> String {
    "direct_pg".into()
}

#[derive(Serialize)]
pub struct ProvisionResponse {
    pub project_id: String,
    pub api_key: String,
}

type ApiResult<T> = Result<T, (StatusCode, Json<serde_json::Value>)>;

fn err(code: StatusCode, error: &str) -> (StatusCode, Json<serde_json::Value>) {
    (code, Json(serde_json::json!({ "error": error })))
}

fn valid_project_id(id: &str) -> bool {
    if id.is_empty() || id.len() > 63 {
        return false;
    }
    let mut chars = id.chars();
    let first = match chars.next() {
        Some(c) => c,
        None => return false,
    };
    if !first.is_ascii_lowercase() {
        return false;
    }
    chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

pub fn extract_bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
}

pub async fn provision(
    headers: HeaderMap,
    State(state): State<AdminState>,
    Json(req): Json<ProvisionRequest>,
) -> ApiResult<(StatusCode, Json<ProvisionResponse>)> {
    // Auth check
    let token = extract_bearer(&headers)
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "missing_auth"))?;
    if token != state.admin_key {
        return Err(err(StatusCode::UNAUTHORIZED, "invalid_auth"));
    }

    // Validate project_id
    if !valid_project_id(&req.project_id) {
        return Err(err(StatusCode::BAD_REQUEST, "invalid_project_id_format"));
    }

    // Duplicate check
    let existing: Option<String> = sqlx::query_scalar("SELECT id FROM projects WHERE id = $1")
        .bind(&req.project_id)
        .fetch_optional(state.system_db.pool())
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    if existing.is_some() {
        return Err(err(StatusCode::CONFLICT, "project_already_exists"));
    }

    // Generate API key: 32 random bytes, base64url-encoded without padding
    let mut key_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut key_bytes);
    let api_key = URL_SAFE_NO_PAD.encode(key_bytes);

    // Argon2id hash API key — offload to blocking thread (heavy CPU op)
    let api_key_for_hash = api_key.clone();
    let hash = tokio::task::spawn_blocking(move || {
        argon2::hash_api_key(api_key_for_hash.as_bytes())
    })
    .await
    .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
    .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    if req.backend_mode == "aws_secret" {
        // AWS Secrets Manager mode: fetch DSN from ARN, probe customer DB,
        // apply migrations, store only the ARN (never the DSN).
        let arn = req
            .secret_arn
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "secret_arn_required"))?;

        let fetcher = state
            .aws_secret_fetcher
            .as_ref()
            .ok_or_else(|| err(StatusCode::INTERNAL_SERVER_ERROR, "aws_secret_fetcher_not_configured"))?;

        let dsn = fetcher.fetch_fresh(arn).await.map_err(|e| match e {
            AwsSecretError::AccessDenied => err(StatusCode::BAD_REQUEST, "backend_secret_fetch_failed"),
            AwsSecretError::NotFound => err(StatusCode::BAD_REQUEST, "backend_secret_fetch_failed"),
            AwsSecretError::FormatInvalid(_) => err(StatusCode::BAD_REQUEST, "backend_secret_format_invalid"),
            AwsSecretError::Sdk(_) => err(StatusCode::BAD_REQUEST, "backend_secret_fetch_failed"),
        })?;

        // Probe customer DSN
        let customer_pool = PgPoolOptions::new()
            .max_connections(2)
            .acquire_timeout(std::time::Duration::from_secs(5))
            .connect(&dsn)
            .await
            .map_err(|_| err(StatusCode::BAD_REQUEST, "backend_unavailable"))?;

        sqlx::query("SELECT 1")
            .execute(&customer_pool)
            .await
            .map_err(|_| err(StatusCode::BAD_REQUEST, "backend_unavailable"))?;

        // Apply customer migrations
        sqlx::migrate!("../../migrations/customer")
            .run(&customer_pool)
            .await
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

        // Insert project row: store ARN only, ecies_encrypted_dsn stays NULL
        sqlx::query(
            "INSERT INTO projects \
             (id, status, backend_mode, api_key_hash_current, backend_secret_arn) \
             VALUES ($1, 'active', 'aws_secret', $2, $3)",
        )
        .bind(&req.project_id)
        .bind(&hash)
        .bind(arn)
        .execute(state.system_db.pool())
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    } else if req.backend_mode == "gcp_secret" {
        // GCP Secret Manager mode: fetch DSN from resource_name via REST API,
        // probe customer DB, apply migrations, store only the resource_name (never the DSN).
        let resource_name = req
            .gcp_resource_name
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "gcp_resource_name_required"))?;

        let fetcher = state
            .gcp_secret_fetcher
            .as_ref()
            .ok_or_else(|| err(StatusCode::INTERNAL_SERVER_ERROR, "gcp_secret_fetcher_not_configured"))?;

        let dsn = fetcher.fetch_fresh(resource_name).await.map_err(|e| match e {
            GcpSecretError::AccessDenied => err(StatusCode::BAD_REQUEST, "backend_secret_fetch_failed"),
            GcpSecretError::NotFound => err(StatusCode::BAD_REQUEST, "backend_secret_fetch_failed"),
            GcpSecretError::FormatInvalid(_) => err(StatusCode::BAD_REQUEST, "backend_secret_format_invalid"),
            GcpSecretError::Http(_) => err(StatusCode::BAD_REQUEST, "backend_secret_fetch_failed"),
        })?;

        // Probe customer DSN
        let customer_pool = PgPoolOptions::new()
            .max_connections(2)
            .acquire_timeout(std::time::Duration::from_secs(5))
            .connect(&dsn)
            .await
            .map_err(|_| err(StatusCode::BAD_REQUEST, "backend_unavailable"))?;

        sqlx::query("SELECT 1")
            .execute(&customer_pool)
            .await
            .map_err(|_| err(StatusCode::BAD_REQUEST, "backend_unavailable"))?;

        // Apply customer migrations
        sqlx::migrate!("../../migrations/customer")
            .run(&customer_pool)
            .await
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

        // Insert project row: store resource_name only, ecies_encrypted_dsn stays NULL
        sqlx::query(
            "INSERT INTO projects \
             (id, status, backend_mode, api_key_hash_current, backend_secret_gcp) \
             VALUES ($1, 'active', 'gcp_secret', $2, $3)",
        )
        .bind(&req.project_id)
        .bind(&hash)
        .bind(resource_name)
        .execute(state.system_db.pool())
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    } else if req.backend_mode == "agent" {
        // Agent-mode: store endpoint + mTLS bundle (ECIES-encrypted); DSN must be NULL.
        let endpoint = req
            .agent_endpoint
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "agent_endpoint_required"))?;

        let ca_pem = req
            .agent_ca_pem
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "agent_ca_pem_required"))?;
        let client_cert_pem = req
            .agent_client_cert_pem
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "agent_client_cert_pem_required"))?;
        let client_key_pem = req
            .agent_client_key_pem
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "agent_client_key_pem_required"))?;

        // Encrypt TLS bundle as JSON with ECIES using api_key bytes as seed.
        let tls_bundle = serde_json::json!({
            "ca_pem": ca_pem,
            "client_cert_pem": client_cert_pem,
            "client_key_pem": client_key_pem,
        })
        .to_string();
        let pubkey = ecies::derive_public_key(api_key.as_bytes());
        let encrypted_bundle = ecies::encrypt(&pubkey, tls_bundle.as_bytes())
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

        // Insert agent-mode project row — backend_pg_creds_enc stays NULL.
        sqlx::query(
            "INSERT INTO projects \
             (id, status, backend_mode, api_key_hash_current, \
              backend_agent_endpoint, agent_tls_bundle_enc) \
             VALUES ($1, 'active', 'agent', $2, $3, $4)",
        )
        .bind(&req.project_id)
        .bind(&hash)
        .bind(endpoint)
        .bind(&encrypted_bundle)
        .execute(state.system_db.pool())
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    } else {
        // Direct-PG mode: existing behavior.
        let dsn = req
            .dsn
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "dsn_required"))?;

        // Probe customer DSN — connect with short timeout
        let customer_pool = PgPoolOptions::new()
            .max_connections(2)
            .acquire_timeout(std::time::Duration::from_secs(5))
            .connect(dsn)
            .await
            .map_err(|_| err(StatusCode::BAD_REQUEST, "backend_unavailable"))?;

        sqlx::query("SELECT 1")
            .execute(&customer_pool)
            .await
            .map_err(|_| err(StatusCode::BAD_REQUEST, "backend_unavailable"))?;

        // ECIES encrypt DSN using api_key bytes as the seed
        let pubkey = ecies::derive_public_key(api_key.as_bytes());
        let encrypted_dsn = ecies::encrypt(&pubkey, dsn.as_bytes())
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

        // Insert project row
        sqlx::query(
            "INSERT INTO projects \
             (id, status, backend_mode, api_key_hash_current, ecies_encrypted_dsn) \
             VALUES ($1, 'active', $2, $3, $4)",
        )
        .bind(&req.project_id)
        .bind(&req.backend_mode)
        .bind(&hash)
        .bind(&encrypted_dsn)
        .execute(state.system_db.pool())
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

        // Apply customer migrations
        sqlx::migrate!("../../migrations/customer")
            .run(&customer_pool)
            .await
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
    }

    Ok((
        StatusCode::CREATED,
        Json(ProvisionResponse {
            project_id: req.project_id,
            api_key,
        }),
    ))
}
