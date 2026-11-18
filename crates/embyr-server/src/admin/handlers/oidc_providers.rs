//! OIDC provider handlers.
//!
//! list_oidc_providers (GET /admin/v1/oidc_providers):
//!   Session auth, Owner only. client_secret_enc never returned.
//!
//! create_oidc_provider (POST /admin/v1/oidc_providers):
//!   Session auth, Owner only.
//!   client_secret → AES-256-GCM under EMBYR_ENCRYPTION_KEY (`ServerKeys`) before storage.
//!   The issuer's discovery document is fetched and vetted first (422 otherwise, AC-SSO-07).
//!   201 { id, issuer, client_id, enabled: true }. client_secret absent.
//!
//! patch_oidc_provider (PATCH /admin/v1/oidc_providers/:provider_id):
//!   Session auth, Owner only. Partial update. Enabling re-validates the issuer (422 otherwise);
//!   disabling ends the provider's SSO sessions (ADR-086, O6); password sessions are untouched.
//!
//! put_oidc_provider_secret (PUT /admin/v1/oidc_providers/:provider_id/secret):
//!   Session auth, Owner only. Replaces the client secret (never returned or logged). 204.
//!
//! delete_oidc_provider (DELETE /admin/v1/oidc_providers/:provider_id):
//!   Session auth, Owner only. 204. Its SSO sessions end by FK cascade; password sessions are untouched.

use axum::{
    extract::{Json, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use embyr_core::admin::account::Role;
use embyr_core::admin::audit::{events, AuditEvent, AuditOutcome};

use crate::adapters::admin_audit::record;
use crate::admin::extractors::session_context::SessionContext;
use crate::admin::state::UserAdminState;

/// Largest accepted `client_id` / `client_secret` (real IdP values are well under 256 bytes).
const MAX_CREDENTIAL_BYTES: usize = 1024;

// ── Errors ────────────────────────────────────────────────────────────────────

/// Handler error: a bare status, or a 422 that tells the Owner which check failed (labels only).
pub struct Reject(StatusCode, Option<(&'static str, &'static str)>);

impl From<StatusCode> for Reject {
    fn from(s: StatusCode) -> Self {
        Reject(s, None)
    }
}

impl IntoResponse for Reject {
    fn into_response(self) -> Response {
        match self.1 {
            Some((error, reason)) => {
                (self.0, Json(serde_json::json!({ "error": error, "reason": reason }))).into_response()
            }
            None => self.0.into_response(),
        }
    }
}

fn invalid(reason: &'static str) -> Reject {
    Reject(StatusCode::UNPROCESSABLE_ENTITY, Some(("invalid_request", reason)))
}

/// AC-SSO-17: credentials are non-empty and bounded; the secret is never echoed or logged.
fn check_credentials(client_id: Option<&str>, client_secret: &str) -> Result<(), Reject> {
    if client_id.is_some_and(|c| c.trim().is_empty() || c.len() > MAX_CREDENTIAL_BYTES) {
        return Err(invalid("client_id_invalid"));
    }
    if client_secret.trim().is_empty() || client_secret.len() > MAX_CREDENTIAL_BYTES {
        return Err(invalid("client_secret_invalid"));
    }
    Ok(())
}

/// AC-SSO-07: the issuer must be reachable under the outbound policy and its discovery document
/// must name exactly this issuer. A known `provider_id` also primes the discovery cache.
async fn check_issuer(state: &UserAdminState, provider_id: Option<Uuid>, issuer: &str) -> Result<(), Reject> {
    let result = match provider_id {
        Some(id) => state.sso_client.refresh_discovery(id, issuer).await.map(|_| ()),
        None => state.sso_client.discover_fresh(issuer).await.map(|_| ()),
    };
    result.map_err(|e| Reject(StatusCode::UNPROCESSABLE_ENTITY, Some(("issuer_invalid", e.reason()))))
}

async fn audit(state: &UserAdminState, session: &SessionContext, event: &str, provider_id: Uuid) {
    let mut ev = AuditEvent::new(event, AuditOutcome::Ok);
    ev.account_id = Some(session.account_id);
    ev.actor_user_id = Some(session.user_id).filter(|u| !u.is_nil());
    ev.provider_id = Some(provider_id);
    record(state.system_db.pool(), &ev).await;
}

// ── Response type ─────────────────────────────────────────────────────────────

/// Shape returned by GET and POST /admin/v1/oidc_providers.
/// client_secret and client_secret_enc are intentionally absent.
#[derive(Serialize)]
pub struct OidcProviderResponse {
    pub id: String,
    pub issuer: String,
    pub client_id: String,
    pub enabled: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

// ── Request types ─────────────────────────────────────────────────────────────

/// Body for POST /admin/v1/oidc_providers.
#[derive(Deserialize)]
pub struct CreateOidcProviderBody {
    pub issuer: String,
    pub client_id: String,
    /// Plaintext OIDC client secret — AES-256-GCM encrypted before storage; never persisted.
    pub client_secret: String,
}

/// Body for PATCH /admin/v1/oidc_providers/:provider_id.
#[derive(Deserialize)]
pub struct PatchOidcProviderBody {
    pub enabled: Option<bool>,
}

/// Body for PUT /admin/v1/oidc_providers/:provider_id/secret.
#[derive(Deserialize)]
pub struct PutSecretBody {
    pub client_secret: String,
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// GET /admin/v1/oidc_providers
///
/// Owner-only. Returns all OIDC providers for the authenticated account.
/// client_secret_enc is never included in the response.
pub async fn list_oidc_providers(
    State(state): State<UserAdminState>,
    session: SessionContext,
) -> Result<Json<Vec<OidcProviderResponse>>, StatusCode> {
    if session.role != Role::Owner {
        return Err(StatusCode::FORBIDDEN);
    }

    let pool = state.system_db.pool();

    let rows = sqlx::query(
        "SELECT id::text AS id, issuer, client_id, enabled, created_at \
         FROM oidc_providers \
         WHERE account_id = $1 \
         ORDER BY created_at ASC",
    )
    .bind(session.account_id)
    .fetch_all(pool)
    .await
    .map_err(|e| {
        tracing::error!("list_oidc_providers: DB error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let providers: Vec<OidcProviderResponse> = rows
        .into_iter()
        .map(|row| OidcProviderResponse {
            id: row.try_get("id").unwrap_or_default(),
            issuer: row.try_get("issuer").unwrap_or_default(),
            client_id: row.try_get("client_id").unwrap_or_default(),
            enabled: row.try_get("enabled").unwrap_or(true),
            created_at: row
                .try_get("created_at")
                .unwrap_or_else(|_| chrono::Utc::now()),
        })
        .collect();

    Ok(Json(providers))
}

/// POST /admin/v1/oidc_providers
///
/// Owner-only. Encrypts client_secret with AES-256-GCM before storage.
/// Returns 201 + OidcProviderResponse (client_secret absent).
/// Duplicate (account_id, issuer) → 409 Conflict. Issuer not https / forbidden address / discovery
/// unreachable or naming another issuer → 422 `{error:"issuer_invalid", reason}`.
pub async fn create_oidc_provider(
    State(state): State<UserAdminState>,
    session: SessionContext,
    Json(body): Json<CreateOidcProviderBody>,
) -> Result<(StatusCode, Json<OidcProviderResponse>), Reject> {
    if session.role != Role::Owner {
        return Err(StatusCode::FORBIDDEN.into());
    }
    check_credentials(Some(&body.client_id), &body.client_secret)?;
    check_issuer(&state, None, &body.issuer).await?;

    // AES-256-GCM under the server key (ADR-083 `ServerKeys`): nonce (12 bytes) || ciphertext.
    // Plaintext client_secret is never written to the database.
    let secret_enc = state.server_keys.encrypt(&body.client_secret);

    let pool = state.system_db.pool();

    let row = sqlx::query(
        "INSERT INTO oidc_providers (account_id, issuer, client_id, client_secret_enc) \
         VALUES ($1, $2, $3, $4) \
         RETURNING id::text AS id, issuer, client_id, enabled, created_at",
    )
    .bind(session.account_id)
    .bind(&body.issuer)
    .bind(&body.client_id)
    .bind(&secret_enc as &[u8])
    .fetch_one(pool)
    .await
    .map_err(|e| {
        if let sqlx::Error::Database(ref db_err) = e {
            // PostgreSQL unique_violation = "23505"
            if db_err.code().as_deref() == Some("23505") {
                return StatusCode::CONFLICT;
            }
        }
        tracing::error!("create_oidc_provider: INSERT error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let response = OidcProviderResponse {
        id: row
            .try_get("id")
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        issuer: row
            .try_get("issuer")
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        client_id: row
            .try_get("client_id")
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        enabled: row
            .try_get("enabled")
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        created_at: row
            .try_get("created_at")
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    };

    if let Ok(id) = Uuid::parse_str(&response.id) {
        audit(&state, &session, events::SSO_PROVIDER_CREATED, id).await;
    }
    Ok((StatusCode::CREATED, Json(response)))
}

/// PATCH /admin/v1/oidc_providers/:provider_id
///
/// Owner-only. Partial update: `enabled` field via COALESCE.
/// Enabling validates the issuer again (422 `issuer_invalid`); disabling deletes the sessions
/// minted through this provider (`sessions.sso_provider_id`).
/// Returns 200 + updated OidcProviderResponse. 404 if provider not found.
pub async fn patch_oidc_provider(
    Path(provider_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
    Json(body): Json<PatchOidcProviderBody>,
) -> Result<Json<OidcProviderResponse>, Reject> {
    if session.role != Role::Owner {
        return Err(StatusCode::FORBIDDEN.into());
    }

    let provider_uuid = Uuid::parse_str(&provider_id).map_err(|_| StatusCode::NOT_FOUND)?;

    let pool = state.system_db.pool();

    let db_err = |e: sqlx::Error| {
        tracing::error!("patch_oidc_provider: DB error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    };

    // AC-SSO-07: no dead button. The network check runs before the transaction opens.
    if body.enabled == Some(true) {
        let issuer: String = sqlx::query_scalar("SELECT issuer FROM oidc_providers WHERE id = $1 AND account_id = $2")
            .bind(provider_uuid)
            .bind(session.account_id)
            .fetch_optional(pool)
            .await
            .map_err(db_err)?
            .ok_or(StatusCode::NOT_FOUND)?;
        check_issuer(&state, Some(provider_uuid), &issuer).await?;
    }

    let mut tx = pool.begin().await.map_err(db_err)?;
    let row = sqlx::query(
        "UPDATE oidc_providers \
         SET enabled = COALESCE($3, enabled) \
         WHERE id = $1 AND account_id = $2 \
         RETURNING id::text AS id, issuer, client_id, enabled, created_at",
    )
    .bind(provider_uuid)
    .bind(session.account_id)
    .bind(body.enabled)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db_err)?
    .ok_or(StatusCode::NOT_FOUND)?;

    // O6: a disabled provider must not leave signed-in SSO sessions behind.
    if body.enabled == Some(false) {
        sqlx::query("DELETE FROM sessions WHERE sso_provider_id = $1")
            .bind(provider_uuid)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    tx.commit().await.map_err(db_err)?;

    match body.enabled {
        Some(true) => audit(&state, &session, events::SSO_PROVIDER_ENABLED, provider_uuid).await,
        Some(false) => audit(&state, &session, events::SSO_PROVIDER_DISABLED, provider_uuid).await,
        None => {}
    }

    Ok(Json(OidcProviderResponse {
        id: row.try_get("id").unwrap_or_default(),
        issuer: row.try_get("issuer").unwrap_or_default(),
        client_id: row.try_get("client_id").unwrap_or_default(),
        enabled: row.try_get("enabled").unwrap_or(true),
        created_at: row
            .try_get("created_at")
            .unwrap_or_else(|_| chrono::Utc::now()),
    }))
}

/// PUT /admin/v1/oidc_providers/:provider_id/secret
///
/// Owner-only. Replaces the encrypted client secret (ADR-086 decision 7). 204; 404 if the provider
/// is not in the caller's account. The secret is never returned or logged.
pub async fn put_oidc_provider_secret(
    Path(provider_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
    Json(body): Json<PutSecretBody>,
) -> Result<StatusCode, Reject> {
    if session.role != Role::Owner {
        return Err(StatusCode::FORBIDDEN.into());
    }
    let provider_uuid = Uuid::parse_str(&provider_id).map_err(|_| StatusCode::NOT_FOUND)?;
    check_credentials(None, &body.client_secret)?;

    let result = sqlx::query("UPDATE oidc_providers SET client_secret_enc = $3 WHERE id = $1 AND account_id = $2")
        .bind(provider_uuid)
        .bind(session.account_id)
        .bind(state.server_keys.encrypt(&body.client_secret))
        .execute(state.system_db.pool())
        .await
        .map_err(|e| {
            tracing::error!("put_oidc_provider_secret: UPDATE error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    if result.rows_affected() == 0 {
        return Err(StatusCode::NOT_FOUND.into());
    }
    audit(&state, &session, events::SSO_PROVIDER_SECRET_ROTATED, provider_uuid).await;
    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /admin/v1/oidc_providers/:provider_id
///
/// Owner-only. Returns 204 on success. 404 if provider not found or belongs to a
/// different account. Sessions minted through the provider are deleted by the FK cascade.
pub async fn delete_oidc_provider(
    Path(provider_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
) -> Result<StatusCode, StatusCode> {
    if session.role != Role::Owner {
        return Err(StatusCode::FORBIDDEN);
    }

    let provider_uuid = Uuid::parse_str(&provider_id).map_err(|_| StatusCode::NOT_FOUND)?;

    let pool = state.system_db.pool();

    let result = sqlx::query(
        "DELETE FROM oidc_providers WHERE id = $1 AND account_id = $2",
    )
    .bind(provider_uuid)
    .bind(session.account_id)
    .execute(pool)
    .await
    .map_err(|e| {
        tracing::error!("delete_oidc_provider: DELETE error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    if result.rows_affected() == 0 {
        return Err(StatusCode::NOT_FOUND);
    }

    audit(&state, &session, events::SSO_PROVIDER_DELETED, provider_uuid).await;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_checks() {
        assert!(check_credentials(Some("id"), "secret").is_ok());
        assert!(check_credentials(None, "secret").is_ok());
        for (id, secret) in [(Some(""), "s"), (Some("  "), "s"), (Some("i"), ""), (Some("i"), "   "), (None, "")] {
            let e = check_credentials(id, secret).err().expect("rejected");
            assert_eq!(e.0, StatusCode::UNPROCESSABLE_ENTITY);
        }
        let long = "x".repeat(MAX_CREDENTIAL_BYTES + 1);
        assert!(check_credentials(Some(&long), "s").is_err());
        assert!(check_credentials(Some("i"), &long).is_err());
        assert!(check_credentials(Some(&"x".repeat(MAX_CREDENTIAL_BYTES)), &"y".repeat(MAX_CREDENTIAL_BYTES)).is_ok());
    }
}
