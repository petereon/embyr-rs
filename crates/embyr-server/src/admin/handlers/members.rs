//! Member handlers.
//!
//! list_members (GET /admin/v1/members):
//!   Session auth, any role. Includes pending invites (joined_at = null).
//!
//! invite_member (POST /admin/v1/members/invite):
//!   Session auth, Owner or Admin. Creates invitations row. Sends via IEmailSender.
//!   202 { invitation_id, email, role, expires_at }.
//!
//! change_member_role (PATCH /admin/v1/members/:member_id/role):
//!   Session auth, Owner or Admin. Check sole-owner invariant via check_rbac.
//!   Admin cannot change another Owner's role.
//!
//! remove_member (DELETE /admin/v1/members/:member_id):
//!   Session auth, Owner or Admin.
//!   Last Owner → 409 "Transfer ownership before removing the last Owner."
//!   Otherwise → 204. Cascade: invalidate all sessions + revoke admin_api_keys.

use axum::{
    extract::{Json, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use embyr_core::admin::account::{AccountId, AccountMember, Role, UserId};
use embyr_core::admin::email::EmailMessage;
use embyr_core::admin::rbac::{check_rbac, RbacAction, RbacError};

use crate::admin::extractors::session_context::SessionContext;
use crate::admin::state::UserAdminState;

// ── Request / response types ──────────────────────────────────────────────────

/// Per-element shape of GET /admin/v1/members response.
///
/// Active members: auth_method = "password", joined_at = non-null.
/// Pending invitations: auth_method = "pending", display_name = null, joined_at = null.
#[derive(Serialize)]
pub struct MemberResponse {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub role: String,
    pub auth_method: String,
    pub mfa_enabled: bool,
    pub last_login_at: Option<chrono::DateTime<chrono::Utc>>,
    pub joined_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Body for POST /admin/v1/members/invite.
#[derive(Deserialize)]
pub struct InviteMemberBody {
    pub email: String,
    pub role: String,
}

/// Response body for POST /admin/v1/members/invite (202).
#[derive(Serialize)]
pub struct InvitationResponse {
    pub invitation_id: String,
    pub email: String,
    pub role: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

/// Body for PATCH /admin/v1/members/:member_id/role.
#[derive(Deserialize)]
pub struct ChangeMemberRoleBody {
    pub role: String,
}

/// Minimal error body for 409 / 403 responses that carry a message.
#[derive(Serialize)]
struct ErrorBody {
    message: String,
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

/// Load all `account_members` rows for `account_id` and build the domain type
/// required by `check_rbac`.
async fn load_account_members(
    pool: &sqlx::PgPool,
    account_id: Uuid,
) -> Result<Vec<AccountMember>, StatusCode> {
    let rows = sqlx::query(
        "SELECT user_id, role FROM account_members WHERE account_id = $1",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await
    .map_err(|e| {
        tracing::error!("load_account_members DB error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let members = rows
        .into_iter()
        .filter_map(|row| {
            let user_id_uuid: Uuid = row.try_get("user_id").ok()?;
            let role_str: String = row.try_get("role").ok()?;
            let role = parse_role(&role_str)?;
            Some(AccountMember {
                // id is not used by check_rbac; Uuid::nil() is a safe placeholder.
                id: Uuid::nil(),
                user_id: UserId(user_id_uuid),
                account_id: AccountId(account_id),
                role,
                joined_at: None,
            })
        })
        .collect();

    Ok(members)
}

// ── Handlers ──────────────────────────────────────────────────────────────────

/// GET /admin/v1/members
///
/// Returns active members (users + account_members JOIN) followed by pending
/// invitations (accepted_at IS NULL AND expires_at > now()).
/// Session auth required; any role may list members.
pub async fn list_members(
    State(state): State<UserAdminState>,
    session: SessionContext,
) -> Result<Json<Vec<MemberResponse>>, StatusCode> {
    let pool = state.system_db.pool();

    // ── Active members ────────────────────────────────────────────────────────
    let member_rows = sqlx::query(
        "SELECT u.id::text AS id, u.email, u.display_name, \
         am.role, am.joined_at, \
         (u.totp_secret_enc IS NOT NULL) AS mfa_enabled \
         FROM account_members am \
         JOIN users u ON u.id = am.user_id \
         WHERE am.account_id = $1 \
         ORDER BY am.joined_at ASC NULLS LAST",
    )
    .bind(session.account_id)
    .fetch_all(pool)
    .await
    .map_err(|e| {
        tracing::error!("list_members: member query error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let mut members: Vec<MemberResponse> = member_rows
        .into_iter()
        .map(|row| MemberResponse {
            id: row.try_get("id").unwrap_or_default(),
            email: row.try_get("email").unwrap_or_default(),
            display_name: row.try_get("display_name").ok(),
            role: row.try_get("role").unwrap_or_default(),
            auth_method: "password".to_string(),
            mfa_enabled: row.try_get("mfa_enabled").unwrap_or(false),
            last_login_at: None,
            joined_at: row.try_get("joined_at").unwrap_or(None),
        })
        .collect();

    // ── Pending invitations ───────────────────────────────────────────────────
    let inv_rows = sqlx::query(
        "SELECT id::text AS id, email, role, expires_at \
         FROM invitations \
         WHERE account_id = $1 AND accepted_at IS NULL AND expires_at > now() \
         ORDER BY created_at ASC",
    )
    .bind(session.account_id)
    .fetch_all(pool)
    .await
    .map_err(|e| {
        tracing::error!("list_members: invitations query error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    for row in inv_rows {
        members.push(MemberResponse {
            id: row.try_get("id").unwrap_or_default(),
            email: row.try_get("email").unwrap_or_default(),
            display_name: None,
            role: row.try_get("role").unwrap_or_default(),
            auth_method: "pending".to_string(),
            mfa_enabled: false,
            last_login_at: None,
            joined_at: None,
        });
    }

    Ok(Json(members))
}

/// POST /admin/v1/members/invite
///
/// Owner or Admin only (Viewer → 403).
/// Inserts an invitations row with a 7-day expiry, then calls IEmailSender::send
/// fire-and-forget (V1 uses NoopEmailSender; errors are ignored).
/// Returns 202 + InvitationResponse.
pub async fn invite_member(
    State(state): State<UserAdminState>,
    session: SessionContext,
    Json(body): Json<InviteMemberBody>,
) -> Result<(StatusCode, Json<InvitationResponse>), StatusCode> {
    // Viewer cannot invite — Owner or Admin only.
    if session.role < Role::Admin {
        return Err(StatusCode::FORBIDDEN);
    }

    let pool = state.system_db.pool();

    // Insert invitation row (7-day expiry).
    let row = sqlx::query(
        "INSERT INTO invitations (account_id, email, role, expires_at) \
         VALUES ($1, $2, $3, now() + interval '7 days') \
         RETURNING id::text AS id, expires_at",
    )
    .bind(session.account_id)
    .bind(&body.email)
    .bind(&body.role)
    .fetch_one(pool)
    .await
    .map_err(|e| {
        tracing::error!("invite_member: INSERT error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let invitation_id: String = row
        .try_get("id")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let expires_at: chrono::DateTime<chrono::Utc> = row
        .try_get("expires_at")
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    // Fire-and-forget: V1 NoopEmailSender drops the message; errors are ignored.
    let _ = state
        .email_sender
        .send(EmailMessage {
            to: body.email.clone(),
            subject: format!("You've been invited to join as {}", body.role),
            body_text: format!(
                "You have been invited to join the account with role '{}'. \
                 This invitation expires at {}.",
                body.role, expires_at
            ),
            body_html: None,
        })
        .await;

    Ok((
        StatusCode::ACCEPTED,
        Json(InvitationResponse {
            invitation_id,
            email: body.email,
            role: body.role,
            expires_at,
        }),
    ))
}

/// PATCH /admin/v1/members/:member_id/role
///
/// Owner or Admin only (Viewer → 403).
/// Enforces sole-owner invariant and Admin-cannot-modify-Owner via check_rbac.
/// Sole Owner attempting self-demotion → 403. Admin attempting to change Owner → 403.
/// Returns 200 { "ok": true } on success.
pub async fn change_member_role(
    Path(member_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
    Json(body): Json<ChangeMemberRoleBody>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    // Viewer cannot change roles.
    if session.role < Role::Admin {
        return Err(StatusCode::FORBIDDEN);
    }

    // Parse target user UUID (invalid UUID → 404).
    let target_user_id = Uuid::parse_str(&member_id).map_err(|_| StatusCode::NOT_FOUND)?;

    // Parse requested new role (unknown string → 422).
    let new_role = parse_role(&body.role).ok_or(StatusCode::UNPROCESSABLE_ENTITY)?;

    let pool = state.system_db.pool();

    // Load all members for RBAC check.
    let all_members = load_account_members(pool, session.account_id).await?;

    // RBAC check — all failure variants map to 403 for this endpoint.
    check_rbac(
        RbacAction::ChangeMemberRole {
            target_id: UserId(target_user_id),
            new_role,
        },
        session.role,
        &UserId(session.user_id),
        &all_members,
    )
    .map_err(|err| {
        tracing::debug!("change_member_role: RBAC rejected: {err}");
        StatusCode::FORBIDDEN
    })?;

    // Apply the role change.
    let result = sqlx::query(
        "UPDATE account_members SET role = $1 WHERE user_id = $2 AND account_id = $3",
    )
    .bind(&body.role)
    .bind(target_user_id)
    .bind(session.account_id)
    .execute(pool)
    .await
    .map_err(|e| {
        tracing::error!("change_member_role: UPDATE error: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    if result.rows_affected() == 0 {
        return Err(StatusCode::NOT_FOUND);
    }

    Ok(Json(serde_json::json!({"ok": true})))
}

/// DELETE /admin/v1/members/:member_id
///
/// Owner or Admin only (Viewer → 403).
/// Last Owner → 409 with `{ "message": "Transfer ownership before removing the last Owner." }`.
/// Admin attempting to remove Owner → 403.
/// On success → 204. Cascade (in one transaction):
///   - All sessions for the member are expired (expires_at = now()).
///   - All admin_api_keys for the member have revoked_at set.
///   - The account_members row is deleted.
pub async fn remove_member(
    Path(member_id): Path<String>,
    State(state): State<UserAdminState>,
    session: SessionContext,
) -> Response {
    // Viewer cannot remove members.
    if session.role < Role::Admin {
        return StatusCode::FORBIDDEN.into_response();
    }

    // Parse target user UUID (invalid UUID → 404).
    let target_user_id = match Uuid::parse_str(&member_id) {
        Ok(u) => u,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };

    let pool = state.system_db.pool();

    // Load all members for RBAC check.
    let all_members = match load_account_members(pool, session.account_id).await {
        Ok(m) => m,
        Err(status) => return status.into_response(),
    };

    // RBAC check — LastOwner → 409, all other errors → 403.
    match check_rbac(
        RbacAction::RemoveMember {
            target_id: UserId(target_user_id),
        },
        session.role,
        &UserId(session.user_id),
        &all_members,
    ) {
        Ok(()) => {}
        Err(RbacError::LastOwner) => {
            return (
                StatusCode::CONFLICT,
                Json(ErrorBody {
                    message: "Transfer ownership before removing the last Owner.".to_string(),
                }),
            )
                .into_response();
        }
        Err(err) => {
            tracing::debug!("remove_member: RBAC rejected: {err}");
            return StatusCode::FORBIDDEN.into_response();
        }
    }

    // Begin transaction: cascade session expiry + key revocation + member deletion.
    let mut tx = match pool.begin().await {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("remove_member: begin transaction: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    // Expire all active sessions for this user in this account.
    if let Err(e) = sqlx::query(
        "UPDATE sessions SET expires_at = now() WHERE user_id = $1 AND account_id = $2",
    )
    .bind(target_user_id)
    .bind(session.account_id)
    .execute(&mut *tx)
    .await
    {
        tracing::error!("remove_member: expire sessions: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // Revoke all un-revoked admin_api_keys linked to this member in this account.
    if let Err(e) = sqlx::query(
        "UPDATE admin_api_keys SET revoked_at = now() \
         WHERE member_id = $1 AND account_id = $2 AND revoked_at IS NULL",
    )
    .bind(target_user_id)
    .bind(session.account_id)
    .execute(&mut *tx)
    .await
    {
        tracing::error!("remove_member: revoke admin keys: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    // Delete the account_members row.
    if let Err(e) = sqlx::query(
        "DELETE FROM account_members WHERE user_id = $1 AND account_id = $2",
    )
    .bind(target_user_id)
    .bind(session.account_id)
    .execute(&mut *tx)
    .await
    {
        tracing::error!("remove_member: delete member: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    if let Err(e) = tx.commit().await {
        tracing::error!("remove_member: commit: {e}");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    }

    StatusCode::NO_CONTENT.into_response()
}
