use axum::{
    extract::State,
    http::StatusCode,
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use embyr_core::{
    auth::{argon2, ecies},
    domain::{project::ProjectId, schema_readiness::SchemaReadiness},
};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPoolOptions;

use crate::adapters::{
    aws_secret_fetcher::AwsSecretError,
    gcp_secret_fetcher::GcpSecretError,
    postgres_backend::PostgresBackendAdapter,
};
use crate::admin::state::OperatorState;

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

fn err_with_detail(
    code: StatusCode,
    error: &str,
    detail: String,
) -> (StatusCode, Json<serde_json::Value>) {
    (
        code,
        Json(serde_json::json!({ "error": error, "detail": detail })),
    )
}

/// Build the enriched `customer_db_not_prepped` / `customer_db_schema_stale`
/// response when the existing (unchanged) `migrate()` attempt itself fails —
/// distinguishing "reachable but not ready" from the pre-existing generic
/// `backend_unavailable` connectivity-failure classification (AC-02-02,
/// AC-02-03, AC-02-04).
fn not_ready_migrate_failure(
    readiness: &SchemaReadiness,
    migrate_err: &str,
) -> (StatusCode, Json<serde_json::Value>) {
    match readiness {
        SchemaReadiness::NotPrepped { missing_tables } => err_with_detail(
            StatusCode::BAD_REQUEST,
            "customer_db_not_prepped",
            format!(
                "table '{}' not found -- run the embyr database-preparation step first",
                missing_tables
                    .first()
                    .map(String::as_str)
                    .unwrap_or("unknown")
            ),
        ),
        SchemaReadiness::Stale {
            expected_version,
            found_version,
        } => err_with_detail(
            StatusCode::BAD_REQUEST,
            "customer_db_schema_stale",
            format!(
                "expected schema version {expected_version}, found {found_version} \
                 -- re-run the embyr database-preparation step"
            ),
        ),
        SchemaReadiness::Ready { .. } => err(StatusCode::INTERNAL_SERVER_ERROR, migrate_err),
    }
}

/// Connect to a customer Postgres DSN and verify it is reachable.
///
/// Uses a 2-connection pool with a 5-second acquire timeout.
/// Returns `backend_unavailable` (400) if the connection or the liveness
/// query fails. The returned pool should be used immediately for migrations.
async fn probe_customer_db(dsn: &str) -> ApiResult<sqlx::PgPool> {
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

    Ok(customer_pool)
}

/// Insert a `rate_buckets` row within the supplied transaction.
///
/// Called after the `projects` INSERT in every backend mode so the two writes
/// are atomic.  The initial token count equals the configured capacity.
async fn insert_rate_bucket_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    project_id: &str,
    capacity: f64,
) -> ApiResult<()> {
    sqlx::query(
        "INSERT INTO rate_buckets (project_id, tokens, last_refill) \
         VALUES ($1, $2, now())",
    )
    .bind(project_id)
    .bind(capacity)
    .execute(&mut **tx)
    .await
    .map_err(|e| {
        tracing::error!("provision: rate_buckets insert: {e}");
        err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())
    })?;
    Ok(())
}

pub async fn provision(
    State(state): State<OperatorState>,
    Json(req): Json<ProvisionRequest>,
) -> ApiResult<(StatusCode, Json<ProvisionResponse>)> {
    // Auth is enforced by operator_auth_middleware applied at the router layer.

    // Validate project_id using the domain rule (same regex as ProjectId::new).
    if ProjectId::new(&req.project_id).is_err() {
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

    let rate_limit_capacity = state.rate_limit_capacity;

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

        let customer_pool = probe_customer_db(&dsn).await?;

        PostgresBackendAdapter::new_from_pool(customer_pool.clone())
            .migrate()
            .await
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

        // Wrap projects INSERT + rate_buckets INSERT in a transaction.
        let mut tx = state.system_db.pool().begin().await.map_err(|e| {
            tracing::error!("provision: begin transaction: {e}");
            err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())
        })?;

        sqlx::query(
            "INSERT INTO projects \
             (id, status, backend_mode, api_key_hash_current, backend_secret_arn) \
             VALUES ($1, 'active', 'aws_secret', $2, $3)",
        )
        .bind(&req.project_id)
        .bind(&hash)
        .bind(arn)
        .execute(&mut *tx)
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

        insert_rate_bucket_in_tx(&mut tx, &req.project_id, rate_limit_capacity).await?;

        tx.commit().await.map_err(|e| {
            tracing::error!("provision: commit: {e}");
            err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())
        })?;
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

        let customer_pool = probe_customer_db(&dsn).await?;

        PostgresBackendAdapter::new_from_pool(customer_pool.clone())
            .migrate()
            .await
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

        // Wrap projects INSERT + rate_buckets INSERT in a transaction.
        let mut tx = state.system_db.pool().begin().await.map_err(|e| {
            tracing::error!("provision: begin transaction: {e}");
            err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())
        })?;

        sqlx::query(
            "INSERT INTO projects \
             (id, status, backend_mode, api_key_hash_current, backend_secret_gcp) \
             VALUES ($1, 'active', 'gcp_secret', $2, $3)",
        )
        .bind(&req.project_id)
        .bind(&hash)
        .bind(resource_name)
        .execute(&mut *tx)
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

        insert_rate_bucket_in_tx(&mut tx, &req.project_id, rate_limit_capacity).await?;

        tx.commit().await.map_err(|e| {
            tracing::error!("provision: commit: {e}");
            err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())
        })?;
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

        // Wrap projects INSERT + rate_buckets INSERT in a transaction.
        let mut tx = state.system_db.pool().begin().await.map_err(|e| {
            tracing::error!("provision: begin transaction: {e}");
            err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())
        })?;

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
        .execute(&mut *tx)
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

        insert_rate_bucket_in_tx(&mut tx, &req.project_id, rate_limit_capacity).await?;

        tx.commit().await.map_err(|e| {
            tracing::error!("provision: commit: {e}");
            err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())
        })?;
    } else {
        // Direct-PG mode: existing behavior.
        let dsn = req
            .dsn
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "dsn_required"))?;

        let customer_pool = probe_customer_db(dsn).await?;

        // Ready-skip-migrate (step 03-01, AC-02-05): verify_schema_readiness()
        // is SELECT-only against _sqlx_migrations. On Ready, migrate() is
        // never invoked at all below — the submitted connection string need
        // not hold elevated (DDL) privilege. On NotPrepped/Stale, fall
        // through to today's migrate() behavior unchanged (04-01 enriches
        // that path with response bodies).
        let customer_adapter = PostgresBackendAdapter::new_from_pool(customer_pool.clone());
        let readiness = customer_adapter
            .verify_schema_readiness()
            .await
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;
        let is_ready = matches!(
            readiness,
            embyr_core::domain::schema_readiness::SchemaReadiness::Ready { .. }
        );

        // ECIES encrypt DSN using api_key bytes as the seed
        let pubkey = ecies::derive_public_key(api_key.as_bytes());
        let encrypted_dsn = ecies::encrypt(&pubkey, dsn.as_bytes())
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

        // Wrap projects INSERT + rate_buckets INSERT in a transaction.
        let mut tx = state.system_db.pool().begin().await.map_err(|e| {
            tracing::error!("provision: begin transaction: {e}");
            err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())
        })?;

        sqlx::query(
            "INSERT INTO projects \
             (id, status, backend_mode, api_key_hash_current, ecies_encrypted_dsn) \
             VALUES ($1, 'active', $2, $3, $4)",
        )
        .bind(&req.project_id)
        .bind(&req.backend_mode)
        .bind(&hash)
        .bind(&encrypted_dsn)
        .execute(&mut *tx)
        .await
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

        insert_rate_bucket_in_tx(&mut tx, &req.project_id, rate_limit_capacity).await?;

        tx.commit().await.map_err(|e| {
            tracing::error!("provision: commit: {e}");
            err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string())
        })?;

        if !is_ready {
            customer_adapter
                .migrate()
                .await
                .map_err(|e| not_ready_migrate_failure(&readiness, &e.to_string()))?;
        }
    }

    Ok((
        StatusCode::CREATED,
        Json(ProvisionResponse {
            project_id: req.project_id,
            api_key,
        }),
    ))
}
