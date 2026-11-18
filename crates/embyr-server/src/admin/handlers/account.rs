//! DELETE /admin/v1/account — Owner-only account deletion (account-deletion feature).
//! PATCH  /admin/v1/account — Owner-only account settings; today `{sso_required}` (admin-oidc-sso,
//! ADR-086 decision 6): see `patch_account`.
//!
//! Soft delete mirroring projects: `accounts.deleted_at` is set, projects are
//! soft-deleted, sessions/keys/invitations are cut off, and the purge sweeper
//! removes the data after the grace window. Flow: cookie session + Owner role →
//! per-IP rate limit → shared signin credential verification (lockout, Argon2id,
//! TOTP/recovery code) → typed account-name confirmation → Stripe cancel
//! (idempotent, BEFORE any local write) → one transaction → cache eviction.
//! A Stripe failure leaves everything untouched (502) and a retry converges;
//! cancelled-but-not-deleted is the safe, retryable half-state.

use axum::{
    extract::{ConnectInfo, Json, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use uuid::Uuid;

use embyr_core::admin::account::Role;
use embyr_core::admin::account_deletion::purge_after;
use embyr_core::admin::audit::{events, AuditEvent, AuditOutcome};
use embyr_core::admin::sso::policy::may_enable_sso_required;

use crate::adapters::admin_audit::record;

use super::auth::{extract_cookie, verify_credentials, UserSecrets};
use crate::admin::extractors::session_context::SessionContext;
use crate::admin::state::UserAdminState;
use crate::sweepers::soft_delete_purge_sweeper::account_delete_grace_days;

#[derive(Deserialize)]
pub struct DeleteAccountRequest {
    pub password: String,
    pub totp_code: Option<String>,
    pub recovery_code: Option<String>,
    pub confirm_account_name: String,
}

#[derive(Serialize)]
struct DeleteAccountResponse {
    deleted_at: chrono::DateTime<chrono::Utc>,
    purge_after: chrono::DateTime<chrono::Utc>,
    projects_deleted: usize,
    subscription: &'static str,
}

fn message(status: StatusCode, msg: &str) -> Response {
    (status, Json(serde_json::json!({ "message": msg }))).into_response()
}

fn internal(context: &str, e: impl std::fmt::Display) -> Response {
    tracing::error!("delete_account: {context}: {e}");
    StatusCode::INTERNAL_SERVER_ERROR.into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchAccountRequest {
    pub sso_required: bool,
}

/// PATCH /admin/v1/account `{sso_required}` — Owner only (cookie session: the guard needs the
/// caller's own links). Disabling is immediate and unconditional. Enabling is refused with 422
/// `{error:"sso_required_prerequisites"}` unless the account has an enabled provider AND the
/// caller already has a link to one (lock-out guard). Owners stay exempt from the effect (O3).
/// Enabling also deletes, in the same transaction, the password sessions of the account's non-Owner
/// members (SSO sessions and all Owner sessions stay); the audit reason carries the count.
/// Every change is audited (labels only); a no-op repeat writes nothing.
pub async fn patch_account(
    session: SessionContext,
    State(state): State<UserAdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<PatchAccountRequest>,
) -> Response {
    let pool = state.system_db.pool();
    if session.role != Role::Owner {
        return StatusCode::FORBIDDEN.into_response();
    }

    let mut ev = AuditEvent::new(events::SSO_REQUIRED_TOGGLED, AuditOutcome::Ok);
    ev.account_id = Some(session.account_id);
    ev.actor_user_id = Some(session.user_id).filter(|u| !u.is_nil());
    ev.source_ip = Some(state.signin_rate_limiter.source_ip(peer, &headers));

    // One transaction with the account row locked: the guard's facts cannot change underneath it
    // (provider handlers take the same lock) and the session cut-off lands with the flag.
    let mut tx = match pool.begin().await {
        Ok(t) => t,
        Err(e) => return internal("begin", e),
    };
    let current: Result<Option<bool>, sqlx::Error> =
        sqlx::query_scalar("SELECT sso_required FROM accounts WHERE id = $1 AND deleted_at IS NULL FOR UPDATE")
            .bind(session.account_id)
            .fetch_optional(&mut *tx)
            .await;
    let current = match current {
        Ok(Some(c)) => c,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => return internal("lock account", e),
    };
    // A repeat is a no-op that writes nothing (no audit row, no session cut-off).
    if current == body.sso_required {
        return Json(serde_json::json!({ "sso_required": body.sso_required })).into_response();
    }

    if body.sso_required {
        let facts: Result<(bool, bool), sqlx::Error> = sqlx::query_as(
            "SELECT EXISTS(SELECT 1 FROM oidc_providers p WHERE p.account_id = $1 AND p.enabled), \
                    EXISTS(SELECT 1 FROM admin_sso_identities i \
                           JOIN oidc_providers p ON p.id = i.provider_id \
                           WHERE i.user_id = $2 AND p.account_id = $1 AND p.enabled)",
        )
        .bind(session.account_id)
        .bind(session.user_id)
        .fetch_one(&mut *tx)
        .await;
        let (has_provider, caller_linked) = match facts {
            Ok(f) => f,
            Err(e) => return internal("sso_required guard", e),
        };
        if !may_enable_sso_required(has_provider, caller_linked) {
            drop(tx);
            ev.outcome = AuditOutcome::Failed;
            ev.reason = Some(if has_provider { "caller_not_linked" } else { "no_enabled_provider" });
            record(pool, &ev).await;
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({ "error": "sso_required_prerequisites" })),
            )
                .into_response();
        }
    }

    if let Err(e) = sqlx::query("UPDATE accounts SET sso_required = $2 WHERE id = $1")
        .bind(session.account_id)
        .bind(body.sso_required)
        .execute(&mut *tx)
        .await
    {
        return internal("update sso_required", e);
    }

    // Enabling ends the password sessions of non-Owner members (their password path just closed);
    // SSO sessions, every Owner session (the caller's included) and API keys stay.
    let mut ended = 0u64;
    if body.sso_required {
        match sqlx::query(
            "DELETE FROM sessions s USING account_members am \
             WHERE s.account_id = $1 AND s.auth_method = 'password' \
               AND am.account_id = s.account_id AND am.user_id = s.user_id AND am.role <> 'Owner'",
        )
        .bind(session.account_id)
        .execute(&mut *tx)
        .await
        {
            Ok(r) => ended = r.rows_affected(),
            Err(e) => return internal("end password sessions", e),
        }
    }
    if let Err(e) = tx.commit().await {
        return internal("commit", e);
    }

    // Label only: how many sessions the switch ended, no identifiers.
    let reason = if body.sso_required { format!("enabled:sessions_ended_{ended}") } else { "disabled".to_string() };
    ev.reason = Some(&reason);
    record(pool, &ev).await;
    Json(serde_json::json!({ "sso_required": body.sso_required })).into_response()
}

pub async fn delete_account(
    session: SessionContext,
    State(state): State<UserAdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<DeleteAccountRequest>,
) -> Response {
    let pool = state.system_db.pool();

    // Re-authentication needs a user: a Bearer admin key (no session cookie) is refused.
    if extract_cookie(&headers, "embyr_session").is_none() || session.role != Role::Owner {
        return StatusCode::FORBIDDEN.into_response();
    }

    // Same per-source-IP limiter as signin.
    if let Err(retry_after_ms) = state
        .signin_rate_limiter
        .check(&peer.ip().to_string())
        .await
    {
        let mut r = message(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many attempts — try again later",
        );
        if let Ok(v) = header::HeaderValue::from_str(&retry_after_ms.to_string()) {
            r.headers_mut().insert("retry-after-ms", v);
        }
        return r;
    }

    let user = match UserSecrets::fetch_by_id(pool, session.user_id).await {
        Ok(Some(u)) => u,
        Ok(None) => return StatusCode::UNAUTHORIZED.into_response(),
        Err(e) => return internal("fetch user", e),
    };
    if let Err(response) = verify_credentials(
        &state,
        &user,
        &body.password,
        body.totp_code.as_deref(),
        body.recovery_code.as_deref(),
    )
    .await
    {
        return response;
    }

    let account_name: String =
        match sqlx::query_scalar("SELECT name FROM accounts WHERE id = $1 AND deleted_at IS NULL")
            .bind(session.account_id)
            .fetch_optional(pool)
            .await
        {
            Ok(Some(n)) => n,
            Ok(None) => return StatusCode::UNAUTHORIZED.into_response(),
            Err(e) => return internal("fetch account name", e),
        };
    if body.confirm_account_name != account_name {
        return message(
            StatusCode::BAD_REQUEST,
            "Account name confirmation does not match",
        );
    }

    // Stripe first: failure here must leave nothing deleted.
    let stripe_subscription_id: Option<String> = match sqlx::query_scalar(
        "SELECT stripe_subscription_id FROM subscriptions \
         WHERE account_id = $1 AND stripe_subscription_id IS NOT NULL",
    )
    .bind(session.account_id)
    .fetch_optional(pool)
    .await
    {
        Ok(v) => v,
        Err(e) => return internal("fetch subscription", e),
    };
    let stripe_outcome = if let Some(sub_id) = &stripe_subscription_id {
        if let Err(e) = state.stripe_gateway.cancel_subscription(sub_id).await {
            tracing::error!(
                account_id = %session.account_id,
                "delete_account: stripe cancel failed: {e}"
            );
            return message(
                StatusCode::BAD_GATEWAY,
                "Could not cancel the subscription — nothing was deleted",
            );
        }
        "cancelled"
    } else {
        "none"
    };

    let (deleted_at, project_ids) = match soft_delete_account(&state, session.account_id).await {
        Ok(Some(v)) => v,
        // Lost a race with a concurrent delete: the session is already gone.
        Ok(None) => return StatusCode::UNAUTHORIZED.into_response(),
        Err(e) => return internal("transaction", e),
    };

    for project_id in &project_ids {
        state.credential_cache.evict_project(project_id).await;
    }

    tracing::info!(
        account_id = %session.account_id,
        actor_user_id = %session.user_id,
        projects_deleted = project_ids.len(),
        stripe = stripe_outcome,
        "account deleted (soft)"
    );

    (
        StatusCode::ACCEPTED,
        [(
            header::SET_COOKIE,
            "embyr_session=; Max-Age=0; Path=/admin".to_string(),
        )],
        Json(DeleteAccountResponse {
            deleted_at,
            purge_after: purge_after(deleted_at, account_delete_grace_days()),
            projects_deleted: project_ids.len(),
            subscription: stripe_outcome,
        }),
    )
        .into_response()
}

/// One transaction. `None` = the account was already deleted (race loser).
async fn soft_delete_account(
    state: &UserAdminState,
    account_id: Uuid,
) -> Result<Option<(chrono::DateTime<chrono::Utc>, Vec<String>)>, sqlx::Error> {
    let mut tx = state.system_db.pool().begin().await?;

    let Some(deleted_at): Option<chrono::DateTime<chrono::Utc>> = sqlx::query_scalar(
        "UPDATE accounts SET deleted_at = now() \
         WHERE id = $1 AND deleted_at IS NULL RETURNING deleted_at",
    )
    .bind(account_id)
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(None);
    };

    sqlx::query(
        "UPDATE sdk_api_keys SET revoked_at = now() \
         WHERE revoked_at IS NULL \
           AND project_id IN (SELECT id FROM projects WHERE account_id = $1)",
    )
    .bind(account_id)
    .execute(&mut *tx)
    .await?;

    let project_ids: Vec<String> = sqlx::query_scalar(
        "UPDATE projects SET status = 'deleted', deleted_at = now(), updated_at = now() \
         WHERE account_id = $1 AND status != 'deleted' RETURNING id",
    )
    .bind(account_id)
    .fetch_all(&mut *tx)
    .await?;

    for stmt in [
        "UPDATE sessions SET expires_at = now() WHERE account_id = $1 AND expires_at > now()",
        "UPDATE admin_api_keys SET revoked_at = now() WHERE account_id = $1 AND revoked_at IS NULL",
        "UPDATE invitations SET revoked_at = now() \
         WHERE account_id = $1 AND accepted_at IS NULL AND revoked_at IS NULL",
        "UPDATE subscriptions SET status = 'canceled', updated_at = now() \
         WHERE account_id = $1 AND stripe_subscription_id IS NOT NULL",
    ] {
        sqlx::query(stmt).bind(account_id).execute(&mut *tx).await?;
    }

    tx.commit().await?;
    Ok(Some((deleted_at, project_ids)))
}
