//! Service account handlers.
//!
//! list_service_accounts (GET /admin/v1/service_accounts):
//!   Session auth, any role.
//!
//! create_service_account (POST /admin/v1/service_accounts):
//!   Session auth, Owner or Admin.
//!   Body: { name, description?, role }. 201 { id, name, description, role, created_at }.
//!
//! delete_service_account (DELETE /admin/v1/service_accounts/:sa_id):
//!   Session auth, Owner or Admin. 204.
//!   Cascade: revoke all admin_api_keys linked to this service account (same transaction).

use axum::{
    extract::{Json, Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use embyr_core::admin::account::Role;

use crate::admin::extractors::session_context::SessionContext;
use crate::admin::state::UserAdminState;

// ── Request / response types ──────────────────────────────────────────────────

/// Per-element shape of GET /admin/v1/service_accounts response.
#[derive(Serialize)]
pub struct ServiceAccountSummary {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub role: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Body for POST /admin/v1/service_accounts.
#[derive(Deserialize)]
pub struct CreateServiceAccountBody {
    pub name: String,
    pub description: Option<String>,
    pub role: String,
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

/// GET /admin/v1/service_accounts
///
/// Session auth, any role. Returns all service accounts for the authenticated account,
/// ordered by creation time ascending.
pub async fn list_service_accounts(
    State(state): State<UserAdminState>,
    session: SessionContext,
) -> Result<Json<Vec<ServiceAccountSummary>>, StatusCode> {
    let pool = state.system_db.pool();

    let rows = sqlx::query(
        "SELECT id::text AS id, name, description, role, created_at \
         FROM service_accounts \
         WHERE account_id = $1 \
         ORDER BY created_at ASC",
    )
    .bind(session.account_id)
    .fetch_all(pool)
    .await
    .map_err(|e| {
        tracing::error!("list_service_accounts: DB error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let accounts: Vec<ServiceAccountSummary> = rows
        .into_iter()
        .map(|row| ServiceAccountSummary {
            id: row.try_get("id").unwrap_or_default(),
            name: row.try_get("name").unwrap_or_default(),
            description: row.try_get("description").unwrap_or(None),
            role: row.try_get("role").unwrap_or_default(),
            created_at: row
                .try_get("created_at")
                .unwrap_or_else(|_| chrono::Utc::now()),
        })
        .collect();

    Ok(Json(accounts))
}

/// POST /admin/v1/service_accounts
///
/// Owner or Admin only (Viewer → 403).
/// Validates `role` field (Owner/Admin/Viewer; unknown value → 422).
/// Returns 201 + ServiceAccountSummary on success.
pub async fn create_service_account(
    State(state): State<UserAdminState>,
    session: SessionContext,
    Json(body): Json<CreateServiceAccountBody>,
) -> Result<(StatusCode, Json<ServiceAccountSummary>), StatusCode> {
    if session.role < Role::Admin {
        return Err(StatusCode::FORBIDDEN);
    }

    if parse_role(&body.role).is_none() {
        return Err(StatusCode::UNPROCESSABLE_ENTITY);
    }

    let pool = state.system_db.pool();

    let row = sqlx::query(
        "INSERT INTO service_accounts (account_id, name, description, role) \
         VALUES ($1, $2, $3, $4) \
         RETURNING id::text AS id, name, description, role, created_at",
    )
    .bind(session.account_id)
    .bind(&body.name)
    .bind(&body.description)
    .bind(&body.role)
    .fetch_one(pool)
    .await
    .map_err(|e| {
        tracing::error!("create_service_account: INSERT error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let summary = ServiceAccountSummary {
        id: row
            .try_get("id")
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        name: row
            .try_get("name")
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        description: row.try_get("description").unwrap_or(None),
        role: row
            .try_get("role")
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
        created_at: row
            .try_get("created_at")
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?,
    };

    Ok((StatusCode::CREATED, Json(summary)))
}

/// DELETE /admin/v1/service_accounts/:sa_id
///
/// Owner or Admin only (Viewer → 403).
/// Invalid UUID → 404. SA not found or not owned by this account → 404.
/// Runs a transaction:
///   1. Revoke all un-revoked admin_api_keys for this service account.
///   2. Delete the service account row.
/// Returns 204 on success.
pub async fn delete_service_account(
    Path(sa_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
) -> Result<StatusCode, StatusCode> {
    if session.role < Role::Admin {
        return Err(StatusCode::FORBIDDEN);
    }

    let sa_uuid = Uuid::parse_str(&sa_id).map_err(|_| StatusCode::NOT_FOUND)?;

    let pool = state.system_db.pool();

    // Verify SA exists and belongs to this account.
    let row = sqlx::query("SELECT account_id FROM service_accounts WHERE id = $1")
        .bind(sa_uuid)
        .fetch_optional(pool)
        .await
        .map_err(|e| {
            tracing::error!("delete_service_account: ownership check error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    let owner_account: Uuid = row
        .try_get("account_id")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if owner_account != session.account_id {
        return Err(StatusCode::NOT_FOUND);
    }

    let mut tx = pool.begin().await.map_err(|e| {
        tracing::error!("delete_service_account: begin transaction: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Cascade: revoke all un-revoked admin_api_keys for this service account.
    sqlx::query(
        "UPDATE admin_api_keys \
         SET revoked_at = now() \
         WHERE service_account_id = $1 AND revoked_at IS NULL",
    )
    .bind(sa_uuid)
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        tracing::error!("delete_service_account: revoke keys error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Delete the service account.
    sqlx::query("DELETE FROM service_accounts WHERE id = $1")
        .bind(sa_uuid)
        .execute(&mut *tx)
        .await
        .map_err(|e| {
            tracing::error!("delete_service_account: DELETE error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    tx.commit().await.map_err(|e| {
        tracing::error!("delete_service_account: commit error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(StatusCode::NO_CONTENT)
}
