//! Admin key handlers.
//!
//! list_admin_keys (GET /admin/v1/admin_keys):
//!   Session auth, any role. Includes revoked keys (audit). Plaintext key never returned.
//!
//! create_admin_key (POST /admin/v1/admin_keys):
//!   Session auth, Owner or Admin.
//!   Body: { name, member_id? | service_account_id?, role }.
//!   Role cap: Admins cannot create Owner keys (check_rbac RoleEscalation → 403).
//!   201 { id, name, key: "embyr_adm_<32chars>", prefix, created_at }. BLAKE3(key) stored.
//!   key shown once only; plaintext never in DB.
//!
//! revoke_admin_key (DELETE /admin/v1/admin_keys/:key_id):
//!   Session auth, Owner or Admin. Sets revoked_at = now(). 204. Immediate effect via middleware.

use axum::{
    extract::{Json, Path, State},
    http::StatusCode,
};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use embyr_core::admin::account::{Role, UserId};
use embyr_core::admin::rbac::{check_rbac, RbacAction};

use crate::admin::extractors::session_context::SessionContext;
use crate::admin::state::UserAdminState;

// ── Request / response types ──────────────────────────────────────────────────

/// Per-element shape of GET /admin/v1/admin_keys response.
///
/// The `key` plaintext field is intentionally absent — it is shown only once at creation.
#[derive(Serialize)]
pub struct AdminKeySummary {
    pub id: String,
    pub name: String,
    pub role: String,
    pub prefix: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub last_used_at: Option<chrono::DateTime<chrono::Utc>>,
    pub revoked_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Response body for POST /admin/v1/admin_keys — `key` shown ONCE only.
#[derive(Serialize)]
pub struct CreateAdminKeyResponse {
    pub id: String,
    pub name: String,
    /// Full plaintext key: "embyr_adm_" + 32-char base64url-no-pad suffix.
    /// Shown once at creation; only BLAKE3 hash is persisted.
    pub key: String,
    pub prefix: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Body for POST /admin/v1/admin_keys.
#[derive(Deserialize)]
pub struct CreateAdminKeyBody {
    pub name: String,
    /// "Owner" | "Admin" | "Viewer" — unknown value → 422.
    pub role: String,
    /// UUID string; links key to a member. Optional.
    pub member_id: Option<String>,
    /// UUID string; links key to a service account. Optional.
    pub service_account_id: Option<String>,
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn parse_role(s: &str) -> Option<Role> {
    match s {
        "Owner" => Some(Role::Owner),
        "Admin" => Some(Role::Admin),
        "Viewer" => Some(Role::Viewer),
        _ => None,
    }
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// GET /admin/v1/admin_keys
///
/// Session auth, any role. Returns all admin API keys for the authenticated account
/// (including revoked keys for audit purposes). The `key` plaintext field is never returned.
pub async fn list_admin_keys(
    State(state): State<UserAdminState>,
    session: SessionContext,
) -> Result<Json<Vec<AdminKeySummary>>, StatusCode> {
    let pool = state.system_db.pool();

    let rows = sqlx::query(
        "SELECT id::text AS id, name, role, prefix, created_at, last_used_at, revoked_at \
         FROM admin_api_keys \
         WHERE account_id = $1 \
         ORDER BY created_at ASC",
    )
    .bind(session.account_id)
    .fetch_all(pool)
    .await
    .map_err(|e| {
        tracing::error!("list_admin_keys: DB error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let keys: Vec<AdminKeySummary> = rows
        .into_iter()
        .map(|row| AdminKeySummary {
            id: row.try_get("id").unwrap_or_default(),
            name: row.try_get("name").unwrap_or_default(),
            role: row.try_get("role").unwrap_or_default(),
            prefix: row.try_get("prefix").unwrap_or_default(),
            created_at: row
                .try_get("created_at")
                .unwrap_or_else(|_| chrono::Utc::now()),
            last_used_at: row.try_get("last_used_at").unwrap_or(None),
            revoked_at: row.try_get("revoked_at").unwrap_or(None),
        })
        .collect();

    Ok(Json(keys))
}

/// POST /admin/v1/admin_keys
///
/// Owner or Admin only (Viewer → 403).
/// Role cap: Admin cannot create an Owner-level key (check_rbac RoleEscalation → 403).
/// Generates "embyr_adm_" + 32-char base64url-no-pad suffix from 24 random bytes.
/// prefix = first 8 chars of the 32-char suffix. BLAKE3(key) stored as key_hash BYTEA.
/// Returns 201 + CreateAdminKeyResponse (with plaintext key shown once only).
pub async fn create_admin_key(
    State(state): State<UserAdminState>,
    session: SessionContext,
    Json(body): Json<CreateAdminKeyBody>,
) -> Result<(StatusCode, Json<CreateAdminKeyResponse>), StatusCode> {
    if session.role < Role::Admin {
        return Err(StatusCode::FORBIDDEN);
    }

    let requested_role = parse_role(&body.role).ok_or(StatusCode::UNPROCESSABLE_ENTITY)?;

    // Role cap: Admin cannot create Owner-level keys.
    check_rbac(
        RbacAction::CreateAdminKey { requested_role },
        session.role,
        &UserId(session.user_id),
        &[],
    )
    .map_err(|err| {
        tracing::debug!("create_admin_key: RBAC rejected: {err}");
        StatusCode::FORBIDDEN
    })?;

    // Generate key: 24 random bytes → URL_SAFE_NO_PAD → 32-char suffix.
    let mut raw_bytes = [0u8; 24];
    OsRng.fill_bytes(&mut raw_bytes);
    let suffix = URL_SAFE_NO_PAD.encode(raw_bytes);
    debug_assert_eq!(suffix.len(), 32, "base64url-no-pad of 24 bytes must be 32 chars");

    let key = format!("embyr_adm_{suffix}");
    let prefix = suffix[..8].to_string();

    // BLAKE3 hash stored; plaintext key never persisted.
    let key_hash: Vec<u8> = blake3::hash(key.as_bytes()).as_bytes().to_vec();

    // Parse optional UUID fields.
    let member_uuid: Option<Uuid> = body
        .member_id
        .as_deref()
        .map(Uuid::parse_str)
        .transpose()
        .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;

    let sa_uuid: Option<Uuid> = body
        .service_account_id
        .as_deref()
        .map(Uuid::parse_str)
        .transpose()
        .map_err(|_| StatusCode::UNPROCESSABLE_ENTITY)?;

    let pool = state.system_db.pool();

    let row = sqlx::query(
        "INSERT INTO admin_api_keys \
         (account_id, name, key_hash, prefix, role, member_id, service_account_id) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) \
         RETURNING id::text AS id, created_at",
    )
    .bind(session.account_id)
    .bind(&body.name)
    .bind(&key_hash as &[u8])
    .bind(&prefix)
    .bind(&body.role)
    .bind(member_uuid)
    .bind(sa_uuid)
    .fetch_one(pool)
    .await
    .map_err(|e| {
        tracing::error!("create_admin_key: INSERT error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let id: String = row
        .try_get("id")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let created_at: chrono::DateTime<chrono::Utc> = row
        .try_get("created_at")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    Ok((
        StatusCode::CREATED,
        Json(CreateAdminKeyResponse {
            id,
            name: body.name,
            key,
            prefix,
            created_at,
        }),
    ))
}

/// DELETE /admin/v1/admin_keys/:key_id
///
/// Owner or Admin only (Viewer → 403).
/// Invalid UUID → 404. Key not found, already revoked, or from a different account → 404.
/// Sets revoked_at = now(). The session_auth_middleware's `revoked_at IS NULL` check makes
/// the revocation immediately effective with no grace period.
/// Returns 204 on success.
pub async fn revoke_admin_key(
    Path(key_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
) -> Result<StatusCode, StatusCode> {
    if session.role < Role::Admin {
        return Err(StatusCode::FORBIDDEN);
    }

    let key_uuid = Uuid::parse_str(&key_id).map_err(|_| StatusCode::NOT_FOUND)?;

    let pool = state.system_db.pool();

    let result = sqlx::query(
        "UPDATE admin_api_keys \
         SET revoked_at = now() \
         WHERE id = $1 AND account_id = $2 AND revoked_at IS NULL",
    )
    .bind(key_uuid)
    .bind(session.account_id)
    .execute(pool)
    .await
    .map_err(|e| {
        tracing::error!("revoke_admin_key: UPDATE error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    if result.rows_affected() == 0 {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(StatusCode::NO_CONTENT)
}
