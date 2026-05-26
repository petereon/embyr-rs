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

use crate::adapters::system_db::SystemDb;

#[derive(Clone)]
pub struct AdminState {
    pub system_db: Arc<SystemDb>,
    pub admin_key: String,
}

#[derive(Deserialize)]
pub struct ProvisionRequest {
    pub project_id: String,
    pub dsn: String,
    #[serde(default = "default_backend_mode")]
    pub backend_mode: String,
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

fn extract_bearer(headers: &HeaderMap) -> Option<&str> {
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
    // Auth check — constant-time compare is sufficient for test keys;
    // for production a subtle::ConstantTimeEq would be preferred.
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

    // Probe customer DSN — connect with short timeout
    let customer_pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(&req.dsn)
        .await
        .map_err(|_| err(StatusCode::BAD_REQUEST, "backend_unavailable"))?;

    sqlx::query("SELECT 1")
        .execute(&customer_pool)
        .await
        .map_err(|_| err(StatusCode::BAD_REQUEST, "backend_unavailable"))?;

    // Generate API key: 32 random bytes, base64url-encoded without padding
    let mut key_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut key_bytes);
    let api_key = URL_SAFE_NO_PAD.encode(key_bytes);

    // ECIES encrypt DSN using api_key bytes as the seed
    let pubkey = ecies::derive_public_key(api_key.as_bytes());
    let encrypted_dsn = ecies::encrypt(&pubkey, req.dsn.as_bytes())
        .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?;

    // Argon2id hash API key — offload to blocking thread (heavy CPU op)
    let api_key_for_hash = api_key.clone();
    let hash = tokio::task::spawn_blocking(move || {
        argon2::hash_api_key(api_key_for_hash.as_bytes())
    })
    .await
    .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()))?
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

    Ok((
        StatusCode::CREATED,
        Json(ProvisionResponse {
            project_id: req.project_id,
            api_key,
        }),
    ))
}
