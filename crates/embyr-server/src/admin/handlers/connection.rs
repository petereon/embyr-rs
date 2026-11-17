//! ADR-083: connection secrets admin routes (session auth).
//!
//! GET /admin/v1/projects/:id/connection — any role; mode + masked summary, never a secret.
//! PUT /admin/v1/projects/:id/connection — Owner/Admin; probe-before-save, mode switch, one tx,
//!   cache eviction. Secrets are written to the server-key store only; the other mode's secrets and
//!   the legacy ECIES columns are NULLed in the same statement.

use std::fmt;
use std::str::FromStr;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use sqlx::{postgres::PgConnectOptions, Row};
use uuid::Uuid;

use embyr_core::{admin::account::Role, domain::schema_readiness::SchemaReadiness};

use crate::adapters::{agent_backend::AgentBackendAdapter, connection_secrets::ServerKeys, dsn_policy};
use crate::admin::extractors::session_context::SessionContext;
use crate::admin::handlers::provision::{not_ready_migrate_failure, probe_customer_db};
use crate::admin::state::UserAdminState;
use crate::adapters::postgres_backend::PostgresBackendAdapter;

pub(crate) type ApiResult<T> = Result<T, (StatusCode, Json<serde_json::Value>)>;

pub(crate) fn err(code: StatusCode, error: &str) -> (StatusCode, Json<serde_json::Value>) {
    (code, Json(serde_json::json!({ "error": error })))
}

fn db_err(context: &str, e: sqlx::Error) -> (StatusCode, Json<serde_json::Value>) {
    tracing::error!("connection {context}: {e}");
    err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
}

#[derive(Deserialize, Default)]
pub struct PutConnectionBody {
    pub backend_mode: String,
    #[serde(default)]
    pub dsn: Option<String>,
    #[serde(default)]
    pub agent_endpoint: Option<String>,
    #[serde(default)]
    pub agent_ca_pem: Option<String>,
    #[serde(default)]
    pub agent_client_cert_pem: Option<String>,
    #[serde(default)]
    pub agent_client_key_pem: Option<String>,
}

// Never prints the DSN or key material.
impl fmt::Debug for PutConnectionBody {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PutConnectionBody")
            .field("backend_mode", &self.backend_mode)
            .finish_non_exhaustive()
    }
}

/// Masked view of a customer DSN: host/port/database/tls only. Userinfo and query parameters
/// (which can carry `password=`) are never emitted.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct DsnSummary {
    pub host: String,
    pub port: u16,
    pub database: Option<String>,
    pub tls: bool,
}

pub fn summarize_dsn(dsn: &str) -> Option<DsnSummary> {
    let opts = PgConnectOptions::from_str(dsn).ok()?;
    Some(DsnSummary {
        host: opts.get_host().to_string(),
        port: opts.get_port(),
        database: opts.get_database().map(str::to_string),
        tls: dsn_policy::tls_acceptable(opts.get_ssl_mode()),
    })
}

/// Expiry (RFC 3339) of the first certificate in `pem`, if parseable.
fn cert_expires_at(pem: &str) -> Option<String> {
    let (_, parsed) = x509_parser::pem::parse_x509_pem(pem.as_bytes()).ok()?;
    let cert = parsed.parse_x509().ok()?;
    chrono::DateTime::from_timestamp(cert.validity().not_after.timestamp(), 0).map(|t| t.to_rfc3339())
}

struct ProjectRow {
    status: String,
    backend_mode: String,
    agent_endpoint: Option<String>,
    secret_arn: Option<String>,
    secret_gcp: Option<String>,
    dsn_enc: Option<Vec<u8>>,
    bundle_srv_enc: Option<Vec<u8>>,
    legacy_present: bool,
}

/// Account isolation (403) and soft-delete (404) for both verbs.
async fn load_project(
    state: &UserAdminState,
    session: &SessionContext,
    project_id: &str,
) -> ApiResult<ProjectRow> {
    let row = sqlx::query(
        "SELECT account_id, status, backend_mode, backend_agent_endpoint, backend_secret_arn, \
                backend_secret_gcp, backend_pg_dsn_enc, agent_tls_bundle_srv_enc, \
                (ecies_encrypted_dsn IS NOT NULL OR agent_tls_bundle_enc IS NOT NULL) AS legacy_present \
         FROM projects WHERE id = $1",
    )
    .bind(project_id)
    .fetch_optional(state.system_db.pool())
    .await
    .map_err(|e| db_err("load", e))?
    .ok_or_else(|| err(StatusCode::NOT_FOUND, "project_not_found"))?;

    let account: Option<Uuid> = row.try_get("account_id").unwrap_or(None);
    if account != Some(session.account_id) {
        return Err(err(StatusCode::NOT_FOUND, "project_not_found"));
    }
    let status: String = row.try_get("status").map_err(|e| db_err("load", e))?;
    if status == "deleted" {
        return Err(err(StatusCode::NOT_FOUND, "project_not_found"));
    }
    Ok(ProjectRow {
        status,
        backend_mode: row.try_get("backend_mode").map_err(|e| db_err("load", e))?,
        agent_endpoint: row.try_get("backend_agent_endpoint").map_err(|e| db_err("load", e))?,
        secret_arn: row.try_get("backend_secret_arn").map_err(|e| db_err("load", e))?,
        secret_gcp: row.try_get("backend_secret_gcp").map_err(|e| db_err("load", e))?,
        dsn_enc: row.try_get("backend_pg_dsn_enc").map_err(|e| db_err("load", e))?,
        bundle_srv_enc: row.try_get("agent_tls_bundle_srv_enc").map_err(|e| db_err("load", e))?,
        legacy_present: row.try_get("legacy_present").map_err(|e| db_err("load", e))?,
    })
}

fn server_keys(state: &UserAdminState) -> ServerKeys {
    ServerKeys { current: state.encryption_key, previous: state.encryption_key_previous }
}

/// GET /admin/v1/projects/:project_id/connection
pub async fn get_connection(
    Path(project_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
) -> ApiResult<Json<serde_json::Value>> {
    let p = load_project(&state, &session, &project_id).await?;
    let keys = server_keys(&state);

    let (summary, secret_stored, converted) = match p.backend_mode.as_str() {
        "direct_pg" => {
            let summary = p
                .dsn_enc
                .as_deref()
                .and_then(|c| keys.decrypt(c))
                .and_then(|dsn| summarize_dsn(&dsn));
            let converted = p.dsn_enc.is_some();
            let value = summary.map(|s| serde_json::to_value(s).unwrap_or_default());
            (value, converted || p.legacy_present, converted)
        }
        "agent" => {
            let bundle: Option<serde_json::Value> = p
                .bundle_srv_enc
                .as_deref()
                .and_then(|c| keys.decrypt(c))
                .and_then(|b| serde_json::from_str(&b).ok());
            let expires = bundle
                .as_ref()
                .and_then(|b| b["client_cert_pem"].as_str())
                .and_then(cert_expires_at);
            let converted = p.bundle_srv_enc.is_some();
            let value = p
                .agent_endpoint
                .as_ref()
                .map(|e| serde_json::json!({ "endpoint": e, "cert_expires_at": expires }));
            (value, converted || p.legacy_present, converted)
        }
        _ => {
            let reference = p.secret_arn.clone().or(p.secret_gcp.clone());
            (reference.map(|r| serde_json::json!({ "reference": r })), true, true)
        }
    };

    Ok(Json(serde_json::json!({
        "backend_mode": p.backend_mode,
        "summary": summary,
        "secret_stored": secret_stored,
        "converted": converted,
    })))
}

/// PUT /admin/v1/projects/:project_id/connection
pub async fn put_connection(
    Path(project_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
    Json(body): Json<PutConnectionBody>,
) -> ApiResult<Json<serde_json::Value>> {
    if session.role < Role::Admin {
        return Err(err(StatusCode::FORBIDDEN, "forbidden"));
    }
    let p = load_project(&state, &session, &project_id).await?;
    if p.status != "active" {
        return Err(err(StatusCode::CONFLICT, "project_not_active"));
    }
    let keys = server_keys(&state);

    // Probe first: a typo must never take a working project offline.
    let (dsn_enc, bundle_enc, endpoint) = match body.backend_mode.as_str() {
        "direct_pg" => {
            let dsn = non_empty(&body.dsn).ok_or_else(|| err(StatusCode::BAD_REQUEST, "dsn_required"))?;
            let pool = probe_customer_db(dsn).await?;
            let readiness = PostgresBackendAdapter::new_from_pool(pool)
                .verify_schema_readiness()
                .await
                .map_err(|e| {
                    tracing::error!("connection: schema readiness: {e}");
                    err(StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
                })?;
            if !matches!(readiness, SchemaReadiness::Ready { .. }) {
                return Err(not_ready_migrate_failure(&readiness, ""));
            }
            (Some(keys.encrypt(dsn)), None, None)
        }
        "agent" => {
            let endpoint = non_empty(&body.agent_endpoint)
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, "agent_endpoint_required"))?;
            let ca = non_empty(&body.agent_ca_pem)
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, "agent_ca_pem_required"))?;
            let cert = non_empty(&body.agent_client_cert_pem)
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, "agent_client_cert_pem_required"))?;
            let key = non_empty(&body.agent_client_key_pem)
                .ok_or_else(|| err(StatusCode::BAD_REQUEST, "agent_client_key_pem_required"))?;
            AgentBackendAdapter::new(endpoint, ca.as_bytes(), cert.as_bytes(), key.as_bytes())
                .await
                .map_err(|_| err(StatusCode::BAD_REQUEST, "backend_unavailable"))?;
            let bundle = serde_json::json!({
                "ca_pem": ca, "client_cert_pem": cert, "client_key_pem": key,
            })
            .to_string();
            (None, Some(keys.encrypt(&bundle)), Some(endpoint.to_string()))
        }
        _ => return Err(err(StatusCode::UNPROCESSABLE_ENTITY, "unsupported_backend_mode")),
    };

    // One statement = one transaction: new mode's secrets in, every other secret out.
    let updated = sqlx::query(
        "UPDATE projects SET \
           backend_mode = $2, backend_pg_dsn_enc = $3, agent_tls_bundle_srv_enc = $4, \
           backend_agent_endpoint = $5, ecies_encrypted_dsn = NULL, agent_tls_bundle_enc = NULL, \
           backend_secret_arn = NULL, backend_secret_gcp = NULL, updated_at = now() \
         WHERE id = $1 AND status = 'active'",
    )
    .bind(&project_id)
    .bind(&body.backend_mode)
    .bind(&dsn_enc)
    .bind(&bundle_enc)
    .bind(&endpoint)
    .execute(state.system_db.pool())
    .await
    .map_err(|e| db_err("update", e))?;
    if updated.rows_affected() == 0 {
        return Err(err(StatusCode::CONFLICT, "project_not_active"));
    }

    state.credential_cache.evict_project(&project_id).await;
    tracing::info!(
        account_id = %session.account_id, project_id = %project_id, mode = %body.backend_mode,
        actor = %session.user_id, "audit: project connection updated"
    );
    Ok(Json(serde_json::json!({ "backend_mode": body.backend_mode, "probe": "ok" })))
}

fn non_empty(v: &Option<String>) -> Option<&str> {
    v.as_deref().filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_never_carries_credentials() {
        let dsn = "postgres://admin:s3cr3t@db.prod.internal:6432/embyr_prod?sslmode=require&password=q9";
        let s = summarize_dsn(dsn).unwrap();
        assert_eq!(
            s,
            DsnSummary {
                host: "db.prod.internal".into(),
                port: 6432,
                database: Some("embyr_prod".into()),
                tls: true,
            }
        );
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("s3cr3t") && !json.contains("admin") && !json.contains("q9"));
    }

    #[test]
    fn summary_flags_plaintext_and_rejects_garbage() {
        assert!(!summarize_dsn("postgres://u:p@h/db").unwrap().tls);
        assert_eq!(summarize_dsn("not a dsn"), None);
    }
}
